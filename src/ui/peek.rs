// the name hover popup (ext tooltips.js user tooltip, 'peek' variant): the same words as the user
// card (name, what heatsync knows) in a small square box beside the pointer, without actions or
// recent lines, their picture at the left (ext .hs-card-avatar: 40px, 1px border). black, 1px white
// border, like the emote popup
use super::canvas::{Canvas, Rect};
use super::card::{self, View};
use super::chat_view::blit_layer;
use super::text::Text;
use super::theme;
use super::tip::place;

const PAD: i32 = 6;
// widest the popup grows, in characters
const COLS: f32 = 44.0;

// a decoded picture: rgba, width, height
pub type Pic<'a> = (&'a [u8], u32, u32);

// paints the popup near `at` inside `bounds`; returns its rect. `side` = the picture's square size
// (0 = none: it isn't known or hasn't loaded)
pub fn draw(
  c: &mut Canvas,
  text: &mut Text,
  bounds: Rect,
  at: (i32, i32),
  v: &View,
  avatar: Option<Pic>,
  side: i32,
) -> Rect {
  let (s, spans) = card::head(v);
  let side = if avatar.is_some() { side } else { 0 };
  let left = if side > 0 { side + PAD } else { 0 };
  let max = ((text.font_px * 0.62 * COLS) as i32).min(bounds.w - 16).max(120);
  let layout = text.layout(&s, &spans, &[], (max - 2 * PAD - 2 - left) as f32);
  let (lw, lh) = (layout.full_width().ceil() as i32, layout.height().ceil() as i32);
  let (w, h) = (lw + left + 2 * PAD + 2, lh.max(side) + 2 * PAD + 2);
  let (x, y) = place(at, w, h, bounds);
  let r = Rect::new(x, y, w, h);
  c.fill(r, theme::WHITE);
  let inner = Rect::new(x + 1, y + 1, w - 2, h - 2);
  c.fill(inner, theme::BLACK);
  if let Some((rgba, pw, ph)) = avatar {
    let sq = Rect::new(inner.x + PAD, inner.y + PAD, side, side);
    // fit inside the square, its own shape kept
    let k = (side - 2) as f32 / pw.max(ph) as f32;
    let dst = (((pw as f32 * k) as i32).max(1), ((ph as f32 * k) as i32).max(1));
    blit_layer(c, rgba, (pw, ph), dst, sq, inner);
    c.outline(sq, inner, theme::WHITE);
  }
  text.draw(&layout, c, (inner.x + PAD + left) as f32, (inner.y + PAD) as f32, inner, |_, _| {});
  r
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_square_box_with_the_name_inside_the_window() {
    let mut text = Text::new(1.0);
    let mut c = Canvas::new(600, 400);
    let bounds = Rect::new(0, 20, 600, 380);
    let rows = [("age", "4y".to_owned()), ("heat", "1.2k".to_owned())];
    let v = View {
      hint: "",
      nick: "bob",
      color: theme::GREEN,
      tag: ("[T] ", theme::TWITCH),
      rows: &rows,
      note: None,
      recent: &[],
    };
    // near the right edge: pulled back in
    let r = draw(&mut c, &mut text, bounds, (590, 200), &v, None, 40);
    assert!(r.x >= 0 && r.right() <= 600 && r.y >= bounds.y && r.bottom() <= bounds.bottom());
    assert_eq!(c.px[(r.y * 600 + r.x) as usize], theme::WHITE);
    assert_eq!(c.px[((r.y + 1) * 600 + r.x + 1) as usize], theme::BLACK);
    // the name is drawn in its color
    let green = (r.y..r.bottom())
      .flat_map(|y| (r.x..r.right()).map(move |x| (x, y)))
      .any(|(x, y)| c.px[(y * 600 + x) as usize] == theme::GREEN);
    assert!(green);
  }
}
