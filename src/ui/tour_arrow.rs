// the tour's pointer: one big yellow ▲ ▼ ◀ ▶ just outside the thing a card talks about, facing the card. placement
// is a pure function of three rects; drawing never takes input and runs every frame the tour does
use super::canvas::{Canvas, Rect};
use super::text::{Ink, Span, Text};
use super::theme;

// the glyph is this many times the body text
const SCALE: f32 = 2.5;
const GLYPHS: [char; 4] = ['▲', '▼', '◀', '▶'];

// the real things the cards describe, as the layout has them now (None = not on screen)
pub struct Anchors {
  // the newest chat line
  pub row: Option<Rect>,
  pub input: Rect,
  pub plus: Option<Rect>,
  pub gear: Option<Rect>,
}

// card `step`'s subject. the last card (the switches) points at nothing, its rows are the card itself
pub fn target(step: usize, a: &Anchors) -> Option<Rect> {
  match step {
    0 | 2 => a.row,
    1 => Some(a.input),
    3 => a.plus,
    4 => a.gear,
    _ => None,
  }
  .filter(|r| !r.is_empty())
}

fn hits(a: Rect, b: Rect) -> bool {
  !a.intersect(b).is_empty()
}

// a target just outside the window (the tab bar above the chat) is pointed at from the window's own edge
// (glyph, x, y) of a `gw`×`gh` arrow touching `target` from the side that faces `card`, else the next best side, kept
// inside `win` and off the card. None: no target, or no side has room
pub fn place(target: Rect, card: Rect, win: Rect, gw: i32, gh: i32) -> Option<(char, i32, i32)> {
  if target.is_empty() || win.is_empty() {
    return None;
  }
  let gap = (gh / 8).max(2);
  // a few glyphs of slack: the bar above the chat is outside it, a rect far away is not on screen
  let s = 3 * gh;
  if !hits(Rect::new(target.x - s, target.y - s, target.w + 2 * s, target.h + 2 * s), win) {
    return None;
  }
  let (tx, ty) = (target.x + target.w / 2, target.y + target.h / 2);
  let (dx, dy) = (card.x + card.w / 2 - tx, card.y + card.h / 2 - ty);
  // sides: 0 below (▲), 1 above (▼), 2 right (◀), 3 left (▶)
  let below = dy >= 0;
  let right = dx >= 0;
  let vert = [if below { 0 } else { 1 }, if below { 1 } else { 0 }];
  let horz = [if right { 2 } else { 3 }, if right { 3 } else { 2 }];
  let order =
    if dy.abs() >= dx.abs() { [vert[0], horz[0], horz[1], vert[1]] } else { [horz[0], vert[0], vert[1], horz[1]] };
  for side in order {
    let (x, y) = match side {
      0 => (tx - gw / 2, (target.bottom() + gap).max(win.y)),
      1 => (tx - gw / 2, (target.y - gap - gh).min(win.bottom() - gh)),
      2 => ((target.right() + gap).max(win.x), ty - gh / 2),
      _ => ((target.x - gap - gw).min(win.right() - gw), ty - gh / 2),
    };
    // along the edge it slides to stay in view, but the target must still be under it
    let (x, y) = if side < 2 {
      if target.right() <= win.x || target.x >= win.right() {
        continue;
      }
      (x.clamp(win.x, (win.right() - gw).max(win.x)), y)
    } else {
      if target.bottom() <= win.y || target.y >= win.bottom() {
        continue;
      }
      (x, y.clamp(win.y, (win.bottom() - gh).max(win.y)))
    };
    let r = Rect::new(x, y, gw, gh);
    if r.intersect(win) == r && !hits(r, card) {
      return Some((GLYPHS[side], x, y));
    }
  }
  None
}

pub fn draw(c: &mut Canvas, t: &mut Text, win: Rect, card: Rect, target: Option<Rect>) {
  let Some(target) = target else { return };
  let (font, line) = (t.font_px, t.line_px);
  t.font_px = font * SCALE;
  t.line_px = line * SCALE;
  let lay = |t: &mut Text, g: char| {
    let s = g.to_string();
    t.layout(&s, &[Span { range: 0..s.len(), ink: Ink(theme::YELLOW), bold: true }], &[], f32::MAX)
  };
  let (mut gw, mut gh) = (0, 0);
  for g in GLYPHS {
    let l = lay(t, g);
    gw = gw.max(l.full_width().ceil() as i32);
    gh = gh.max(l.height().ceil() as i32);
  }
  if let Some((g, x, y)) = place(target, card, win, gw, gh) {
    let l = lay(t, g);
    let ox = x as f32 + (gw as f32 - l.full_width()) / 2.0;
    t.draw(&l, c, ox, y as f32, win, |_, _| {});
  }
  t.font_px = font;
  t.line_px = line;
}

#[cfg(test)]
mod tests {
  use super::*;

  const WIN: Rect = Rect::new(0, 0, 400, 600);
  const G: (i32, i32) = (20, 30);

  fn at(t: Rect, card: Rect) -> Option<(char, i32, i32)> {
    place(t, card, WIN, G.0, G.1)
  }

  #[test]
  fn each_edge_faces_the_card_and_centres_on_the_target() {
    let t = Rect::new(180, 300, 40, 20);
    let (g, x, y) = at(t, Rect::new(100, 450, 200, 100)).unwrap();
    assert_eq!((g, x), ('▲', 190));
    assert!(y >= t.bottom() && y - t.bottom() <= 6);
    let (g, x, y) = at(t, Rect::new(100, 20, 200, 100)).unwrap();
    assert_eq!((g, x), ('▼', 190));
    assert!(y + G.1 <= t.y && t.y - (y + G.1) <= 6);
    let (g, x, y) = at(t, Rect::new(300, 290, 90, 40)).unwrap();
    assert_eq!((g, y), ('◀', 295));
    assert!(x >= t.right() && x - t.right() <= 6);
    let (g, x, y) = at(t, Rect::new(10, 290, 90, 40)).unwrap();
    assert_eq!((g, y), ('▶', 295));
    assert!(x + G.0 <= t.x && t.x - (x + G.0) <= 6);
  }

  #[test]
  fn it_slides_along_the_edge_to_stay_inside_the_window() {
    let (_, x, _) = at(Rect::new(0, 300, 6, 20), Rect::new(100, 550, 200, 40)).unwrap();
    assert_eq!(x, 0);
    let (_, x, _) = at(Rect::new(394, 300, 6, 20), Rect::new(100, 550, 200, 40)).unwrap();
    assert_eq!(x, 400 - G.0);
    let (_, _, y) = at(Rect::new(380, 0, 20, 8), Rect::new(10, 100, 100, 100)).unwrap();
    assert!(y >= 0);
    let (g, _, y) = at(Rect::new(300, 596, 100, 4), Rect::new(10, 100, 100, 100)).unwrap();
    assert!(g == '▼' && y + G.1 <= 600);
  }

  #[test]
  fn a_target_just_outside_the_window_is_pointed_at_from_its_edge() {
    let win = Rect::new(0, 40, 400, 560);
    let (g, x, y) = place(Rect::new(250, 0, 20, 20), Rect::new(10, 240, 380, 150), win, 20, 30).unwrap();
    assert_eq!((g, x, y), ('▲', 250, 40));
  }

  #[test]
  fn no_room_on_the_facing_side_takes_the_next_one() {
    // the target hugs the bottom with the card above it: below has no room, above is taken by the card
    let (g, ..) = at(Rect::new(20, 560, 200, 40), Rect::new(10, 200, 380, 330)).unwrap();
    assert_eq!(g, '◀');
  }

  #[test]
  fn it_never_lands_on_the_card() {
    let card = Rect::new(60, 200, 280, 160);
    for t in
      [Rect::new(0, 0, 400, 30), Rect::new(0, 570, 400, 30), Rect::new(190, 380, 20, 20), Rect::new(10, 100, 30, 30)]
    {
      if let Some((_, x, y)) = at(t, card) {
        assert!(!hits(Rect::new(x, y, G.0, G.1), card), "{t:?}");
      }
    }
  }

  #[test]
  fn nothing_to_point_at_is_no_arrow() {
    let card = Rect::new(100, 200, 200, 100);
    assert_eq!(at(Rect::new(50, 50, 0, 20), card), None);
    assert_eq!(at(Rect::new(50, 50, 20, 0), card), None);
    assert_eq!(at(Rect::new(-300, 50, 40, 20), card), None);
    assert_eq!(at(Rect::new(900, 50, 40, 20), card), None);
    assert_eq!(place(Rect::new(50, 50, 20, 20), card, Rect::new(0, 0, 0, 0), 20, 30), None);
  }

  #[test]
  fn every_card_but_the_last_has_a_target() {
    let a = Anchors {
      row: Some(Rect::new(0, 500, 400, 20)),
      input: Rect::new(0, 560, 400, 40),
      plus: Some(Rect::new(50, 0, 20, 20)),
      gear: Some(Rect::new(70, 0, 20, 20)),
    };
    let n = crate::model::tour::CARDS.len();
    for step in 0..n - 1 {
      assert!(target(step, &a).is_some(), "card {step}");
    }
    assert_eq!(target(n - 1, &a), None);
    let none = Anchors { row: None, plus: None, gear: None, ..a };
    assert_eq!(target(0, &none), None);
    assert!(target(1, &none).is_some());
  }
}
