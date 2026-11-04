// tab / shift-tab in the message box: the first press gathers candidates for the word around the
// cursor, the next ones cycle them in place, and a tooltip over the box names the shown one
use super::App;
use crate::emotes::Provider;
use crate::model::catalog::{self, Cycle, Page, Rec};
use crate::model::complete::{Cand, Complete, MIN_WORD, Pick, commands, emoji, emotes, people};
use crate::model::slash;
use crate::net::Cmd;
use crate::net::emote_ops::EmoteCmd;
use crate::util::time::now_ms;

// people who spoke in this tab lately lead a bare word (ext RECENCY_WINDOW_MS / RECENCY_MAX)
const RECENT_MS: i64 = 10 * 60 * 1000;
const RECENT_MAX: usize = 150;

// the cycle tooltip's "where it's from · who sees it"
fn what(channel: bool, p: Provider) -> &'static str {
  match (channel, p) {
    (true, Provider::SevenTv) => "channel · 7tv users",
    (true, Provider::Bttv) => "channel · bttv users",
    (true, Provider::Ffz) => "channel · ffz users",
    (true, Provider::Twitch) => "channel · all twitch",
    (false, Provider::SevenTv) => "global · 7tv users",
    (false, Provider::Bttv) => "global · bttv users",
    (false, Provider::Ffz) => "global · ffz users",
    (false, Provider::Twitch) => "global · all twitch",
  }
}

impl App {
  // everyone heard in any tab (the ext's username cache), for `@` and the no-emote fallback
  fn everyone(&self) -> Vec<&str> {
    let mut seen = std::collections::HashSet::new();
    self.tabs.iter().flat_map(|t| t.view.chatters()).filter(|n| seen.insert(n.to_lowercase())).collect()
  }

  pub(super) fn candidates(&self, word: &str, slash: bool) -> Vec<Pick> {
    let tab = self.tabs.get(self.active);
    let chat = |n: Box<str>| Pick { name: n, what: "chatter", emote: false, code: None };
    if slash {
      return commands(word, [slash::all().into_iter().map(|c| c.name).collect()]);
    }
    let recent = tab.map_or_else(Vec::new, |t| t.view.recent_chatters(now_ms() - RECENT_MS, RECENT_MAX));
    if let Some(q) = word.strip_prefix('@') {
      let all = self.everyone();
      let found = people(q, &recent, &all, false);
      return found.into_iter().map(|(n, _)| chat(format!("@{}", n.trim_start_matches('@')).into())).collect();
    }
    // `:kek` searches emotes too, then emoji shortcodes (any length, like the ext)
    let q = word.strip_prefix(':').filter(|q| q.len() >= MIN_WORD).unwrap_or(word);
    let chan = tab.map_or("", |t| &*t.view.channel);
    let now = now_ms();
    let blocked = |n: &str, url: &str| self.res.blocked.contains(n, url);
    let channel = self.res.emotes.channel_names(chan).into_iter().filter(|(n, _, e)| !blocked(n, &e.url));
    let channel = channel.map(|(name, p, _)| Cand { name, tier: 0, what: what(true, p) });
    let own = self.own_id().filter(|_| self.set.suggest_inventory);
    let own = own.map_or_else(Vec::new, |id| self.res.senders.entries(id, now));
    let own = own.into_iter().filter(|(n, e)| !blocked(n, &e.url));
    let own = own.map(|(name, _)| Cand { name, tier: 1, what: "your inventory · heatsync only" });
    let global = self.res.emotes.global_names().into_iter().filter(|(n, _, e)| !blocked(n, &e.url));
    let global = global.map(|(name, p, _)| Cand { name, tier: 2, what: what(false, p) });
    let found = emotes(q, channel.chain(own).chain(global), |n| self.frecency.score(n, now));
    // a bare word: people who spoke here lately first; with no emote at all, anyone heard anywhere
    let mut out: Vec<Pick> =
      if q == word { people(q, &recent, &[], false) } else { Vec::new() }.into_iter().map(|(n, _)| chat(n)).collect();
    if found.is_empty() && q == word {
      let all: Vec<&str> = self.everyone().into_iter().filter(|n| !recent.contains(n)).collect();
      out.extend(people(q, &[], &all, true).into_iter().map(|(n, _)| chat(n)));
    }
    out.extend(found);
    if let Some(e) = word.strip_prefix(':') {
      out.extend(emoji(e));
    }
    out
  }

  pub(super) fn tab_complete(&mut self, back: bool) {
    if self.complete.is_none() {
      let r = self.line.word_around();
      let word = &self.line.text[r.clone()];
      // a lone `/word` (no arguments yet) completes commands
      let slash = r.start == 0 && word.starts_with('/') && r.end == self.line.text.trim_end().len();
      if !slash && word.chars().count() < MIN_WORD {
        return;
      }
      let matches = self.candidates(word, slash);
      // `:nyan` is nyan to the catalogs; the pick replaces the colon too
      let q = word.strip_prefix(':').unwrap_or(word);
      let remote = !slash && catalog::eligible(q);
      if matches.is_empty() && !remote {
        self.say(format!("no match for {word}"));
        self.bar_dirty = true;
        return self.wake();
      }
      let cycle = remote.then(|| Cycle::new(q));
      self.line.checkpoint(false);
      let empty = matches.is_empty();
      self.complete =
        Some(Complete { start: r.start, end: r.end, matches, at: usize::MAX, chipped: None, remote: cycle });
      // the catalogs are asked on the first tab, local matches or not (ext): their picks join the end
      // of the cycle. nothing local: the first hit lands in the box when they answer
      self.ask_catalog(true);
      if empty {
        self.input_dirty = true;
        return self.wake();
      }
    }
    let Some(c) = &mut self.complete else { return };
    let n = c.matches.len();
    if n == 0 {
      return;
    }
    // forward past the last pick holds while the catalogs have more (the ext's cycle never wraps early)
    let more = c.remote.as_ref().is_some_and(|r| !r.done());
    if !back && c.at != usize::MAX && c.at + 1 == n && more {
      self.ask_catalog(true);
      self.input_dirty = true;
      return self.wake();
    }
    c.at = match (c.at, back) {
      (usize::MAX, false) => 0,
      (usize::MAX, true) => n - 1,
      (i, false) => (i + 1) % n,
      (i, true) => (i + n - 1) % n,
    };
    // nearing the end: fetch the next page now, so the cycle rarely has to wait
    if !back && c.at + catalog::LOOKAHEAD >= n {
      self.ask_catalog(true);
    }
    self.show_pick();
  }

  // ask the catalogs for the cycle's next page (a trigger resets the chase budget; a chase does not)
  fn ask_catalog(&mut self, fresh: bool) {
    let Some(c) = &mut self.complete else { return };
    let Some(r) = &mut c.remote else { return };
    self.catalog_epoch = self.catalog_epoch.wrapping_add(1);
    let Some(a) = r.ask(self.catalog_epoch, c.matches.len(), fresh) else { return };
    self.net.send(Cmd::Catalog {
      q: r.search.clone(),
      epoch: self.catalog_epoch,
      page: a.page,
      stv: a.stv,
      ffz: a.ffz,
    });
  }

  // a catalog page came back. an answer to anything but the latest ask (the word moved on, the cycle
  // ended) is dropped. new picks only append, so the cycle under the user's fingers never shifts
  pub(super) fn on_catalog(&mut self, epoch: u32, stv: Option<Page>, ffz: Option<Page>) {
    let Some(c) = &mut self.complete else { return };
    let Some(r) = c.remote.as_mut().filter(|r| r.pending && r.epoch == epoch) else { return };
    let res = &self.res;
    let ffz_first = self.set.emote_provider.order().iter().find(|p| matches!(p, Provider::Ffz | Provider::SevenTv))
      == Some(&Provider::Ffz);
    let step = r.absorb(stv, ffz, ffz_first, &mut c.matches, &mut self.catalog_reg, |n, u| res.blocked.contains(n, u));
    let (word, none, done) = (r.search.clone(), c.matches.is_empty(), r.done());
    if step.chase {
      self.ask_catalog(false);
    } else if none && done {
      self.complete = None;
      self.say(format!("no match for {word}"));
      self.bar_dirty = true;
    } else if step.first {
      c.at = 0;
      self.show_pick();
    }
    self.input_dirty = true;
    self.wake();
  }

  // a sent line adds the catalog picks in it to your heatsync inventory, so they show for everyone
  // from then on (the ext's autoAddInputEmotes). not what you own, block or already get as a global
  pub(super) fn auto_add(&mut self, text: &str) {
    if self.me.is_none() {
      return;
    }
    let (own, now, res) = (self.own_id(), now_ms(), &self.res);
    let skip = |w: &str, rec: &Rec| {
      res.blocked.contains(w, &rec.url)
        || own.is_some_and(|id| res.senders.peek(id, w, now).is_some())
        || res.emotes.twitch_global(w).is_some()
    };
    for (name, rec) in self.catalog_reg.take_sent(text, skip) {
      self.net.send(Cmd::Emote(EmoteCmd::Add { name, url: rec.url, hash: None, zero_width: rec.zero_width }));
    }
  }

  // put the cycle's current candidate in the box, followed by one space to keep typing (kept, not
  // doubled, while cycling). an emote shows as its picture; the one it replaces stops showing
  pub(super) fn show_pick(&mut self) {
    let Some(c) = &mut self.complete else { return };
    let Some(pick) = c.matches.get(c.at).map(|m| m.name.clone()) else { return };
    if let Some(old) = c.chipped.take() {
      self.line.chips.remove(&old);
    }
    // a catalog pick is remembered the moment it's taken, for the box's picture and auto-add on send
    if let Some(h) = c.remote.as_ref().and_then(|r| r.hit(&pick)) {
      self.catalog_reg.track(h);
    }
    self.line.replace(c.start..c.end, &pick);
    c.end = c.start + pick.len();
    if !self.line.text[c.end..].starts_with(' ') {
      self.line.insert(" ");
    } else {
      self.line.cursor = c.end + 1;
    }
    if let Some(name) = c.emote()
      && !self.line.chips.contains(name)
    {
      self.line.chips.insert(name.into());
      c.chipped = Some(name.into());
    }
    self.input_dirty = true;
    self.wake();
  }
}

#[cfg(test)]
mod tests {
  use crate::app::App;
  use crate::model::catalog::Page;
  use crate::net::emote_ops::EmoteCmd;
  use crate::net::{Cmd, Net};
  use crate::util::time::now_ms;

  // suggestInventoryEmotes: your own inventory is offered in tab-complete, or it is not
  #[test]
  fn your_inventory_is_suggested_only_while_its_switch_is_on() {
    let (net, _rx) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
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
    let row = serde_json::json!({"custom_name": "zzmine", "url": "https://cdn.7tv.app/emote/x/1x.webp", "width": 32,
      "height": 32, "added_at": "2026-10-30T22:26:51.099Z"});
    app.res.senders.lookup(1, "zzmine", 0);
    app.res.senders.put(1, crate::heatsync::senders::parse_set(&serde_json::json!([row])));
    let has = |app: &App| app.candidates("zzmi", false).iter().any(|p| &*p.name == "zzmine");
    assert!(has(&app));
    app.set.suggest_inventory = false;
    assert!(!has(&app));
  }

  fn signed_in(app: &mut App) {
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
  }

  fn hit(name: &str, id: &str) -> crate::model::catalog::Hit {
    crate::model::catalog::Hit {
      name: name.into(),
      url: format!("https://cdn.7tv.app/emote/{id}/1x.webp").into(),
      source: crate::model::catalog::Source::SevenTv,
      zero_width: false,
    }
  }

  fn page(hits: Vec<crate::model::catalog::Hit>, more: bool) -> Page {
    Page { hits, more }
  }

  fn asked(rx: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>) -> Vec<Cmd> {
    std::iter::from_fn(|| rx.try_recv().ok())
      .filter(|c| matches!(c, Cmd::Catalog { .. } | Cmd::Emote(_) | Cmd::Send { .. }))
      .collect()
  }

  // the first tab asks the catalogs even with a local match, and `:word` asks for the word
  #[test]
  fn first_tab_searches_the_catalogs_with_or_without_a_colon() {
    let (net, mut rx) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    // more local matches than the cycle's lookahead, so only the first-tab ask can reach the catalogs
    let rows: Vec<_> = ["zzcatHere"]
      .into_iter()
      .chain((0..20).map(|_| ""))
      .enumerate()
      .map(|(i, n)| {
        let name = if n.is_empty() { format!("zzcatHere{i}") } else { n.into() };
        serde_json::json!({"custom_name": name, "url": format!("https://cdn.7tv.app/emote/x{i}/1x.webp"),
        "width": 32, "height": 32, "added_at": "2026-10-30T22:26:51.099Z"})
      })
      .collect();
    signed_in(&mut app);
    app.res.senders.lookup(1, "zzcatHere", 0);
    app.res.senders.put(1, crate::heatsync::senders::parse_set(&serde_json::json!(rows)));
    for typed in ["zzcat", ":zzcat"] {
      app.complete = None;
      app.line = Default::default();
      app.line.insert(&format!("say {typed}"));
      app.tab_complete(false);
      assert!(
        matches!(asked(&mut rx).as_slice(), [Cmd::Catalog { q, .. }] if &**q == "zzcat"),
        "{typed} asks the catalogs for zzcat"
      );
      assert_eq!(app.line.text, "say zzcatHere ", "{typed}: the local match shows meanwhile");
    }
  }

  // tab with nothing local asks the catalogs, a stale answer is dropped, the live one fills the cycle
  // (box only), and sending the tabbed name adds it to your inventory exactly once
  #[test]
  fn catalog_tab_search_cycle_and_auto_add_on_send() {
    use crate::net::NetEvent;
    let (net, mut rx) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    signed_in(&mut app);
    app.line.insert("say zzcat");
    app.tab_complete(false);
    let ask = asked(&mut rx);
    let (epoch, page_no) = match ask.as_slice() {
      [Cmd::Catalog { q, epoch, page, stv: true, ffz: true }] if &**q == "zzcat" => (*epoch, *page),
      _ => panic!("one catalog ask"),
    };
    assert_eq!(page_no, 1);
    assert_eq!(app.complete.as_ref().and_then(|c| c.tip()).as_deref(), Some("searching 7tv · ffz…"));
    assert_eq!(app.line.text, "say zzcat", "nothing in the box yet");
    // tab while waiting does nothing and asks nothing more
    app.tab_complete(false);
    assert!(asked(&mut rx).is_empty());
    // an answer to some other ask is dropped
    app.on_net(NetEvent::Catalog {
      epoch: epoch.wrapping_add(9),
      stv: Some(page(vec![hit("zzcatStale", "S1")], false)),
      ffz: None,
    });
    assert!(app.complete.as_ref().unwrap().matches.is_empty());
    let hits = vec![hit("zzcatLol", "A1"), hit("zzcat", "A2"), hit("zzcatWide", "A3")];
    app.on_net(NetEvent::Catalog { epoch, stv: Some(page(hits, false)), ffz: None });
    // the exact name leads, and the first hit went into the box with its picture
    let c = app.complete.as_ref().unwrap();
    assert_eq!(c.matches.iter().map(|p| &*p.name).collect::<Vec<_>>(), ["zzcat", "zzcatLol", "zzcatWide"]);
    assert_eq!(app.line.text, "say zzcat ");
    assert!(app.line.chips.contains("zzcat"));
    assert_eq!(app.box_url("zzcat").as_deref(), Some("https://cdn.7tv.app/emote/A2/1x.webp"));
    assert!(app.complete.as_ref().unwrap().tip().unwrap().starts_with("1/3 zzcat · catalog · 7tv"));
    // chat never sees it: the render path only knows the channel's sets and senders' inventories
    assert!(app.res.emotes.channel("forsen", "zzcat").is_none() && app.res.emotes.global("zzcat").is_none());
    assert!(app.res.senders.peek(1, "zzcat", now_ms()).is_none());
    // cycle on: the catalog is exhausted (a short page), so it wraps and asks nothing
    app.tab_complete(false);
    assert_eq!(app.line.text, "say zzcatLol ");
    assert!(!app.line.chips.contains("zzcat"), "the chip follows the pick");
    app.tab_complete(false);
    app.tab_complete(false);
    assert_eq!(app.line.text, "say zzcat ");
    assert!(asked(&mut rx).is_empty());
    // end the cycle on zzcatLol and send: it is the one that gets added, once
    app.tab_complete(false);
    app.complete = None;
    app.line.insert("zzcatLol hello zzcatLol zzcatWide zzcatNever");
    app.send();
    let sent = asked(&mut rx);
    let adds: Vec<_> = sent
      .iter()
      .filter_map(|c| match c {
        Cmd::Emote(EmoteCmd::Add { name, url, hash, zero_width }) => {
          Some((&**name, &**url, hash.is_none(), *zero_width))
        }
        _ => None,
      })
      .collect();
    assert_eq!(
      adds,
      [
        ("zzcatLol", "https://cdn.7tv.app/emote/A1/1x.webp", true, false),
        ("zzcatWide", "https://cdn.7tv.app/emote/A3/1x.webp", true, false)
      ],
      "only names that were tabbed onto, each once"
    );
    assert!(sent.iter().any(|c| matches!(c, Cmd::Send { text, .. } if text.contains("zzcatLol hello"))));
    app.line.insert("zzcatLol");
    app.send();
    assert!(asked(&mut rx).iter().all(|c| !matches!(c, Cmd::Emote(_))), "added once");
  }

  // a name you own, block or get as a global is never added; logged out adds nothing; blocked names
  // never surface from the catalog
  #[test]
  fn auto_add_skips_owned_blocked_and_logged_out() {
    use crate::emotes::block::Scope;
    let (net, mut rx) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    let url = |i: &str| format!("https://cdn.7tv.app/emote/{i}/1x.webp");
    for (n, i) in [("aaown", "1"), ("aablk", "2"), ("aaok", "3")] {
      app.catalog_reg.track(&hit(n, i));
    }
    app.auto_add("aaok");
    assert!(asked(&mut rx).is_empty(), "logged out");
    signed_in(&mut app);
    let row = serde_json::json!({"custom_name": "aaown", "url": url("1"), "width": 32, "height": 32,
      "added_at": "2026-10-30T22:26:51.099Z"});
    app.res.senders.lookup(1, "aaown", 0);
    app.res.senders.put(1, crate::heatsync::senders::parse_set(&serde_json::json!([row])));
    app.res.blocked.block("aablk", &url("2"), Scope::Hs);
    app.auto_add("aaown aablk aaok");
    let names: Vec<_> = asked(&mut rx)
      .into_iter()
      .filter_map(|c| match c {
        Cmd::Emote(EmoteCmd::Add { name, .. }) => Some(name),
        _ => None,
      })
      .collect();
    assert_eq!(names, ["aaok".into()]);
    // a blocked name is filtered out of a catalog page
    let mut cy = crate::model::catalog::Cycle::new("aa");
    let mut m = Vec::new();
    let mut reg = crate::model::catalog::Registry::default();
    cy.ask(1, 0, true);
    let blocked = |n: &str, u: &str| app.res.blocked.contains(n, u);
    cy.absorb(Some(page(vec![hit("aablk", "2"), hit("aafine", "4")], false)), None, true, &mut m, &mut reg, blocked);
    assert_eq!(m.iter().map(|p| &*p.name).collect::<Vec<_>>(), ["aafine"]);
  }

  // local hits first; nearing the end asks for the next page, the cycle holds at the end until it
  // lands, the new picks append behind what was cycled
  #[test]
  fn catalog_pages_in_near_the_end_and_holds_there() {
    use crate::emotes::{Emote, Provider};
    use crate::net::NetEvent;
    let (net, mut rx) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    let e =
      |i: &str| Emote { url: format!("https://cdn.7tv.app/emote/{i}/1x.webp").into(), w: 28, h: 28, zero_width: false };
    app.res.emotes.set(None, Provider::SevenTv, [("qqone".into(), e("L1")), ("qqtwo".into(), e("L2"))].into());
    app.line.insert("qq");
    app.tab_complete(false);
    let first = asked(&mut rx);
    let epoch = match first.as_slice() {
      [Cmd::Catalog { epoch, page: 1, .. }] => *epoch,
      _ => panic!("two local hits is within the lookahead: the first page is asked at once"),
    };
    assert_eq!(app.line.text, "qqone ");
    app.tab_complete(false);
    assert_eq!(app.line.text, "qqtwo ");
    // at the end with an ask out: held, not wrapped
    app.tab_complete(false);
    assert_eq!(app.line.text, "qqtwo ");
    assert!(asked(&mut rx).is_empty());
    assert!(app.complete.as_ref().unwrap().tip().unwrap().contains("searching"));
    app.on_net(NetEvent::Catalog {
      epoch,
      stv: Some(page(vec![hit("qqone", "R1"), hit("qqthree", "R2")], true)),
      ffz: None,
    });
    assert_eq!(app.complete.as_ref().unwrap().matches.len(), 3, "the duplicate of a local is dropped");
    assert_eq!(app.line.text, "qqtwo ", "the pick under the cursor did not move");
    app.tab_complete(false);
    assert_eq!(app.line.text, "qqthree ");
    // the next trigger asks page 2, from 7tv only (ffz had nothing / failed)
    let next = asked(&mut rx);
    assert!(matches!(next.as_slice(), [Cmd::Catalog { page: 2, stv: true, ffz: false, .. }]), "{}", next.len());
  }
}
