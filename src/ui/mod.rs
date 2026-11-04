pub mod badge_text;
pub mod canvas;
pub mod card;
pub mod chat_view;
pub mod embed;
pub mod form;
pub mod input_bar;
pub mod logo;
pub mod menu;
pub mod mode_label;
pub mod name_art;
pub mod peek;
pub mod picker;
pub mod pin_bar;
pub mod pred_bar;
pub mod res;
pub mod sheet;
pub mod suggest;
pub mod tab_bar;
pub mod text;
pub mod theme;
pub mod tip;

use canvas::Rect;

pub fn union(a: Rect, b: Rect) -> Rect {
  let x0 = a.x.min(b.x);
  let y0 = a.y.min(b.y);
  Rect::new(x0, y0, (a.x + a.w).max(b.x + b.w) - x0, a.bottom().max(b.bottom()) - y0)
}
