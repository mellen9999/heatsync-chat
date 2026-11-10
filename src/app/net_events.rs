// what the net hands the app: chat lines and relay frames land in their tabs, and every NetEvent has its answer here

use super::{App, FLOOD_TAB, Tab};
use crate::cosmetics::id::PaintId;
use crate::i18n::{f, t};
use crate::model::message::{Message, Platform, flag};
use crate::model::origin::RACE_MS;
use crate::model::settings::Counter;
use crate::model::whisper;
use crate::net::hs_ws::Cmd as HsCmd;
use crate::net::irc::Chat;
use crate::net::{Cmd, NetEvent};
use crate::ui::chat_view::ChatView;
use crate::ui::theme;
use crate::util::time::now_ms;
use std::time::{Duration, Instant};

// your twitch id goes to 7tv (to find your 7tv user, for your own paint and badge) only while 7tv paints +
// badges are on: with them off there is nothing to look up, so nothing leaves (docs/PRIVACY.md)
pub(super) fn seventv_id(twitch_id: Option<u64>, paints: bool) -> Option<u64> {
  twitch_id.filter(|_| paints)
}

impl App {
  pub(super) fn pace_next(&mut self) {
    let now = now_ms();
    self.pace_due = self.pacer.next().map(|t| Instant::now() + Duration::from_millis((t - now).max(0) as u64));
  }

  pub(super) fn pace_tick(&mut self) {
    for (key, msgs) in self.pacer.due(now_ms()) {
      self.relay_show(Platform::Youtube, &key, msgs, false);
    }
    self.pace_next();
  }

  // a chat message landed in tab `i`: its counter moves (the bar repaints when the number shown does)
  pub(super) fn bump_count(&mut self, i: usize) {
    let active = i == self.active;
    self.tabs[i].count.bump(active, now_ms() / 1000);
    match self.set.tab_counter {
      Counter::Unread if !active => self.bar_dirty = true,
      Counter::Heat if self.heat_due.is_none() => self.bar_dirty = true,
      _ => {}
    }
  }

  pub(super) fn on_relay(&mut self, p: Platform, key: &str, mut msgs: Vec<Message>, history: bool) {
    if p == Platform::Kick && !history {
      self.kick_role_seen(key, &msgs);
    }
    // your own line sent to several feeds comes back from each: one shows, the other feeds' copies fold into it
    if !history {
      let (sent, mine) = (&mut self.sentfold, &self.mentions);
      let now = now_ms();
      msgs.retain(|m| !(m.kind.said() && mine.mine(m) && sent.folds(now, p, &m.body)));
    }
    for m in &mut msgs {
      if history {
        m.flags |= flag::HISTORY;
      }
      self.res.chatters.heard(m);
      self.scan_mention(p, key, m);
      self.hs_origin(m);
    }
    // youtube's batches drip out at their real pace (model::pace); the rest waits for pace_due
    if p == Platform::Youtube && !history {
      msgs = self.pacer.push(key, msgs, now_ms());
      self.pace_next();
    }
    self.relay_show(p, key, msgs, history);
  }

  // your own line, sent through heatsync.org: tagged [H]
  fn hs_origin(&self, m: &mut Message) {
    if m.kind.said() && self.mentions.mine(m) && self.origins.sent(now_ms(), &m.body) {
      m.extra().origin = true;
    }
  }

  // heatsync.org says you sent this: remember it, and tag an echo that already landed
  fn on_origin(&mut self, text: Box<str>) {
    let now = now_ms();
    let me = &self.mentions;
    let mut any = false;
    // the feed whose echo already landed (the site's send can go to several feeds, ext retroFoldOwnEchoes)
    let landed = std::cell::Cell::new(None);
    for v in self.tabs.iter_mut().flat_map(Tab::views) {
      any |= v.mark_origin(|m| {
        let hit = m.kind.said() && now - m.ts < RACE_MS && me.mine(m) && m.body.trim() == text.trim();
        if hit {
          landed.set(Some(m.platform));
        }
        hit
      });
    }
    // the other feeds' copies of this one send fold into the line already shown (model::sentfold)
    self.sentfold.track(now, &text, false);
    if let Some(p) = landed.get() {
      self.sentfold.folds(now, p, &text);
    }
    self.origins.note(now, text);
    if any {
      self.wake();
    }
  }

  // lines for every tab carrying that channel: history at the top, live at the bottom
  pub(super) fn relay_show(&mut self, p: Platform, key: &str, msgs: Vec<Message>, history: bool) {
    if msgs.is_empty() {
      return;
    }
    if !history {
      for m in &msgs {
        self.logs_live(p, key, m);
        self.chatlog.log(p, key, m);
      }
    }
    for i in self.tabs_with(p, key) {
      let v = self.tabs[i].chat();
      if history {
        v.prepend(msgs.clone());
        if i == self.active {
          self.wake();
        }
      } else {
        for m in &msgs {
          v.push(m.clone());
        }
        // lines of a platform this tab hides still count for the stream's stats, but move nothing else
        let mut loud = false;
        for m in &msgs {
          self.tally(i, key, m);
          if !self.plat_quiet(i, m) {
            loud = true;
            self.bump_count(i);
          }
          self.chain_live(i, m);
        }
        if loud {
          self.touched(i);
        }
      }
    }
  }

  // each tab's chat key draws on its kick channel's emotes and sub art, rebuilt from the tabs whenever a
  // tab's kick leg or a kick channel's sets change
  pub(super) fn route_kick(&mut self) {
    fn kick(t: &Tab) -> Option<(&str, &str)> {
      Some((&t.view.channel, &t.sources.iter().find(|s| s.0 == Platform::Kick)?.1))
    }
    self.res.emotes.route_kick(self.tabs.iter().filter_map(kick));
    self.res.badges.route_kick(self.tabs.iter().filter_map(kick));
  }

  pub(super) fn on_links(&mut self, tab: &str, kick: Option<Box<str>>, yt: Option<(Box<str>, Option<Box<str>>)>) {
    let Some(i) = self.tabs_with(Platform::Twitch, tab).first().copied() else { return };
    let mut add = Vec::new();
    if let Some(slug) = kick {
      self.net.send(Cmd::Hs(HsCmd::Join(Platform::Kick, slug.clone())));
      self.net.send(Cmd::Backfill(Platform::Kick, slug.clone(), slug.clone()));
      add.push((Platform::Kick, slug));
    }
    if let Some((handle, ucid)) = yt {
      self.net.send(Cmd::Hs(HsCmd::YtSub(handle.clone())));
      if let Some(uc) = ucid {
        self.net.send(Cmd::Backfill(Platform::Youtube, handle.clone(), uc));
      }
      add.push((Platform::Youtube, handle));
    }
    let t = &mut self.tabs[i];
    for src in add {
      if !t.sources.contains(&src) {
        t.sources.push(src);
      }
    }
    let tags = t.sources.len() > 1;
    self.route_kick();
    self.live_soon();
    let t = &mut self.tabs[i];
    let v = t.chat();
    v.tags = tags;
    v.relayout();
    self.input_dirty = true;
    if i == self.active {
      self.wake();
    }
  }

  pub(super) fn on_chat(&mut self, ch: &str, ev: Chat) {
    let ev = match ev {
      Chat::Msg(mut m) if ch != FLOOD_TAB => {
        if self.echo_folded(Platform::Twitch, &m) {
          return;
        }
        if self.res.chatters.heard(&m) {
          for v in self.tabs.iter_mut().flat_map(Tab::views) {
            v.adopt_id(&m.login, &m.user_id);
          }
        }
        if let Some(refs) = self.late_refs.take(ch, &m.id) {
          crate::net::relay::merge_hs(&mut m.hs, &refs);
        }
        self.scan_mention(Platform::Twitch, ch, &mut m);
        self.hs_origin(&mut m);
        self.logs_live(Platform::Twitch, ch, &m);
        self.chatlog.log(Platform::Twitch, ch, &m);
        Chat::Msg(m)
      }
      ev => ev,
    };
    let i = if ch == FLOOD_TAB {
      self.tabs.iter().position(|t| &*t.name == FLOOD_TAB)
    } else {
      self.tabs_with(Platform::Twitch, ch).first().copied()
    };
    let Some(i) = i else { return };
    if let Chat::Msg(m) = &ev {
      self.tally(i, ch, m);
    }
    let tab = self.tabs[i].chat();
    match ev {
      Chat::Msg(m) => {
        let live = self.chain.is_some().then(|| m.clone());
        let quiet = self.plat_quiet(i, &m);
        self.tabs[i].chat().push(m);
        if quiet {
          return;
        }
        self.bump_count(i);
        if let Some(m) = live {
          self.chain_live(i, &m);
        }
      }
      Chat::ClearMsg(id, login) => {
        tab.clear_msg(&id);
        let row = removed(Platform::Twitch, Removed::Line(&login));
        self.chatlog.log(Platform::Twitch, ch, &row);
        tab.push(row);
        let said = if login.is_empty() {
          t("app.net.deleted_a_message").to_owned()
        } else {
          f("app.net.deleted_message_of", &[("login", &login)])
        };
        self.mod_log(Platform::Twitch, ch, said);
      }
      Chat::ClearUser { login, secs } => {
        tab.clear_user(&login);
        let row = removed(Platform::Twitch, Removed::User(&login, secs));
        self.mod_log(Platform::Twitch, ch, row.body.to_string());
        self.chatlog.log(Platform::Twitch, ch, &row);
        let fold = self.set.collapse_timeouts;
        self.tabs[i].chat().push_act(row, fold);
        self.lock_clearchat(ch, &login, secs);
      }
      Chat::ClearAll => {
        let row = removed(Platform::Twitch, Removed::All);
        self.chatlog.log(Platform::Twitch, ch, &row);
        tab.push(row);
      }
      Chat::Room { id, modes } => {
        self.modes_seen(ch, modes);
        if self.res.rooms.seat(&id, ch) {
          for v in self.tabs.iter_mut().flat_map(Tab::views) {
            v.touch_shared(&id);
          }
        }
        if !self.rooms.iter().any(|r| *r.0 == *ch) {
          self.rooms.push((ch.into(), id.clone()));
          self.net.send(Cmd::Room(ch.into(), id));
          self.sync_personal();
          self.held_sweep();
        }
        return;
      }
      Chat::Mod(on) => return self.on_mod_state(ch, on),
      Chat::Lock(c) => return self.lock_told(Platform::Twitch, ch, Some(c)),
      Chat::Sent => return self.lock_told(Platform::Twitch, ch, None),
    }
    if i == self.active {
      self.wake();
    } else if !self.tabs[i].unread {
      self.tabs[i].unread = true;
      self.bar_dirty = true;
      self.wake();
    }
  }

  // every net event lands here; the window handler and the headless live probe share it
  pub(super) fn on_net(&mut self, ev: NetEvent) {
    match ev {
      NetEvent::Chat(ch, c) => self.on_chat(&ch, c),
      NetEvent::Backfill(ch, events) => {
        let Some(i) = self.tabs.iter().position(|t| t.name == ch) else {
          return;
        };
        let mut history: Vec<Message> = events
          .into_iter()
          .filter_map(|e| match e {
            Chat::Msg(m) => Some(m),
            _ => None,
          })
          .collect();
        // every line first, so an older line without an id (heatsync's archived part) can take
        // the one a newer line from the same person carries
        for m in &history {
          self.res.chatters.heard(m);
        }
        for m in &mut history {
          m.flags |= flag::HISTORY;
          if m.user_id.is_empty()
            && let Some(id) = self.res.chatters.twitch_id(&m.login)
          {
            m.user_id = id.to_string().into();
          }
          self.scan_mention(Platform::Twitch, &ch, m);
        }
        self.tabs[i].chat().prepend(history);
        self.ask_went_live(&ch);
        self.trim_soon();
        if i == self.active {
          self.wake();
        }
      }
      NetEvent::ModDone(d) => self.on_mod_done(*d),
      NetEvent::Clip { login, res } => self.on_clip(&login, res),
      NetEvent::NukeDone(d) => self.on_nuke_done(*d),
      NetEvent::PinDone(d) => self.on_pin_done(*d),
      NetEvent::Card { platform, login, res } => self.on_card(platform, &login, res),
      NetEvent::Pronouns(uid, res) => self.on_pronouns(uid, res),
      NetEvent::Held(h) => self.on_held(*h),
      NetEvent::HeldEnd { chan, id, end } => self.on_held_end(chan, id, end),
      NetEvent::HeldWatched { chan, res } => self.on_held_watched(chan, res),
      NetEvent::HeldActed { chan, id, allow, res } => self.on_held_acted(chan, id, allow, res),
      NetEvent::Blocked { room, res } => self.on_blocked(room, res),
      // an answer that outlived its login must not show the next one
      NetEvent::Following(_) | NetEvent::FollowSnapshot(_) | NetEvent::FollowChange(_) if self.me.is_none() => {}
      NetEvent::Following(res) => self.on_following(res),
      NetEvent::Import(_) if self.me.is_none() => {}
      NetEvent::Import(res) => self.on_import(res),
      NetEvent::LiveDots { twitch, kick } => {
        self.on_live_dots([(Platform::Twitch, twitch.0, twitch.1), (Platform::Kick, kick.0, kick.1)])
      }
      NetEvent::FollowSnapshot(rows) => self.on_follow_snapshot(rows),
      NetEvent::FollowChange(c) => self.on_follow_change(c),
      NetEvent::ModesDone { chan, set, res } => self.on_modes_done(chan, set, res),
      NetEvent::StreamInfo { chan, info } => self.on_stream_info(chan, info),
      NetEvent::Pred { chan, res } => self.on_pred(chan, res),
      NetEvent::Suite { chan, cmd, res } => self.on_suite(chan, cmd, res),
      NetEvent::Chan { chan, cmd, res } => self.on_chan_tools(chan, cmd, res),
      NetEvent::Tools { chan, res } => self.on_tools(chan, res),
      NetEvent::Followed { platform, login, want, res } => self.on_followed(platform, login, want, res),
      NetEvent::TagCard(tag, res) => self.on_tag_card(tag, res),
      NetEvent::NoteSaved { platform, login, text, ok } => self.note_saved(platform, &login, &text, ok),
      // an answer that outlived its login (logout raced the fetch) must not seed the next state
      NetEvent::Lists(_) | NetEvent::BlockedHashes(_) if self.me.is_none() => {}
      NetEvent::Lists(l) => self.on_lists(l),
      NetEvent::MentionRules(res) => self.on_mrules(res),
      NetEvent::RuleHit(h) => self.on_rule_hit(h),
      NetEvent::MuteSync(s) => self.on_mute_sync(s),
      NetEvent::ProfileColor(_)
      | NetEvent::PostHidden(_)
      | NetEvent::PostEdited(_)
      | NetEvent::PostMeta(_)
      | NetEvent::PostModerated(..)
      | NetEvent::Bookmark(..)
      | NetEvent::Unread(_)
      | NetEvent::PlusMoved
      | NetEvent::ContentPrefs(_)
      | NetEvent::Refetch(_) => self.on_pushed(ev),
      NetEvent::Whispers { items, live, epoch } => {
        if whisper::current(epoch, self.whisper.epoch) {
          self.on_whispers(items, live)
        }
      }
      NetEvent::WhisperSent { key, res } => self.on_whisper_sent(key, res),
      NetEvent::DmStored { key, id } => self.on_dm_stored(key, &id),
      NetEvent::DmDeleted { id, res } => self.on_dm_deleted(&id, res),
      NetEvent::DmGone(key) => self.on_dm_gone(&key),
      NetEvent::Feed(p) => self.on_feed(p),
      NetEvent::FeedFailed { status } => self.on_feed_failed(status),
      NetEvent::Thread { id, res } => self.on_thread(id, res),
      NetEvent::Update(r) => self.on_update(r),
      NetEvent::Updated(res) => self.on_updated(res),
      NetEvent::Media { id, res } => {
        self.feed.media_loaded(&id, res);
        self.feed_show();
      }
      NetEvent::Seen { surface, at } => self.on_seen(surface, at),
      NetEvent::Notif(it) => self.on_notif(it),
      // an answer that outlived its login must not seed the next one
      NetEvent::UiState(_) if self.me.is_none() => {}
      NetEvent::UiState(st) => self.on_ui_state(st),
      NetEvent::TabSync(_) if self.me.is_none() => {}
      NetEvent::TabSync(f) => self.on_tab_sync(f),
      NetEvent::HsAuth(on) => {
        self.hs_authed = on;
        if !on {
          self.hs_uid = None;
        }
        if on {
          self.ui_state_fetch();
          self.held_reconnected();
          self.tab_sync_ready();
        }
      }
      NetEvent::Voted { id, res } => self.on_voted(id, res),
      NetEvent::Reacted { id, emote, on, res } => self.on_reacted(id, emote, on, res),
      NetEvent::Reaction(f) => self.on_react_frame(f),
      NetEvent::PostHeat { id, heat } => self.on_post_heat(id, heat),
      NetEvent::HsUser(id) => self.hs_uid = Some(id),
      NetEvent::Posted { res } => self.on_posted(res),
      NetEvent::Bookmarked { id, on, res } => self.on_bookmarked(id, on, res),
      NetEvent::Marks { ids, res } => self.on_marks(ids, res),
      NetEvent::Edited { id, res } => self.on_edited(id, res),
      NetEvent::Deleted { id, res } => self.on_deleted(id, res),
      NetEvent::Logs { epoch, res } => self.on_logs(epoch, res),
      NetEvent::Gifs { epoch, q, res } => self.on_gifs(epoch, q, res),
      NetEvent::Catalog { epoch, stv, ffz } => self.on_catalog(epoch, stv, ffz),
      NetEvent::PickerSearch { epoch, pages } => self.on_picker_search(epoch, pages),
      NetEvent::FeedLive(p) => self.on_feed_live(p),
      NetEvent::Pin { platform, key, pin } => self.on_pin(platform, &key, pin),
      NetEvent::Stream(l) => self.on_stream(l),
      NetEvent::StreamHistory(rows) => self.on_stream_history(rows),
      NetEvent::Origin(text) => self.on_origin(text),
      NetEvent::FollowColors(c) => self.on_follow_colors(c),
      NetEvent::Moment { platform, key, text } => self.on_moment(platform, &key, &text),
      NetEvent::Emotes(ch, p, set) => {
        self.res.emotes.set(ch.as_deref(), p, set);
        self.trim_soon();
        self.relayout_where(ch.as_deref());
      }
      NetEvent::Preview(url, p) => {
        if self.res.previews.answered(&url, p) {
          for t in &mut self.tabs {
            t.views().for_each(ChatView::relayout);
          }
          self.wake();
        }
      }
      NetEvent::Image(id, h, res) => {
        // a chip's picture may be the one that arrived
        self.input_dirty |= !self.line.chips.is_empty();
        if self.res.images.loaded(id, h, res) {
          if let Some(t) = self.tabs.get_mut(self.active) {
            t.view.relayout();
          }
        } else if let Some(t) = self.tabs.get_mut(self.active) {
          t.view.invalidate();
        }
        self.wake();
      }
      NetEvent::TwitchBadges(ch, set) => {
        self.res.badges.set_twitch(ch.as_deref(), set);
        self.relayout_where(ch.as_deref());
      }
      NetEvent::KickBadges(slug, art) => {
        self.res.badges.set_kick(&slug, art);
        self.route_kick();
        for i in self.tabs_with(Platform::Kick, &slug) {
          self.tabs[i].views().for_each(ChatView::relayout);
        }
        self.wake();
      }
      NetEvent::KickEmotes(slug, set) => {
        self.res.emotes.set_kick(&slug, set);
        self.route_kick();
        for i in self.tabs_with(Platform::Kick, &slug) {
          self.tabs[i].views().for_each(ChatView::relayout);
        }
        self.input_dirty = true;
        self.wake();
      }
      NetEvent::FfzRoomBadges(ch, set) => {
        let had = !set.is_empty();
        self.res.badges.set_ffz_room(&ch, set);
        if had {
          self.relayout_where(Some(&ch));
        }
      }
      NetEvent::UserBadges(src, set) => {
        self.res.badges.set_users(src, set);
        self.relayout_where(None);
        self.trim_soon();
      }
      NetEvent::Senders(sets) => {
        if self.prof {
          eprintln!("senders {} owning {}", sets.len(), sets.iter().filter(|(_, s)| !s.is_empty()).count());
        }
        for (id, set) in sets {
          if self.res.senders.put(id, set) {
            self.touch_sender(id);
          }
        }
        self.trim_soon();
      }
      NetEvent::Me(me) => {
        // our own inventory feeds tab-complete: fetch it now, not on first sight of our own line
        self.net.send(Cmd::SevenTvMe(seventv_id(me.as_ref().and_then(|m| m.twitch_id), self.set.paints)));
        self.res.senders.set_me(me.as_ref().and_then(|m| m.twitch_id));
        if let Some(id) = me.as_ref().and_then(|m| m.twitch_id) {
          self.res.senders.lookup(id, "", now_ms());
          self.wake();
        }
        if let Some(m) = &me {
          self.say(f("app.logged_in_as", &[("name", &m.username)]));
        }
        // a different login starts with a clean whispers tab
        let switched = self.me.as_ref().map(|m| &m.username) != me.as_ref().map(|m| &m.username);
        // logged out, or another account over this one: their mentions are not the next one's
        if me.is_none() || (switched && self.me.is_some()) {
          self.mentions_forget();
        }
        if switched {
          self.mrules_forget();
          self.whispers_reset();
          self.locks.clear_all();
          // the new account's own seen times arrive with SeenGet below
          self.unseen = Default::default();
          // nothing pending belongs to the new account
          self.acct.reset(&self.set);
          self.ui_get_at = None;
          self.tab_sync_login(me.as_ref().map(|m| &*m.username));
          self.filters_login_changed();
          self.feed_reset();
          self.res.blocked.clear_server();
          self.mod_in.clear();
          self.kick_mod.clear();
          self.held = Default::default();
          self.following_reset();
          self.pushed.clear();
          self.import_reset();
          self.relayout_where(None);
        }
        self.me = me;
        self.list_sync();
        // a relink may have brought the automod permission
        self.held.reset();
        self.held_sweep();
        if self.me.is_some() {
          self.following_arm();
          self.mrules_ask();
          self.net.send(Cmd::Social(crate::net::social::Cmd::SeenGet));
          self.net.send(Cmd::Emote(crate::net::emote_ops::EmoteCmd::Subs));
          self.ui_state_fetch();
          self.tab_sync_ready();
          if self.screen_is(super::screen::Page::Following) {
            self.following_ask();
          }
          if self.screen_is(super::screen::Page::Import) {
            self.import_reload();
          }
        } else {
          self.res.emotes.set_subs(Default::default());
        }
        self.whisper_empty();
        self.set_mention_names();
        if switched {
          self.mentions_restore();
        }
        self.filter_names();
        self.entered();
        self.bar_dirty = true;
        self.input_dirty = true;
        self.wake();
      }
      NetEvent::HsRefs { chan, id, refs } => {
        let mut hit = false;
        for v in self.tabs.iter_mut().flat_map(Tab::views) {
          hit |= v.merge_hs(&id, &refs);
        }
        if hit {
          self.wake();
        } else {
          self.late_refs.hold(chan, id, refs);
        }
      }
      // linked or unlinked on heatsync.org: who you are (and so whose emotes are yours) is read again
      NetEvent::AccountChanged => {
        self.net.send(Cmd::Recheck);
        // a flair set or cleared on heatsync says so with this frame: ask again now, not after the ttl
        self.res.flairs.stale();
        self.wake();
      }
      NetEvent::SendersStale { ids, ver, removed } => {
        if self.prof {
          eprintln!("senders stale {}", ids.len());
        }
        for id in ids {
          let Some(scrubbed) = self.res.senders.stale(id, removed.as_deref()) else { continue };
          self.net.send(Cmd::Sender(id, ver.clone()));
          if scrubbed {
            self.touch_sender(id);
          }
        }
      }
      NetEvent::Cosmetics(got) => {
        for (id, c) in got {
          if self.res.cosmetics.put(&id, c) {
            self.touch_paint(&id);
          }
        }
        self.trim_soon();
      }
      NetEvent::Heat(got) => {
        for (login, heat) in got {
          if self.res.heats.put(&login, heat) {
            for v in self.tabs.iter_mut().flat_map(Tab::views) {
              v.touch_chatter(Platform::Twitch, &login);
            }
            self.wake();
          }
        }
      }
      NetEvent::Flair(got) => {
        for (id, badge) in got {
          if self.res.flairs.put(id, badge) {
            self.touch_sender(id);
          }
        }
        self.trim_soon();
      }
      NetEvent::Avatar(p, login, url) => {
        if self.res.avatars.put(p, &login, url) {
          for v in self.tabs.iter_mut().flat_map(Tab::views) {
            v.touch_chatter(p, &login);
          }
          self.wake();
        }
      }
      NetEvent::RoomLogin(id, login) => {
        if self.res.rooms.put(&id, login.as_deref()) {
          for v in self.tabs.iter_mut().flat_map(Tab::views) {
            v.touch_shared(&id);
          }
          self.wake();
        }
      }
      NetEvent::CosmeticsChanged(ids) => {
        ids.iter().for_each(|id| self.res.cosmetics.stale(id));
        // the server sends this for a flair or avatar change too: the sub flair is asked again for the same ids
        let twitch: Vec<u64> =
          ids.iter().filter_map(|id| if let PaintId::Twitch(n) = id { Some(*n) } else { None }).collect();
        self.res.flairs.stale_ids(&twitch);
        self.wake();
      }
      NetEvent::CosmeticsStale(id) => {
        self.res.cosmetics.stale(&id);
        self.wake();
      }
      NetEvent::Personal(ev) => self.on_personal(ev),
      NetEvent::EmoteDelta(ch, d) => {
        if self.res.emotes.patch(&ch, &d) {
          self.relayout_where(Some(&ch));
        }
      }
      NetEvent::BlockedHashes(h) => {
        self.res.blocked.set_server(h);
        self.relayout_where(None);
      }
      NetEvent::SubEmotes(set) => {
        self.res.emotes.set_subs(set);
        self.relayout_where(None);
        self.input_dirty = true;
        self.wake();
      }
      NetEvent::Inventory { removed } => {
        if let Some(id) = self.me.as_ref().and_then(|m| m.twitch_id) {
          let scrubbed = self.res.senders.stale(id, removed.as_deref());
          self.net.send(Cmd::Sender(id, Some(now_ms().to_string().into())));
          if scrubbed == Some(true) {
            self.touch_sender(id);
          }
        }
        self.bar_dirty = true;
        self.wake();
      }
      NetEvent::Relay { platform, key, msgs, history } => self.on_relay(platform, &key, msgs, history),
      NetEvent::CatchUp { platform, key, msgs } => self.on_catch_up(platform, &key, msgs),
      NetEvent::LoginWait(on) => self.login_wait = on,
      NetEvent::Lock(p, key, cause) => self.lock_told(p, &key, cause),
      NetEvent::RelayClear { platform, key, login, secs } => {
        // a removal must find every line: what is still waiting shows first
        let waiting = self.pacer.flush(&key);
        self.relay_show(platform, &key, waiting, false);
        let row = removed(platform, Removed::User(&login, secs));
        self.chatlog.log(platform, &key, &row);
        for i in self.tabs_with(platform, &key) {
          let v = self.tabs[i].chat();
          v.clear_user(&login);
          v.push_act(row.clone(), self.set.collapse_timeouts);
          self.touched(i);
        }
        self.mod_log(platform, &key, row.body.to_string());
      }
      NetEvent::YtGone { key, gone } => {
        // lines still waiting in the pacer are deleted too: show them first, then grey
        let waiting = self.pacer.flush(&key);
        self.relay_show(Platform::Youtube, &key, waiting, false);
        for i in self.tabs_with(Platform::Youtube, &key) {
          self.tabs[i].chat().clear_yt(&gone.ids, &gone.authors);
          self.touched(i);
        }
      }
      NetEvent::YtStatus { key, status } => {
        match &*status {
          "connected" => self.live_mark(Platform::Youtube, &key, true),
          "not-live" | "ended" | "chat-off" => self.live_mark(Platform::Youtube, &key, false),
          _ => {}
        }
        let line = match &*status {
          "not-live" => t("app.net.yt_not_live"),
          "ended" => t("app.net.yt_ended"),
          "chat-off" => t("app.net.yt_chat_off"),
          "no-slot" => t("app.net.yt_no_slot"),
          _ => return,
        };
        for i in self.tabs_with(Platform::Youtube, &key) {
          self.tabs[i].chat().push(Message::system(
            now_ms(),
            Platform::Youtube,
            f("app.net.yt_line", &[("line", line), ("key", &key)]),
          ));
          self.touched(i);
        }
      }
      NetEvent::Links { tab, kick, yt } => self.on_links(&tab, kick, yt),
      NetEvent::Upload(u) => self.on_upload(u),
      NetEvent::Pasted { to_picker, got } => self.on_pasted(to_picker, got),
      NetEvent::Font { asked, found } => self.font_found(asked, found),
      // user_event answers it: making a window again needs the event loop
      NetEvent::Raise(_) => {}
      NetEvent::Inv { op, res } => self.on_inv(op, res),
      NetEvent::Status(s) => self.say(s),
    }
  }
}

// what a moderator took away, as the ext's notice row words it (irc.js CLEARCHAT/CLEARMSG): a timeout
// green, a ban, a wipe or a deletion red. the line itself stays, dimmed
pub(super) enum Removed<'a> {
  Line(&'a str),
  User(&'a str, Option<u32>),
  All,
}

pub(super) fn removed(p: Platform, what: Removed) -> Message {
  let (text, ink) = match what {
    Removed::Line("") => (t("app.net.removed_line").to_owned(), theme::RED),
    Removed::Line(who) => (f("app.net.removed_line_of", &[("who", who)]), theme::RED),
    Removed::User(who, Some(s)) => (f("app.net.timed_out", &[("who", who), ("secs", &s.to_string())]), theme::GREEN),
    Removed::User(who, None) => (f("app.net.banned", &[("who", who)]), theme::RED),
    Removed::All => (t("app.net.cleared").to_owned(), theme::RED),
  };
  let run = match what {
    Removed::User(who, secs) => Some(Box::new(crate::model::runs::Run::new(who, &text, secs.is_none()))),
    _ => None,
  };
  let mut m = Message::system(now_ms(), p, text);
  m.color = ink;
  m.x = run.map(|run| Box::new(crate::model::message::Extra { run: Some(run), ..Default::default() }));
  m
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::net::Net;
  use crate::net::auth::Me;

  fn me() -> Me {
    Me {
      username: "mellen".into(),
      platform: "twitch".into(),
      twitch: Some("mellen".into()),
      twitch_id: Some(42),
      names: vec!["mellen".into()],
      kick: None,
      youtube: None,
      packs: vec![],
    }
  }

  fn asked(rx: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>) -> Vec<Option<u64>> {
    std::iter::from_fn(|| rx.try_recv().ok())
      .filter_map(|c| if let Cmd::SevenTvMe(id) = c { Some(id) } else { None })
      .collect()
  }

  // privacy: with 7tv paints + badges off, your twitch id is not handed to the 7tv lookup
  #[test]
  fn your_id_goes_to_7tv_only_while_its_paints_are_on() {
    assert_eq!(seventv_id(Some(42), true), Some(42));
    assert_eq!(seventv_id(Some(42), false), None);
    let (net, mut rx) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    app.set.paints = false;
    app.on_net(NetEvent::Me(Some(me())));
    assert_eq!(asked(&mut rx), [None], "paints off: no id leaves");
    app.set.paints = true;
    app.applied(crate::model::schema::Fx::Paints);
    assert_eq!(asked(&mut rx), [Some(42)], "turned on: the lookup goes out");
    app.set.paints = false;
    app.applied(crate::model::schema::Fx::Paints);
    assert_eq!(asked(&mut rx), [None], "turned off: it is dropped");
  }
}
