// reaction chips under feed posts (ext feed-engage.js feedReactionToggle + the live thread frames): a click on
// a chip adds your reaction or takes it back, shown at once and put back if the server says no. while a thread
// is open its room is joined, so other people's reactions and votes arrive as they happen
use super::App;
use super::feed::{LOGIN_FIRST, TRY_AGAIN};
use crate::model::reaction::Frame;
use crate::net::hs_ws::Cmd as HsCmd;
use crate::net::{Cmd, social};
use std::time::{Duration, Instant};

// how long a heat redraw waits for more frames (the ext's 500ms)
const HEAT_BEAT: Duration = Duration::from_millis(500);

impl App {
  // a click on a chip. true = the click was spent
  pub(super) fn chip_press(&mut self) -> bool {
    let (x, y) = self.pointer;
    let chat = self.chat_area();
    let res = &mut self.res;
    let Some((id, emote)) = self.tabs.get_mut(self.active).and_then(|t| t.view.chip_at(chat, x, y, res)) else {
      return false;
    };
    self.react_toggle(id, emote);
    true
  }

  pub(super) fn react_toggle(&mut self, id: Box<str>, emote: u64) {
    if self.me.is_none() {
      return self.say(LOGIN_FIRST);
    }
    let Some((on, _)) = self.feed.react_flip(&id, emote) else { return };
    self.net.send(Cmd::Social(social::Cmd::React { id: id.clone(), emote, on }));
    self.react_shown(&id);
  }

  // a post's chips changed: redraw it wherever it shows
  fn react_shown(&mut self, id: &str) {
    self.refresh_inline(id);
    self.feed_show();
  }

  pub(super) fn on_reacted(&mut self, id: Box<str>, emote: u64, on: bool, res: Result<bool, u16>) {
    let Some(chip) = self.feed.react_end(&id, emote) else { return };
    let mut why = None;
    match res {
      // the server already had it: the count we were given held you
      Ok(false) if on => self.feed.react_dup(&id, emote),
      Ok(_) => {}
      // the press showed at once: a refusal puts it back
      Err(status) => {
        self.feed.react_set(&id, &chip, !on);
        why = Some(if status == 401 { LOGIN_FIRST } else { TRY_AGAIN });
      }
    }
    self.react_shown(&id);
    // after the redraw: the feed tab reloads its rows and would wipe the line
    if let Some(why) = why {
      self.say(why);
    }
  }

  // someone else's reaction, live. your own is already on screen from the press
  pub(super) fn on_react_frame(&mut self, f: Frame) {
    if self.hs_uid.as_deref().is_some_and(|me| !f.user.is_empty() && *me == *f.user) {
      return;
    }
    if self.feed.react_frame(&f) {
      self.react_shown(&f.post);
    }
  }

  // a post's heat moved: a vote on the open thread, or the site's heat beat (`heat:updated`, up to 100 posts a
  // tick for every client). the number is kept at once; the redraw waits one beat from the first change, so a
  // run of frames is one redraw (ext feedHeatRepaintSoon), and a post we do not show costs nothing
  pub(super) fn on_post_heat(&mut self, id: Box<str>, heat: u32) {
    if self.feed.heat_set(&id, heat) && self.feed_heat_due.is_none() {
      self.feed_heat_due = Some(Instant::now() + HEAT_BEAT);
    }
  }

  pub(super) fn feed_heat_tick(&mut self) {
    self.feed_heat_due = None;
    self.feed_show();
  }

  // the room of the thread on screen (None = no thread)
  pub(super) fn thread_room(&mut self, root: Option<Box<str>>) {
    self.net.send(Cmd::Hs(HsCmd::Thread(root)));
  }
}
