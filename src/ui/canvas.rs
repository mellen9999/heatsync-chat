// cpu framebuffer: 0x00rrggbb, rects + alpha masks + rgba blits. square ui needs nothing more

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
        let rgb = (s[0] as u32) << 16 | (s[1] as u32) << 8 | s[2] as u32;
        match s[3] {
          0 => {}
          255 => *d = rgb,
          a => *d = lerp(*d, rgb, a as u32),
        }
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

#[cfg(test)]
mod tests {
  use super::*;

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
