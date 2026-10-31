pub mod canvas;
pub mod chat_view;
pub mod input_bar;
pub mod menu;
pub mod picker;
pub mod res;
pub mod tab_bar;
pub mod text;
pub mod theme;

use canvas::Rect;

pub fn union(a: Rect, b: Rect) -> Rect {
  let x0 = a.x.min(b.x);
  let y0 = a.y.min(b.y);
  Rect::new(x0, y0, (a.x + a.w).max(b.x + b.w) - x0, a.bottom().max(b.bottom()) - y0)
}
