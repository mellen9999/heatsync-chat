// the emote hover popup, like the ext's #hs-emote-tooltip: the emote blown up 4x (nearest, so
// pixel art stays crisp), its name, and a source chip. a stacked emote shows the whole stack, base
// with its zero-width overlays on top (the chat's own centering), and its name line lists every
// piece in the ext's order: "base + overlay + overlay", each in its provider's color. square,
// black, 1px white border. the big rung comes through the image cache and is released when the
// popup closes
use super::canvas::{Canvas, Rect};
use super::chat_view::{Layer, blit_layer};
use super::res::{EMOTE_PX, Res};
use super::text::{Ink, Span, TextLayout};
use super::theme;
use crate::emotes::block::Scope;
use crate::images::{ImgId, url};

const PAD: i32 = 6;
const GAP: i32 = 4;
// the ext's STACK_PREVIEW_SCALE over the 28px chat emote
const BIG_PX: u16 = 112;
// breathing room to the window edge (ext PREVIEW_VIEWPORT_MARGIN)
const MARGIN: i32 = 12;
const NAME_MAX: f32 = 480.0;
// pieces named in the line before "+N more" (ext MAX); the picture always shows them all
const NAMED_MAX: usize = 8;

// how the source chip is drawn: a filled white chip, or colored text in a 1px colored border
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Chip {
  Plain,
  Border(u32),
}

// one piece of the stack: the base first, then its overlays
struct Piece {
  name: Box<str>,
  url: Box<str>,
  // the bigger rung the provider offers, if any
  big: Option<Box<str>>,
  hs: bool,
}

// a piece's pictures: the big rung, the url as chat has it (for a rung the provider lacks), and
// the chat's own copy, which says how big the piece is in a line
struct Pic {
  big: ImgId,
  alt: Option<ImgId>,
  chat: ImgId,
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
  pieces: Vec<Piece>,
  // per piece, made once per fit so a repaint allocates nothing
  pics: Vec<Pic>,
  fit: (u16, u32),
  // laid-out name line (keyed on its width) and source chip
  nl: Option<(u32, TextLayout)>,
  cl: Option<TextLayout>,
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

// a piece's name color in the composition line: the ext's provider colors; heatsync and anything
// unknown stay white
fn piece_ink(url: &str, hs: bool) -> u32 {
  match source(url, false, hs, false).0 {
    "7tv" => theme::CYAN,
    "bttv" => theme::RED,
    "twitch" => theme::TWITCH,
    "kick" => theme::GREEN,
    _ => theme::WHITE,
  }
}

impl Tip {
  // `over` = the zero-width overlays stacked on the emote, bottom to top
  pub fn new(name: &str, url: &str, owned: bool, hs: bool, blocked: bool, anchor: (i32, i32), over: &[Layer]) -> Self {
    let (label, chip) = source(url, owned, hs, blocked);
    let piece = |name: &str, url: &str, hs: bool| Piece {
      name: name.into(),
      url: url.into(),
      big: url::hires(url).map(Into::into),
      hs,
    };
    let pieces = std::iter::once(piece(name, url, hs))
      .chain(over.iter().map(|l| piece(&l.name, &l.url, l.scope == Scope::Hs)))
      .collect();
    Tip {
      name: name.into(),
      url: url.into(),
      label,
      chip,
      blocked,
      anchor,
      ids: Vec::new(),
      pieces,
      pics: Vec::new(),
      fit: (0, 0),
      nl: None,
      cl: None,
    }
  }

  // same emote (and same stack) as before? the popup stays put
  pub fn is(&self, name: &str, url: &str, over: &[Layer]) -> bool {
    *self.name == *name
      && *self.url == *url
      && self.pieces.len() == over.len() + 1
      && self.pieces[1..].iter().zip(over).all(|(p, l)| p.name == l.name && p.url == l.url)
  }

  // the name line: "base + overlay + ..." with each piece in its provider's color, the base bold.
  // a lone emote is its name in bold white
  fn compose(&self) -> (String, Vec<Span>) {
    let mut text = String::new();
    let mut spans = Vec::new();
    let mut push = |text: &mut String, t: &str, ink: u32, bold: bool| {
      spans.push(Span { range: text.len()..text.len() + t.len(), ink: Ink(ink), bold });
      text.push_str(t);
    };
    if self.pieces.len() == 1 {
      push(&mut text, &self.name, theme::WHITE, true);
      return (text, spans);
    }
    for (i, p) in self.pieces.iter().take(NAMED_MAX).enumerate() {
      if i > 0 {
        push(&mut text, " + ", theme::WHITE, false);
      }
      push(&mut text, &p.name, piece_ink(&p.url, p.hs), i == 0);
    }
    if self.pieces.len() > NAMED_MAX {
      push(&mut text, &format!(" +{} more", self.pieces.len() - NAMED_MAX), theme::WHITE, false);
    }
    (text, spans)
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

    if !self.blocked && (self.pics.is_empty() || self.fit != (lh, max_w as u32)) {
      self.fit = (lh, max_w as u32);
      self.pics.clear();
      for p in &self.pieces {
        let first = images.id_fit(p.big.as_deref().unwrap_or(&p.url), lh, max_w as u32);
        // a rung the provider lacks falls back to the url as chat has it
        let alt = p.big.is_some().then(|| images.id_fit(&p.url, lh, max_w as u32));
        for i in [Some(first), alt].into_iter().flatten() {
          if !self.ids.contains(&i) {
            self.ids.push(i);
          }
        }
        self.pics.push(Pic { big: first, alt, chat: images.id(&p.url, EMOTE_PX) });
      }
    }
    let pick = |images: &crate::images::Images, p: &Pic| match p.alt {
      Some(a) if images.failed(p.big) => a,
      _ => p.big,
    };
    // each piece is as big as chat draws it, times one whole number shared by the stack, so
    // overlays keep their place and proportion on the base
    let (mut ready, mut cw_max) = (true, 1);
    for pr in &self.pics {
      let (big, chat) = (pick(images, pr), pr.chat);
      let (b, k) = (images.frame(big).map(|f| f.1), images.frame(chat).map(|f| f.1));
      match (b, k) {
        (Some(_), Some(w)) => cw_max = cw_max.max(w as i32),
        _ => ready &= images.failed(big) || images.failed(chat),
      }
    }
    // every piece is asked for before giving up, so they load together
    if !ready {
      return None;
    }
    let n = (th / images.px(EMOTE_PX) as i32).min(max_w / cw_max).max(1);
    let (mut pw, mut ph) = (0, 0);
    for pr in &self.pics {
      let big = pick(images, pr);
      if !images.failed(big)
        && let Some((_, cw, ch, _)) = images.frame(pr.chat)
      {
        pw = pw.max(cw as i32 * n);
        ph = ph.max(ch as i32 * n);
      }
    }

    let name_w = ((NAME_MAX * scale).min(max_w as f32)).max(8.0);
    if self.nl.as_ref().is_none_or(|(k, _)| *k != name_w.to_bits()) {
      let (t, spans) = self.compose();
      self.nl = Some((name_w.to_bits(), text.layout(&t, &spans, &[], name_w)));
    }
    let nl = &self.nl.as_ref().unwrap().1;
    let (nw, nh) = (nl.full_width().ceil() as i32, nl.height().ceil() as i32);
    let chip_ink = match self.chip {
      Chip::Plain => theme::BLACK,
      Chip::Border(col) => col,
    };
    let label = self.label;
    let cl = self.cl.get_or_insert_with(|| {
      text.layout(label, &[Span { range: 0..label.len(), ink: Ink(chip_ink), bold: false }], &[], 1e5)
    });
    let cw = cl.full_width().ceil() as i32 + 2 * PAD;

    let inner_w = pw.max(nw).max(cw).max(1);
    let w = inner_w + 2 * PAD + 2;
    let gap = if pw > 0 { GAP } else { 0 };
    let h = 1 + PAD + ph + gap + nh + GAP + line + 1;
    let (x, y) = place(self.anchor, w, h, bounds);
    let r = Rect::new(x, y, w, h);

    c.fill(r, theme::WHITE);
    c.fill(Rect::new(x + 1, y + 1, w - 2, h - 2), theme::BLACK);
    let mut ty = y + 1 + PAD;
    let mut ms: Option<u32> = None;
    if pw > 0 {
      let pic = Rect::new(x + 1 + (w - 2 - pw) / 2, ty, pw, ph);
      for pr in &self.pics {
        let Some((_, cw, ch, _)) = images.frame(pr.chat) else { continue };
        let Some((f, w, h, m)) = images.frame(pick(images, pr)) else { continue };
        blit_layer(c, &f.rgba, (w, h), (cw as i32 * n, ch as i32 * n), pic, r);
        ms = ms.into_iter().chain(m).min();
      }
      ty += ph + gap;
    }
    let inner = Rect::new(x + 1, y + 1, w - 2, h - 2);
    text.draw(nl, c, (x + 1 + (w - 2 - nw) / 2) as f32, ty as f32, inner, |_, _| {});
    let chip = Rect::new(x + 1, y + h - 1 - line, w - 2, line);
    match self.chip {
      Chip::Plain => c.fill(chip, theme::WHITE),
      Chip::Border(col) => {
        c.fill(chip, col);
        c.fill(Rect::new(chip.x + 1, chip.y + 1, chip.w - 2, chip.h - 2), theme::BLACK);
      }
    }
    let lw = cl.full_width();
    text.draw(cl, c, chip.x as f32 + (chip.w as f32 - lw) / 2.0, chip.y as f32, chip, |_, _| {});
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

  fn bounds() -> Rect {
    Rect::new(0, 20, 600, 400)
  }

  #[test]
  fn a_blocked_emote_shows_text_only() {
    let (mut c, mut res) = (Canvas::new(600, 440), Res::new(1.0));
    let mut t = Tip::new("Kappa", "https://cdn.7tv.app/emote/x/1x.webp", false, false, true, (100, 200), &[]);
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
    let mut t = Tip::new("Kappa", "https://cdn.7tv.app/emote/x/1x.webp", false, false, false, (300, 300), &[]);
    assert!(t.draw(&mut c, &mut res, bounds()).is_none(), "nothing paints until the picture is here");
    let wanted = res.images.take_wanted();
    assert_eq!(wanted.len(), 2, "the big rung, and chat's own copy to size it by");
    assert!(wanted.iter().any(|w| &*w.1 == "https://cdn.7tv.app/emote/x/4x.webp"));
    for (id, _, h) in wanted {
      res.images.loaded(id, h, Ok(solid(28, 28, [255, 0, 0])));
    }
    let (r, _) = t.draw(&mut c, &mut res, bounds()).unwrap();
    // 28px grows to 112: a red square at least that wide inside the box
    let reds = (r.y..r.bottom())
      .flat_map(|y| (r.x..r.right()).map(move |x| (x, y)))
      .filter(|&(x, y)| c.px[(y * 600 + x) as usize] == 0xff0000)
      .count();
    assert_eq!(reds, 112 * 112);
    assert!(res.images.bytes() > 0);
    t.release(&mut res);
    // the big rung is gone; chat's own copy stays, it's chat's to drop
    assert_eq!(res.images.bytes(), 28 * 28 * 4);
  }

  fn layer(name: &str, url: &str) -> Layer {
    Layer { name: name.into(), url: url.into(), scope: Scope::Channel }
  }

  fn solid(w: u32, h: u32, rgb: [u8; 3]) -> crate::images::decode::Decoded {
    let px = [rgb[0], rgb[1], rgb[2], 255].repeat((w * h) as usize);
    crate::images::decode::Decoded { w, h, frames: vec![crate::images::decode::Frame { rgba: px.into(), delay_ms: 0 }] }
  }

  #[test]
  fn a_stack_lists_every_piece_in_order_in_its_provider_color() {
    let over = [
      layer("frozen0", "https://cdn.betterttv.net/emote/a/1x.webp"),
      layer("wide", "https://cdn.frankerfacez.com/emote/9/1"),
    ];
    let t = Tip::new("Kappa", "https://cdn.7tv.app/emote/x/1x.webp", false, false, false, (0, 0), &over);
    let (text, spans) = t.compose();
    assert_eq!(text, "Kappa + frozen0 + wide");
    let ink = |i: usize| (&text[spans[i].range.clone()], spans[i].ink.0, spans[i].bold);
    assert_eq!(ink(0), ("Kappa", theme::CYAN, true));
    assert_eq!(ink(1), (" + ", theme::WHITE, false));
    assert_eq!(ink(2), ("frozen0", theme::RED, false));
    assert_eq!(ink(4), ("wide", theme::WHITE, false));
    // a lone emote is just its name
    let one = Tip::new("Kappa", "https://cdn.7tv.app/emote/x/1x.webp", false, false, false, (0, 0), &[]);
    assert_eq!(one.compose().0, "Kappa");
    // long stacks name the first eight and count the rest
    let many: Vec<Layer> = (0..9).map(|i| layer(&format!("o{i}"), "https://cdn.7tv.app/emote/o/1x.webp")).collect();
    let big = Tip::new("B", "https://cdn.7tv.app/emote/x/1x.webp", false, false, false, (0, 0), &many);
    assert!(big.compose().0.ends_with("o6 +2 more"));
    // the popup stays put only for the same stack
    assert!(t.is("Kappa", "https://cdn.7tv.app/emote/x/1x.webp", &over));
    assert!(!t.is("Kappa", "https://cdn.7tv.app/emote/x/1x.webp", &over[..1]));
  }

  #[test]
  fn a_stack_draws_the_overlay_on_the_base_and_frees_both() {
    let (mut c, mut res) = (Canvas::new(600, 440), Res::new(1.0));
    let over = [layer("frozen0", "https://cdn.7tv.app/emote/o/1x.webp")];
    let mut t = Tip::new("Kappa", "https://cdn.7tv.app/emote/x/1x.webp", false, false, false, (300, 300), &over);
    assert!(t.draw(&mut c, &mut res, bounds()).is_none());
    let wanted = res.images.take_wanted();
    assert_eq!(wanted.len(), 4, "both pieces, each with its big rung and chat's copy");
    for (id, url, h) in wanted {
      // the base is a red square the height asked for, the overlay a green one half its size.
      // `h` packs the max width above bit 16; the height is the low half
      let px = h & 0xffff;
      let img = if url.contains("/x/") { solid(px, px, [255, 0, 0]) } else { solid(px / 2, px / 2, [0, 255, 0]) };
      res.images.loaded(id, h, Ok(img));
    }
    let (r, _) = t.draw(&mut c, &mut res, bounds()).unwrap();
    let count = |rgb| {
      (r.y..r.bottom())
        .flat_map(|y| (r.x..r.right()).map(move |x| (x, y)))
        .filter(|&(x, y)| c.px[(y * 600 + x) as usize] == rgb)
        .count()
    };
    // 4x: the overlay's 56x56 sits on the base's 112x112, which shows around it
    assert_eq!(count(0x00ff00), 56 * 56);
    assert_eq!(count(0xff0000), 112 * 112 - 56 * 56);
    // the overlay is centered on the base
    let mid = (r.x + r.w / 2, r.y + 1 + PAD + 56);
    assert_eq!(c.px[(mid.1 * 600 + mid.0) as usize], 0x00ff00);
    t.release(&mut res);
    // the big rungs are gone; chat's own copies (28px base, 14px overlay) stay
    assert_eq!(res.images.bytes(), (28 * 28 + 14 * 14) * 4);
  }
}
