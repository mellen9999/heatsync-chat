// a closed overlay leaves nothing behind: the strips it sat over (the filter line, the pinned and
// prediction bars) repaint on the next frame, so no row of its text stays on screen
use super::*;
use crate::model::settings::Side;

fn boot(side: Side, h: i32) -> App {
  let (net, _asked) = Net::tap();
  let mut app = App::new(net, vec!["forsen".into()], false);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.canvas.resize(640, h);
  app.set.tab_side = side;
  app.relayout_body();
  let chat = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
  app.select(chat);
  for _ in 0..3 {
    app.redraw();
  }
  app
}

fn pixels(app: &App, r: Rect) -> Vec<u32> {
  let c = &app.canvas;
  (r.y..r.bottom()).flat_map(|y| (r.x..r.right()).map(move |x| c.px[(y * c.w + x) as usize])).collect()
}

// the filter line is on screen; the overlay opens over it and closes again; the strip reads as before
fn closes_clean(side: Side, h: i32, open: fn(&mut App), close: fn(&mut App)) {
  let mut app = boot(side, h);
  let strip = app.find_area();
  assert!(strip.h > 0, "the filter line is shown");
  let before = pixels(&app, strip);
  open(&mut app);
  app.redraw();
  assert!(pixels(&app, strip) != before, "the overlay covers the filter line");
  close(&mut app);
  app.redraw();
  assert!(pixels(&app, strip) == before, "the filter line repaints once the overlay is gone");
}

fn open_tab_menu(app: &mut App) {
  let p = app.bar_hits[0];
  app.pointer = (p.x + p.w / 2, p.y + p.h / 2);
  app.right_click();
  assert!(app.tab_menu().is_some(), "the menu is open");
}

fn right_click_away(app: &mut App) {
  app.right_click();
}

#[test]
fn a_second_right_click_on_a_tab_leaves_no_menu_on_the_filter_line_bar_on_top() {
  closes_clean(Side::Top, 230, open_tab_menu, right_click_away);
}

#[test]
fn a_second_right_click_on_a_tab_leaves_no_menu_on_the_filter_line_bar_at_the_bottom() {
  closes_clean(Side::Bottom, 80, open_tab_menu, right_click_away);
}

#[test]
fn a_closed_account_menu_leaves_no_row_on_the_filter_line() {
  closes_clean(Side::Top, 230, |a| a.login(), right_click_away);
}
