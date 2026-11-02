// what floats over the chat just above the message box: the tab cycle's tooltip ("2/14 PogU · channel ·
// 7tv users"), boxed in a 1px white border on black
use super::canvas::{Canvas, Rect};
use super::text::Text;
use super::theme;

pub fn tip(c: &mut Canvas, text: &mut Text, input: Rect, s: &str) -> Rect {
  let line = text.line_px as i32;
  let l = text.layout(s, &[], &[], (input.w as f32 - 2.0 * theme::PAD_X - 2.0).max(1.0));
  let w = ((l.full_width() + 2.0 * theme::PAD_X).ceil() as i32 + 2).min(input.w);
  let r = Rect::new(input.x, input.y - line - 2, w, line + 2);
  c.fill(r, theme::WHITE);
  let inner = Rect::new(r.x + 1, r.y + 1, r.w - 2, r.h - 2);
  c.fill(inner, theme::BLACK);
  text.draw(&l, c, inner.x as f32 + theme::PAD_X, inner.y as f32, inner, |_, _| {});
  r
}
