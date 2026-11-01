// the mentions tab (ext mentions.js): every live line that pings you, from every joined room, plus
// replies to you. in the room's own tab the line gets a red edge (the ext tints the row dark red; a
// fill behind text is off-palette here). click or Enter jumps to the line in its room's tab
use std::sync::atomic::{AtomicBool, Ordering};

use super::App;
use crate::model::message::{Extra, Message, Platform, flag};
use crate::model::tabs::Kind;

// notification body cap (ext: 200 chars then "...")
const BODY_MAX: usize = 200;

impl App {
  // names that ping you changed (login / logout)
  pub(super) fn set_mention_names(&mut self) {
    self.mentions.set(self.me.iter().flat_map(|m| m.names.iter().map(|n| &**n)));
  }

  // tint a line that pings you; a live one also lands in the mentions tab (and may notify).
  // `room` is the tab key it came from (twitch channel, kick slug, youtube handle)
  pub(super) fn scan_mention(&mut self, p: Platform, room: &str, m: &mut Message) {
    // a blocked person never pings you
    if !self.mentions.hit(m) || crate::model::filter::with(|f| f.blocked(&crate::model::filter::key(&m.login))) {
      return;
    }
    m.flags |= flag::ALERT;
    if m.has(flag::HISTORY) {
      return;
    }
    let mut copy = m.clone();
    copy.x = Some(Box::new(Extra { chan: room.into(), ..Extra::default() }));
    copy.platform = p;
    // a copy of a line is not a new person: the first-line glow never applies here
    copy.flags &= !(flag::NEW | flag::FIRST);
    if let Some(i) = self.tab_of(Kind::Mentions) {
      self.tabs[i].view.push(copy);
      self.unseen_note(crate::model::unseen::Surface::Mentions);
    }
    if self.set.notify && !self.focused {
      notify(&m.nick, room, &m.body);
    }
  }

  // Enter / click on a mention: open its room's tab with the line selected
  pub(super) fn jump_to_mention(&mut self) {
    let Some(t) = self.tabs.get(self.active).filter(|t| t.kind == Kind::Mentions) else { return };
    let Some(m) = t.view.selected() else { return };
    let (p, id) = (m.platform, m.id.clone());
    let Some(room) = m.x.as_ref().map(|x| x.chan.clone()) else { return };
    let Some(i) = self.tabs_with(p, &room).first().copied() else {
      return self.say(format!("{room} is not open as a tab"));
    };
    self.select(i);
    let h = self.chat_area().h;
    let res = &mut self.res;
    if !self.tabs[i].view.select_id(&id, res, h) {
      self.say("that line has scrolled out of its tab");
    }
    self.wake();
  }
}

// escape what a notification daemon may read as markup; control bytes become spaces
fn clean(s: &str, max: usize) -> String {
  let mut out = String::with_capacity(s.len().min(max * 2));
  for (n, c) in s.chars().enumerate() {
    if n == max {
      out.push_str("...");
      break;
    }
    match c {
      '&' => out.push_str("&amp;"),
      '<' => out.push_str("&lt;"),
      '>' => out.push_str("&gt;"),
      c if c.is_control() => out.push(' '),
      c => out.push(c),
    }
  }
  out
}

// best effort: no notify-send is not an error, said once on stderr
fn notify(user: &str, room: &str, body: &str) {
  static MISSING: AtomicBool = AtomicBool::new(false);
  let title = format!("{} in #{}", clean(user, 40), clean(room, 40));
  let spawned = std::process::Command::new("notify-send")
    .args(["-a", "heatsync", "-t", "8000", "--"])
    .arg(title)
    .arg(clean(body, BODY_MAX))
    .stdin(std::process::Stdio::null())
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::null())
    .spawn();
  match spawned {
    // reap it so a long session leaves no zombies
    Ok(mut child) => {
      std::thread::spawn(move || {
        let _ = child.wait();
      });
    }
    Err(e) => {
      if !MISSING.swap(true, Ordering::Relaxed) {
        eprintln!("mention notifications need notify-send: {e}");
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn notification_text_is_escaped_and_capped() {
    assert_eq!(clean("a<b>&\n\u{7}c", 50), "a&lt;b&gt;&amp;  c");
    let long = "x".repeat(300);
    let c = clean(&long, BODY_MAX);
    assert_eq!(c.len(), BODY_MAX + 3);
    assert!(c.ends_with("..."));
    assert_eq!(clean("-h", 10), "-h");
  }
}
