// the system clipboard on windows, through the win32 clipboard api: text as utf-16 (lines end in CRLF there),
// a picture as the registered "PNG" format (browsers, the snipping tool) or, failing that, a plain bitmap
// that is re-encoded as png. every call blocks, so callers run it off the ui thread (see net/clip.rs).
// another program may hold the clipboard open for a moment: opening is retried for a fraction of a second,
// then said plainly
use super::{Fail, MAX_IMAGE, MAX_TEXT, Paste};
use crate::util::win::wide;
use std::ptr::{null, null_mut};
use std::time::Duration;
use windows_sys::Win32::Foundation::GlobalFree;
use windows_sys::Win32::System::DataExchange::{
  CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
  RegisterClipboardFormatW, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock};
use windows_sys::Win32::System::Ole::{CF_DIB, CF_UNICODETEXT};

const UNICODE_TEXT: u32 = CF_UNICODETEXT as u32;
const DIB: u32 = CF_DIB as u32;
use windows_sys::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, HWND_MESSAGE};

const OPEN_TRIES: u32 = 10;
const OPEN_PAUSE: Duration = Duration::from_millis(25);
// a bitmap this big in pixels is not a screenshot: it would not fit the picture cap as png either
const MAX_PIXELS: usize = 40_000_000;

// the clipboard held open for us; closed when dropped. `owner` is a hidden window: a copy needs one, since
// emptying a clipboard that has no owner makes the next set fail
struct Open {
  owner: windows_sys::Win32::Foundation::HWND,
}

impl Open {
  fn new() -> Result<Open, Fail> {
    let class = wide("STATIC");
    // SAFETY: a message-only window of a system class; nothing is shown or drawn
    let owner = unsafe {
      CreateWindowExW(0, class.as_ptr(), null(), 0, 0, 0, 0, 0, HWND_MESSAGE, null_mut(), null_mut(), null())
    };
    for n in 0..OPEN_TRIES {
      // SAFETY: opening the clipboard for the window we made (or none, for a read)
      if unsafe { OpenClipboard(owner) } != 0 {
        return Ok(Open { owner });
      }
      if n + 1 < OPEN_TRIES {
        std::thread::sleep(OPEN_PAUSE);
      }
    }
    // SAFETY: the window is ours
    unsafe { DestroyWindow(owner) };
    Err(Fail::Timeout)
  }
}

impl Drop for Open {
  fn drop(&mut self) {
    // SAFETY: the clipboard was opened by `new`; the window is ours
    unsafe {
      CloseClipboard();
      if !self.owner.is_null() {
        DestroyWindow(self.owner);
      }
    }
  }
}

// lines end in CRLF on the windows clipboard; chat text has bare LF
fn to_crlf(s: &str) -> String {
  s.replace("\r\n", "\n").replace('\n', "\r\n")
}

fn from_crlf(s: &str) -> String {
  s.replace("\r\n", "\n")
}

// a block of the clipboard's memory as bytes, for as long as `f` runs. none when it can't be locked
fn with_bytes<T>(format: u32, f: impl FnOnce(&[u8]) -> T) -> Option<T> {
  // SAFETY: the clipboard is open (the caller holds an `Open`); the lock keeps the block in place while
  // `f` reads it, and the slice does not outlive the unlock
  unsafe {
    let h = GetClipboardData(format);
    if h.is_null() {
      return None;
    }
    let (len, p) = (GlobalSize(h), GlobalLock(h));
    if p.is_null() {
      return None;
    }
    let out = f(std::slice::from_raw_parts(p.cast::<u8>(), len));
    GlobalUnlock(h);
    Some(out)
  }
}

// put text on the clipboard (the size was checked by the caller)
pub fn copy(text: &str) -> Result<(), Fail> {
  let units: Vec<u16> = to_crlf(text).encode_utf16().chain(Some(0)).collect();
  let bytes = units.len() * 2;
  let held = Open::new()?;
  // SAFETY: a fresh block, filled before it is handed over; on success the clipboard owns it, else we free it
  unsafe {
    let h = GlobalAlloc(GMEM_MOVEABLE, bytes);
    if h.is_null() {
      return Err(Fail::Tool("the clipboard"));
    }
    let p = GlobalLock(h);
    if p.is_null() {
      GlobalFree(h);
      return Err(Fail::Tool("the clipboard"));
    }
    std::ptr::copy_nonoverlapping(units.as_ptr().cast::<u8>(), p.cast::<u8>(), bytes);
    GlobalUnlock(h);
    if EmptyClipboard() == 0 || SetClipboardData(UNICODE_TEXT, h).is_null() {
      GlobalFree(h);
      return Err(Fail::Tool("the clipboard"));
    }
  }
  drop(held);
  Ok(())
}

// text up to its null, as utf-8
fn text_of(raw: &[u8]) -> Result<String, Fail> {
  if raw.len() > MAX_TEXT * 4 {
    return Err(Fail::TooBig(MAX_TEXT));
  }
  let units: Vec<u16> = raw.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&u| u != 0).collect();
  let text = from_crlf(&String::from_utf16_lossy(&units));
  if text.len() > MAX_TEXT { Err(Fail::TooBig(MAX_TEXT)) } else { Ok(text) }
}

// the png inside a block that may be padded past it: up to the end of the IEND chunk (4 name + 4 crc bytes)
fn png_end(raw: &[u8]) -> Option<&[u8]> {
  if !raw.starts_with(b"\x89PNG\r\n\x1a\n") {
    return None;
  }
  let at = raw.windows(4).rposition(|w| w == b"IEND")?;
  raw.get(..at + 8)
}

// a bottom-up or top-down 24/32-bit device-independent bitmap as png bytes. other shapes are not taken
fn dib_to_png(d: &[u8]) -> Option<Vec<u8>> {
  let u32_at = |o: usize| d.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
  let (size, w, h) = (u32_at(0)? as usize, u32_at(4)? as i32, u32_at(8)? as i32);
  let (bits, compression, used) =
    (u32::from(u16::from_le_bytes([*d.get(14)?, *d.get(15)?])), u32_at(16)?, u32_at(32)? as usize);
  if size < 40 || w <= 0 || h == 0 || !matches!(bits, 24 | 32) || !matches!(compression, 0 | 3) {
    return None;
  }
  let (w, rows, up) = (w as usize, h.unsigned_abs() as usize, h > 0);
  if w.checked_mul(rows)? > MAX_PIXELS {
    return None;
  }
  // a 40-byte header with bitfields has three masks after it; only the usual b-g-r ones are understood
  let masks = if compression == 3 && size == 40 { 12 } else { 0 };
  if masks == 12 && (u32_at(40)? != 0xff0000 || u32_at(44)? != 0xff00 || u32_at(48)? != 0xff) {
    return None;
  }
  let start = size.checked_add(masks)?.checked_add(used.checked_mul(4)?)?;
  let stride = (w * bits as usize).div_ceil(32) * 4;
  let px = d.get(start..start.checked_add(stride.checked_mul(rows)?)?)?;
  let step = bits as usize / 8;
  let mut rgba = Vec::with_capacity(w * rows * 4);
  // a 32-bit bitmap with every alpha byte zero is one that never used alpha: opaque
  let opaque = step == 3 || (0..rows).all(|r| px[r * stride..r * stride + w * 4].chunks_exact(4).all(|p| p[3] == 0));
  for r in 0..rows {
    let row = &px[(if up { rows - 1 - r } else { r }) * stride..][..w * step];
    for p in row.chunks_exact(step) {
      rgba.extend_from_slice(&[p[2], p[1], p[0], if opaque { 255 } else { p[3] }]);
    }
  }
  let mut out = Vec::new();
  let mut enc = png::Encoder::new(&mut out, w as u32, rows as u32);
  enc.set_color(png::ColorType::Rgba);
  enc.set_depth(png::BitDepth::Eight);
  enc.set_compression(png::Compression::Fast);
  enc.write_header().ok()?.write_image_data(&rgba).ok()?;
  Some(out)
}

// what is on the clipboard: text if there is some, else a picture
pub fn paste() -> Result<Paste, Fail> {
  let _held = Open::new()?;
  // SAFETY: the clipboard is open; these only ask what formats are there
  let has = |f: u32| unsafe { IsClipboardFormatAvailable(f) } != 0;
  if has(UNICODE_TEXT) {
    let text = with_bytes(UNICODE_TEXT, text_of).transpose()?;
    if let Some(t) = text.filter(|t| !t.trim().is_empty()) {
      return Ok(Paste::Text(t));
    }
  }
  let png = wide("PNG");
  // SAFETY: registering (or finding) a format by name
  let png_format = unsafe { RegisterClipboardFormatW(png.as_ptr()) };
  if png_format != 0 && has(png_format) {
    let bytes = with_bytes(png_format, |b| png_end(b).map(<[u8]>::to_vec)).flatten();
    match bytes {
      Some(b) if b.len() > MAX_IMAGE => return Err(Fail::TooBig(MAX_IMAGE)),
      Some(b) => return Ok(Paste::Image(b)),
      None => {}
    }
  }
  if has(DIB)
    && let Some(b) = with_bytes(DIB, dib_to_png).flatten()
  {
    return if b.len() > MAX_IMAGE { Err(Fail::TooBig(MAX_IMAGE)) } else { Ok(Paste::Image(b)) };
  }
  Ok(Paste::Nothing)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn lines_change_ends_without_doubling() {
    assert_eq!(to_crlf("a\nb"), "a\r\nb");
    assert_eq!(to_crlf("a\r\nb"), "a\r\nb");
    assert_eq!(from_crlf("a\r\nb\r\n"), "a\nb\n");
  }

  fn utf16(s: &str) -> Vec<u8> {
    s.encode_utf16().chain([0, 0x41]).flat_map(u16::to_le_bytes).collect()
  }

  #[test]
  fn text_ends_at_its_null_and_is_bounded() {
    assert_eq!(text_of(&utf16("hi \u{1f9ea}\r\nthere")).unwrap(), "hi \u{1f9ea}\nthere");
    assert_eq!(text_of(&[]).unwrap(), "");
    assert_eq!(text_of(&vec![b'a'; MAX_TEXT * 4 + 2]), Err(Fail::TooBig(MAX_TEXT)));
  }

  #[test]
  fn a_png_is_cut_at_its_end_chunk() {
    let mut p = b"\x89PNG\r\n\x1a\n....".to_vec();
    p.extend_from_slice(b"\0\0\0\0IEND\xae\x42\x60\x82");
    let padded = [p.clone(), vec![0; 5]].concat();
    assert_eq!(png_end(&padded), Some(&p[..]));
    assert_eq!(png_end(b"not a png IEND...."), None);
    assert_eq!(png_end(b"\x89PNG\r\n\x1a\nno end"), None);
  }

  // a 2x2 bottom-up 24-bit bitmap: bottom row (blue, green) then top row (red, white); rows pad to 8 bytes
  fn tiny_dib() -> Vec<u8> {
    let mut d = Vec::new();
    for v in [40u32, 2, 2, 0x0018_0001, 0, 16, 0, 0, 0, 0] {
      d.extend_from_slice(&v.to_le_bytes());
    }
    d.extend_from_slice(&[255, 0, 0, 0, 255, 0, 0, 0]); // bgr: blue, green, 2 pad
    d.extend_from_slice(&[0, 0, 255, 255, 255, 255, 0, 0]); // red, white
    d
  }

  #[test]
  fn a_bitmap_becomes_a_png_the_right_way_up() {
    let png = dib_to_png(&tiny_dib()).expect("png");
    let mut dec = png::Decoder::new(std::io::Cursor::new(png));
    dec.set_transformations(png::Transformations::EXPAND);
    let mut r = dec.read_info().unwrap();
    let mut buf = vec![0; r.output_buffer_size().unwrap()];
    let i = r.next_frame(&mut buf).unwrap();
    assert_eq!((i.width, i.height), (2, 2));
    assert_eq!(
      buf[..16],
      [255, 0, 0, 255, 255, 255, 255, 255, 0, 0, 255, 255, 0, 255, 0, 255],
      "red white / blue green"
    );
  }

  #[test]
  fn a_hostile_bitmap_is_not_taken() {
    let mut d = tiny_dib();
    assert!(dib_to_png(&d[..30]).is_none(), "a header cut short");
    d.truncate(50);
    assert!(dib_to_png(&d).is_none(), "pixels cut short");
    let mut huge = tiny_dib();
    huge[4..8].copy_from_slice(&0x7fff_ffffu32.to_le_bytes());
    assert!(dib_to_png(&huge).is_none(), "an absurd width");
    let mut paletted = tiny_dib();
    paletted[14] = 8;
    assert!(dib_to_png(&paletted).is_none(), "8 bits is not taken");
    assert!(dib_to_png(&[]).is_none());
  }
}
