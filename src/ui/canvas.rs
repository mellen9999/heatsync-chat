// cpu framebuffer: 0x00rrggbb, rects + alpha masks + rgba blits. square ui needs nothing more

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Rect {
  pub x: i32,
  pub y: i32,
  pub w: i32,
  pub h: i32,
}

impl Rect {
  pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
    Self { x, y, w, h }
  }

  pub fn intersect(self, o: Rect) -> Rect {
    let x0 = self.x.max(o.x);
    let y0 = self.y.max(o.y);
    let x1 = (self.x + self.w).min(o.x + o.w);
    let y1 = (self.y + self.h).min(o.y + o.h);
    Rect::new(x0, y0, (x1 - x0).max(0), (y1 - y0).max(0))
  }

  pub fn is_empty(self) -> bool {
    self.w <= 0 || self.h <= 0
  }

  pub fn contains(self, x: i32, y: i32) -> bool {
    x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
  }

  pub fn right(self) -> i32 {
    self.x + self.w
  }

  pub fn bottom(self) -> i32 {
    self.y + self.h
  }
}

pub struct Canvas {
  pub w: i32,
  pub h: i32,
  pub px: Vec<u32>,
}

#[inline]
fn lerp(dst: u32, src: u32, a: u32) -> u32 {
  // exact (dst * (255 - a) + src * a) / 255 per channel, red+blue packed together
  let inv = 255 - a;
  let rb = (dst & 0xff00ff) * inv + (src & 0xff00ff) * a + 0x800080;
  let rb = ((rb + ((rb >> 8) & 0xff00ff)) >> 8) & 0xff00ff;
  let g = (dst & 0x00ff00) * inv + (src & 0x00ff00) * a + 0x008000;
  let g = ((g + ((g >> 8) & 0x00ff00)) >> 8) & 0x00ff00;
  rb | g
}

// one straight-alpha rgba8 pixel over a destination pixel
fn blend(d: &mut u32, s: &[u8; 4]) {
  let rgb = (s[0] as u32) << 16 | (s[1] as u32) << 8 | s[2] as u32;
  match s[3] {
    0 => {}
    255 => *d = rgb,
    a => *d = lerp(*d, rgb, a as u32),
  }
}

impl Canvas {
  pub fn new(w: i32, h: i32) -> Self {
    Self { w, h, px: vec![0; (w.max(0) * h.max(0)) as usize] }
  }

  pub fn resize(&mut self, w: i32, h: i32) {
    self.w = w;
    self.h = h;
    self.px.clear();
    self.px.resize((w.max(0) * h.max(0)) as usize, 0);
    self.px.shrink_to_fit();
  }

  pub fn bounds(&self) -> Rect {
    Rect::new(0, 0, self.w, self.h)
  }

  pub fn fill(&mut self, r: Rect, rgb: u32) {
    let r = r.intersect(self.bounds());
    if r.is_empty() {
      return;
    }
    for y in r.y..r.bottom() {
      let row = (y * self.w + r.x) as usize;
      self.px[row..row + r.w as usize].fill(rgb);
    }
  }

  // 1px outline just inside `r`
  pub fn outline(&mut self, r: Rect, clip: Rect, rgb: u32) {
    for e in [
      Rect::new(r.x, r.y, r.w, 1),
      Rect::new(r.x, r.bottom() - 1, r.w, 1),
      Rect::new(r.x, r.y, 1, r.h),
      Rect::new(r.right() - 1, r.y, 1, r.h),
    ] {
      self.fill(e.intersect(clip), rgb);
    }
  }

  // 2px dashed outline (4 on, 4 off) just inside `r`
  pub fn dashed(&mut self, r: Rect, clip: Rect, rgb: u32) {
    const T: i32 = 2;
    const D: i32 = 4;
    let mut x = r.x;
    while x < r.right() {
      let w = D.min(r.right() - x);
      self.fill(Rect::new(x, r.y, w, T).intersect(clip), rgb);
      self.fill(Rect::new(x, r.bottom() - T, w, T).intersect(clip), rgb);
      x += 2 * D;
    }
    let mut y = r.y;
    while y < r.bottom() {
      let h = D.min(r.bottom() - y);
      self.fill(Rect::new(r.x, y, T, h).intersect(clip), rgb);
      self.fill(Rect::new(r.right() - T, y, T, h).intersect(clip), rgb);
      y += 2 * D;
    }
  }

  // 8-bit coverage mask tinted with rgb
  #[allow(clippy::too_many_arguments)]
  pub fn mask(&mut self, x: i32, y: i32, w: i32, h: i32, data: &[u8], rgb: u32, clip: Rect) {
    let c = Rect::new(x, y, w, h).intersect(clip).intersect(self.bounds());
    if c.is_empty() {
      return;
    }
    for py in c.y..c.bottom() {
      let src = ((py - y) * w + (c.x - x)) as usize;
      let dst = (py * self.w + c.x) as usize;
      for (d, &a) in self.px[dst..dst + c.w as usize].iter_mut().zip(&data[src..src + c.w as usize]) {
        match a {
          0 => {}
          255 => *d = rgb,
          a => *d = lerp(*d, rgb, a as u32),
        }
      }
    }
  }

  // straight-alpha rgba8
  #[allow(clippy::too_many_arguments)]
  pub fn rgba(&mut self, x: i32, y: i32, w: i32, h: i32, data: &[u8], clip: Rect) {
    let c = Rect::new(x, y, w, h).intersect(clip).intersect(self.bounds());
    if c.is_empty() {
      return;
    }
    for py in c.y..c.bottom() {
      let src = (((py - y) * w + (c.x - x)) * 4) as usize;
      let dst = (py * self.w + c.x) as usize;
      for (d, s) in self.px[dst..dst + c.w as usize].iter_mut().zip(data[src..].as_chunks::<4>().0) {
        blend(d, s);
      }
    }
  }

  // the same, stretched nearest-neighbor from a sw×sh source to dw×dh at (x, y) (pixel art stays
  // crisp; a whole-number ratio repeats whole pixels). no scratch buffer
  #[allow(clippy::too_many_arguments)]
  pub fn rgba_to(&mut self, x: i32, y: i32, (sw, sh): (i32, i32), (dw, dh): (i32, i32), data: &[u8], clip: Rect) {
    let c = Rect::new(x, y, dw, dh).intersect(clip).intersect(self.bounds());
    if c.is_empty() || sw <= 0 || sh <= 0 {
      return;
    }
    for py in c.y..c.bottom() {
      let row = ((py - y) as i64 * sh as i64 / dh as i64) as usize * sw as usize;
      let dst = (py * self.w) as usize;
      for px in c.x..c.right() {
        let i = (row + ((px - x) as i64 * sw as i64 / dw as i64) as usize) * 4;
        blend(&mut self.px[dst + px as usize], data[i..i + 4].try_into().unwrap());
      }
    }
  }

  // premultiplied rgba8 (the animated name frames): src + dst * (1 - a)
  pub fn rgba_pm(&mut self, x: i32, y: i32, w: i32, h: i32, data: &[[u8; 4]], clip: Rect) {
    let c = Rect::new(x, y, w, h).intersect(clip).intersect(self.bounds());
    if c.is_empty() {
      return;
    }
    for py in c.y..c.bottom() {
      let src = ((py - y) * w + (c.x - x)) as usize;
      let dst = (py * self.w + c.x) as usize;
      for (d, s) in self.px[dst..dst + c.w as usize].iter_mut().zip(&data[src..src + c.w as usize]) {
        if s[3] == 0 {
          continue;
        }
        let k = 255 - s[3] as u32;
        let ch = |sh: u32, v: u8| (v as u32 + (((*d >> sh) & 255) * k + 127) / 255).min(255) << sh;
        *d = ch(16, s[0]) | ch(8, s[1]) | ch(0, s[2]);
      }
    }
  }

  // move r's pixels up by dy (terminal-style scroll); the freed strip is left for the caller
  pub fn scroll_up(&mut self, r: Rect, dy: i32) {
    let r = r.intersect(self.bounds());
    if dy <= 0 || dy >= r.h || r.is_empty() {
      return;
    }
    for y in r.y..r.bottom() - dy {
      let dst = (y * self.w + r.x) as usize;
      let src = ((y + dy) * self.w + r.x) as usize;
      self.px.copy_within(src..src + r.w as usize, dst);
    }
  }
}

// tests: pixels that are a shade of `ink` (a pure ansi color) over black: a glyph's edge pixels count as
// much as its full ones, so what a font rasterizes at full coverage (none, for some fonts at some sizes)
// does not decide a test
#[cfg(test)]
impl Canvas {
  // pixels drawn in `ink` at any coverage: text antialiasing (and fonts like terminus that never
  // reach full coverage) leaves shades of it, so tests count those instead of exact matches
  pub fn inked(&self, ink: u32) -> usize {
    self.px.iter().filter(|&&p| Self::shade(p, ink)).count()
  }

  pub fn shade(p: u32, ink: u32) -> bool {
    p != 0
      && [16, 8, 0].iter().all(|&s| {
        let (i, c) = ((ink >> s) & 0xff, (p >> s) & 0xff);
        if i == 0xff { c > 0 } else { c == 0 }
      })
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn rgba_to_stretches_whole_pixels() {
    let mut c = Canvas::new(8, 4);
    let px = [1, 2, 3, 255, 9, 8, 7, 255];
    c.rgba_to(1, 1, (2, 1), (6, 3), &px, c.bounds());
    assert_eq!(c.px[(8 + 1) as usize], 0x010203);
    assert_eq!(c.px[(3 * 8 + 3) as usize], 0x010203);
    assert_eq!(c.px[(8 + 4) as usize], 0x090807);
    assert_eq!(c.px[(8 + 7) as usize], 0);
    // clipped: nothing past the clip rect
    let mut d = Canvas::new(8, 4);
    d.rgba_to(1, 1, (2, 1), (6, 3), &px, Rect::new(0, 0, 2, 4));
    assert_eq!(d.px[(8 + 2) as usize], 0);
  }

  #[test]
  fn lerp_endpoints() {
    assert_eq!(lerp(0x000000, 0xffffff, 255), 0xffffff);
    assert_eq!(lerp(0x123456, 0xffffff, 0), 0x123456);
    assert_eq!(lerp(0x000000, 0xff8700, 128), 0x804400);
    assert_eq!(lerp(0xffffff, 0x000000, 1), 0xfefefe);
    for a in 0..=255 {
      assert_eq!(lerp(0x000000, 0xff00ff, a) >> 16, a);
    }
  }

  #[test]
  fn fill_clips() {
    let mut c = Canvas::new(4, 4);
    c.fill(Rect::new(-2, -2, 4, 4), 0xff0000);
    assert_eq!(c.px[0], 0xff0000);
    assert_eq!(c.px[1], 0xff0000);
    assert_eq!(c.px[2], 0);
    assert_eq!(c.px[4 * 2], 0);
  }

  #[test]
  fn scroll_moves_rows() {
    let mut c = Canvas::new(2, 3);
    c.px = vec![1, 1, 2, 2, 3, 3];
    c.scroll_up(c.bounds(), 1);
    assert_eq!(&c.px[..4], &[2, 2, 3, 3]);
  }
}
