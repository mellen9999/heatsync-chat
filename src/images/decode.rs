// webp (animated), gif (animated), png → straight-alpha rgba frames scaled to a target height.
// decoding at display size is the whole memory story: a 28px frame is 3KB, a 128px one 64KB
use std::io::Cursor;

pub struct Frame {
  pub rgba: Box<[u8]>,
  pub delay_ms: u32,
}

pub struct Decoded {
  pub w: u32,
  pub h: u32,
  pub frames: Vec<Frame>,
}

impl Decoded {
  pub fn bytes(&self) -> usize {
    self.frames.iter().map(|f| f.rgba.len()).sum()
  }
}

// hostile-input bounds: no emote needs more than this
const MAX_SIDE: u32 = 4096;
const MAX_FRAMES: usize = 512;
const MAX_CANVAS_PX: u64 = 1024 * 1024;
// source frames held at once while decoding (full size, before scaling down)
const MAX_SRC_BYTES: usize = 48 << 20;
// decoded frames of one chat picture
const EMBED_BYTES: usize = 6 << 20;

// browsers treat ≤10ms as "unset" and play it at 100ms; match them
fn delay(ms: u32) -> u32 {
  if ms <= 10 { 100 } else { ms }
}

// `target` = height in the low 16 bits. a nonzero high half is a chat picture: fit inside that width
// and the height, never enlarge, and keep its frames under EMBED_BYTES
pub fn decode(bytes: &[u8], target: u32) -> Result<Decoded, String> {
  let (target_h, max_w) = ((target & 0xffff).clamp(1, 512), target >> 16);
  let (w, h, frames) = if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
    webp(bytes)?
  } else if bytes.starts_with(b"GIF8") {
    gif(bytes)?
  } else if bytes.starts_with(b"\x89PNG") {
    png(bytes)?
  } else {
    return Err("unknown image format".into());
  };
  if frames.is_empty() {
    return Err("no frames".into());
  }
  let (tw, th) = if max_w == 0 {
    (((w as u64 * target_h as u64).div_ceil(h as u64) as u32).clamp(1, MAX_SIDE), target_h)
  } else {
    let k = 1f64.min(max_w as f64 / w as f64).min(target_h as f64 / h as f64);
    (((w as f64 * k).round() as u32).clamp(1, MAX_SIDE), ((h as f64 * k).round() as u32).max(1))
  };
  let keep = if max_w == 0 { usize::MAX } else { (EMBED_BYTES / (tw as usize * th as usize * 4)).max(1) };
  let frames = frames
    .into_iter()
    .take(keep)
    .map(|f| Frame {
      rgba: if (tw, th) == (w, h) { f.rgba } else { resize(&f.rgba, w, h, tw, th) },
      delay_ms: f.delay_ms,
    })
    .collect();
  Ok(Decoded { w: tw, h: th, frames })
}

fn max_frames(w: u32, h: u32) -> usize {
  (MAX_SRC_BYTES / (w as usize * h as usize * 4).max(1)).clamp(1, MAX_FRAMES)
}

fn check_dims(w: u32, h: u32) -> Result<(), String> {
  if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE || w as u64 * h as u64 > MAX_CANVAS_PX {
    return Err(format!("bad dimensions {w}x{h}"));
  }
  Ok(())
}

fn webp(bytes: &[u8]) -> Result<(u32, u32, Vec<Frame>), String> {
  let mut d = image_webp::WebPDecoder::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
  let (w, h) = d.dimensions();
  check_dims(w, h)?;
  let alpha = d.has_alpha();
  let len = d.output_buffer_size().ok_or("too large")?;
  let mut buf = vec![0u8; len];
  let to_rgba = |buf: &[u8]| -> Box<[u8]> {
    if alpha { buf.into() } else { buf.as_chunks::<3>().0.iter().flat_map(|p| [p[0], p[1], p[2], 255]).collect() }
  };
  let mut frames = Vec::new();
  if d.is_animated() {
    while frames.len() < max_frames(w, h) {
      match d.read_frame(&mut buf) {
        Ok(ms) => frames.push(Frame { rgba: to_rgba(&buf), delay_ms: delay(ms) }),
        Err(image_webp::DecodingError::NoMoreFrames) => break,
        Err(e) if frames.is_empty() => return Err(e.to_string()),
        Err(_) => break,
      }
    }
  } else {
    d.read_image(&mut buf).map_err(|e| e.to_string())?;
    frames.push(Frame { rgba: to_rgba(&buf), delay_ms: 0 });
  }
  Ok((w, h, frames))
}

fn gif(bytes: &[u8]) -> Result<(u32, u32, Vec<Frame>), String> {
  let mut opts = gif::DecodeOptions::new();
  opts.set_color_output(gif::ColorOutput::RGBA);
  let mut d = opts.read_info(Cursor::new(bytes)).map_err(|e| e.to_string())?;
  let (w, h) = (d.width() as u32, d.height() as u32);
  check_dims(w, h)?;
  let mut canvas = vec![0u8; (w * h * 4) as usize];
  let mut frames = Vec::new();
  while frames.len() < max_frames(w, h) {
    let f = match d.read_next_frame() {
      Ok(Some(f)) => f,
      Ok(None) => break,
      Err(e) if frames.is_empty() => return Err(e.to_string()),
      Err(_) => break,
    };
    let saved = (f.dispose == gif::DisposalMethod::Previous).then(|| canvas.clone());
    let (fx, fy, fw, fh) = (f.left as u32, f.top as u32, f.width as u32, f.height as u32);
    for y in 0..fh {
      for x in 0..fw {
        let (cx, cy) = (fx + x, fy + y);
        if cx >= w || cy >= h {
          continue;
        }
        let s = ((y * fw + x) * 4) as usize;
        let Some(px) = f.buffer.get(s..s + 4) else { continue };
        if px[3] != 0 {
          let c = ((cy * w + cx) * 4) as usize;
          canvas[c..c + 4].copy_from_slice(px);
        }
      }
    }
    frames.push(Frame { rgba: canvas.clone().into(), delay_ms: delay(f.delay as u32 * 10) });
    match f.dispose {
      gif::DisposalMethod::Background => {
        for y in fy..(fy + fh).min(h) {
          let a = ((y * w + fx) * 4) as usize;
          let b = ((y * w + (fx + fw).min(w)) * 4) as usize;
          canvas[a..b].fill(0);
        }
      }
      gif::DisposalMethod::Previous => {
        if let Some(s) = saved {
          canvas = s;
        }
      }
      _ => {}
    }
  }
  if frames.len() == 1 {
    frames[0].delay_ms = 0;
  }
  Ok((w, h, frames))
}

fn png(bytes: &[u8]) -> Result<(u32, u32, Vec<Frame>), String> {
  let mut d = png::Decoder::new(Cursor::new(bytes));
  d.set_transformations(png::Transformations::normalize_to_color8());
  let mut r = d.read_info().map_err(|e| e.to_string())?;
  let (w, h) = (r.info().width, r.info().height);
  check_dims(w, h)?;
  let mut buf = vec![0u8; r.output_buffer_size().ok_or("too large")?];
  let info = r.next_frame(&mut buf).map_err(|e| e.to_string())?;
  let px = &buf[..info.buffer_size()];
  let rgba: Box<[u8]> = match info.color_type {
    png::ColorType::Rgba => px.into(),
    png::ColorType::Rgb => px.as_chunks::<3>().0.iter().flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
    png::ColorType::GrayscaleAlpha => px.as_chunks::<2>().0.iter().flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
    png::ColorType::Grayscale => px.iter().flat_map(|&g| [g, g, g, 255]).collect(),
    png::ColorType::Indexed => return Err("unexpanded palette".into()),
  };
  if rgba.len() != (w * h * 4) as usize {
    return Err("short png".into());
  }
  Ok((w, h, vec![Frame { rgba, delay_ms: 0 }]))
}

// area-average resample in premultiplied space (no dark fringes on transparent edges)
pub fn resize(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Box<[u8]> {
  let mut out = vec![0u8; (dw * dh * 4) as usize];
  let (fx, fy) = (sw as f32 / dw as f32, sh as f32 / dh as f32);
  for dy in 0..dh {
    let y0 = dy as f32 * fy;
    let y1 = (y0 + fy).min(sh as f32);
    for dx in 0..dw {
      let x0 = dx as f32 * fx;
      let x1 = (x0 + fx).min(sw as f32);
      let mut acc = [0f32; 4];
      let mut area = 0f32;
      let mut sy = y0.floor() as u32;
      while (sy as f32) < y1 && sy < sh {
        let wy = (y1.min(sy as f32 + 1.0) - y0.max(sy as f32)).max(0.0);
        let mut sx = x0.floor() as u32;
        while (sx as f32) < x1 && sx < sw {
          let wx = (x1.min(sx as f32 + 1.0) - x0.max(sx as f32)).max(0.0);
          let wgt = wx * wy;
          let i = ((sy * sw + sx) * 4) as usize;
          let a = src[i + 3] as f32;
          acc[0] += src[i] as f32 * a * wgt;
          acc[1] += src[i + 1] as f32 * a * wgt;
          acc[2] += src[i + 2] as f32 * a * wgt;
          acc[3] += a * wgt;
          area += wgt;
          sx += 1;
        }
        sy += 1;
      }
      let o = ((dy * dw + dx) * 4) as usize;
      if acc[3] > 0.0 {
        out[o] = (acc[0] / acc[3]).round() as u8;
        out[o + 1] = (acc[1] / acc[3]).round() as u8;
        out[o + 2] = (acc[2] / acc[3]).round() as u8;
        out[o + 3] = (acc[3] / area.max(1e-6)).round().min(255.0) as u8;
      }
    }
  }
  out.into()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn rejects_garbage() {
    assert!(decode(b"", 28).is_err());
    assert!(decode(b"RIFF\0\0\0\0WEBPjunk", 28).is_err());
    assert!(decode(b"GIF89a", 28).is_err());
    assert!(decode(b"\x89PNG\r\n\x1a\n", 28).is_err());
  }

  #[test]
  fn resize_keeps_flat_color_and_alpha() {
    let src: Vec<u8> = [255, 135, 0, 255].repeat(4 * 4);
    let out = resize(&src, 4, 4, 2, 2);
    assert_eq!(&out[..4], &[255, 135, 0, 255]);
    let clear = vec![0u8; 4 * 4 * 4];
    assert!(resize(&clear, 4, 4, 2, 2).iter().all(|&b| b == 0));
  }

  #[test]
  fn decodes_png() {
    let mut bytes = Vec::new();
    {
      let mut e = png::Encoder::new(&mut bytes, 2, 2);
      e.set_color(png::ColorType::Rgba);
      e.set_depth(png::BitDepth::Eight);
      let mut w = e.write_header().unwrap();
      w.write_image_data(&[255, 0, 0, 255].repeat(4)).unwrap();
    }
    let d = decode(&bytes, 4).unwrap();
    assert_eq!((d.w, d.h, d.frames.len()), (4, 4, 1));
    assert_eq!(&d.frames[0].rgba[..4], &[255, 0, 0, 255]);
  }
}
