// a picture leaves the machine without what the camera or editor wrote into it: gps, camera and
// phone model, timestamps, names, thumbnails, comments, xmp. every format is rebuilt from the
// segments a decoder needs (an allowlist, so a tag nobody has heard of is dropped too), and the
// pixels are never touched, so the picture stays exactly the same picture.
// a file that doesn't parse is refused, not sent: better "damaged" than a leak
//
// jpeg: image data, tables, the color profile and adobe's color flag stay. the exif block goes, except
//   its rotation, which is rewritten as a block of its own (a phone photo would lie on its side without it)
// png: everything a decoder needs; text, exif, time and every unknown chunk go
// webp: image, alpha, animation and color profile; the exif and xmp chunks go and their flags clear
// gif: frames, timing, transparency and the loop count; comments, text and application blocks go

const DAMAGED: &str = "that picture looks damaged, so it wasn't sent";

// jpeg, png, gif, webp are cleaned; any other mime (avif, bmp, tiff, video) passes through untouched
pub fn clean(mime: &str, bytes: Vec<u8>) -> Result<Vec<u8>, String> {
  let out = match mime {
    "image/jpeg" => jpeg(&bytes),
    "image/png" => png(&bytes),
    "image/gif" => gif(&bytes),
    "image/webp" => webp(&bytes),
    _ => return Ok(bytes),
  };
  out.ok_or_else(|| DAMAGED.to_string())
}

// ---- jpeg

fn jpeg(b: &[u8]) -> Option<Vec<u8>> {
  if !b.starts_with(&[0xff, 0xd8]) {
    return None;
  }
  let mut out = Vec::with_capacity(b.len());
  out.extend_from_slice(&[0xff, 0xd8]);
  let mut i = 2;
  let mut seen_exif = false;
  loop {
    // a marker: 0xff, any fill 0xff, its code
    if *b.get(i)? != 0xff {
      return None;
    }
    while *b.get(i)? == 0xff {
      i += 1;
    }
    let m = *b.get(i)?;
    i += 1;
    match m {
      0xd9 => {
        out.extend_from_slice(&[0xff, 0xd9]);
        return Some(out);
      }
      // not valid between segments
      0x00 | 0x02..=0xbf | 0xd8 => return None,
      // no length: restart markers and TEM
      0x01 | 0xd0..=0xd7 => {
        out.extend_from_slice(&[0xff, m]);
        continue;
      }
      _ => {}
    }
    let len = usize::from(u16::from_be_bytes([*b.get(i)?, *b.get(i + 1)?]));
    if len < 2 {
      return None;
    }
    let seg = b.get(i + 2..i + len)?;
    i += len;
    match m {
      // jfif header: kept, its embedded thumbnail emptied
      0xe0 if seg.starts_with(b"JFIF\0") && seg.len() >= 14 => {
        out.extend_from_slice(&[0xff, 0xe0, 0, 16]);
        out.extend_from_slice(&seg[..12]);
        out.extend_from_slice(&[0, 0]);
      }
      // exif: only the rotation survives, as a block with nothing else in it
      0xe1 if !seen_exif && seg.starts_with(b"Exif\0\0") => {
        seen_exif = true;
        if let Some(o) = orientation(seg).filter(|o| *o > 1) {
          out.extend_from_slice(&orientation_block(o));
        }
      }
      // icc color profile, adobe's color-transform flag
      0xe2 if seg.starts_with(b"ICC_PROFILE\0") => keep(&mut out, m, seg),
      0xee if seg.starts_with(b"Adobe") => keep(&mut out, m, seg),
      // every other app block (xmp, photoshop 13, mpf, vendor), comments, jpeg extensions
      0xe0..=0xfe => {}
      // tables, frame header, scan header, restart interval, ...
      _ => keep(&mut out, m, seg),
    }
    if m == 0xda {
      let end = scan_end(b, i);
      out.extend_from_slice(&b[i..end]);
      i = end;
      if i >= b.len() {
        // the file stops inside its image data: close it so it still ends like a jpeg
        out.extend_from_slice(&[0xff, 0xd9]);
        return Some(out);
      }
    }
  }
}

fn keep(out: &mut Vec<u8>, m: u8, seg: &[u8]) {
  out.extend_from_slice(&[0xff, m]);
  out.extend_from_slice(&((seg.len() + 2) as u16).to_be_bytes());
  out.extend_from_slice(seg);
}

// where the entropy-coded data of a scan ends: the next real marker (not a stuffed 0xff00, not a restart)
fn scan_end(b: &[u8], from: usize) -> usize {
  let mut j = from;
  while j < b.len() {
    if b[j] != 0xff {
      j += 1;
      continue;
    }
    match b.get(j + 1) {
      Some(0x00 | 0xd0..=0xd7) => j += 2,
      Some(0xff) => j += 1,
      Some(_) => return j,
      None => return b.len(),
    }
  }
  b.len()
}

// the rotation (1..=8) out of an exif block, bounds-checked at every step
fn orientation(app1: &[u8]) -> Option<u16> {
  let t = app1.strip_prefix(b"Exif\0\0")?;
  let le = match t.get(..2)? {
    b"II" => true,
    b"MM" => false,
    _ => return None,
  };
  let u16_at = |o: usize| -> Option<u16> {
    let b = t.get(o..o.checked_add(2)?)?;
    Some(if le { u16::from_le_bytes([b[0], b[1]]) } else { u16::from_be_bytes([b[0], b[1]]) })
  };
  let u32_at = |o: usize| -> Option<u32> {
    let b = t.get(o..o.checked_add(4)?)?;
    let a = [b[0], b[1], b[2], b[3]];
    Some(if le { u32::from_le_bytes(a) } else { u32::from_be_bytes(a) })
  };
  if u16_at(2)? != 42 {
    return None;
  }
  let ifd = u32_at(4)? as usize;
  let n = usize::from(u16_at(ifd)?);
  for k in 0..n.min(512) {
    let e = ifd.checked_add(2 + 12 * k)?;
    if u16_at(e)? == 0x0112 {
      return u16_at(e.checked_add(8)?).filter(|v| (1..=8).contains(v));
    }
  }
  None
}

// APP1 "Exif" holding one tag: orientation. big-endian tiff, one directory, one entry
fn orientation_block(o: u16) -> [u8; 36] {
  let mut b = [0u8; 36];
  b[..4].copy_from_slice(&[0xff, 0xe1, 0, 34]);
  b[4..10].copy_from_slice(b"Exif\0\0");
  b[10..18].copy_from_slice(&[b'M', b'M', 0, 42, 0, 0, 0, 8]);
  b[18..20].copy_from_slice(&[0, 1]);
  b[20..32].copy_from_slice(&[0x01, 0x12, 0, 3, 0, 0, 0, 1, (o >> 8) as u8, o as u8, 0, 0]);
  b
}

// ---- png

// ancillary chunks a viewer needs: transparency, color, size, animation. text, time, exif and
// whatever else a tool tucked in (c2pa, previews, private chunks) are not on it
const PNG_KEEP: [&[u8; 4]; 12] =
  [b"tRNS", b"gAMA", b"cHRM", b"sRGB", b"iCCP", b"sBIT", b"bKGD", b"hIST", b"pHYs", b"acTL", b"fcTL", b"fdAT"];

fn png(b: &[u8]) -> Option<Vec<u8>> {
  const SIG: &[u8] = b"\x89PNG\r\n\x1a\n";
  if !b.starts_with(SIG) {
    return None;
  }
  let mut out = Vec::with_capacity(b.len());
  out.extend_from_slice(SIG);
  let mut i = SIG.len();
  let mut first = true;
  loop {
    let head = b.get(i..i.checked_add(8)?)?;
    let len = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize;
    let kind: [u8; 4] = [head[4], head[5], head[6], head[7]];
    // length, type, data, crc
    let end = i.checked_add(12)?.checked_add(len)?;
    let chunk = b.get(i..end)?;
    if first && &kind != b"IHDR" {
      return None;
    }
    first = false;
    // critical chunks (capital first letter) are the picture itself
    if kind[0] & 0x20 == 0 || PNG_KEEP.contains(&&kind) {
      out.extend_from_slice(chunk);
    }
    if &kind == b"IEND" {
      return Some(out);
    }
    i = end;
  }
}

// ---- webp

const WEBP_KEEP: [&[u8; 4]; 6] = [b"VP8 ", b"VP8L", b"ALPH", b"ANIM", b"ANMF", b"ICCP"];
const VP8X_EXIF: u8 = 0x08;
const VP8X_XMP: u8 = 0x04;

fn webp(b: &[u8]) -> Option<Vec<u8>> {
  if b.get(..4)? != b"RIFF" || b.get(8..12)? != b"WEBP" {
    return None;
  }
  let size = u32::from_le_bytes([b[4], b[5], b[6], b[7]]) as usize;
  let body = b.get(12..size.checked_add(8)?)?;
  let mut out = b[..12].to_vec();
  let (mut i, mut image) = (0, false);
  while i < body.len() {
    let head = body.get(i..i.checked_add(8)?)?;
    let kind: [u8; 4] = [head[0], head[1], head[2], head[3]];
    let len = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as usize;
    let data_end = i.checked_add(8)?.checked_add(len)?;
    let data = body.get(i + 8..data_end)?;
    // chunks sit on even offsets; a last chunk missing its pad byte is fine
    i = data_end + (len & 1);
    let vp8x = &kind == b"VP8X";
    if !vp8x && !WEBP_KEEP.contains(&&kind) {
      continue;
    }
    image |= matches!(&kind, b"VP8 " | b"VP8L" | b"ANMF");
    out.extend_from_slice(&kind);
    out.extend_from_slice(&head[4..8]);
    let at = out.len();
    out.extend_from_slice(data);
    if vp8x && len >= 1 {
      out[at] &= !(VP8X_EXIF | VP8X_XMP);
    }
    if len & 1 == 1 {
      out.push(0);
    }
  }
  if !image {
    return None;
  }
  let riff = u32::try_from(out.len() - 8).ok()?;
  out[4..8].copy_from_slice(&riff.to_le_bytes());
  Some(out)
}

// ---- gif

fn gif(b: &[u8]) -> Option<Vec<u8>> {
  if !(b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a")) {
    return None;
  }
  // header, screen descriptor, global color table
  let packed = *b.get(10)?;
  let mut i = 13 + if packed & 0x80 != 0 { 3usize << ((packed & 7) + 1) } else { 0 };
  let mut out = b.get(..i)?.to_vec();
  loop {
    match b.get(i) {
      Some(0x3b) => break,
      // the file ends between blocks without its trailer: still a gif once we add it
      None => break,
      Some(0x21) => {
        let label = *b.get(i + 1)?;
        let end = blocks_end(b, i + 2)?;
        // graphic control (timing, transparency) and the loop count are what a viewer needs
        let wanted = label == 0xf9 || (label == 0xff && looping(b.get(i + 2..end)?));
        if wanted {
          out.extend_from_slice(&b[i..end]);
        }
        i = end;
      }
      Some(0x2c) => {
        let packed = *b.get(i + 9)?;
        let table = if packed & 0x80 != 0 { 3usize << ((packed & 7) + 1) } else { 0 };
        // descriptor (10), color table, lzw size (1), data blocks
        let end = blocks_end(b, i.checked_add(10 + table + 1)?)?;
        out.extend_from_slice(b.get(i..end)?);
        i = end;
      }
      Some(_) => return None,
    }
  }
  out.push(0x3b);
  Some(out)
}

// the sub-blocks starting at `from` (a size byte, that many bytes, ... a zero size): where they end
fn blocks_end(b: &[u8], from: usize) -> Option<usize> {
  let mut j = from;
  loop {
    let n = usize::from(*b.get(j)?);
    j = j.checked_add(1 + n)?;
    if n == 0 {
      return Some(j);
    }
  }
}

// an application extension body (size 11, name + code, ...) that is the loop count
fn looping(body: &[u8]) -> bool {
  body.get(..12).is_some_and(|h| h[0] == 11 && (&h[1..12] == b"NETSCAPE2.0" || &h[1..12] == b"ANIMEXTS1.0"))
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::images::decode::decode;

  const SECRET: &[u8] = b"SECRETDATA";

  fn find(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
  }

  fn crc32(data: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in data {
      c ^= u32::from(b);
      for _ in 0..8 {
        c = if c & 1 == 1 { (c >> 1) ^ 0xedb8_8320 } else { c >> 1 };
      }
    }
    !c
  }

  fn same_pixels(a: &[u8], b: &[u8]) {
    let (x, y) = (decode(a, 8).unwrap(), decode(b, 8).unwrap());
    assert_eq!((x.w, x.h, x.frames.len()), (y.w, y.h, y.frames.len()));
    for (p, q) in x.frames.iter().zip(&y.frames) {
      assert_eq!((&p.rgba, p.delay_ms), (&q.rgba, q.delay_ms));
    }
  }

  // ---- jpeg: a real 8x8 jpeg (pillow, quality 20), with what a phone adds crafted in around it

  const JPEG_HEX: &str = "ffd8ffe000104a46494600010100000100010000ffdb004300281c1e231e19282321232d2b28303c64413c37373c7b585d4964918099968f808c8aa0b4e6c3a0aadaad8a8cc8ffcbdaeef5ffffff9bc1fffffffaffe6fdfff8ffc0000b080008000801011100ffc4001f0000010501010101010100000000000000000102030405060708090a0bffc400b5100002010303020403050504040000017d01020300041105122131410613516107227114328191a1082342b1c11552d1f02433627282090a161718191a25262728292a3435363738393a434445464748494a535455565758595a636465666768696a737475767778797a838485868788898a92939495969798999aa2a3a4a5a6a7a8a9aab2b3b4b5b6b7b8b9bac2c3c4c5c6c7c8c9cad2d3d4d5d6d7d8d9dae1e2e3e4e5e6e7e8e9eaf1f2f3f4f5f6f7f8f9faffda0008010100003f002bffd9";

  fn base_jpeg() -> Vec<u8> {
    (0..JPEG_HEX.len()).step_by(2).map(|i| u8::from_str_radix(&JPEG_HEX[i..i + 2], 16).unwrap()).collect()
  }

  fn seg(m: u8, payload: &[u8]) -> Vec<u8> {
    let mut v = vec![0xff, m];
    v.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
    v.extend_from_slice(payload);
    v
  }

  // an exif block: make, orientation and a pointer to gps, then a fake gps string (0,0)
  fn exif(orientation: u16, little: bool) -> Vec<u8> {
    let w16 = |v: u16| if little { v.to_le_bytes() } else { v.to_be_bytes() };
    let w32 = |v: u32| if little { v.to_le_bytes() } else { v.to_be_bytes() };
    let mut t: Vec<u8> = if little { b"II".to_vec() } else { b"MM".to_vec() };
    t.extend_from_slice(&w16(42));
    t.extend_from_slice(&w32(8));
    t.extend_from_slice(&w16(3));
    let short = w16(orientation).into_iter().chain([0, 0]).collect::<Vec<u8>>();
    for (tag, ty, val) in [(0x010fu16, 2u16, w32(50).to_vec()), (0x0112, 3, short), (0x8825, 4, w32(60).to_vec())] {
      t.extend_from_slice(&w16(tag));
      t.extend_from_slice(&w16(ty));
      t.extend_from_slice(&w32(1));
      t.extend_from_slice(&val);
    }
    t.extend_from_slice(&w32(0));
    t.extend_from_slice(b"GPSLatitude=0,0 SECRETDATA");
    let mut p = b"Exif\0\0".to_vec();
    p.extend_from_slice(&t);
    p
  }

  // soi, jfif, [crafted block], rest
  fn with(extra: &[Vec<u8>]) -> Vec<u8> {
    let b = base_jpeg();
    let (head, rest) = b.split_at(2 + 2 + 16);
    let mut v = head.to_vec();
    for e in extra {
      v.extend_from_slice(e);
    }
    v.extend_from_slice(rest);
    v
  }

  #[test]
  fn jpeg_loses_exif_gps_xmp_photoshop_comments_and_trailers_but_not_a_pixel() {
    let icc = seg(0xe2, b"ICC_PROFILE\0\x01\x01profile");
    let junk = [
      seg(0xe1, &exif(1, true)),
      seg(0xe1, b"http://ns.adobe.com/xap/1.0/\0<x:xmpmeta>SECRETDATA</x:xmpmeta>"),
      seg(0xed, b"Photoshop 3.0\08BIMSECRETDATA"),
      seg(0xfe, b"SECRETDATA comment"),
      seg(0xe2, b"MPF\0SECRETDATA"),
      seg(0xe5, b"vendor SECRETDATA"),
      seg(0xe0, b"JFXX\0\x10SECRETDATA"),
    ];
    let mut dirty = junk.to_vec();
    dirty.insert(2, icc.clone());
    let mut file = with(&dirty);
    file.extend_from_slice(b"\xff\xd8\xff\xe1motion photo video SECRETDATA appended after the picture");
    let out = jpeg(&file).unwrap();
    assert!(!find(&out, SECRET) && !find(&out, b"GPS") && !find(&out, b"xmpmeta") && !find(&out, b"Photoshop"));
    // what is left is the clean file with the color profile
    assert_eq!(out, with(std::slice::from_ref(&icc)));
    assert_eq!(clean("image/jpeg", file.clone()).unwrap(), out);
    // and once clean, always clean
    assert_eq!(jpeg(&out).unwrap(), out);
  }

  #[test]
  fn jpeg_keeps_a_rotation_and_nothing_else_of_the_exif() {
    for (little, o) in [(true, 6u16), (false, 8), (true, 3)] {
      let out = jpeg(&with(&[seg(0xe1, &exif(o, little))])).unwrap();
      assert!(!find(&out, b"GPS") && !find(&out, SECRET));
      // the one block that is left says the same turn
      let at = out.windows(6).position(|w| w == b"Exif\0\0").expect("a rotation block");
      assert_eq!(orientation(&out[at..]), Some(o));
      assert_eq!(out.len(), base_jpeg().len() + 36);
    }
    // upright (1) or no tag at all: no exif block remains
    for e in [exif(1, true), b"Exif\0\0II*\0\x08\0\0\0\0\0".to_vec(), b"Exif\0\0garbage".to_vec()] {
      assert_eq!(jpeg(&with(&[seg(0xe1, &e)])).unwrap(), base_jpeg());
    }
    // a hostile exif (offsets out of range, endless directory) is just dropped
    let mut bad = b"Exif\0\0II*\0\xff\xff\xff\xff".to_vec();
    bad.extend_from_slice(&[0xff; 8]);
    assert_eq!(jpeg(&with(&[seg(0xe1, &bad)])).unwrap(), base_jpeg());
  }

  #[test]
  fn jpeg_thumbnail_in_the_jfif_header_is_emptied() {
    let mut p = b"JFIF\0\x01\x01\0\0\x01\0\x01\x02\x02".to_vec();
    p.extend_from_slice(b"thumbSECRETDATA");
    let b = base_jpeg();
    let mut file = vec![0xff, 0xd8];
    file.extend_from_slice(&seg(0xe0, &p));
    file.extend_from_slice(&b[2 + 18..]);
    let out = jpeg(&file).unwrap();
    assert_eq!(out, b);
  }

  #[test]
  fn jpeg_that_does_not_parse_is_refused() {
    let good = base_jpeg();
    assert!(jpeg(&good).is_some());
    for bad in [
      &good[..3],
      &good[..40],
      b"\xff\xd8".as_slice(),
      b"\xff\xd8\x00\x00".as_slice(),
      b"\xff\xd8\xff\xe1\x00\x01".as_slice(),
      b"\xff\xd8\xff\xe1\xff\xff abc".as_slice(),
      b"\xff\xd8\xff\x80\x00\x04\0\0".as_slice(),
      b"GIF89a".as_slice(),
    ] {
      assert!(jpeg(bad).is_none(), "{bad:?}");
      assert!(clean("image/jpeg", bad.to_vec()).unwrap_err().contains("damaged"));
    }
    // stopping inside the image data closes it
    let cut = &good[..good.len() - 2];
    assert!(jpeg(cut).unwrap().ends_with(&[0xff, 0xd9]));
  }

  #[test]
  fn jpeg_data_with_stuffed_bytes_and_restarts_is_copied_exactly() {
    let b = base_jpeg();
    let at = b.windows(2).rposition(|w| w == [0xff, 0xda]).unwrap();
    let hdr = 2 + usize::from(u16::from_be_bytes([b[at + 2], b[at + 3]]));
    let mut file = b[..at + hdr].to_vec();
    let data = [0x12, 0xff, 0x00, 0x34, 0xff, 0xd0, 0x56, 0xff, 0xff, 0x00, 0x78];
    file.extend_from_slice(&data);
    file.extend_from_slice(&[0xff, 0xd9]);
    let mut dirty = file.clone();
    dirty.splice(2..2, seg(0xfe, b"x"));
    let out = jpeg(&dirty).unwrap();
    assert_eq!(out, file);
  }

  // ---- png

  fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut v = (data.len() as u32).to_be_bytes().to_vec();
    let mut body = kind.to_vec();
    body.extend_from_slice(data);
    v.extend_from_slice(&body);
    v.extend_from_slice(&crc32(&body).to_be_bytes());
    v
  }

  fn png_with_text() -> Vec<u8> {
    let mut out = Vec::new();
    let mut e = png::Encoder::new(&mut out, 2, 2);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    e.set_pixel_dims(Some(png::PixelDimensions { xppu: 2835, yppu: 2835, unit: png::Unit::Meter }));
    e.add_text_chunk("Comment".into(), "SECRETDATA tEXt".into()).unwrap();
    e.add_ztxt_chunk("Software".into(), "SECRETDATA zTXt".into()).unwrap();
    e.add_itxt_chunk("XML:com.adobe.xmp".into(), "SECRETDATA iTXt".into()).unwrap();
    let mut w = e.write_header().unwrap();
    w.write_image_data(&[255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 0, 9, 9, 9, 255]).unwrap();
    w.finish().unwrap();
    out
  }

  fn kinds(png: &[u8]) -> Vec<String> {
    let mut i = 8;
    let mut v = Vec::new();
    while i + 8 <= png.len() {
      let len = u32::from_be_bytes([png[i], png[i + 1], png[i + 2], png[i + 3]]) as usize;
      v.push(String::from_utf8_lossy(&png[i + 4..i + 8]).into_owned());
      i += 12 + len;
    }
    v
  }

  #[test]
  fn png_loses_text_exif_time_and_unknown_chunks_but_keeps_the_picture() {
    let plain = png_with_text();
    // after the header chunk (8 + 25): exif with gps, a timestamp, a private chunk, a c2pa-like one
    let mut dirty = plain[..33].to_vec();
    dirty.extend_from_slice(&chunk(b"eXIf", b"MM\0*\0\0\0\x08GPSLatitude=0,0 SECRETDATA"));
    dirty.extend_from_slice(&chunk(b"tIME", &[0x07, 0xea, 11, 4, 12, 0, 0]));
    dirty.extend_from_slice(&chunk(b"prVt", b"SECRETDATA private"));
    dirty.extend_from_slice(&chunk(b"caBX", b"SECRETDATA c2pa"));
    dirty.extend_from_slice(&plain[33..]);
    dirty.extend_from_slice(b"trailing SECRETDATA after iend");
    assert!(find(&dirty, SECRET));
    let out = png(&dirty).unwrap();
    assert!(!find(&out, SECRET) && !find(&out, b"GPS"));
    let k = kinds(&out);
    for gone in ["tEXt", "zTXt", "iTXt", "eXIf", "tIME", "prVt", "caBX"] {
      assert!(!k.iter().any(|x| x == gone), "{gone} survived: {k:?}");
    }
    assert_eq!(k, ["IHDR", "pHYs", "IDAT", "IEND"]);
    same_pixels(&dirty, &out);
    assert_eq!(png(&out).unwrap(), out);
    assert_eq!(clean("image/png", dirty).unwrap(), out);
  }

  #[test]
  fn png_keeps_what_color_and_transparency_need() {
    let plain = png_with_text();
    let mut f = plain[..33].to_vec();
    for (k, d) in
      [(b"gAMA", &[0u8, 1, 0x86, 0xa0][..]), (b"sRGB", &[0u8][..]), (b"tRNS", &[][..]), (b"iCCP", &b"p\0\0xx"[..])]
    {
      f.extend_from_slice(&chunk(k, d));
    }
    f.extend_from_slice(&plain[33..]);
    let k = kinds(&png(&f).unwrap());
    for need in ["gAMA", "sRGB", "tRNS", "iCCP", "pHYs"] {
      assert!(k.iter().any(|x| x == need), "{need} lost: {k:?}");
    }
  }

  #[test]
  fn png_that_does_not_parse_is_refused() {
    let good = png_with_text();
    for bad in [&good[..20], &good[..good.len() - 3], b"\x89PNG\r\n\x1a\n".as_slice(), b"nope".as_slice()] {
      assert!(png(bad).is_none());
    }
    // a length that points past the end
    let mut huge = good.clone();
    huge[33..37].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(png(&huge).is_none());
    // the first chunk must be the header
    let mut swapped = b"\x89PNG\r\n\x1a\n".to_vec();
    swapped.extend_from_slice(&chunk(b"tEXt", b"a\0b"));
    swapped.extend_from_slice(&good[8..]);
    assert!(png(&swapped).is_none());
  }

  // ---- gif

  fn gif_two_frames() -> Vec<u8> {
    let mut out = Vec::new();
    {
      let mut e = gif::Encoder::new(&mut out, 2, 2, &[0, 0, 0, 255, 0, 0]).unwrap();
      e.set_repeat(gif::Repeat::Infinite).unwrap();
      for (i, delay) in [(0u8, 7u16), (1, 9)] {
        let mut f = gif::Frame::from_indexed_pixels(2, 2, vec![i; 4], None);
        f.delay = delay;
        e.write_frame(&f).unwrap();
      }
    }
    out
  }

  fn gif_head(g: &[u8]) -> usize {
    13 + 3 * (1 << ((g[10] & 7) + 1))
  }

  #[test]
  fn gif_loses_comments_text_xmp_and_trailers_but_keeps_frames_timing_and_the_loop() {
    let g = gif_two_frames();
    let at = gif_head(&g);
    let mut app = vec![0x21, 0xff, 11];
    app.extend_from_slice(b"XMP DataXMP");
    app.extend_from_slice(&[10]);
    app.extend_from_slice(b"SECRETDATA");
    app.extend_from_slice(&[0]);
    let mut dirty = g[..at].to_vec();
    dirty.extend_from_slice(&[0x21, 0xfe, 14]);
    dirty.extend_from_slice(b"SECRETDATA gps");
    dirty.extend_from_slice(&[0]);
    dirty.extend_from_slice(&app);
    dirty.extend_from_slice(&[0x21, 0x01, 12, 0, 0, 0, 0, 1, 0, 1, 0, 1, 1, 0, 0, 3]);
    dirty.extend_from_slice(b"abc");
    dirty.extend_from_slice(&[0]);
    dirty.extend_from_slice(&g[at..]);
    dirty.extend_from_slice(b"trailing SECRETDATA");
    assert!(find(&dirty, SECRET));
    let out = gif(&dirty).unwrap();
    assert!(!find(&out, SECRET) && !find(&out, b"XMP Data"));
    assert!(find(&out, b"NETSCAPE2.0"), "the loop count stays");
    assert_eq!(out, g, "back to what the encoder wrote");
    same_pixels(&dirty, &out);
    assert_eq!(decode(&out, 8).unwrap().frames.len(), 2);
    assert_eq!(gif(&out).unwrap(), out);
    assert_eq!(clean("image/gif", dirty).unwrap(), out);
  }

  #[test]
  fn gif_that_does_not_parse_is_refused() {
    let g = gif_two_frames();
    assert!(gif(&g).is_some());
    for bad in
      [&g[..8], &g[..g.len() - 6], b"GIF89a".as_slice(), b"GIF89a\x01\0\x01\0\0\0\0\x99".as_slice(), b"nope".as_slice()]
    {
      assert!(gif(bad).is_none(), "{bad:?}");
    }
    // between blocks with no trailer: still a gif
    let cut = &g[..g.len() - 1];
    assert_eq!(gif(cut).unwrap(), g);
  }

  // ---- webp

  fn webp_plain() -> Vec<u8> {
    let mut out = Vec::new();
    image_webp::WebPEncoder::new(&mut out)
      .encode(&[255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 0, 9, 9, 9, 255], 2, 2, image_webp::ColorType::Rgba8)
      .unwrap();
    out
  }

  fn riff(chunks: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
    let mut body = b"WEBP".to_vec();
    for (k, d) in chunks {
      body.extend_from_slice(*k);
      body.extend_from_slice(&(d.len() as u32).to_le_bytes());
      body.extend_from_slice(d);
      if d.len() % 2 == 1 {
        body.push(0);
      }
    }
    let mut v = b"RIFF".to_vec();
    v.extend_from_slice(&(body.len() as u32).to_le_bytes());
    v.extend_from_slice(&body);
    v
  }

  fn vp8x(flags: u8) -> Vec<u8> {
    vec![flags, 0, 0, 0, 1, 0, 0, 1, 0, 0]
  }

  #[test]
  fn webp_loses_exif_and_xmp_and_clears_their_flags() {
    let plain = webp_plain();
    assert_eq!(&plain[12..16], b"VP8L");
    let image = plain[12..].to_vec();
    let (kind, len) = (image[..4].to_vec(), u32::from_le_bytes([image[4], image[5], image[6], image[7]]) as usize);
    let data = image[8..8 + len].to_vec();
    assert_eq!(kind, b"VP8L");
    let dirty = riff(&[
      (b"VP8X", vp8x(0x08 | 0x04 | 0x10 | 0x20)),
      (b"ICCP", b"profile".to_vec()),
      (b"VP8L", data.clone()),
      (b"EXIF", b"MM\0*GPSLatitude=0,0 SECRETDATA".to_vec()),
      (b"XMP ", b"<x:xmpmeta>SECRETDATA</x:xmpmeta>".to_vec()),
      (b"JUNK", b"SECRETDATA odd!".to_vec()),
      (b"vndr", b"SECRETDATA".to_vec()),
    ]);
    // the odd-sized chunk is padded, and that is where the next chunk starts
    assert!(find(&dirty, SECRET));
    let out = webp(&dirty).unwrap();
    assert!(!find(&out, SECRET) && !find(&out, b"GPS") && !find(&out, b"xmpmeta"));
    assert_eq!(out, riff(&[(b"VP8X", vp8x(0x10 | 0x20)), (b"ICCP", b"profile".to_vec()), (b"VP8L", data)]));
    assert_eq!(u32::from_le_bytes([out[4], out[5], out[6], out[7]]) as usize, out.len() - 8);
    same_pixels(&dirty, &out);
    assert_eq!(webp(&out).unwrap(), out);
    assert_eq!(clean("image/webp", dirty).unwrap(), out);
    // trailing bytes past the riff size go too
    let mut tail = out.clone();
    tail.extend_from_slice(b"SECRETDATA");
    assert_eq!(webp(&tail).unwrap(), out);
  }

  #[test]
  fn webp_that_does_not_parse_is_refused() {
    let good = webp_plain();
    assert_eq!(webp(&good).unwrap(), good);
    for bad in [&good[..10], &good[..good.len() - 4], b"RIFF\0\0\0\0WEBP".as_slice(), b"nope".as_slice()] {
      assert!(webp(bad).is_none(), "{bad:?}");
    }
    // a chunk longer than the file
    let mut long = good.clone();
    long[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(webp(&long).is_none());
    // only metadata, no picture
    assert!(webp(&riff(&[(b"EXIF", b"x".to_vec())])).is_none());
  }

  #[test]
  fn other_formats_pass_through() {
    let v = b"\0\0\0\x18ftypisom\0\0\0\0 and SECRETDATA".to_vec();
    assert_eq!(clean("video/mp4", v.clone()).unwrap(), v);
    assert_eq!(clean("image/avif", v.clone()).unwrap(), v);
  }
}
