// the one floating menu for choices that are not about an emote or a tab: the login chooser (ctrl-l)
// and, further on, the actions on a chat line. keyboard j/k/enter/esc and the mouse both drive it
use winit::keyboard::{Key, NamedKey};

use super::App;
use super::engage::{PostCtx, engage_rows, own_rows};
use crate::cosmetics::id::PaintId;
use crate::model::filter::{self, DEFAULT_MUTE_MS};
use crate::model::message::{Kind as MsgKind, Platform};
use crate::model::modcmd::{DURATIONS, Verb};
use crate::model::tabs::Kind;
use crate::net::Cmd;
use crate::net::auth::{Me, Provider};
use crate::ui::canvas::Rect;
use crate::ui::menu::Menu;
use crate::util::time::now_ms;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Act {
  Login(Provider),
  // attach another platform to the account we're signed in as
  Link(Provider),
  // ask youtube for the extra permission to post chat as the user (opt-in, never at sign-in)
  AllowYoutubePosting,
  // ask twitch for the automod permission (the hold queue in rooms you moderate)
  AllowAutomod,
  Logout,
  // allow (true) / deny the selected automod hold
  Held(bool),
  // the room's twitch blocked terms (mods)
  BlockedTerms,
  // follow (true) / unfollow them on heatsync
  Follow(bool),
  // /warn typed for them (the reason goes after), and a twitch shoutout
  Warn,
  Shoutout,
  // a mod action on the person the menu was opened for
  Do(Verb),
  // the answers to "ban bob?"
  Yes,
  // the ban with the nth of the ban reasons (the ext's reason chips)
  Why(u8),
  No,
  Reply,
  CopyName,
  CopyMessage,
  // the message's words into the box
  Quote,
  // true = hide them in this tab, false = bring them back
  HideInTab(bool),
  // type /note for them, to edit
  Note,
  // flip hiding their heatsync paint
  Paint,
  // twitch's viewer card popout: report, gift a sub
  Viewer,
  // this tab shows only their lines
  FilterTo,
  // their archived lines, over this tab
  Logs,
  // the person's card, and the things a card (or a line menu) does to them
  Card,
  Mention,
  Whisper,
  Mute,
  Unmute,
  Block,
  Unblock,
  // feed posts: your vote, your bookmark, your own post's edit and delete (asked first), and a chat
  // line quoted into a new post
  Upvote,
  Bookmark,
  Edit,
  Delete,
  DeletePost,
  OpToFeed,
}

// the person a line menu acts on, from the line it was opened on
#[derive(Clone, Debug)]
pub struct Subject {
  pub platform: Platform,
  // twitch login / kick slug of the room
  pub chan: Box<str>,
  pub login: Box<str>,
  pub nick: Box<str>,
  pub user_id: Box<str>,
  pub msg_id: Box<str>,
  pub color: u32,
  // one of our own accounts: no mod rows
  pub own: bool,
  // the paint their name wears (may be hidden): what the hide / show paint row flips
  pub paint: Option<PaintId>,
}

// a mod action waiting for "yes"
pub struct Todo {
  pub verb: Verb,
  pub who: Vec<Subject>,
  pub reason: Option<String>,
  // the ban reasons on offer, in row order
  pub reasons: Vec<String>,
}

pub struct Pop {
  pub(super) menu: Menu<Act>,
  pub(super) subject: Option<Subject>,
  todo: Option<Todo>,
}

impl Pop {
  pub fn confirm(head: String, at: (i32, i32), todo: Todo) -> Self {
    let mut rows = vec![(Act::Yes, "yes, ban".to_owned())];
    // the first nine answer to their digit
    for (i, r) in todo.reasons.iter().enumerate() {
      let r = match r.char_indices().nth(REASON_SHOWN) {
        Some((cut, _)) => format!("{}\u{2026}", &r[..cut]),
        None => r.clone(),
      };
      rows.push((Act::Why(i as u8), if i < 9 { format!("{} {r}", i + 1) } else { r }));
    }
    rows.push((Act::No, "cancel".to_owned()));
    Pop { menu: Menu::new(rows, at).titled(head), subject: None, todo: Some(todo) }
  }

  // a question with its own rows (delete post?)
  pub fn ask(head: String, at: (i32, i32), rows: Vec<(Act, String)>) -> Self {
    Pop { menu: Menu::new(rows, at).titled(head), subject: None, todo: None }
  }
}

// how much of a long ban reason its row shows
const REASON_SHOWN: usize = 60;

// the chooser rows, plain words
fn login_rows() -> Vec<(Act, String)> {
  [Provider::Twitch, Provider::Kick, Provider::Youtube]
    .into_iter()
    .map(|p| (Act::Login(p), p.name().to_owned()))
    .collect()
}

// "logged in as mellen (kick)"
fn account_head(me: &Me) -> String {
  if me.platform.is_empty() {
    format!("logged in as {}", me.username)
  } else {
    format!("logged in as {} ({})", me.username, me.platform)
  }
}

// the signed-in menu: a link row for each platform not linked yet, the youtube posting opt-in once
// youtube is linked, the automod permission while you moderate a room without it, then log out
fn account_rows(me: &Me, mods: bool) -> Vec<(Act, String)> {
  let linked = [
    (Provider::Twitch, me.twitch.is_some()),
    (Provider::Kick, me.kick.is_some()),
    (Provider::Youtube, me.youtube.is_some()),
  ];
  let mut rows: Vec<(Act, String)> =
    linked.into_iter().filter(|(_, on)| !on).map(|(p, _)| (Act::Link(p), format!("link {}", p.name()))).collect();
  if me.youtube.is_some() {
    rows.push((Act::AllowYoutubePosting, "allow youtube posting".to_owned()));
  }
  if mods && me.twitch.is_some() && !me.has_pack("automod") {
    rows.push((Act::AllowAutomod, "allow automod holds".to_owned()));
  }
  rows.push((Act::Logout, "log out".to_owned()));
  rows
}

// what the line menu's rows depend on, so the order and the gates are one tested function
pub struct Who {
  pub platform: Platform,
  pub own: bool,
  // the room is known
  pub chan: bool,
  pub nick: String,
  pub noted: bool,
}

pub struct Ctx {
  // None: the line has no person we can act on
  pub who: Option<Who>,
  pub can_reply: bool,
  // a channel tab (not mentions)
  pub chat_tab: bool,
  // their paint: None = none, Some(hidden)
  pub paint: Option<bool>,
  // the line is a feed post and you are signed in: its rows
  pub post: Option<PostCtx>,
  // a chat line you could post to the feed
  pub op: bool,
}

// the ext's order: card, reply, op to feed, the copies, the person (filters, mod), then note, a post's
// upvote and bookmark, paint, twitch's viewer card, filter to, chat logs, and last your own post's
// edit and delete
pub fn line_rows(c: &Ctx, person: Vec<(Act, String)>) -> Vec<(Act, String)> {
  let mut rows: Vec<(Act, String)> = Vec::new();
  if c.who.is_some() {
    rows.push((Act::Card, "user card".into()));
  }
  if c.can_reply {
    rows.push((Act::Reply, "reply".into()));
  }
  if c.op {
    rows.push((Act::OpToFeed, "op to feed".into()));
  }
  rows.push((Act::CopyName, "copy name".into()));
  rows.push((Act::CopyMessage, "copy message".into()));
  rows.push((Act::Quote, "copy \u{2192} input".into()));
  rows.extend(person);
  let post = |rows: &mut Vec<(Act, String)>, f: fn(&PostCtx) -> Vec<(Act, String)>| {
    rows.extend(c.post.iter().flat_map(f));
  };
  let Some(w) = &c.who else {
    post(&mut rows, engage_rows);
    post(&mut rows, own_rows);
    return rows;
  };
  rows.push((Act::Note, if w.noted { "edit note" } else { "add note" }.into()));
  post(&mut rows, engage_rows);
  if let Some(hidden) = c.paint {
    let verb = if hidden { "show" } else { "hide" };
    rows.push((Act::Paint, format!("{verb} {}\u{2019}s paint", w.nick)));
  }
  if w.platform == Platform::Twitch && w.chan && !w.own {
    rows.push((Act::Viewer, "report / gift sub".into()));
  }
  if c.chat_tab {
    rows.push((Act::FilterTo, format!("filter to {}", w.nick)));
  }
  rows.push((Act::Logs, "chat logs".into()));
  post(&mut rows, own_rows);
  rows
}

// the twitch viewer card popout for a person in a room (ext: report, gift sub)
pub fn viewer_url(chan: &str, login: &str) -> String {
  let seg = |s: &str| crate::util::path_seg(&s.to_ascii_lowercase());
  format!("https://www.twitch.tv/popout/{}/viewercard/{}", seg(chan), seg(login))
}

impl App {
  // ctrl-l: logged in opens the account menu (who, link more, log out); logged out asks which platform
  pub(super) fn login(&mut self) {
    let b = self.body();
    let at = (b.x + b.w / 2 - 80, b.y);
    let menu = match &self.me {
      Some(me) => Menu::new(account_rows(me, !self.mod_in.is_empty()), at).titled(account_head(me)),
      None => Menu::new(login_rows(), at).titled("log in with"),
    };
    self.pop = Some(Pop { menu, subject: None, todo: None });
    self.pop_changed();
  }

  pub(super) fn pop_changed(&mut self) {
    if let Some(t) = self.tabs.get_mut(self.active) {
      t.view.invalidate();
    }
    self.bar_dirty = true;
    self.wake();
  }

  fn pop_pick(&mut self) {
    let Some(p) = self.pop.take() else { return };
    self.pop_changed();
    if let Some(act) = p.menu.chosen() {
      self.act(act, p.subject, p.todo);
    }
  }

  // what a chosen menu row does; shared by the line menu, the card and the login chooser
  pub(super) fn act(&mut self, act: Act, who: Option<Subject>, todo: Option<Todo>) {
    match (act, who, todo) {
      (Act::Login(provider), ..) => {
        self.say(format!("finish logging in with {} in your browser…", provider.name()));
        self.net.send(Cmd::Login(provider));
      }
      (Act::Link(provider), ..) => {
        self.say(format!("finish linking {} in your browser…", provider.name()));
        self.net.send(Cmd::Link(provider));
      }
      (Act::AllowYoutubePosting, ..) => {
        self.say("finish allowing youtube posting in your browser…");
        self.net.send(Cmd::AllowYoutubePosting);
      }
      (Act::AllowAutomod, ..) => {
        self.say("finish allowing automod holds in your browser…");
        self.net.send(Cmd::Relink("automod"));
      }
      (Act::BlockedTerms, Some(who), _) => self.blocked_open(who.chan),
      (Act::Follow(want), Some(who), _) => self.follow(who.platform, who.login, want),
      (Act::Warn, Some(who), _) => self.type_into_line(&format!("/warn {} ", who.login)),
      (Act::Shoutout, Some(who), _) => self.suite_run(who.chan, crate::model::modsuite::Cmd::Shoutout(who.login)),
      (Act::Held(allow), ..) => {
        if let Some(id) = self.held_selected() {
          self.held_act(&id, allow);
        }
      }
      (Act::Logout, ..) => self.net.send(Cmd::Logout),
      (Act::Do(verb), Some(who), _) => self.mod_run(verb, vec![who], None),
      (Act::Yes, _, Some(t)) => self.mod_send(t.verb, t.who, t.reason),
      (Act::Why(i), _, Some(t)) => {
        let reason = t.reasons.get(usize::from(i)).cloned().or(t.reason);
        self.mod_send(t.verb, t.who, reason)
      }
      (Act::No, ..) => self.say("cancelled"),
      (Act::Reply, ..) if self.feedish() && self.selected_row_post_id().is_some() => self.feed_reply(),
      (Act::Reply, ..) => self.reply_selected(),
      (Act::Upvote, ..) => self.feed_vote(),
      (Act::Bookmark, ..) => self.feed_bookmark(),
      (Act::Edit, ..) => self.edit_begin(),
      (Act::Delete, ..) => self.delete_ask(),
      (Act::DeletePost, ..) => self.delete_go(),
      (Act::OpToFeed, ..) => self.op_to_feed(),
      (Act::CopyName, ..) => self.copy_name(),
      (Act::CopyMessage, ..) => self.copy_selected(),
      (Act::Quote, ..) => self.quote_selected(),
      (Act::HideInTab(hide), Some(who), _) => {
        self.filter_cmd(crate::model::filtercmd::Cmd::Hide(who.login.to_string(), hide))
      }
      (Act::Note, Some(who), _) => self.note_to_line(who.platform, &who.login),
      (Act::Paint, Some(who), _) => self.paint_toggle(&who),
      (Act::Viewer, Some(who), _) => self.viewer_card(&who),
      (Act::FilterTo, Some(who), _) => self.find_for(&who.login),
      (Act::Logs, Some(who), _) => self.logs_open(&who),
      (Act::Card, Some(who), _) => self.open_card(who),
      (Act::Mention, Some(who), _) => self.type_into_line(&format!("@{} ", who.nick)),
      (Act::Whisper, Some(who), _) => self.type_into_line(&format!("/w {} ", who.login)),
      (Act::Mute, Some(who), _) => self.mute(&who.login, Some(now_ms() + DEFAULT_MUTE_MS)),
      (Act::Unmute, Some(who), _) => self.unmute(&who.login),
      (Act::Block, Some(who), _) => self.block(&who.login),
      (Act::Unblock, Some(who), _) => self.unblock(&who.login),
      _ => {}
    }
  }

  // hide or show their heatsync paint everywhere: the name wears its plain color while hidden
  fn paint_toggle(&mut self, who: &Subject) {
    let Some(id) = &who.paint else { return };
    let cos = &mut self.res.cosmetics;
    let was = cos.hidden.has(id);
    if cos.hidden.toggle(id) == was {
      return self.say("too many hidden paints");
    }
    cos.hidden.save();
    self.relayout_where(None);
    self.say(if was {
      format!("{}\u{2019}s paint is back", who.nick)
    } else {
      format!("{}\u{2019}s paint hidden", who.nick)
    });
  }

  // twitch's viewer card (report, gift a sub) in the browser, where twitch's own login lives
  fn viewer_card(&mut self, who: &Subject) {
    let url = viewer_url(&who.chan, &who.login);
    if let Err(e) = crate::util::open::open_url(&url) {
      self.say(format!("couldn't open the browser: {e}"));
    }
  }

  // start (or add to) the message with some text and put the cursor in the box
  pub(super) fn type_into_line(&mut self, text: &str) {
    self.line.insert(text);
    self.complete = None;
    self.enter_insert();
    self.input_dirty = true;
    self.wake();
  }

  // the rows for a person: the filters first, then what a moderator can do. shared by the line menu and the card
  pub(super) fn person_rows(&self, w: &Subject) -> Vec<(Act, String)> {
    let mut rows: Vec<(Act, String)> = Vec::new();
    if w.own {
      return rows;
    }
    rows.push((Act::Mention, "mention".into()));
    if w.platform == Platform::Twitch && self.me.is_some() {
      rows.push((Act::Whisper, "whisper".into()));
    }
    let key = filter::key(&w.login);
    let (muted, blocked) = filter::with(|f| (f.muted(&key, now_ms()), f.blocked(&key)));
    rows.push(if muted { (Act::Unmute, "unmute".into()) } else { (Act::Mute, "mute".into()) });
    rows.push(if blocked { (Act::Unblock, "unblock".into()) } else { (Act::Block, "block".into()) });
    if self.kind() == Kind::Chat {
      let hidden = self.tabs.get(self.active).is_some_and(|t| t.view.hidden_user(&w.login));
      rows.push(if hidden {
        (Act::HideInTab(false), "unhide in tab".into())
      } else {
        (Act::HideInTab(true), "hide in tab".into())
      });
    }
    if self.can_mod(w.platform, &w.chan) {
      if !w.msg_id.is_empty() {
        rows.push((Act::Do(Verb::Delete), "delete message".into()));
      }
      for (label, secs) in DURATIONS {
        rows.push((Act::Do(Verb::Timeout(secs)), format!("timeout {label}")));
      }
      rows.push((Act::Do(Verb::Purge), "purge".into()));
      rows.push((Act::Do(Verb::Ban), "ban".into()));
      rows.push((Act::Do(Verb::Unban), "unban".into()));
      if w.platform == Platform::Twitch {
        rows.push((Act::Warn, "warn".into()));
        rows.push((Act::Shoutout, "shoutout".into()));
      }
      rows.push((Act::BlockedTerms, "blocked terms".into()));
    }
    rows
  }

  // the menu for the selected line: its own actions, then mod actions when we moderate the room
  pub(super) fn open_line_menu(&mut self, at: (i32, i32)) {
    // a line in the chat logs has no reply and no mod rows: they would act on an old line
    if self.logs_shown().is_some() {
      return;
    }
    let Some(t) = self.tabs.get(self.active).filter(|t| matches!(t.kind, Kind::Chat | Kind::Mentions | Kind::Feed))
    else {
      return;
    };
    let Some(m) = t.view.selected().filter(|m| m.kind != MsgKind::System) else { return };
    let (chat_tab, head) = (t.kind == Kind::Chat, m.nick.to_string());
    // a feed post: its rows, and (in the feed, a thread) the reply that answers the thread. in a chat
    // tab its id is no platform message id, so there is no reply to send
    let post = self.row_post(m);
    let signed_in = self.me.is_some();
    let can_reply = match post {
      Some(_) => self.feedish() && signed_in,
      None => !m.id.is_empty() && t.kind == Kind::Chat,
    };
    let op = post.is_none() && signed_in && !m.body.is_empty();
    let post = post.and_then(|p| self.post_ctx(p));
    let who = self.subject_of(m);
    let person = who.as_ref().map(|w| self.person_rows(w)).unwrap_or_default();
    let paint = who.as_ref().and_then(|w| w.paint.as_ref()).and_then(|id| {
      let hidden = self.res.cosmetics.hidden.has(id);
      self.res.cosmetics.has_paint(id).then_some(hidden)
    });
    let ctx = Ctx {
      who: who.as_ref().map(|w| Who {
        platform: w.platform,
        own: w.own,
        chan: !w.chan.is_empty(),
        nick: w.nick.to_string(),
        noted: self.notes.get(w.platform, &w.login).is_some(),
      }),
      can_reply,
      chat_tab,
      paint,
      post,
      op,
    };
    let mut rows = line_rows(&ctx, person);
    // an automod hold: allow / deny first
    if self.held_selected().is_some() {
      rows.splice(0..0, [(Act::Held(true), "allow".to_owned()), (Act::Held(false), "deny".to_owned())]);
    }
    self.pop = Some(Pop { menu: Menu::new(rows, at).titled(head), subject: who, todo: None });
    self.pop_changed();
  }

  // right-click on a line (not an emote): select it and open its menu
  pub(super) fn line_right_click(&mut self) {
    let (x, y) = self.pointer;
    let chat = self.chat_area();
    if !chat.contains(x, y) {
      return;
    }
    let res = &mut self.res;
    if self.tabs.get_mut(self.active).is_some_and(|t| t.view.select_at(chat, y, res)) {
      self.open_line_menu((x, y));
    }
  }

  // `m` on a selected line
  pub(super) fn line_menu_key(&mut self) {
    let chat = self.chat_area();
    self.open_line_menu((chat.x + 24, chat.y + chat.h / 4));
  }

  pub(super) fn pop_key(&mut self, key: Key<&str>) {
    let Some(p) = &mut self.pop else { return };
    match key {
      Key::Character("j") | Key::Named(NamedKey::ArrowDown) => p.menu.step(1),
      Key::Character("k") | Key::Named(NamedKey::ArrowUp) => p.menu.step(-1),
      Key::Named(NamedKey::Enter) => return self.pop_pick(),
      // a ban's reasons answer to their digit
      Key::Character(d) if p.todo.is_some() && d.len() == 1 && d.as_bytes()[0].is_ascii_digit() => {
        match usize::from(d.as_bytes()[0] - b'0') {
          n @ 1..=9 if n < p.menu.items.len() && matches!(p.menu.items[n].0, Act::Why(_)) => p.menu.at = n,
          _ => return,
        }
        return self.pop_pick();
      }
      Key::Named(NamedKey::Escape) => self.pop = None,
      _ => return,
    }
    self.pop_changed();
  }

  pub(super) fn pop_click(&mut self) {
    let line = self.res.text.line_px as i32;
    let (x, y) = self.pointer;
    let row = self.pop.as_ref().and_then(|p| p.menu.row_at(x, y, line));
    match (row, &mut self.pop) {
      (Some(i), Some(p)) => {
        p.menu.at = i;
        self.pop_pick();
      }
      _ => {
        self.pop = None;
        self.pop_changed();
      }
    }
  }

  pub(super) fn pop_hover(&mut self) {
    let line = self.res.text.line_px as i32;
    let (x, y) = self.pointer;
    if let Some(p) = &mut self.pop
      && let Some(i) = p.menu.row_at(x, y, line)
      && i != p.menu.at
    {
      p.menu.at = i;
      self.wake();
    }
  }

  pub(super) fn draw_pop(&mut self) -> Option<Rect> {
    let bounds = self.menu_room();
    let p = self.pop.as_mut()?;
    Some(p.menu.draw(&mut self.canvas, &mut self.res.text, bounds))
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn me(platform: &str, twitch: bool, kick: bool, youtube: bool) -> Me {
    let n = |on: bool| on.then(|| "mellen".into());
    Me {
      platform: platform.into(),
      username: "mellen".into(),
      twitch: n(twitch),
      twitch_id: None,
      names: vec![],
      kick: n(kick),
      youtube: n(youtube),
      packs: vec![],
    }
  }

  fn labels(m: &Me) -> Vec<String> {
    account_rows(m, false).into_iter().map(|r| r.1).collect()
  }

  #[test]
  fn kick_only_account_offers_twitch_and_youtube() {
    let m = me("kick", false, true, false);
    assert_eq!(account_head(&m), "logged in as mellen (kick)");
    assert_eq!(labels(&m), ["link twitch", "link youtube", "log out"]);
    assert_eq!(account_rows(&m, false)[0].0, Act::Link(Provider::Twitch));
  }

  #[test]
  fn fully_linked_account_only_logs_out() {
    assert_eq!(labels(&me("twitch", true, true, true)), ["allow youtube posting", "log out"]);
    assert_eq!(labels(&me("twitch", true, false, true)), ["link kick", "allow youtube posting", "log out"]);
  }

  #[test]
  fn posting_is_offered_only_with_youtube_linked() {
    assert_eq!(labels(&me("kick", false, true, false)), ["link twitch", "link youtube", "log out"]);
    let m = me("youtube", false, false, true);
    assert_eq!(labels(&m), ["link twitch", "link kick", "allow youtube posting", "log out"]);
    assert_eq!(account_rows(&m, false)[2].0, Act::AllowYoutubePosting);
  }

  #[test]
  fn head_without_a_known_platform() {
    assert_eq!(account_head(&me("", true, false, false)), "logged in as mellen");
  }

  fn who(platform: Platform, own: bool, noted: bool) -> Option<Who> {
    Some(Who { platform, own, chan: true, nick: "Bob".into(), noted })
  }

  fn rows(c: &Ctx) -> Vec<String> {
    let person = vec![(Act::Mute, "mute".into()), (Act::HideInTab(true), "hide in tab".into())];
    line_rows(c, person).into_iter().map(|r| r.1).collect()
  }

  #[test]
  fn a_twitch_line_in_a_channel_tab_has_every_row_in_the_exts_order() {
    let c = Ctx {
      who: who(Platform::Twitch, false, false),
      can_reply: true,
      chat_tab: true,
      paint: Some(false),
      post: None,
      op: false,
    };
    assert_eq!(
      rows(&c),
      [
        "user card",
        "reply",
        "copy name",
        "copy message",
        "copy \u{2192} input",
        "mute",
        "hide in tab",
        "add note",
        "hide Bob\u{2019}s paint",
        "report / gift sub",
        "filter to Bob",
        "chat logs"
      ]
    );
  }

  #[test]
  fn kick_has_no_twitch_viewer_card_and_mentions_has_no_filter() {
    let c = Ctx {
      who: who(Platform::Kick, false, true),
      can_reply: false,
      chat_tab: false,
      paint: Some(true),
      post: None,
      op: false,
    };
    let r = rows(&c);
    assert!(r.contains(&"edit note".to_owned()) && r.contains(&"show Bob\u{2019}s paint".to_owned()));
    assert!(!r.iter().any(|l| l.starts_with("report") || l.starts_with("filter") || l == "reply"));
  }

  #[test]
  fn no_paint_no_paint_row_and_own_lines_keep_the_copies_and_the_note() {
    let c = Ctx {
      who: who(Platform::Twitch, true, false),
      can_reply: true,
      chat_tab: true,
      paint: None,
      post: None,
      op: false,
    };
    let r = line_rows(&c, Vec::new()).into_iter().map(|r| r.1).collect::<Vec<_>>();
    assert_eq!(
      r,
      [
        "user card",
        "reply",
        "copy name",
        "copy message",
        "copy \u{2192} input",
        "add note",
        "filter to Bob",
        "chat logs"
      ]
    );
  }

  #[test]
  fn a_line_with_no_person_still_copies() {
    let c = Ctx { who: None, can_reply: false, chat_tab: true, paint: None, post: None, op: false };
    assert_eq!(
      rows(&c),
      ["copy name", "copy message", "copy \u{2192} input", "mute", "hide in tab"],
      "no person, no logs"
    );
  }

  fn post(own: bool, voted: bool, marked: bool, left: Option<i64>) -> Option<PostCtx> {
    Some(PostCtx { own, voted, marked, left })
  }

  #[test]
  fn a_post_of_someone_else_offers_upvote_and_bookmark_in_the_exts_words() {
    let c = Ctx {
      who: None,
      can_reply: true,
      chat_tab: false,
      paint: None,
      post: post(false, false, false, None),
      op: false,
    };
    assert_eq!(rows_of(&c), ["reply", "copy name", "copy message", "copy \u{2192} input", "upvote", "bookmark"]);
    let c = Ctx { post: post(false, true, true, None), ..c };
    let r = rows_of(&c);
    assert_eq!(&r[r.len() - 2..], ["remove upvote", "remove bookmark"]);
  }

  #[test]
  fn your_own_post_has_no_upvote_but_edit_with_its_clock_and_delete_last() {
    let who = who(Platform::Twitch, true, false);
    let c = Ctx {
      who,
      can_reply: true,
      chat_tab: false,
      paint: None,
      post: post(true, false, false, Some(412_000)),
      op: false,
    };
    let r = rows_of(&c);
    assert!(!r.iter().any(|l| l.contains("upvote")));
    assert_eq!(r[r.len() - 3..], ["chat logs", "edit (6:52)", "delete"]);
    assert!(r.contains(&"bookmark".to_owned()), "you can bookmark your own");
    let c = Ctx { post: post(true, false, false, None), ..c };
    assert!(rows_of(&c).contains(&"edit (expired)".to_owned()));
  }

  #[test]
  fn logged_out_a_post_has_no_post_rows_and_a_chat_line_no_op_to_feed() {
    let c = Ctx { who: None, can_reply: false, chat_tab: false, paint: None, post: None, op: false };
    assert_eq!(rows_of(&c), ["copy name", "copy message", "copy \u{2192} input"]);
  }

  #[test]
  fn a_chat_line_offers_op_to_feed_after_reply() {
    let c = Ctx { who: None, can_reply: true, chat_tab: true, paint: None, post: None, op: true };
    assert_eq!(rows_of(&c)[..2], ["reply", "op to feed"]);
  }

  fn rows_of(c: &Ctx) -> Vec<String> {
    line_rows(c, Vec::new()).into_iter().map(|r| r.1).collect()
  }

  #[test]
  fn the_viewer_card_url_is_lowercase_and_encoded() {
    assert_eq!(viewer_url("SomeChan", "Bob_1"), "https://www.twitch.tv/popout/somechan/viewercard/bob_1");
    assert_eq!(viewer_url("a/b", "x?y"), "https://www.twitch.tv/popout/a%2Fb/viewercard/x%3Fy");
  }
}
