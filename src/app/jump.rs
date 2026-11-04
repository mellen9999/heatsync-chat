// the jump-to-bottom bar (ext #hs-mc-new-msgs): while the active chat is scrolled up, a chip at the
// bottom middle counts the lines that came in below; a click takes you down (G does too, the count
// clears itself once the view is back at the newest line). it repaints only when its words, place or
// hover change, or when something painted over it
use super::App;
use crate::ui::canvas::Rect;
use crate::ui::{jump_bar, union};

// what is on screen now: where, the count it shows, whether the pointer is on it
type Shown = (Rect, usize, bool);

#[derive(Default)]
pub(super) struct Jump {
  shown: Option<Shown>,
}

impl App {
  // the chip as it should look this frame
  fn jump_want(&mut self) -> Option<(Rect, usize)> {
    let unread = self.tabs.get(self.active)?.view.jump()?;
    let area = self.chat_area();
    let r = jump_bar::rect(&mut self.res.text, area, unread);
    (!r.is_empty()).then_some((r, unread))
  }

  // `full`: the chat was just repainted whole (the chip with it); `anim`: an emote animation repainted
  // this rect. returns what changed on the canvas
  pub(super) fn sync_jump(&mut self, full: bool, anim: Option<Rect>) -> Option<Rect> {
    let want = self.jump_want();
    let (px, py) = self.pointer;
    let new: Option<Shown> = want.map(|(r, u)| (r, u, r.contains(px, py)));
    if full {
      // the chip went with the old pixels
      self.jump.shown = None;
    }
    let old = self.jump.shown;
    let hit = old.zip(anim).is_some_and(|((r, ..), a)| !r.intersect(a).is_empty());
    if new == old && !hit {
      return None;
    }
    let mut dirty = None;
    let area = self.chat_area();
    if let Some((r, ..)) = old.filter(|o| Some(*o) != new)
      && let Some(t) = self.tabs.get_mut(self.active)
    {
      // the chat under the old chip comes back
      t.view.draw(&mut self.canvas, &mut self.res, area, r);
      dirty = Some(r);
    }
    if let Some((r, u, h)) = new {
      jump_bar::draw(&mut self.canvas, &mut self.res.text, r, u, h);
      dirty = Some(dirty.map_or(r, |d| union(d, r)));
    }
    self.jump.shown = new;
    dirty
  }

  // the chip is gone from the screen (a screen or picker covers the chat)
  pub(super) fn jump_forget(&mut self) {
    self.jump.shown = None;
  }

  // the pointer moved: the chip lights up while it is on it
  pub(super) fn jump_hover(&mut self) -> bool {
    let (x, y) = self.pointer;
    let changed = self.jump.shown.is_some_and(|(r, _, h)| r.contains(x, y) != h);
    if changed {
      self.wake();
    }
    changed
  }

  // true when the click landed on the chip
  pub(super) fn jump_click(&mut self) -> bool {
    let (x, y) = self.pointer;
    if !self.jump.shown.is_some_and(|(r, ..)| r.contains(x, y)) {
      return false;
    }
    if let Some(t) = self.tabs.get_mut(self.active) {
      t.view.scroll_to_bottom();
    }
    self.wake();
    true
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::model::message::{Message, Platform};
  use crate::net::Net;
  use crate::ui::theme;

  fn boot() -> App {
    let (net, _rx) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    app.dpi = 1.0;
    app.res.set_scale(app.scale());
    app.resized(420, 400);
    app.redraw();
    app
  }

  fn say(app: &mut App, n: usize) {
    for i in 0..n {
      let mut m = Message::system(i as i64, Platform::Twitch, format!("line {i}"));
      m.id = format!("id{i}-{}", app.tabs[app.active].view.len()).into();
      app.tabs[app.active].chat().push(m);
    }
  }

  fn scrolled_up() -> App {
    let mut app = boot();
    say(&mut app, 60);
    app.redraw();
    app.tabs[app.active].view.scroll_by(300, &mut app.res);
    app.redraw();
    app
  }

  #[test]
  fn the_bar_counts_lines_below_and_a_click_goes_down_and_clears_it() {
    let mut app = scrolled_up();
    let (r, n, h) = app.jump.shown.expect("scrolled up shows the bar");
    assert_eq!((n, h), (0, false), "nothing new yet reads resume");
    assert!(app.chat_area().intersect(r) == r, "inside the chat");
    // lines arriving below count up, and the bar repaints with the new words
    say(&mut app, 3);
    app.redraw();
    assert_eq!(app.jump.shown.map(|s| s.1), Some(3));
    say(&mut app, 2);
    app.redraw();
    assert_eq!(app.jump.shown.map(|s| s.1), Some(5));
    let r = app.jump.shown.unwrap().0;
    // nothing changed: nothing to repaint
    assert_eq!(app.sync_jump(false, None), None);
    // a click beside it does nothing; on it goes down
    app.pointer = (r.x - 5, r.y - 5);
    assert!(!app.jump_click());
    app.pointer = (r.x + r.w / 2, r.y + r.h / 2);
    assert!(app.jump_click());
    app.redraw();
    assert!(
      app.tabs[app.active].view.pinned() && app.jump.shown.is_none() && app.tabs[app.active].view.jump().is_none()
    );
    assert!(!app.jump_click(), "gone: nothing there to click");
    // scrolling up again starts a fresh count
    app.tabs[app.active].view.scroll_by(300, &mut app.res);
    app.redraw();
    assert_eq!(app.jump.shown.map(|s| s.1), Some(0));
  }

  #[test]
  fn going_down_by_key_clears_it_and_hover_goes_white() {
    let mut app = scrolled_up();
    say(&mut app, 4);
    app.redraw();
    let (r, ..) = app.jump.shown.unwrap();
    assert!(px_yellow_edge(&app, r), "yellow edge at rest");
    app.pointer = (r.x + 3, r.y + 3);
    assert!(app.jump_hover());
    app.redraw();
    assert_eq!(app.canvas.px[((r.y + 2) * app.canvas.w + r.x + 2) as usize], theme::WHITE, "hover is white");
    assert!(!app.jump_hover(), "no change, no redraw asked");
    app.tabs[app.active].view.scroll_to_bottom();
    app.redraw();
    assert!(app.jump.shown.is_none() && app.tabs[app.active].view.jump().is_none());
    let (cx, cy) = (r.x + r.w / 2, r.y + 1);
    assert_ne!(app.canvas.px[(cy * app.canvas.w + cx) as usize], theme::WHITE, "the chip is gone");
  }

  fn px_yellow_edge(app: &App, r: Rect) -> bool {
    app.canvas.px[(r.y * app.canvas.w + r.x + r.w / 2) as usize] == theme::YELLOW
  }

  #[test]
  fn a_pinned_chat_shows_no_bar() {
    let mut app = boot();
    say(&mut app, 60);
    app.redraw();
    assert!(app.jump.shown.is_none());
    assert_eq!(app.tabs[app.active].view.jump(), None);
  }
}
