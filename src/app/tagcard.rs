// #tags in chat (ext tag-card.js): a click opens the tag's heatsync page; resting the pointer on one
// 120ms asks heatsync about it and shows its card (posts, followers, last used, whether you follow
// it) beside the pointer. a tag heatsync doesn't know shows nothing
use std::time::{Duration, Instant};

use super::App;
use crate::model::tagcard;
use crate::net::Cmd;
use crate::ui::canvas::Rect;
use crate::ui::card::View;
use crate::ui::{peek, theme};

// #tag hover cards: what heatsync said, the open one, and when a resting pointer asks
#[derive(Default)]
pub(super) struct TagUi {
  pub(super) cards: tagcard::Cards,
  pub(super) peek: Option<TagPeek>,
  pub(super) due: Option<Instant>,
}

// ext TAG_CARD_HOVER_MS
const REST: Duration = Duration::from_millis(120);

pub(super) struct TagPeek {
  tag: Box<str>,
  // which tag on which line: a second copy is a new spot
  spot: (i64, usize),
  at: (i32, i32),
}

impl App {
  fn tag_under_pointer(&mut self) -> Option<(Box<str>, (i64, usize))> {
    if !self.tip_free() || self.picker.is_some() {
      return None;
    }
    let (x, y) = self.pointer;
    let chat = self.chat_area();
    if !chat.contains(x, y) {
      return None;
    }
    let res = &mut self.res;
    self.tabs.get_mut(self.active)?.view.tag_at(chat, x, y, res)
  }

  // the pointer moved. true = it is on a #tag (its card is up or coming)
  pub(super) fn tag_hover(&mut self) -> bool {
    let Some((tag, spot)) = self.tag_under_pointer() else {
      self.hide_tag_peek();
      return false;
    };
    if self.tag.peek.as_ref().is_some_and(|p| p.spot == spot) {
      return true;
    }
    self.hide_peek();
    if let Some(mut t) = self.tip.take() {
      t.release(&mut self.res);
      self.tip_next = None;
    }
    self.tag.due = self.tag.cards.get(&tag).is_none().then(|| Instant::now() + REST);
    self.tag.peek = Some(TagPeek { tag, spot, at: self.pointer });
    self.tip_repaint();
    true
  }

  pub(super) fn tag_due(&self) -> Option<Instant> {
    self.tag.due
  }

  // the pointer rested on it: ask
  pub(super) fn tag_fire(&mut self) {
    self.tag.due = None;
    let Some(tag) = self.tag.peek.as_ref().map(|p| p.tag.clone()) else { return };
    if self.tag.cards.want(&tag) {
      self.net.send(Cmd::TagCard(tag));
    }
  }

  pub(super) fn hide_tag_peek(&mut self) {
    self.tag.due = None;
    if self.tag.peek.take().is_some() {
      self.tip_repaint();
    }
  }

  pub(super) fn on_tag_card(&mut self, tag: Box<str>, res: Result<Option<tagcard::Card>, ()>) {
    self.tag.cards.put(&tag, res);
    if self.tag.peek.as_ref().is_some_and(|p| p.tag.eq_ignore_ascii_case(&tag)) {
      self.tip_repaint();
    }
  }

  // a click on a #tag: its heatsync page. true = spent
  pub(super) fn tag_press(&mut self) -> bool {
    let Some((tag, _)) = self.tag_under_pointer() else { return false };
    self.hide_tag_peek();
    self.open_link(&tagcard::page(&tag));
    true
  }

  pub(super) fn draw_tag_peek(&mut self) -> Option<Rect> {
    let bounds = self.chat_area();
    let p = self.tag.peek.as_ref()?;
    let card = self.tag.cards.get(&p.tag)?;
    let nick = format!("#{}", card.name);
    let v = View {
      hint: "",
      nick: &nick,
      color: theme::MAGENTA,
      pronouns: None,
      tag: ("", theme::WHITE),
      rows: &card.rows,
      note: None,
      mine: None,
      recent: &[],
    };
    Some(peek::draw(&mut self.canvas, &mut self.res.text, bounds, p.at, &v, None, 0))
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::model::message::{Message, Platform};
  use crate::net::{Net, NetEvent};
  use crate::util::time::now_ms;

  #[test]
  fn resting_on_a_tag_asks_once_and_shows_its_card() {
    let (net, mut rx) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    app.dpi = 1.0;
    app.res.set_scale(app.scale());
    app.resized(720, 400);
    let tab = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
    app.select(tab);
    let mut m = Message::system(now_ms(), Platform::Twitch, "check #rust out https://x.example/a#frag");
    m.kind = crate::model::message::Kind::Chat;
    (m.login, m.nick) = ("bob".into(), "bob".into());
    app.tabs[tab].chat().push(m);
    app.redraw();
    let area = app.chat_area();
    let mut hits = Vec::new();
    for y in (area.y..area.bottom()).step_by(2) {
      for x in (area.x..area.right()).step_by(2) {
        let res = &mut app.res;
        if let Some((t, _)) = app.tabs[tab].view.tag_at(area, x, y, res) {
          hits.push(((x, y), t));
        }
      }
    }
    assert!(!hits.is_empty() && hits.iter().all(|h| &*h.1 == "rust"), "the url's #frag is no tag");
    assert!(app.canvas.px.contains(&theme::MAGENTA), "drawn magenta");
    app.pointer = hits[0].0;
    app.tip_hover();
    assert!(app.tag.peek.is_some() && app.tag.due.is_some(), "rests first");
    app.tag_fire();
    let asks =
      std::iter::from_fn(|| rx.try_recv().ok()).filter(|c| matches!(c, Cmd::TagCard(t) if &**t == "rust")).count();
    assert_eq!(asks, 1);
    assert!(app.draw_tag_peek().is_none(), "nothing until it lands");
    let card = tagcard::card(&serde_json::json!({"name": "rust", "post_count": 3, "follower_count": 1}), now_ms());
    app.on_net(NetEvent::TagCard("rust".into(), Ok(card)));
    let r = app.draw_tag_peek().expect("the card");
    assert_eq!(app.canvas.px[(r.y * app.canvas.w + r.x) as usize], theme::WHITE, "square, 1px white border");
    // off the tag: gone; back on it: shown at once, nothing asked
    app.pointer = (area.x + 1, area.bottom() - 1);
    app.tip_hover();
    assert!(app.tag.peek.is_none());
    app.pointer = hits[0].0;
    app.tip_hover();
    assert!(app.tag.peek.is_some() && app.tag.due.is_none());
  }
}
