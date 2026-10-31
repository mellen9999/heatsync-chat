// the small cpu rasterizer the scenes are drawn with: premultiplied rgba tiles, css-style gradients
// (parsed from the ext's own strings, so the catalog stays a verbatim port) and filled vector shapes
// (the ext's inline svg). everything here runs once per scene build, never per frame

// premultiplied rgba8
pub type Px = [u8; 4];

#[derive(Clone, Debug, PartialEq)]
pub struct Buf {
  pub w: usize,
  pub h: usize,
  pub px: Vec<Px>,
}

impl Buf {
  pub fn new(w: usize, h: usize) -> Buf {
    Buf { w, h, px: vec![[0; 4]; w * h] }
  }
}

pub fn div255(v: u32) -> u32 {
  (v + 128 + ((v + 128) >> 8)) >> 8
}

// src over dst, both premultiplied
#[inline]
pub fn over(d: &mut Px, s: Px) {
  match s[3] {
    0 => {}
    255 => *d = s,
    a => {
      let k = 255 - a as u32;
      for c in 0..4 {
        d[c] = (s[c] as u32 + div255(d[c] as u32 * k)).min(255) as u8;
      }
    }
  }
}

// float premultiplied color for the build-time math
pub type Fc = [f32; 4];

pub fn rgba(hex: u32, a: f32) -> Fc {
  let c = |s: u32| ((hex >> s) & 255) as f32 / 255.0 * a;
  [c(16), c(8), c(0), a]
}

pub fn to_px(c: Fc) -> Px {
  let q = |v: f32| (v * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
  let a = q(c[3]);
  [q(c[0]).min(a), q(c[1]).min(a), q(c[2]).min(a), a]
}

fn add(a: Fc, b: Fc, k: f32) -> Fc {
  [a[0] + b[0] * k, a[1] + b[1] * k, a[2] + b[2] * k, a[3] + b[3] * k]
}

// ---- lengths --------------------------------------------------------------------------------

// `pct`% of a reference + `px` css pixels
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Len {
  pub pct: f32,
  pub px: f32,
}

// "12px" | "-.5px" | "40%" | "0" | "calc(46% + 5px)"
pub fn len(tok: &str) -> Option<Len> {
  let t = tok.trim();
  if let Some(inner) = t.strip_prefix("calc(").and_then(|r| r.strip_suffix(')')) {
    let mut out = Len::default();
    let flat = inner.replace(" + ", " +").replace(" - ", " -");
    for part in flat.split_whitespace() {
      let l = len(part)?;
      out.pct += l.pct;
      out.px += l.px;
    }
    return Some(out);
  }
  if let Some(p) = t.strip_suffix('%') {
    return Some(Len { pct: p.parse().ok()?, px: 0.0 });
  }
  let n = t.strip_suffix("px").unwrap_or(t);
  Some(Len { pct: 0.0, px: n.parse().ok()? })
}

// ---- gradients ------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
struct Stop {
  c: Fc,
  pos: Len,
}

#[derive(Clone, Debug, PartialEq)]
enum Shape {
  Linear { deg: f32, repeat: bool },
  // `circle` reaches the farthest corner; an ellipse has radii (rx, ry) as lengths of the box
  Radial { circle: bool, rx: Len, ry: Len, at: (Len, Len) },
  // repeating-conic checker: each stop's `pos` pct is a fraction of the turn
  Conic,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Grad {
  shape: Shape,
  stops: Vec<Stop>,
}

fn color(t: &str) -> Option<Fc> {
  if t == "transparent" {
    return Some([0.0; 4]);
  }
  let h = t.strip_prefix('#')?;
  if !h.bytes().all(|b| b.is_ascii_hexdigit()) {
    return None;
  }
  match h.len() {
    6 => Some(rgba(u32::from_str_radix(h, 16).ok()?, 1.0)),
    8 => {
      let v = u32::from_str_radix(h, 16).ok()?;
      Some(rgba(v >> 8, (v & 255) as f32 / 255.0))
    }
    _ => None,
  }
}

// split on top-level commas
fn args(s: &str) -> Vec<&str> {
  let (mut out, mut depth, mut from) = (Vec::new(), 0, 0);
  for (i, b) in s.bytes().enumerate() {
    match b {
      b'(' => depth += 1,
      b')' => depth -= 1,
      b',' if depth == 0 => {
        out.push(s[from..i].trim());
        from = i + 1;
      }
      _ => {}
    }
  }
  out.push(s[from..].trim());
  out
}

impl Grad {
  pub fn parse(s: &str) -> Option<Grad> {
    let (name, rest) = s.split_once('(')?;
    let body = rest.strip_suffix(')')?;
    let mut a = args(body);
    let shape = match name {
      "linear-gradient" | "repeating-linear-gradient" => {
        let deg = match a.first().and_then(|f| f.strip_suffix("deg")) {
          Some(d) => {
            let d = d.parse().ok()?;
            a.remove(0);
            d
          }
          None => 180.0,
        };
        Shape::Linear { deg, repeat: name.starts_with("repeating") }
      }
      "radial-gradient" => {
        let head = a.first()?;
        let is_shape =
          head.starts_with("circle") || head.contains(" at ") || head.starts_with("at ") || head.ends_with('%');
        let (mut circle, mut rx, mut ry, mut at) =
          (false, Len::default(), Len::default(), (Len { pct: 50.0, px: 0.0 }, Len { pct: 50.0, px: 0.0 }));
        if is_shape {
          let (dims, pos) = match head.split_once("at ") {
            Some((d, p)) => (d.trim(), Some(p.trim())),
            None => (head.trim(), None),
          };
          if let Some(p) = pos {
            let mut it = p.split_whitespace();
            at = (len(it.next()?)?, len(it.next()?)?);
          }
          if dims.starts_with("circle") {
            circle = true;
          } else {
            let mut it = dims.split_whitespace();
            rx = len(it.next()?)?;
            ry = len(it.next()?)?;
          }
          a.remove(0);
        } else {
          circle = false;
          (rx, ry) = (Len { pct: 70.710_68, px: 0.0 }, Len { pct: 70.710_68, px: 0.0 });
        }
        Shape::Radial { circle, rx, ry, at }
      }
      "repeating-conic-gradient" => Shape::Conic,
      _ => return None,
    };
    let mut stops: Vec<(Fc, Vec<Len>)> = Vec::new();
    for st in a {
      let mut it = st.split_whitespace();
      let c = color(it.next()?)?;
      let mut ps = Vec::new();
      for p in it {
        ps.push(len(p)?);
      }
      stops.push((c, ps));
    }
    if stops.len() < 2 {
      return None;
    }
    // a color with two positions is two stops; unplaced ones spread evenly
    let mut flat: Vec<(Fc, Option<Len>)> = Vec::new();
    for (c, ps) in stops {
      if ps.is_empty() {
        flat.push((c, None));
      }
      for p in ps.into_iter().take(2) {
        flat.push((c, Some(p)));
      }
    }
    let n = flat.len();
    if flat[0].1.is_none() {
      flat[0].1 = Some(Len::default());
    }
    if flat[n - 1].1.is_none() {
      flat[n - 1].1 = Some(Len { pct: 100.0, px: 0.0 });
    }
    let mut out: Vec<Stop> = Vec::with_capacity(n);
    let mut i = 0;
    while i < n {
      if let Some(p) = flat[i].1 {
        out.push(Stop { c: flat[i].0, pos: p });
        i += 1;
        continue;
      }
      let j = (i..n).find(|&j| flat[j].1.is_some())?;
      let (a, b) = (out[i - 1].pos, flat[j].1?);
      for (k, item) in flat.iter().enumerate().take(j).skip(i) {
        let f = (k - i + 1) as f32 / (j - i + 1) as f32;
        out.push(Stop { c: item.0, pos: Len { pct: a.pct + (b.pct - a.pct) * f, px: a.px + (b.px - a.px) * f } });
      }
      i = j;
    }
    Some(Grad { shape, stops: out })
  }

  // paints a w×h tile; `s` scales css px
  pub fn render(&self, w: usize, h: usize, s: f32) -> Buf {
    let mut buf = Buf::new(w, h);
    let (fw, fh) = (w as f32, h as f32);
    // resolved stop distances along the gradient ray, and the ray's length
    let (ray, unit) = match &self.shape {
      Shape::Linear { deg, .. } => {
        let (sn, cs) = deg.to_radians().sin_cos();
        let l = (fw * sn).abs() + (fh * cs).abs();
        (l, l)
      }
      Shape::Radial { circle: true, at, .. } => {
        let (cx, cy) = (at.0.pct / 100.0 * fw + at.0.px * s, at.1.pct / 100.0 * fh + at.1.px * s);
        let far = [(0.0, 0.0), (fw, 0.0), (0.0, fh), (fw, fh)]
          .iter()
          .map(|(x, y): &(f32, f32)| ((x - cx).powi(2) + (y - cy).powi(2)).sqrt())
          .fold(0.0, f32::max);
        (far, far)
      }
      Shape::Radial { .. } => (1.0, 1.0),
      Shape::Conic => (1.0, 1.0),
    };
    let pos: Vec<f32> = {
      let mut v: Vec<f32> = self.stops.iter().map(|st| st.pos.pct / 100.0 * unit + st.pos.px * s).collect();
      for i in 1..v.len() {
        v[i] = v[i].max(v[i - 1]);
      }
      v
    };
    let eval = |d: f32, rep: bool| -> Fc {
      let (first, last) = (pos[0], pos[pos.len() - 1]);
      let mut d = d;
      if rep && last > first {
        d = first + (d - first).rem_euclid(last - first);
      }
      if d <= first {
        return self.stops[0].c;
      }
      for i in 1..pos.len() {
        if d < pos[i] {
          let span = pos[i] - pos[i - 1];
          if span <= 0.0 {
            return self.stops[i].c;
          }
          return add(
            add([0.0; 4], self.stops[i - 1].c, 1.0 - (d - pos[i - 1]) / span),
            self.stops[i].c,
            (d - pos[i - 1]) / span,
          );
        }
      }
      self.stops[self.stops.len() - 1].c
    };
    const SS: usize = 2;
    for y in 0..h {
      for x in 0..w {
        let mut acc = [0f32; 4];
        for sy in 0..SS {
          for sx in 0..SS {
            let (px, py) = (x as f32 + (sx as f32 + 0.5) / SS as f32, y as f32 + (sy as f32 + 0.5) / SS as f32);
            let c = match &self.shape {
              Shape::Linear { deg, repeat } => {
                let (sn, cs) = deg.to_radians().sin_cos();
                let d = (px - fw / 2.0) * sn - (py - fh / 2.0) * cs + ray / 2.0;
                eval(d, *repeat)
              }
              Shape::Radial { circle, rx, ry, at } => {
                let (cx, cy) = (at.0.pct / 100.0 * fw + at.0.px * s, at.1.pct / 100.0 * fh + at.1.px * s);
                if *circle {
                  eval(((px - cx).powi(2) + (py - cy).powi(2)).sqrt(), false)
                } else {
                  let (ex, ey) =
                    ((rx.pct / 100.0 * fw + rx.px * s).max(1e-3), (ry.pct / 100.0 * fh + ry.px * s).max(1e-3));
                  eval((((px - cx) / ex).powi(2) + ((py - cy) / ey).powi(2)).sqrt(), false)
                }
              }
              Shape::Conic => {
                let turn = (px - fw / 2.0).atan2(-(py - fh / 2.0)).to_degrees().rem_euclid(360.0) / 360.0;
                eval(turn, true)
              }
            };
            acc = add(acc, c, 1.0 / (SS * SS) as f32);
          }
        }
        buf.px[y * w + x] = to_px(acc);
      }
    }
    buf
  }
}

// ---- vector shapes (the ext's inline svg) ----------------------------------------------------

type Pt = (f32, f32);

#[derive(Clone)]
struct Fill {
  // closed subpaths, nonzero winding
  polys: Vec<Vec<Pt>>,
  c: Fc,
}

// a drawing in a viewBox, painted in order
#[derive(Clone, Default)]
pub struct Svg {
  pub vw: f32,
  pub vh: f32,
  fills: Vec<Fill>,
}

fn rect_poly(x: f32, y: f32, w: f32, h: f32) -> Vec<Pt> {
  vec![(x, y), (x + w, y), (x + w, y + h), (x, y + h)]
}

fn rot(p: Vec<Pt>, deg: f32, cx: f32, cy: f32) -> Vec<Pt> {
  let (s, c) = deg.to_radians().sin_cos();
  p.into_iter().map(|(x, y)| (cx + (x - cx) * c - (y - cy) * s, cy + (x - cx) * s + (y - cy) * c)).collect()
}

fn circle_poly(cx: f32, cy: f32, r: f32, ccw: bool) -> Vec<Pt> {
  const N: usize = 28;
  (0..N)
    .map(|i| {
      let a = (if ccw { -1.0 } else { 1.0 }) * i as f32 / N as f32 * std::f32::consts::TAU;
      (cx + r * a.cos(), cy + r * a.sin())
    })
    .collect()
}

// the path subset the catalog uses: M m L l H h V v Z z (absolute and relative)
fn path_polys(d: &str) -> Vec<Vec<Pt>> {
  let mut out: Vec<Vec<Pt>> = Vec::new();
  let mut cur: Vec<Pt> = Vec::new();
  let (mut x, mut y, mut sx, mut sy) = (0f32, 0f32, 0f32, 0f32);
  let b = d.as_bytes();
  let mut i = 0;
  let mut cmd = b'M';
  let num = |i: &mut usize| -> Option<f32> {
    while *i < b.len() && (b[*i] == b' ' || b[*i] == b',') {
      *i += 1;
    }
    let st = *i;
    if *i < b.len() && (b[*i] == b'-' || b[*i] == b'+') {
      *i += 1;
    }
    while *i < b.len() && (b[*i].is_ascii_digit() || b[*i] == b'.') {
      *i += 1;
    }
    d[st..*i].parse().ok()
  };
  while i < b.len() {
    if b[i] == b' ' || b[i] == b',' {
      i += 1;
      continue;
    }
    if b[i].is_ascii_alphabetic() {
      cmd = b[i];
      i += 1;
      if cmd == b'Z' || cmd == b'z' {
        if !cur.is_empty() {
          out.push(std::mem::take(&mut cur));
        }
        (x, y) = (sx, sy);
      }
      continue;
    }
    let rel = cmd.is_ascii_lowercase();
    match cmd.to_ascii_uppercase() {
      b'M' | b'L' => {
        let (Some(a), Some(c)) = (num(&mut i), num(&mut i)) else { break };
        let (nx, ny) = if rel { (x + a, y + c) } else { (a, c) };
        if cmd.eq_ignore_ascii_case(&b'M') {
          if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
          }
          (sx, sy) = (nx, ny);
          // further pairs after M are implicit lines
          cmd = if rel { b'l' } else { b'L' };
        }
        (x, y) = (nx, ny);
        cur.push((x, y));
      }
      b'H' => {
        let Some(a) = num(&mut i) else { break };
        x = if rel { x + a } else { a };
        cur.push((x, y));
      }
      b'V' => {
        let Some(a) = num(&mut i) else { break };
        y = if rel { y + a } else { a };
        cur.push((x, y));
      }
      _ => break,
    }
  }
  if !cur.is_empty() {
    out.push(cur);
  }
  out
}

impl Svg {
  pub fn new(vw: f32, vh: f32) -> Svg {
    Svg { vw, vh, fills: Vec::new() }
  }

  fn col(hex: &str, op: f32) -> Fc {
    u32::from_str_radix(hex.trim_start_matches('#'), 16).map_or([0.0; 4], |v| rgba(v, op))
  }

  pub fn path(mut self, d: &str, fill: &str, op: f32) -> Self {
    self.fills.push(Fill { polys: path_polys(d), c: Self::col(fill, op) });
    self
  }

  pub fn rect(mut self, x: f32, y: f32, w: f32, h: f32, fill: &str, op: f32) -> Self {
    self.fills.push(Fill { polys: vec![rect_poly(x, y, w, h)], c: Self::col(fill, op) });
    self
  }

  // a rect inside `<g transform='rotate(deg cx cy)'>`
  pub fn rect_rot(mut self, r: (f32, f32, f32, f32), deg: f32, c: (f32, f32), fill: &str, op: f32) -> Self {
    self.fills.push(Fill { polys: vec![rot(rect_poly(r.0, r.1, r.2, r.3), deg, c.0, c.1)], c: Self::col(fill, op) });
    self
  }

  pub fn circle(mut self, cx: f32, cy: f32, r: f32, fill: &str, op: f32) -> Self {
    self.fills.push(Fill { polys: vec![circle_poly(cx, cy, r, false)], c: Self::col(fill, op) });
    self
  }

  // a stroked circle outline (width `sw`)
  pub fn ring(mut self, cx: f32, cy: f32, r: f32, sw: f32, stroke: &str, op: f32) -> Self {
    let polys = vec![circle_poly(cx, cy, r + sw / 2.0, false), circle_poly(cx, cy, (r - sw / 2.0).max(0.01), true)];
    self.fills.push(Fill { polys, c: Self::col(stroke, op) });
    self
  }

  // a stroked half circle arc over the top (the crater rims)
  pub fn arc_top(mut self, cx: f32, cy: f32, r: f32, sw: f32, stroke: &str, op: f32) -> Self {
    let n = 14;
    let mut poly: Vec<Pt> = (0..=n)
      .map(|i| {
        let a = std::f32::consts::PI * (1.0 + i as f32 / n as f32);
        (cx + (r + sw / 2.0) * a.cos(), cy + (r + sw / 2.0) * a.sin())
      })
      .collect();
    poly.extend((0..=n).rev().map(|i| {
      let a = std::f32::consts::PI * (1.0 + i as f32 / n as f32);
      (cx + (r - sw / 2.0) * a.cos(), cy + (r - sw / 2.0) * a.sin())
    }));
    self.fills.push(Fill { polys: vec![poly], c: Self::col(stroke, op) });
    self
  }

  // a stroked straight line (butt caps)
  pub fn line(mut self, a: Pt, b: Pt, sw: f32, stroke: &str, op: f32) -> Self {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let l = (dx * dx + dy * dy).sqrt().max(1e-3);
    let (nx, ny) = (-dy / l * sw / 2.0, dx / l * sw / 2.0);
    let poly = vec![(a.0 + nx, a.1 + ny), (b.0 + nx, b.1 + ny), (b.0 - nx, b.1 - ny), (a.0 - nx, a.1 - ny)];
    self.fills.push(Fill { polys: vec![poly], c: Self::col(stroke, op) });
    self
  }

  // paints the drawing stretched over a w×h tile
  pub fn render(&self, w: usize, h: usize) -> Buf {
    let mut buf = Buf::new(w, h);
    let (kx, ky) = (w as f32 / self.vw, h as f32 / self.vh);
    const SUB: usize = 4;
    let mut cover = vec![0f32; w];
    for f in &self.fills {
      // edges in tile space
      let mut edges: Vec<(Pt, Pt)> = Vec::new();
      for p in &f.polys {
        for i in 0..p.len() {
          let (a, b) = (p[i], p[(i + 1) % p.len()]);
          if a.1 != b.1 {
            edges.push(((a.0 * kx, a.1 * ky), (b.0 * kx, b.1 * ky)));
          }
        }
      }
      if edges.is_empty() {
        continue;
      }
      let ymin = edges.iter().map(|e| e.0.1.min(e.1.1)).fold(f32::MAX, f32::min).floor().max(0.0) as usize;
      let ymax = (edges.iter().map(|e| e.0.1.max(e.1.1)).fold(f32::MIN, f32::max).ceil().max(0.0) as usize).min(h);
      let mut xs: Vec<(f32, i32)> = Vec::new();
      for y in ymin..ymax {
        cover.iter_mut().for_each(|c| *c = 0.0);
        for sub in 0..SUB {
          let sy = y as f32 + (sub as f32 + 0.5) / SUB as f32;
          xs.clear();
          for (a, b) in &edges {
            let (lo, hi, dir) = if a.1 < b.1 { (a, b, 1) } else { (b, a, -1) };
            if sy >= lo.1 && sy < hi.1 {
              xs.push((lo.0 + (hi.0 - lo.0) * (sy - lo.1) / (hi.1 - lo.1), dir));
            }
          }
          xs.sort_by(|a, b| a.0.total_cmp(&b.0));
          let mut wind = 0;
          for i in 0..xs.len() {
            wind += xs[i].1;
            if wind != 0 && i + 1 < xs.len() {
              let (x0, x1) = (xs[i].0.max(0.0), xs[i + 1].0.min(w as f32));
              let mut x = x0.floor() as usize;
              while (x as f32) < x1 && x < w {
                let ov = (x1.min(x as f32 + 1.0) - x0.max(x as f32)).max(0.0);
                cover[x] += ov / SUB as f32;
                x += 1;
              }
            }
          }
        }
        for (x, &cv) in cover.iter().enumerate() {
          if cv > 0.0 {
            over(&mut buf.px[y * w + x], to_px([f.c[0] * cv, f.c[1] * cv, f.c[2] * cv, f.c[3] * cv]));
          }
        }
      }
    }
    buf
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn lengths_parse() {
    assert_eq!(len("40%"), Some(Len { pct: 40.0, px: 0.0 }));
    assert_eq!(len("-90px"), Some(Len { pct: 0.0, px: -90.0 }));
    assert_eq!(len("0"), Some(Len::default()));
    assert_eq!(len("calc(46% + 5px)"), Some(Len { pct: 46.0, px: 5.0 }));
    assert_eq!(len("calc(100% - 5px)"), Some(Len { pct: 100.0, px: -5.0 }));
    assert_eq!(len("auto"), None);
  }

  #[test]
  fn linear_gradient_runs_dark_to_light_upward() {
    // 0deg = to top: the first stop is at the bottom
    let g = Grad::parse("linear-gradient(0deg,#000000 0%,#ffffff 100%)").unwrap().render(4, 10, 1.0);
    assert!(g.px[9 * 4][0] < 30 && g.px[0][0] > 225);
  }

  #[test]
  fn repeating_lines_and_radial_discs() {
    let g =
      Grad::parse("repeating-linear-gradient(0deg,#ffffffff 0 1px,transparent 1px 3px)").unwrap().render(2, 6, 1.0);
    let lit: Vec<bool> = (0..6).map(|y| g.px[y * 2][3] > 200).collect();
    assert_eq!(lit.iter().filter(|&&l| l).count(), 2);
    let d = Grad::parse("radial-gradient(circle at 50% 100%,#ff0000 0 5px,#0000ff 5.5px 14px,#000000 100%)")
      .unwrap()
      .render(40, 20, 1.0);
    assert!(d.px[19 * 40 + 20][0] > 200 && d.px[19 * 40 + 30][2] > 200 && d.px[19 * 40 + 30][0] < 5);
    assert!(Grad::parse("sparkle(1,2)").is_none());
  }

  #[test]
  fn paths_fill_exact_pixels_and_circles_are_round() {
    let s = Svg::new(8.0, 8.0).path("M0 8V4h4V0h4v8z", "#ffffff", 1.0).render(8, 8);
    assert_eq!(s.px[7 * 8][3], 255);
    assert_eq!(s.px[0][3], 0);
    assert_eq!(s.px[7 * 8 + 7][3], 255);
    assert_eq!(s.px[2 * 8 + 6][3], 255);
    let c = Svg::new(10.0, 10.0).circle(5.0, 5.0, 3.0, "#ff0000", 1.0).render(10, 10);
    assert_eq!(c.px[5 * 10 + 5], [255, 0, 0, 255]);
    assert_eq!(c.px[0][3], 0);
    let r = Svg::new(10.0, 10.0).ring(5.0, 5.0, 3.0, 1.0, "#00ff00", 1.0).render(10, 10);
    assert_eq!(r.px[5 * 10 + 5][3], 0);
    assert!(r.px[5 * 10 + 8][3] > 100);
  }
}
