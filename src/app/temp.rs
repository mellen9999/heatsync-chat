// the temp tab, like the ext's and the site's: one unsaved channel tab for the stream you're looking at.
// opening a channel to look at (enter on a following row, a click on a went-live line) fills the slot, the next one takes it; /keep, the
// tab's menu or chatting in it saves it. it is never written to the tabs file and never synced: every place
// that lists tabs to save skips `temp` (store_tabs, tabsync::is_channel, other_specs)
use super::tabs::{make_tab, seat_hosts, sources_of};
use super::{App, Tab};
use crate::model::chan::Spec;
use crate::model::message::Platform;

// does tab `t` hold any feed `spec` names
pub(super) fn overlaps(t: &Tab, spec: &Spec) -> bool {
  let want = sources_of(spec);
  t.sources.iter().any(|s| want.iter().any(|w| w.0 == s.0 && w.1 == s.1))
}

impl App {
  // a click on a went-live line (not its name: that's their card) for a channel this tab isn't
  pub(super) fn went_live_press(&mut self) -> bool {
    let (chat, y) = (self.chat_area(), self.pointer.1);
    let res = &mut self.res;
    let Some((p, chan)) = self.tabs.get_mut(self.active).and_then(|t| t.view.opens_at(chat, y, res)) else {
      return false;
    };
    let own = self.tabs[self.active].sources.iter().any(|s| s.0 == p && s.1.eq_ignore_ascii_case(&chan));
    if own {
      return false;
    }
    self.open_channel(p, &chan);
    self.wake();
    true
  }

  // a click on a folded run of timeouts opens or closes it
  pub(super) fn run_press(&mut self) -> bool {
    let (chat, y) = (self.chat_area(), self.pointer.1);
    let res = &mut self.res;
    let hit = self.tabs.get_mut(self.active).is_some_and(|t| t.view.toggle_run_at(chat, y, res));
    if hit {
      self.wake();
    }
    hit
  }

  // a channel a line names opens: its own tab when one shows it, else the temp tab. youtube's channel key
  // isn't something a tab can be made from, so only a tab it already has
  pub(super) fn open_channel(&mut self, p: Platform, chan: &str) {
    if let Some(i) = self.tabs_with(p, chan).into_iter().find(|&i| self.visible(i)) {
      return self.select(i);
    }
    let spec = match p {
      Platform::Twitch => Spec::parse(chan, "", ""),
      Platform::Kick => Spec::parse("", chan, ""),
      Platform::Youtube => return self.say(format!("{chan} has no tab")),
    };
    match spec.and_then(|s| s.check(&self.other_specs(None)).map(|_| s)) {
      Ok(s) => self.open_look(&s),
      Err(e) => self.say(e.msg()),
    }
  }

  pub(super) fn temp_at(&self) -> Option<usize> {
    self.tabs.iter().position(|t| t.temp)
  }

  // the temp tab goes if `f` says so (nothing saved, nothing sent to the account)
  pub(super) fn drop_temp_if(&mut self, f: impl Fn(&Tab) -> bool) {
    if let Some(i) = self.temp_at().filter(|&i| f(&self.tabs[i])) {
      self.remove_tab(i);
    }
  }

  // a channel opened to look at: a saved tab that has it is selected, the temp tab that has it is selected,
  // else it takes the temp slot
  pub(super) fn open_look(&mut self, spec: &Spec) {
    if let Some(i) = self.tabs.iter().position(|t| overlaps(t, spec)) {
      return if self.visible(i) {
        self.select(i)
      } else {
        self.say(format!("the {} tab is hidden: /show {}", spec.name(), sources_of(spec)[0].1))
      };
    }
    self.drop_temp_if(|_| true);
    let mut tab = make_tab(&self.net, spec);
    tab.temp = true;
    tab.view.resize(self.body().w);
    seat_hosts(&mut self.res.chatters, &tab.sources);
    self.tabs.push(tab);
    self.live_soon();
    self.activate(self.tabs.len() - 1);
    self.say("temporary tab: opening another stream takes its slot · /keep keeps it");
  }

  // the temp tab becomes a saved one, at the end of the bar
  pub(super) fn keep_tab(&mut self, i: usize) {
    let Some(t) = self.tabs.get_mut(i).filter(|t| t.temp) else { return };
    t.temp = false;
    let name = t.name.clone();
    self.save_tabs();
    self.bar_dirty = true;
    self.say(format!("kept {name}"));
    self.wake();
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::model::chan::Cmd as ChanCmd;
  use crate::net::{Cmd, Net, hs_ws::Cmd as HsCmd};

  fn boot() -> (App, tokio::sync::mpsc::UnboundedReceiver<Cmd>) {
    let (net, mut rx) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    app.dpi = 1.0;
    app.res.set_scale(app.scale());
    app.resized(720, 300);
    app.me = Some(crate::net::auth::Me {
      username: "mellen".into(),
      platform: "twitch".into(),
      twitch: Some("mellen".into()),
      twitch_id: Some(1),
      names: vec!["mellen".into()],
      kick: None,
      youtube: None,
      packs: vec![],
    });
    app.hs_authed = true;
    app.redraw();
    while rx.try_recv().is_ok() {}
    (app, rx)
  }

  fn look(app: &mut App, s: &str) {
    app.open_look(&Spec::from_line(s).unwrap());
  }

  fn forsen(app: &App) -> usize {
    app.tabs.iter().position(|t| &*t.name == "forsen").unwrap()
  }

  // a click on the newest line of the active tab, away from the name at its left
  fn click_last(app: &mut App) -> bool {
    app.redraw();
    let c = app.chat_area();
    app.pointer = (c.x + c.w - 40, c.bottom() - 4);
    app.went_live_press()
  }

  #[test]
  fn a_click_on_a_went_live_line_opens_its_channel_as_the_temp_tab_or_its_own_tab() {
    use crate::model::stream_events::Line;
    let (mut app, _rx) = boot();
    let f = forsen(&app);
    app.select(f);
    app.on_stream(Line::went_live(Platform::Twitch, "xqc", "just chatting"));
    assert!(click_last(&mut app));
    let t = &app.tabs[app.active];
    assert!(t.temp && &*t.name == "xqc", "{}", t.name);
    assert!(!saved(&app).iter().any(|l| l.contains("xqc")), "a look is never saved");
    // the same line in xqc's own tab is that tab's: the click is an ordinary one
    assert!(!click_last(&mut app));
    // a channel with a tab of its own opens that tab, no temp tab
    app.on_stream(Line::went_live(Platform::Twitch, "forsen", ""));
    assert!(click_last(&mut app));
    assert_eq!(app.active, forsen(&app));
    assert!(app.temp_at().is_some(), "the temp tab stays till the next look");
    // any other line is not one
    app.say("hello");
    assert!(!click_last(&mut app));
  }

  fn saved(app: &App) -> Vec<String> {
    app.store_tabs();
    crate::store::load_tabs().iter().map(|l| l.to_string()).collect()
  }

  #[test]
  fn looking_makes_a_temp_tab_not_a_saved_one_and_the_next_takes_its_slot() {
    let (mut app, _rx) = boot();
    look(&mut app, "xqc");
    let t = app.temp_at().expect("a temp tab");
    assert_eq!((&*app.tabs[t].name, app.active), ("xqc", t));
    assert_eq!(saved(&app), ["forsen"]);
    let n = app.tabs.len();
    look(&mut app, "kick:big");
    assert_eq!(app.tabs.len(), n, "one slot");
    assert_eq!(&*app.tabs[app.temp_at().unwrap()].name, "kick:big");
    assert!(app.tabs.iter().all(|t| &*t.name != "xqc"));
    assert_eq!(saved(&app), ["forsen"]);
  }

  #[test]
  fn a_saved_tab_that_covers_it_is_selected_and_the_temp_one_too() {
    let (mut app, _rx) = boot();
    let n = app.tabs.len() + 1;
    look(&mut app, "xqc");
    let f = forsen(&app);
    app.select(0);
    look(&mut app, "forsen kick:forsen-k");
    assert_eq!((app.active, app.tabs.iter().filter(|t| t.temp).count()), (f, 1), "forsen covers a merged spec's leg");
    assert!(app.tabs.iter().filter(|t| &*t.name == "forsen").count() == 1);
    look(&mut app, "xqc");
    assert_eq!(app.active, app.temp_at().unwrap(), "asking again goes to the temp tab");
    assert_eq!(app.tabs.len(), n);
  }

  #[test]
  fn keep_by_slash_and_by_menu_save_it() {
    let (mut app, _rx) = boot();
    look(&mut app, "xqc");
    app.chan_cmd(ChanCmd::Keep);
    assert!(app.temp_at().is_none());
    assert_eq!(saved(&app), ["forsen", "xqc"]);
    // keep with nothing temporary says so
    app.chan_cmd(ChanCmd::Keep);
    // the menu's keep
    look(&mut app, "big");
    let i = app.temp_at().unwrap();
    app.keep_tab(i);
    assert_eq!(saved(&app), ["forsen", "xqc", "big"]);
    // keep only works on the active temp tab: /keep elsewhere does nothing
    look(&mut app, "small");
    let f = forsen(&app);
    app.select(f);
    app.chan_cmd(ChanCmd::Keep);
    assert!(app.temp_at().is_some());
    assert_eq!(saved(&app), ["forsen", "xqc", "big"]);
  }

  #[test]
  fn adding_what_the_temp_tab_holds_keeps_that_tab() {
    let (mut app, _rx) = boot();
    look(&mut app, "xqc");
    let t = app.temp_at().unwrap();
    app.tabs[t].unread = true;
    let n = app.tabs.len();
    app.add_tab(&Spec::from_line("xqc").unwrap());
    assert_eq!((app.tabs.len(), app.temp_at()), (n, None));
    assert_eq!(saved(&app), ["forsen", "xqc"]);
    // a different spec sharing a leg replaces it
    look(&mut app, "big");
    app.add_tab(&Spec::from_line("big kick:big").unwrap());
    assert!(app.temp_at().is_none());
    assert_eq!(app.tabs.iter().filter(|t| t.sources.iter().any(|s| &*s.1 == "big")).count(), 1);
  }

  #[test]
  fn a_send_keeps_it_only_once_it_got_through() {
    use crate::net::irc::Chat;
    let (mut app, mut rx) = boot();
    look(&mut app, "xqc");
    app.line.set_text("hello".into());
    app.send();
    assert!(
      std::iter::from_fn(|| rx.try_recv().ok()).any(|c| matches!(c, Cmd::Send { ref chan, .. } if &**chan == "xqc")),
      "the send goes out unchanged"
    );
    assert!(app.temp_at().is_some(), "sent is not delivered");
    // a refusal keeps nothing
    app.on_chat("xqc", Chat::Lock(crate::model::lock::Cause::Subs));
    assert!(app.temp_at().is_some());
    // another channel getting a line through is not this tab
    app.on_chat("forsen", Chat::Sent);
    assert!(app.temp_at().is_some());
    app.on_chat("xqc", Chat::Sent);
    assert!(app.temp_at().is_none());
    assert_eq!(saved(&app), ["forsen", "xqc"]);
  }

  #[test]
  fn it_never_reaches_the_file_or_the_account() {
    let (mut app, mut rx) = boot();
    look(&mut app, "xqc");
    app.save_tabs();
    app.tab_sync_push();
    assert_eq!(saved(&app), ["forsen"]);
    assert_eq!(app.channel_specs().iter().map(|s| s.name()).collect::<Vec<_>>(), ["forsen"]);
    assert!(app.other_specs(None).iter().all(|s| s.name() != "xqc"), "not in /share either");
    let sent: Vec<String> = std::iter::from_fn(|| rx.try_recv().ok())
      .filter_map(|c| if let Cmd::Hs(HsCmd::TabSync(l)) = c { Some(format!("{l:?}")) } else { None })
      .collect();
    assert!(sent.iter().all(|s| !s.contains("xqc")), "{sent:?}");
    // the account's list landing keeps it too
    let f = crate::model::tabsync::frame(&serde_json::json!({"type": "multichat:config", "rev": 9, "channels": [
      {"id": "forsen", "twitch": "forsen"}, {"id": "big", "twitch": "big"}]}))
    .unwrap();
    app.on_net(crate::net::NetEvent::TabSync(f));
    assert!(app.temp_at().is_some());
    assert_eq!(saved(&app), ["forsen", "big"]);
    // closing it drops it: nothing saved, nothing sent
    while rx.try_recv().is_ok() {}
    let before = saved(&app);
    app.remove_tab(app.temp_at().unwrap());
    assert!(app.temp_at().is_none());
    assert_eq!(saved(&app), before);
    assert!(!std::iter::from_fn(|| rx.try_recv().ok()).any(|c| matches!(c, Cmd::Hs(HsCmd::TabSync(_)))));
  }

  #[test]
  fn its_name_is_underlined_and_a_saved_tab_is_not() {
    let (mut app, _rx) = boot();
    look(&mut app, "xqc");
    let f = forsen(&app);
    app.select(0);
    let white_row = |app: &mut App, i: usize| {
      app.bar_dirty = true;
      app.redraw();
      let at = (0..app.tabs.len()).filter(|&j| app.visible(j)).position(|j| j == i).unwrap();
      let r = app.bar_hits[at];
      let y = r.bottom() - 2;
      (r.x..r.right())
        .filter(|&x| app.canvas.px[(y * app.canvas.w + x) as usize] & 0xffffff == crate::ui::theme::WHITE)
        .count()
    };
    let t = app.temp_at().unwrap();
    assert!(white_row(&mut app, t) > 10, "the underline");
    assert!(white_row(&mut app, f) <= 2, "a saved tab has none");
  }
}
