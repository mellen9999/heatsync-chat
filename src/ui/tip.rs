// the emote hover popup, like the ext's #hs-emote-tooltip: the emote blown up 4x (nearest, so
// pixel art stays crisp), its name, and a source chip. square, black, 1px white border. the big
// rung comes through the image cache and is released when the popup closes
use super::canvas::{Canvas, Rect};
use super::res::Res;
use super::text::{Ink, Span};
use super::theme;
use crate::images::{ImgId, url};

const PAD: i32 = 6;
const GAP: i32 = 4;
// the ext's STACK_PREVIEW_SCALE over the 28px chat emote
const BIG_PX: u16 = 112;
// breathing room to the window edge (ext PREVIEW_VIEWPORT_MARGIN)
const MARGIN: i32 = 12;
const NAME_MAX: f32 = 480.0;

// how the source chip is drawn: a filled white chip, or colored text in a 1px colored border
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Chip {
  Plain,
  Border(u32),
}

pub struct Tip {
  pub name: Box<str>,
  pub url: Box<str>,
  pub label: &'static str,
  pub chip: Chip,
  // a blocked emote's picture never paints, here or anywhere
  pub blocked: bool,
  // the pointer when it appeared; the popup keeps still while the pointer stays on the emote
  pub anchor: (i32, i32),
  ids: Vec<ImgId>,
}

// the source chip for an emote: `owned` = in your own heatsync set, `hs` = a heatsync inventory
pub fn source(url: &str, owned: bool, hs: bool, blocked: bool) -> (&'static str, Chip) {
  if blocked {
    return ("blocked (click to unblock)", Chip::Border(theme::RED));
  }
  if owned {
    return ("inventory", Chip::Plain);
  }
  let host = url::host(url).unwrap_or("").to_ascii_lowercase();
  let by = |d: &str| host == d || host.ends_with(&format!(".{d}"));
  if hs || by("heatsync.org") {
    ("heatsync", Chip::Border(theme::HEATSYNC))
  } else if by("7tv.app") {
    ("7tv", Chip::Border(theme::CYAN))
  } else if by("betterttv.net") {
    ("bttv", Chip::Border(theme::RED))
  } else if by("frankerfacez.com") {
    ("ffz", Chip::Plain)
  } else if by("jtvnw.net") {
    ("twitch", Chip::Border(theme::TWITCH))
  } else if by("kick.com") {
    ("kick", Chip::Border(theme::GREEN))
  } else {
    ("unknown", Chip::Plain)
  }
}

// top-left of a w×h popup for a pointer at `at`: above-right of it, below when there is no room
// above, always inside `bounds`
pub fn place(at: (i32, i32), w: i32, h: i32, bounds: Rect) -> (i32, i32) {
  let mut y = at.1 - h - 8;
  if y < bounds.y {
    y = at.1 + 20;
  }
  let x = (at.0 + 14).min(bounds.right() - w).max(bounds.x);
  (x, y.min(bounds.bottom() - h).max(bounds.y))
}

// integer nearest-neighbor enlargement of straight rgba8
fn enlarge(rgba: &[u8], w: usize, h: usize, n: usize) -> Vec<u8> {
  let mut out = vec![0u8; w * n * h * n * 4];
  for y in 0..h * n {
    for x in 0..w * n {
      let (s, d) = (((y / n) * w + x / n) * 4, (y * w * n + x) * 4);
      out[d..d + 4].copy_from_slice(&rgba[s..s + 4]);
    }
  }
  out
}

impl Tip {
  pub fn new(name: &str, url: &str, owned: bool, hs: bool, blocked: bool, anchor: (i32, i32)) -> Self {
    let (label, chip) = source(url, owned, hs, blocked);
    Tip { name: name.into(), url: url.into(), label, chip, blocked, anchor, ids: Vec::new() }
  }

  // same emote as before? (the popup stays put)
  pub fn is(&self, name: &str, url: &str) -> bool {
    *self.name == *name && *self.url == *url
  }

  // frees the big pictures this popup decoded
  pub fn release(&mut self, res: &mut Res) {
    for id in self.ids.drain(..) {
      res.images.release(id);
    }
  }

  // paints the popup inside `bounds`. None = not yet (the picture is still on its way: nothing
  // paints, so there is no small-then-big swap). Some((rect, ms until the next animated frame))
  pub fn draw(&mut self, c: &mut Canvas, res: &mut Res, bounds: Rect) -> Option<(Rect, Option<u32>)> {
    let Res { text, images, .. } = res;
    let line = text.line_px as i32;
    let scale = images.px(1000) as f32 / 1000.0;
    // the picture fits the window: 4x unless that would not leave room for the text rows
    let room_h = (bounds.h - 2 * MARGIN - 2 * PAD - 2 * line - 2 * GAP - 2).max(8);
    let th = (images.px(BIG_PX) as i32).min(room_h);
    let lh = (((th as f32 / scale) as u16) / 8 * 8).max(8);
    let max_w = (bounds.w - 2 * MARGIN - 2 * PAD - 2).max(8);

    let mut pic = None;
    if !self.blocked {
      let big = url::hires(&self.url);
      let first = images.id_fit(big.as_deref().unwrap_or(&self.url), lh, max_w as u32);
      // a rung the provider lacks falls back to the url as chat has it
      let id = if big.is_some() && images.failed(first) { images.id_fit(&self.url, lh, max_w as u32) } else { first };
      for i in [first, id] {
        if !self.ids.contains(&i) {
          self.ids.push(i);
        }
      }
      let failed = images.failed(id);
      match images.frame(id) {
        Some((f, w, h, ms)) => pic = Some((f.rgba.as_ref(), w as i32, h as i32, ms)),
        None if !failed => return None,
        None => {}
      }
    }
    // whole pixels only: n× the decoded size, as big as the room allows
    let (pw, ph, n) = pic.map_or((0, 0, 1), |(_, w, h, _)| {
      let n = (th / h.max(1)).min(max_w / w.max(1)).max(1);
      (w * n, h * n, n)
    });

    let name_w = ((NAME_MAX * scale).min(max_w as f32)).max(8.0);
    let nl =
      text.layout(&self.name, &[Span { range: 0..self.name.len(), ink: Ink(theme::WHITE), bold: true }], &[], name_w);
    let (nw, nh) = (nl.full_width().ceil() as i32, nl.height().ceil() as i32);
    let chip_ink = match self.chip {
      Chip::Plain => theme::BLACK,
      Chip::Border(col) => col,
    };
    let cl = text.layout(self.label, &[Span { range: 0..self.label.len(), ink: Ink(chip_ink), bold: false }], &[], 1e5);
    let cw = cl.full_width().ceil() as i32 + 2 * PAD;

    let inner_w = pw.max(nw).max(cw).max(1);
    let w = inner_w + 2 * PAD + 2;
    let gap = if pic.is_some() { GAP } else { 0 };
    let h = 1 + PAD + ph + gap + nh + GAP + line + 1;
    let (x, y) = place(self.anchor, w, h, bounds);
    let r = Rect::new(x, y, w, h);

    c.fill(r, theme::WHITE);
    c.fill(Rect::new(x + 1, y + 1, w - 2, h - 2), theme::BLACK);
    let mut ty = y + 1 + PAD;
    let mut ms = None;
    if let Some((rgba, pw0, ph0, m)) = pic {
      ms = m;
      let px = x + 1 + (w - 2 - pw) / 2;
      if n > 1 {
        c.rgba(px, ty, pw, ph, &enlarge(rgba, pw0 as usize, ph0 as usize, n as usize), r);
      } else {
        c.rgba(px, ty, pw, ph, rgba, r);
      }
      ty += ph + gap;
    }
    let inner = Rect::new(x + 1, y + 1, w - 2, h - 2);
    text.draw(&nl, c, (x + 1 + (w - 2 - nw) / 2) as f32, ty as f32, inner, |_, _| {});
    let chip = Rect::new(x + 1, y + h - 1 - line, w - 2, line);
    match self.chip {
      Chip::Plain => c.fill(chip, theme::WHITE),
      Chip::Border(col) => {
        c.fill(chip, col);
        c.fill(Rect::new(chip.x + 1, chip.y + 1, chip.w - 2, chip.h - 2), theme::BLACK);
      }
    }
    let lw = cl.full_width();
    text.draw(&cl, c, chip.x as f32 + (chip.w as f32 - lw) / 2.0, chip.y as f32, chip, |_, _| {});
    Some((r, ms))
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn sources_read_off_the_host() {
    let s = |u, o, h, b| source(u, o, h, b);
    assert_eq!(s("https://cdn.7tv.app/emote/x/1x.webp", false, false, false), ("7tv", Chip::Border(theme::CYAN)));
    assert_eq!(s("https://cdn.betterttv.net/emote/x/1x.webp", false, false, false).0, "bttv");
    assert_eq!(s("https://cdn.frankerfacez.com/emote/9/1", false, false, false), ("ffz", Chip::Plain));
    assert_eq!(s("https://static-cdn.jtvnw.net/emoticons/v2/1/default/dark/1.0", false, false, false).0, "twitch");
    assert_eq!(s("https://cdn.7tv.app/emote/x/1x.webp", false, true, false).0, "heatsync");
    assert_eq!(s("https://cdn.heatsync.org/e.png", false, false, false).0, "heatsync");
    assert_eq!(s("https://cdn.7tv.app.evil.example/x", false, false, false).0, "unknown");
    // yours, and blocked, outrank the host
    assert_eq!(s("https://cdn.7tv.app/emote/x/1x.webp", true, false, false).0, "inventory");
    assert_eq!(s("https://cdn.7tv.app/emote/x/1x.webp", true, false, true).0, "blocked (click to unblock)");
  }

  #[test]
  fn the_popup_sits_above_right_and_never_leaves_the_window() {
    let b = Rect::new(0, 20, 400, 300);
    assert_eq!(place((100, 200), 120, 80, b), (114, 112));
    // no room above: below the pointer
    assert_eq!(place((100, 40), 120, 80, b), (114, 60));
    // right and bottom edges pull it back in
    assert_eq!(place((390, 310), 120, 80, b), (280, 222));
    // bigger than the window: pinned to the top-left
    assert_eq!(place((50, 50), 900, 900, b), (0, 20));
  }

  #[test]
  fn enlarging_repeats_whole_pixels() {
    let px = [1, 2, 3, 255, 9, 8, 7, 255];
    let big = enlarge(&px, 2, 1, 3);
    assert_eq!(big.len(), 6 * 3 * 4);
    assert_eq!(&big[0..4], &[1, 2, 3, 255]);
    assert_eq!(&big[8..12], &[1, 2, 3, 255]);
    assert_eq!(&big[12..16], &[9, 8, 7, 255]);
    assert_eq!(&big[48..52], &[1, 2, 3, 255]);
  }

  fn bounds() -> Rect {
    Rect::new(0, 20, 600, 400)
  }

  #[test]
  fn a_blocked_emote_shows_text_only() {
    let (mut c, mut res) = (Canvas::new(600, 440), Res::new(1.0));
    let mut t = Tip::new("Kappa", "https://cdn.7tv.app/emote/x/1x.webp", false, false, true, (100, 200));
    let (r, ms) = t.draw(&mut c, &mut res, bounds()).unwrap();
    assert!(ms.is_none() && bounds().intersect(r) == r);
    assert!(res.images.take_wanted().is_empty(), "a blocked emote's picture is never fetched");
    // 1px white border around black
    assert_eq!(
      (c.px[(r.y * 600 + r.x) as usize], c.px[((r.y + 3) * 600 + r.x + 3) as usize]),
      (theme::WHITE, theme::BLACK)
    );
  }

  #[test]
  fn waits_for_the_big_rung_then_draws_it_and_frees_it() {
    let (mut c, mut res) = (Canvas::new(600, 440), Res::new(1.0));
    let mut t = Tip::new("Kappa", "https://cdn.7tv.app/emote/x/1x.webp", false, false, false, (300, 300));
    assert!(t.draw(&mut c, &mut res, bounds()).is_none(), "nothing paints until the picture is here");
    let wanted = res.images.take_wanted();
    assert_eq!(wanted.len(), 1);
    let (id, url, h) = (wanted[0].0, wanted[0].1.clone(), wanted[0].2);
    assert_eq!(&*url, "https://cdn.7tv.app/emote/x/4x.webp");
    let red = [255u8, 0, 0, 255].repeat(28 * 28);
    let img = crate::images::decode::Decoded {
      w: 28,
      h: 28,
      frames: vec![crate::images::decode::Frame { rgba: red.into(), delay_ms: 0 }],
    };
    res.images.loaded(id, h, Ok(img));
    let (r, _) = t.draw(&mut c, &mut res, bounds()).unwrap();
    // 28px grows to 112: a red square at least that wide inside the box
    let reds = (r.y..r.bottom())
      .flat_map(|y| (r.x..r.right()).map(move |x| (x, y)))
      .filter(|&(x, y)| c.px[(y * 600 + x) as usize] == 0xff0000)
      .count();
    assert_eq!(reds, 112 * 112);
    assert!(res.images.bytes() > 0);
    t.release(&mut res);
    assert_eq!(res.images.bytes(), 0);
  }
}
