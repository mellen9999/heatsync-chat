// a flat svg badge (bttv's) drawn to pixels: only `<path fill=… d=…>` inside a `viewBox`, no scripts, no
// styles, no references, nothing fetched. anything else in the file is ignored. every size is capped
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Transform};

use super::decode::Frame;

const MAX_BYTES: usize = 64 * 1024;
const MAX_PATHS: usize = 32;
const MAX_D: usize = 16 * 1024;
const MAX_SEGMENTS: usize = 4096;
// the long side it is drawn at; the caller scales it down to the badge height
const SIDE: u32 = 72;

pub fn is_svg(bytes: &[u8]) -> bool {
  let head = &bytes[..bytes.len().min(512)];
  head.windows(4).any(|w| w == b"<svg")
}

pub fn render(bytes: &[u8]) -> Result<(u32, u32, Vec<Frame>), String> {
  if bytes.len() > MAX_BYTES {
    return Err("svg too large".into());
  }
  let text = std::str::from_utf8(bytes).map_err(|_| "svg is not text")?;
  let root = tag_at(text, "<svg").ok_or("no svg tag")?;
  let vb: Vec<f32> = attr(root, "viewBox")
    .map(|v| v.split([' ', ',']).filter(|p| !p.is_empty()).filter_map(|p| p.parse().ok()).collect())
    .unwrap_or_default();
  let &[x0, y0, vw, vh] = &vb[..] else { return Err("svg has no viewBox".into()) };
  if !(vw.is_finite() && vh.is_finite() && vw > 0.0 && vh > 0.0 && x0.is_finite() && y0.is_finite()) {
    return Err("bad viewBox".into());
  }
  let k = SIDE as f32 / vw.max(vh);
  let (w, h) = (((vw * k).round() as u32).clamp(1, SIDE), ((vh * k).round() as u32).clamp(1, SIDE));
  let mut pix = Pixmap::new(w, h).ok_or("bad size")?;
  let at = Transform::from_translate(-x0, -y0).post_scale(k, k);
  let mut drawn = 0;
  let mut rest = text;
  while let Some(i) = rest.find("<path") {
    rest = &rest[i..];
    let Some(tag) = tag_at(rest, "<path") else { break };
    rest = &rest[tag.len()..];
    if drawn >= MAX_PATHS {
      break;
    }
    let (Some(d), Some(color)) = (attr(tag, "d"), attr(tag, "fill").and_then(color)) else { continue };
    let Some(path) = path(d) else { continue };
    let mut paint = Paint::default();
    paint.set_color_rgba8(color[0], color[1], color[2], 255);
    paint.anti_alias = true;
    let rule = if attr(tag, "fill-rule") == Some("evenodd") { FillRule::EvenOdd } else { FillRule::Winding };
    pix.fill_path(&path, &paint, rule, at, None);
    drawn += 1;
  }
  if drawn == 0 {
    return Err("svg has nothing to draw".into());
  }
  // tiny-skia keeps premultiplied pixels; frames are straight alpha
  let mut rgba = pix.take();
  for p in rgba.chunks_exact_mut(4) {
    let a = p[3] as u32;
    if a != 0 && a != 255 {
      for c in &mut p[..3] {
        *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
      }
    }
  }
  Ok((w, h, vec![Frame { rgba: rgba.into(), delay_ms: 0 }]))
}

// the opening tag starting at the first `open` in `s`, through its `>`
fn tag_at<'a>(s: &'a str, open: &str) -> Option<&'a str> {
  let rest = &s[s.find(open)?..];
  let end = rest.find('>')?;
  Some(&rest[..=end])
}

// ` name="value"` inside one tag, with whitespace before the name (`d` is not `id`)
fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
  let pat = format!("{name}=\"");
  let mut from = 0;
  while let Some(i) = tag[from..].find(&pat) {
    let at = from + i;
    let v = &tag[at + pat.len()..];
    if tag[..at].chars().next_back().is_some_and(char::is_whitespace) {
      return v.find('"').map(|e| &v[..e]);
    }
    from = at + pat.len();
  }
  None
}

fn color(v: &str) -> Option<[u8; 3]> {
  let v = v.trim();
  if v.eq_ignore_ascii_case("white") {
    return Some([255; 3]);
  }
  if v.eq_ignore_ascii_case("black") {
    return Some([0; 3]);
  }
  let h = v.strip_prefix('#')?;
  if !h.bytes().all(|b| b.is_ascii_hexdigit()) {
    return None;
  }
  let nib = |i: usize| u8::from_str_radix(&h[i..=i], 16).ok().map(|n| n * 17);
  let byte = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
  match h.len() {
    3 => Some([nib(0)?, nib(1)?, nib(2)?]),
    6 => Some([byte(0)?, byte(2)?, byte(4)?]),
    _ => None,
  }
}

struct Nums<'a> {
  s: &'a [u8],
  i: usize,
}

impl Nums<'_> {
  fn skip(&mut self) {
    while self.i < self.s.len() && (self.s[self.i].is_ascii_whitespace() || self.s[self.i] == b',') {
      self.i += 1;
    }
  }

  fn peek(&mut self) -> Option<u8> {
    self.skip();
    self.s.get(self.i).copied()
  }

  // a number: `-.004`, `1.5e-3`; `.5.5` is two numbers
  fn num(&mut self) -> Option<f32> {
    self.skip();
    let a = self.i;
    let mut dot = false;
    if matches!(self.s.get(self.i), Some(b'+' | b'-')) {
      self.i += 1;
    }
    while let Some(&c) = self.s.get(self.i) {
      match c {
        b'0'..=b'9' => self.i += 1,
        b'.' if !dot => {
          dot = true;
          self.i += 1;
        }
        b'e' | b'E' if self.i > a => {
          self.i += 1;
          if matches!(self.s.get(self.i), Some(b'+' | b'-')) {
            self.i += 1;
          }
        }
        _ => break,
      }
    }
    let v: f32 = std::str::from_utf8(&self.s[a..self.i]).ok()?.parse().ok()?;
    v.is_finite().then_some(v)
  }

  // an arc flag is one digit, and may touch the number after it (`011.5`)
  fn flag(&mut self) -> Option<bool> {
    self.skip();
    let c = self.s.get(self.i).copied()?;
    self.i += 1;
    match c {
      b'0' => Some(false),
      b'1' => Some(true),
      _ => None,
    }
  }

  // an x, y pair, moved by `o` when the command is relative
  fn pt(&mut self, o: (f32, f32)) -> Option<(f32, f32)> {
    Some((self.num()? + o.0, self.num()? + o.1))
  }
}

type Pt = (f32, f32);

// svg path data → a path. None on anything it does not follow
fn path(d: &str) -> Option<tiny_skia::Path> {
  if d.len() > MAX_D {
    return None;
  }
  let mut n = Nums { s: d.as_bytes(), i: 0 };
  let mut pb = PathBuilder::new();
  let (mut cur, mut start): (Pt, Pt) = ((0.0, 0.0), (0.0, 0.0));
  // the last control point, for the smooth curves
  let mut last: Option<Pt> = None;
  let mut cmd = b' ';
  let mut segs = 0;
  while let Some(c) = n.peek() {
    if c.is_ascii_alphabetic() {
      cmd = c;
      n.i += 1;
      if matches!(c, b'z' | b'Z') {
        pb.close();
        (cur, last) = (start, None);
        continue;
      }
    } else if cmd == b' ' || matches!(cmd, b'z' | b'Z') {
      return None;
    }
    segs += 1;
    if segs > MAX_SEGMENTS {
      return None;
    }
    let o = if cmd.is_ascii_lowercase() { cur } else { (0.0, 0.0) };
    let mirror = |l: Option<Pt>, cur: Pt| l.map_or(cur, |l| (2.0 * cur.0 - l.0, 2.0 * cur.1 - l.1));
    let mut ctrl = None;
    match cmd.to_ascii_uppercase() {
      b'M' => {
        cur = n.pt(o)?;
        start = cur;
        pb.move_to(cur.0, cur.1);
        // more pairs after a move are lines
        cmd = if o == (0.0, 0.0) && cmd == b'M' { b'L' } else { b'l' };
      }
      b'L' => {
        cur = n.pt(o)?;
        pb.line_to(cur.0, cur.1);
      }
      b'H' => {
        cur.0 = n.num()? + o.0;
        pb.line_to(cur.0, cur.1);
      }
      b'V' => {
        cur.1 = n.num()? + o.1;
        pb.line_to(cur.0, cur.1);
      }
      b'C' => {
        let (a, b, e) = (n.pt(o)?, n.pt(o)?, n.pt(o)?);
        pb.cubic_to(a.0, a.1, b.0, b.1, e.0, e.1);
        (cur, ctrl) = (e, Some(b));
      }
      b'S' => {
        let a = mirror(last, cur);
        let (b, e) = (n.pt(o)?, n.pt(o)?);
        pb.cubic_to(a.0, a.1, b.0, b.1, e.0, e.1);
        (cur, ctrl) = (e, Some(b));
      }
      b'Q' => {
        let (a, e) = (n.pt(o)?, n.pt(o)?);
        pb.quad_to(a.0, a.1, e.0, e.1);
        (cur, ctrl) = (e, Some(a));
      }
      b'T' => {
        let a = mirror(last, cur);
        let e = n.pt(o)?;
        pb.quad_to(a.0, a.1, e.0, e.1);
        (cur, ctrl) = (e, Some(a));
      }
      b'A' => {
        let (rx, ry, rot) = (n.num()?, n.num()?, n.num()?);
        let (large, sweep) = (n.flag()?, n.flag()?);
        let e = n.pt(o)?;
        arc(&mut pb, cur, (rx, ry, rot), (large, sweep), e);
        cur = e;
      }
      _ => return None,
    }
    last = ctrl;
  }
  pb.finish()
}

// an elliptical arc as cubic curves, ≤90° each (the svg endpoint → center conversion)
fn arc(pb: &mut PathBuilder, from: Pt, r: (f32, f32, f32), flags: (bool, bool), to: Pt) {
  let (mut rx, mut ry) = (r.0.abs(), r.1.abs());
  if from == to {
    return;
  }
  if rx < 1e-6 || ry < 1e-6 {
    pb.line_to(to.0, to.1);
    return;
  }
  let (sin, cos) = r.2.to_radians().sin_cos();
  let (dx, dy) = ((from.0 - to.0) / 2.0, (from.1 - to.1) / 2.0);
  let (x1, y1) = (cos * dx + sin * dy, -sin * dx + cos * dy);
  let lam = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
  if lam > 1.0 {
    let s = lam.sqrt();
    rx *= s;
    ry *= s;
  }
  let num = rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1;
  let den = rx * rx * y1 * y1 + ry * ry * x1 * x1;
  let mut co = if den == 0.0 { 0.0 } else { (num / den).max(0.0).sqrt() };
  if flags.0 == flags.1 {
    co = -co;
  }
  let (cx1, cy1) = (co * rx * y1 / ry, -co * ry * x1 / rx);
  let (cx, cy) = (cos * cx1 - sin * cy1 + (from.0 + to.0) / 2.0, sin * cx1 + cos * cy1 + (from.1 + to.1) / 2.0);
  let ang = |ux: f32, uy: f32, vx: f32, vy: f32| (ux * vy - uy * vx).atan2(ux * vx + uy * vy);
  let t1 = ang(1.0, 0.0, (x1 - cx1) / rx, (y1 - cy1) / ry);
  let mut dt = ang((x1 - cx1) / rx, (y1 - cy1) / ry, (-x1 - cx1) / rx, (-y1 - cy1) / ry);
  if !flags.1 && dt > 0.0 {
    dt -= std::f32::consts::TAU;
  } else if flags.1 && dt < 0.0 {
    dt += std::f32::consts::TAU;
  }
  let n = (dt.abs() / std::f32::consts::FRAC_PI_2).ceil().max(1.0) as usize;
  let step = dt / n as f32;
  let kappa = 4.0 / 3.0 * (step / 4.0).tan();
  let at = |t: f32| {
    let (s, c) = t.sin_cos();
    (cx + rx * c * cos - ry * s * sin, cy + rx * c * sin + ry * s * cos)
  };
  let slope = |t: f32| {
    let (s, c) = t.sin_cos();
    (-rx * s * cos - ry * c * sin, -rx * s * sin + ry * c * cos)
  };
  for i in 0..n {
    let (a, b) = (t1 + step * i as f32, t1 + step * (i + 1) as f32);
    let (p, q) = (at(a), at(b));
    let (da, db) = (slope(a), slope(b));
    let end = if i + 1 == n { to } else { q };
    pb.cubic_to(p.0 + kappa * da.0, p.1 + kappa * da.1, q.0 - kappa * db.0, q.1 - kappa * db.1, end.0, end.1);
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn px(f: &(u32, u32, Vec<Frame>), x: u32, y: u32) -> [u8; 4] {
    let i = ((y * f.0 + x) * 4) as usize;
    f.2[0].rgba[i..i + 4].try_into().unwrap()
  }

  #[test]
  fn draws_flat_paths_and_arcs() {
    // left half red by lines, right half a blue half-disc by an arc
    let s = br##"<svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 10 10">
      <path fill="#ff0000" d="M0 0h5v10H0z"/><path fill="#00f" d="M5 0 a5 5 0 0 1 0 10z"/></svg>"##;
    assert!(is_svg(s));
    let f = render(s).unwrap();
    assert_eq!((f.0, f.1), (SIDE, SIDE));
    assert_eq!(px(&f, 10, 36), [255, 0, 0, 255]);
    assert_eq!(px(&f, 40, 36), [0, 0, 255, 255]);
    assert_eq!(px(&f, 70, 2)[3], 0, "outside the half-disc stays clear");
  }

  #[test]
  fn compact_numbers_and_flags() {
    // `a5 5 0 011 0`-style flags touch the number after them; `.5.5` is two numbers
    let s = br##"<svg viewBox="0 0 10 10"><path fill="#00ff00" d="M0 5a5 5 0 0110 0v.5.5H0z"/></svg>"##;
    let f = render(s).unwrap();
    assert_eq!(px(&f, 36, 20), [0, 255, 0, 255]);
  }

  #[test]
  fn hostile_files_fail_quietly() {
    let d = "M0 0".to_string() + &" L1 1".repeat(MAX_SEGMENTS + 10);
    let big = format!(r##"<svg viewBox="0 0 10 10"><path fill="#fff" d="{d}"/></svg>"##);
    assert!(render(big.as_bytes()).is_err(), "too many segments");
    for bad in [
      &b"<svg><path fill=\"#fff\" d=\"M0 0h1v1z\"/></svg>"[..],
      b"<svg viewBox=\"0 0 0 0\"><path fill=\"#fff\" d=\"M0 0h1v1z\"/></svg>",
      b"<svg viewBox=\"0 0 1e40 1\"><path fill=\"#fff\" d=\"M0 0h1v1z\"/></svg>",
      b"<svg viewBox=\"0 0 10 10\"><path fill=\"url(#x)\" d=\"M0 0h1v1z\"/></svg>",
      b"<svg viewBox=\"0 0 10 10\"><path fill=\"#fff\" d=\"M0 0 Q\"/></svg>",
      b"<svg viewBox=\"0 0 10 10\"><path fill=\"#fff\" d=\"hello\"/></svg>",
      b"<svg viewBox=\"0 0 10 10\"><image href=\"https://evil.example/x.png\"/></svg>",
      b"\xff\xfe<svg",
      b"<svg viewBox=\"0 0 10 10\"",
      b"",
    ] {
      assert!(render(bad).is_err(), "{}", String::from_utf8_lossy(bad));
    }
    assert!(render(&vec![b' '; MAX_BYTES + 1]).is_err());
    assert!(!is_svg(b"\x89PNG....") && !is_svg(b"RIFF"));
  }

  #[test]
  fn a_real_bttv_badge_shape() {
    // the white disc and the colored ring of bttv's own art
    let s = br##"<svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 300 300">
      <path fill="white" d="M249.771 150A99.771 99.922 0 0 1 150 249.922 99.771 99.922 0 0 1 50.229 150 99.771 99.922 0 0 1 150 50.078 99.771 99.922 0 0 1 249.771 150Z"/>
      <path fill="#d50014" d="M150 1.74C68.409 1.74 1.74 68.41 1.74 150S68.41 298.26 150 298.26h148.26V150.17h-.004c0-.057.004-.113.004-.17C298.26 68.409 231.59 1.74 150 1.74zm0 49c55.11 0 99.26 44.15 99.26 99.26 0 55.11-44.15 99.26-99.26 99.26-55.11 0-99.26-44.15-99.26-99.26 0-55.11 44.15-99.26 99.26-99.26z"/></svg>"##;
    let f = render(s).unwrap();
    // the ring is red, the middle white, the corner clear
    assert_eq!(px(&f, 36, 3), [213, 0, 20, 255]);
    assert_eq!(px(&f, 36, 36), [255, 255, 255, 255]);
    assert_eq!(px(&f, 1, 1)[3], 0);
  }
}
