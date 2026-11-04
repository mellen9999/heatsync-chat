// a line typed in a merged tab goes to every feed the tab sends to, like the ext's composer: ctrl-n steps all
// feeds, then each alone; alt-t, alt-k, alt-y flip one. the choice rides in the tab's synced entry
// (model::sendto). this file picks the feeds, builds each feed's copy of the line and folds the echoes back
// into one row
use super::{App, Tab};
use crate::model::message::{Message, Platform};
use crate::model::sendto::{self, Targets};
use crate::model::tabs::Kind;
use crate::net::{Cmd, Reply};
use crate::ui::theme;
use crate::util::time::now_ms;

// one piece of the box's tag: its text and ink
pub(super) type Piece = (&'static str, u32);

fn letter(p: Platform) -> (&'static str, &'static str, u32) {
  match p {
    Platform::Twitch => ("[T] ", "T", theme::TWITCH),
    Platform::Kick => ("[K] ", "K", theme::GREEN),
    Platform::Youtube => ("[Y] ", "Y", theme::RED),
  }
}

impl Tab {
  pub(super) fn linked(&self) -> Vec<Platform> {
    self.sources.iter().map(|s| s.0).collect()
  }

  // the sources a send reaches, in the tab's order
  pub(super) fn legs(&self) -> Vec<usize> {
    let on = self.send.resolve(&self.linked());
    (0..self.sources.len()).filter(|i| on.contains(&self.sources[*i].0)).collect()
  }

  // the source the box speaks as: the one last picked while it still gets the send, else the first that does
  pub(super) fn focus(&self) -> usize {
    let legs = self.legs();
    if legs.contains(&self.target) { self.target } else { legs.first().copied().unwrap_or(0) }
  }
}

// what one feed is sent: twitch takes /me itself, kick and youtube get the bare words; a reply threads on its
// own platform only, and youtube (no threading) opens its copy with the author's @name
pub(super) fn leg_line(p: Platform, text: &str, reply: Option<&Reply>) -> (String, Option<Reply>) {
  let bare = || text.strip_prefix("/me ").map_or(text, str::trim_start);
  let mine = reply.filter(|r| r.platform == p).map(|r| Reply {
    platform: p,
    id: r.id.clone(),
    nick: r.nick.clone(),
    text: r.text.clone(),
  });
  match p {
    Platform::Twitch => (text.to_owned(), mine),
    Platform::Kick => (bare().to_owned(), mine),
    Platform::Youtube => (reply.map_or_else(|| bare().to_owned(), |r| sendto::yt_reply_text(bare(), &r.nick)), None),
  }
}

impl App {
  // the feeds the box sends to
  pub(super) fn legs(&self) -> Vec<(Platform, &str)> {
    let Some(t) = self.tabs.get(self.active) else { return Vec::new() };
    t.legs().into_iter().map(|i| (t.sources[i].0, &*t.sources[i].1)).collect()
  }

  // the box's tag on a merged tab: `[T] ` for one feed, `[TK] ` for several, each letter in its colour
  pub(super) fn send_tag(&self) -> Vec<Piece> {
    let Some(t) = self.tabs.get(self.active).filter(|t| t.sources.len() > 1) else { return Vec::new() };
    let legs: Vec<Platform> = t.legs().into_iter().map(|i| t.sources[i].0).collect();
    match legs.as_slice() {
      [] => Vec::new(),
      [one] => {
        let (tag, _, ink) = letter(*one);
        vec![(tag, ink)]
      }
      many => {
        let mut v: Vec<Piece> = vec![("[", theme::WHITE)];
        v.extend(many.iter().map(|p| (letter(*p).1, letter(*p).2)));
        v.push(("] ", theme::WHITE));
        v
      }
    }
  }

  // ctrl-n: all feeds, then each alone, then all again
  pub(super) fn cycle_send(&mut self) {
    let Some(t) = self.tabs.get(self.active).filter(|t| t.kind == Kind::Chat && t.sources.len() > 1) else { return };
    let next = t.send.cycled(&t.linked());
    self.set_send(next);
  }

  // alt-t, alt-k, alt-y: that feed in or out of the send, never the last one out
  pub(super) fn toggle_send(&mut self, p: Platform) -> bool {
    let Some(t) = self.tabs.get(self.active).filter(|t| t.kind == Kind::Chat && t.sources.len() > 1) else {
      return false;
    };
    let linked = t.linked();
    if !linked.contains(&p) {
      return false;
    }
    match t.send.toggled(&linked, p) {
      Some(next) => self.set_send(next),
      None => self.say(format!("{} has to stay on: it's the only one left", p.name())),
    }
    true
  }

  fn set_send(&mut self, next: Targets) {
    let Some(t) = self.tabs.get_mut(self.active) else { return };
    t.send = next;
    t.target = t.legs().first().copied().unwrap_or(0);
    self.send_targets_up();
    self.input_dirty = true;
    self.wake();
  }

  // send `text` to each feed in `legs`, each its own copy; several feeds' echoes will fold into one row
  pub(super) fn fan_out(&mut self, legs: &[(Platform, Box<str>)], text: &str, reply: Option<Reply>) {
    for (platform, chan) in legs {
      let (text, reply) = leg_line(*platform, text, reply.as_ref());
      self.net.send(Cmd::Send { platform: *platform, chan: chan.clone(), text, reply });
    }
    if legs.len() > 1 {
      let bare = text.strip_prefix("/me ").unwrap_or(text);
      self.sentfold.track(now_ms(), bare, reply.is_some());
    }
  }

  // one of your own lines came back from `p`: true when it is another feed's copy of a send already shown
  pub(super) fn echo_folded(&mut self, p: Platform, m: &Message) -> bool {
    m.kind.said() && self.mentions.mine(m) && self.sentfold.folds(now_ms(), p, &m.body)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn reply(p: Platform) -> Reply {
    Reply { platform: p, id: "abc".into(), nick: "bob".into(), text: "hey".into() }
  }

  #[test]
  fn each_feed_gets_its_own_copy_of_a_reply() {
    let r = reply(Platform::Kick);
    let (t, rep) = leg_line(Platform::Twitch, "gg", Some(&r));
    assert_eq!((t.as_str(), rep.is_none()), ("gg", true), "twitch can't thread a kick line");
    let (t, rep) = leg_line(Platform::Kick, "gg", Some(&r));
    assert_eq!((t.as_str(), rep.map(|r| r.id)), ("gg", Some("abc".into())));
    let (t, rep) = leg_line(Platform::Youtube, "gg", Some(&r));
    assert_eq!((t.as_str(), rep.is_none()), ("@bob gg", true), "youtube has no threads: an @name");
  }

  #[test]
  fn a_me_line_is_bare_off_twitch() {
    assert_eq!(leg_line(Platform::Twitch, "/me waves", None).0, "/me waves");
    assert_eq!(leg_line(Platform::Kick, "/me waves", None).0, "waves");
    assert_eq!(leg_line(Platform::Youtube, "/me waves", None).0, "waves");
    assert_eq!(leg_line(Platform::Kick, "/mean", None).0, "/mean");
  }
}
