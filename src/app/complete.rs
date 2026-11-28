// tab / shift-tab in the message box: the first press gathers candidates for the word around the
// cursor, the next ones cycle them in place, and a tooltip over the box names the shown one
use super::App;
use crate::emotes::Provider;
use crate::emotes::draft::{From, Pic, Src};
use crate::emotes::mods;
use crate::i18n::{f, t};
use crate::law;
use crate::model::catalog::{self, Cycle, Page, Rec, Registry};
use crate::model::complete::{Complete, MAX, MIN_WORD, Pick, commands, emoji, people};
use crate::model::message::Platform;
use crate::model::slash;
use crate::net::Cmd;
use crate::net::emote_ops::EmoteCmd;
use crate::net::hs_ws::Vouch;
use crate::util::time::now_ms;

// people who spoke in this tab lately lead a bare word (ext RECENCY_WINDOW_MS / RECENCY_MAX)
const RECENT_MS: i64 = 10 * 60 * 1000;
const RECENT_MAX: usize = 150;

// the cycle tooltip's "where it's from · who sees it"
fn what(channel: bool, p: Provider) -> &'static str {
  match (channel, p) {
    (true, Provider::SevenTv) => t("app.complete.channel_7tv"),
    (true, Provider::Bttv) => t("app.complete.channel_bttv"),
    (true, Provider::Ffz) => t("app.complete.channel_ffz"),
    (true, Provider::Twitch) => t("app.complete.channel_twitch"),
    // kick only ever sits in a channel scope (a merged tab's kick channel)
    (_, Provider::Kick) => t("app.complete.channel_kick"),
    (false, Provider::SevenTv) => t("app.complete.global_7tv"),
    (false, Provider::Bttv) => t("app.complete.global_bttv"),
    (false, Provider::Ffz) => t("app.complete.global_ffz"),
    (false, Provider::Twitch) => t("app.complete.global_twitch"),
  }
}

// one of the law's candidates as tab shows it: its name, its own picture, where it is from, and who sees it
fn offered(c: &law::Candidate<Pic>, mark: Option<char>) -> Pick {
  let e = c.emote;
  // a copy of a name twitch marks itself: offered, but twitch will draw its own there
  let what = match e.src {
    _ if c.shadowed => t("app.complete.wont_show"),
    Src::Owner => t("app.complete.channel_hs"),
    Src::Channel(p) => what(true, p),
    Src::Global(p) => what(false, p),
    Src::Inventory => t("app.complete.your_inventory"),
    Src::Personal => t("app.complete.your_set"),
    Src::Sub => t("app.complete.your_sub"),
    Src::Favorite => t("app.complete.favorite"),
  };
  let pic = Rec { url: e.url.into(), zero_width: e.zw, provider: e.p };
  Pick { name: e.name.into(), what, emote: true, code: None, pic: Some(pic), mark }
}

impl App {
  // your own commands that start with what's typed, ahead of the built-in ones (a built-in's name is its own)
  fn own_commands(&self, word: &str) -> Vec<Pick> {
    let low = word.to_lowercase();
    self
      .ucmds
      .iter()
      .map(|(n, _)| format!("/{n}"))
      .filter(|n| n.starts_with(&low) && !super::usercmd::builtin(&n[1..]))
      .map(|n| Pick {
        name: n.into(),
        what: t("app.complete.your_command"),
        emote: false,
        code: None,
        pic: None,
        mark: None,
      })
      .collect()
  }

  // everyone heard in any tab (the ext's username cache), for `@` and the no-emote fallback
  fn everyone(&self) -> Vec<&str> {
    let mut seen = std::collections::HashSet::new();
    self.tabs.iter().flat_map(|t| t.view.chatters()).filter(|n| seen.insert(n.to_lowercase())).collect()
  }

  // the emote law's offer for `q` in the room this line goes to (docs/emote-law.md, offer): the exact name,
  // the names you use, the room's, yours, the globals. a name is the one picture typing it would send, so
  // that picture is the one tile of it. with the two places people may join
  pub(super) fn offer(&self, q: &str, limit: Option<usize>) -> (Vec<Pick>, usize, usize) {
    let d = self.draft();
    let w = d.world(&self.res.blocked, self.res.favorites.list());
    let usage = |n: &str| self.frecency.score(n, d.now);
    let hidden = |k: &str| w.hidden.contains(k);
    let ctx = law::OfferCtx {
      room: w.room.as_ref(),
      own: &w.own,
      globals: &w.globals,
      catalog: &[],
      emoji: &[],
      usage: Some(&usage),
      pop: None,
      blocked: (!w.hidden.is_empty()).then_some(&hidden),
      suggest_own: self.set.suggest_inventory,
      favorites: &w.favorites,
      // the emoji are not in this offer (`candidates` sets them in), so the law's "emoji first" has nothing to move
      emoji_first: false,
    };
    // one name, one picture, one tile: a copy of a name the room answers with another picture is not offered
    // (law::mark), so the groups end that much sooner. asked for the whole list, so the cap counts what is left
    let o = law::offer_seams(q, &ctx, None);
    let judge = w.judge();
    let (mut list, mut exact, mut room) = (Vec::new(), 0, 0);
    for (i, c) in o.list.iter().enumerate() {
      if limit.is_some_and(|n| n > 0 && list.len() >= n) {
        break;
      }
      let says = judge.says(c.emote, c.tier.as_str());
      if says.offered {
        exact += usize::from(i < o.exact);
        room += usize::from(i < o.room);
        list.push(offered(c, says.mark));
      }
    }
    (list, exact, room)
  }

  // Tab's list, and everything but the `:` list: the law's one order
  pub(super) fn candidates(&self, word: &str, slash: bool) -> Vec<Pick> {
    self.candidates_in(word, slash, false)
  }

  // the same, and for the `:` list alone `emoji_first` (setting colonListOrder) puts every emoji above the emotes
  pub(super) fn candidates_in(&self, word: &str, slash: bool, emoji_first: bool) -> Vec<Pick> {
    let tab = self.tabs.get(self.active);
    let chat =
      |n: Box<str>| Pick { name: n, what: t("app.complete.chatter"), emote: false, code: None, pic: None, mark: None };
    if slash {
      let mut out = self.own_commands(word);
      let builtin = commands(word, [slash::all().into_iter().map(|c| c.name).collect()]);
      out.extend(builtin);
      return out;
    }
    let recent = tab.map_or_else(Vec::new, |t| t.view.recent_chatters(now_ms() - RECENT_MS, RECENT_MAX));
    if let Some(q) = word.strip_prefix('@') {
      let all = self.everyone();
      let found = people(q, &recent, &all, false);
      return found.into_iter().map(|(n, _)| chat(format!("@{}", n.trim_start_matches('@')).into())).collect();
    }
    // `:kek` searches emotes too, then emoji shortcodes (any length, like the ext)
    let q = word.strip_prefix(':').filter(|q| q.len() >= MIN_WORD).unwrap_or(word);
    let (mut out, exact, room) = self.offer(q, Some(MAX));
    // a bare word is a name too. the law ranks emotes; people join it where it says its groups end: the
    // person typed whole after the emote of that exact name, the other recent talkers after the names you
    // use and the room's emotes, before everything else. with no emote at all, anyone heard anywhere
    if q == word {
      let low = q.to_lowercase();
      let mut who: Vec<Box<str>> = people(q, &recent, &[], false).into_iter().map(|(n, _)| n).collect();
      if out.is_empty() {
        let all: Vec<&str> = self.everyone().into_iter().filter(|n| !recent.contains(n)).collect();
        who.extend(people(q, &[], &all, true).into_iter().map(|(n, _)| n));
      }
      let (whole, rest): (Vec<_>, Vec<_>) = who.into_iter().partition(|n| n.to_lowercase() == low);
      out.splice(room..room, rest.into_iter().map(chat));
      out.splice(exact..exact, whole.into_iter().map(chat));
    }
    if let Some(e) = word.strip_prefix(':') {
      if emoji_first {
        out.splice(0..0, emoji(e));
      } else {
        out.extend(emoji(e));
      }
    }
    out
  }

  pub(super) fn tab_complete(&mut self, back: bool) {
    if self.complete.is_none() {
      let r = self.line.word_around();
      if self.settle_modifier(r.clone()) {
        return;
      }
      let word = &self.line.text[r.clone()];
      // a lone `/word` (no arguments yet) completes commands
      let slash = r.start == 0 && word.starts_with('/') && r.end == self.line.text.trim_end().len();
      if !slash && word.chars().count() < MIN_WORD {
        return;
      }
      if self.set.tab_list && !back && !slash && self.open_tab_list(r.clone()) {
        return;
      }
      let word = &self.line.text[r.clone()];
      let matches = self.candidates(word, slash);
      // `:nyan` is nyan to the catalogs; the pick replaces the colon too
      let q = word.strip_prefix(':').unwrap_or(word);
      let remote = !slash && catalog::eligible(q);
      if matches.is_empty() && !remote {
        self.say(f("app.complete.no_match", &[("word", word)]));
        self.bar_dirty = true;
        return self.wake();
      }
      let cycle = remote.then(|| Cycle::new(q));
      self.line.checkpoint(false);
      let empty = matches.is_empty();
      self.complete = Some(Complete {
        start: r.start,
        end: r.end,
        matches,
        at: usize::MAX,
        chipped: None,
        passed: None,
        remote: cycle,
      });
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

  // a modifier word is never an emote to complete (ext findEmoteMatches): tab settles it on its emote with
  // a space, and a bare letter right after a picture (`Okayge p`) becomes its modifier (`p!`) first
  fn settle_modifier(&mut self, r: std::ops::Range<usize>) -> bool {
    let word = &self.line.text[r.clone()];
    let after_chip = || {
      let head = self.line.text[..r.start].strip_suffix(' ')?;
      let prev = &head[head.rfind(' ').map_or(0, |i| i + 1)..];
      self.line.chips.contains(prev).then_some(())
    };
    let tok = match word.len() {
      1 if after_chip().is_some() => format!("{}!", word.to_ascii_lowercase()),
      _ => word.to_owned(),
    };
    if !mods::is(&tok) {
      return false;
    }
    self.line.checkpoint(false);
    let spaced = self.line.text[r.end..].starts_with(' ');
    self.line.replace(r.clone(), &if spaced { tok.clone() } else { format!("{tok} ") });
    self.line.cursor = r.start + tok.len() + 1;
    self.chip_spaced();
    self.input_dirty = true;
    self.wake();
    true
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
    // a name that already means an emote here keeps it: a search look-alike never joins the cycle (the
    // picker's search is where another picture of a name is picked on purpose). nor does what you blocked
    let (d, blocked) = (self.draft(), &self.res.blocked);
    let held = |h: &&catalog::Hit| blocked.contains(&h.name, &h.url) || d.word(None, &h.name).is_some();
    let hits = stv.iter().chain(&ffz).flat_map(|p| &p.hits);
    let held: std::collections::HashSet<Box<str>> = hits.filter(held).map(|h| h.url.clone()).collect();
    let Some(c) = &mut self.complete else { return };
    let Some(r) = c.remote.as_mut().filter(|r| r.pending && r.epoch == epoch) else { return };
    let step = r.absorb(stv, ffz, &mut c.matches, |_, u| held.contains(u));
    let (word, none, done) = (r.search.clone(), c.matches.is_empty(), r.done());
    if step.chase {
      self.ask_catalog(false);
    } else if none && done {
      self.complete = None;
      self.say(f("app.complete.no_match", &[("word", &word)]));
      self.bar_dirty = true;
    } else if step.first {
      c.at = 0;
      self.show_pick();
    }
    self.input_dirty = true;
    self.wake();
  }

  // a line is leaving with the picks made in it (tab, the picker, a clicked emote). a pick that is not
  // what the word would have sent anyway is told to the room (`emotes:used`, the vouches returned), so
  // everyone draws the picture you saw in the box. one that is not here yet (not yours, not the room's, not
  // a global) also joins your heatsync inventory, as on the site; another emote you own under that name is
  // swapped for the one you picked, on purpose. the line never waits on the add (rule 7): the room draws
  // from the vouch while it lands, and a swap that fails leaves your old emote in place. never what you
  // block, and never over a name twitch marks itself on twitch (twitch draws its own whatever we say)
  pub(super) fn ship_picks(&mut self, text: &str, picks: &Registry) -> Vec<Vouch> {
    if self.me.is_none() || picks.is_empty() {
      return Vec::new();
    }
    let (d, res) = (self.draft(), &self.res);
    let chan = self.tabs.get(self.active).map_or("", |t| &*t.view.channel);
    let mut vouches = Vec::new();
    let mut adds = Vec::new();
    for (name, rec) in picks.sent(text) {
      let plain = d.word(None, &name);
      let native = d.leg == Some(Platform::Twitch) && matches!(plain, Some((_, From::Native)));
      if native || res.blocked.contains(&name, &rec.url) || plain.as_ref().is_some_and(|(f, _)| f.0 == rec.url) {
        continue;
      }
      vouches.push(Vouch { name: name.clone(), url: rec.url.clone(), provider: rec.provider });
      let owned = d.me.and_then(|id| res.senders.peek(id, &name, d.now)).map(|e| &*e.url);
      let here = |e: Option<&crate::emotes::Emote>| e.is_some_and(|e| e.url == rec.url);
      let room = d.leg.is_some_and(|p| here(res.emotes.channel_on(chan, p, &name)));
      // twitch draws its own emote of a name wherever it goes: never a heatsync emote under it
      let twitch = res.emotes.twitch_global(&name).is_some() || res.emotes.sub(&name).is_some();
      if room || twitch || owned == Some(&*rec.url) || (owned.is_none() && here(res.emotes.global(&name))) {
        continue;
      }
      let (url, zero_width) = (rec.url, rec.zero_width);
      adds.push(match owned {
        Some(_) => EmoteCmd::Swap { name, url, zero_width },
        None => EmoteCmd::Add { name, url, hash: None, zero_width },
      });
    }
    for cmd in adds {
      self.net.send(Cmd::Emote(cmd));
    }
    vouches
  }

  // put the cycle's current candidate in the box, followed by one space to keep typing (kept, not
  // doubled, while cycling). an emote shows as its picture; the one it replaces stops showing
  pub(super) fn show_pick(&mut self) {
    let Some(c) = &mut self.complete else { return };
    let Some((pick, pic)) = c.matches.get(c.at).map(|m| (m.name.clone(), m.pic.clone())) else { return };
    if let Some(old) = c.chipped.take() {
      self.line.chips.remove(&old);
    }
    // the candidate stepped onto is this draft's pick for its name: the box draws that picture and the
    // line ships it. a step only passed gives its name back as it found it, so an earlier pick of that
    // name in the same message stays
    if let Some((name, was)) = c.passed.take() {
      self.line.picks.put(&name, was);
    }
    if let Some(rec) = pic {
      c.passed = Some((pick.clone(), self.line.picks.put(&pick, Some(rec))));
    }
    self.line.replace(c.start..c.end, &pick);
    c.end = c.start + pick.len();
    if !self.line.text[c.end..].starts_with(' ') {
      self.line.insert(" ");
    } else {
      self.line.cursor = c.end + 1;
    }
    if let Some((name, _)) = c.emote()
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
      .filter(|c| matches!(c, Cmd::Catalog { .. } | Cmd::Emote(_) | Cmd::Hs(_) | Cmd::Send { .. }))
      .collect()
  }

  // your twitch sub emotes (dellor's delloreat) are tab-completed and shown in the box
  #[test]
  fn sub_emotes_tab_complete() {
    let (net, _rx) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    let url = "https://static-cdn.jtvnw.net/emoticons/v2/emotesv2_x/default/dark/1.0";
    let e = crate::emotes::Emote { url: url.into(), w: 28, h: 28, zero_width: false };
    app.res.emotes.set_subs([("delloreat".into(), e)].into());
    let c = app.candidates("delloreat", false);
    assert!(c.iter().any(|p| &*p.name == "delloreat" && p.what == "your sub emote · all twitch"), "{:?}", c.len());
    assert!(app.candidates(":dellore", false).iter().any(|p| &*p.name == "delloreat"));
    assert_eq!(app.box_url("delloreat").as_deref(), Some(url));
  }

  // setting tab_list: tab on a bare word opens the emote list instead of completing in place (ext 1de19fe5)
  #[test]
  fn tab_opens_the_emote_list_only_when_the_setting_is_on() {
    use winit::keyboard::{Key, NamedKey};
    let (net, _rx) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    signed_in(&mut app);
    let rows: Vec<_> = ["zzlistA", "zzlistB"]
      .iter()
      .map(|n| {
        serde_json::json!({"custom_name": n, "url": "https://cdn.7tv.app/emote/x/1x.webp", "width": 32,
        "height": 32, "added_at": "2026-10-30T22:26:51.099Z"})
      })
      .collect();
    app.res.senders.lookup(1, "zzlistA", 0);
    app.res.senders.put(1, crate::heatsync::senders::parse_set(&serde_json::json!(rows)));
    // off (the default): the classic tab completes in place and opens no list
    app.line.insert("say zzli");
    app.tab_complete(false);
    assert!(app.sug.list.is_none() && app.line.text.starts_with("say zzlist"), "{}", app.line.text);
    // on: the first tab lists, the text is untouched, down moves, enter inserts
    app.complete = None;
    app.line = Default::default();
    app.set.tab_list = true;
    app.line.insert("say zzli");
    app.tab_complete(false);
    let s = app.sug.list.as_ref().expect("the list is open");
    assert_eq!((s.items.len(), s.sel, app.line.text.as_str()), (2, 0, "say zzli"));
    app.sync_suggest();
    assert!(app.sug.list.is_some(), "nothing typed keeps it open");
    assert!(app.suggest_key(&Key::Named(NamedKey::ArrowDown)));
    assert!(app.suggest_key(&Key::Named(NamedKey::Enter)));
    assert!(app.sug.list.is_none() && app.line.text.starts_with("say zzlist"), "{}", app.line.text);
    // esc closes without touching the text
    app.complete = None;
    app.line = Default::default();
    app.line.insert("say zzli");
    app.tab_complete(false);
    assert!(app.suggest_key(&Key::Named(NamedKey::Escape)));
    assert_eq!((app.sug.list.is_none(), app.line.text.as_str()), (true, "say zzli"));
    // one letter is no word; a word nothing matches falls back to the classic tab
    for typed in ["z", "qqxxnothing"] {
      app.complete = None;
      app.line = Default::default();
      app.line.insert(typed);
      app.tab_complete(false);
      assert!(app.sug.list.is_none(), "{typed}");
    }
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

  // IDK means an emote here; the catalogs hold another IDK. tab never offers the look-alike (the name typed
  // in full keeps its one meaning), and the picker's search is where the other one is picked on purpose:
  // the box shows it, and sending it puts it in your inventory (swapping out an IDK you already had)
  #[test]
  fn another_emote_of_a_name_is_never_in_tab_and_is_picked_on_purpose_in_the_picker() {
    use crate::emotes::{Emote, Provider};
    use crate::model::catalog::Rec;
    use crate::net::NetEvent;
    let (net, mut rx) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    signed_in(&mut app);
    let url = |id: &str| format!("https://cdn.7tv.app/emote/{id}/1x.webp");
    let e = |id: &str| Emote { url: url(id).into(), w: 28, h: 28, zero_width: false };
    app.res.emotes.set(None, Provider::SevenTv, [("IDK".into(), e("CHAN"))].into());
    for (owned, swap) in [(None, false), (Some("OLD"), true)] {
      if let Some(o) = owned {
        let row = serde_json::json!({"custom_name": "IDK", "url": url(o), "width": 32, "height": 32,
          "added_at": "2026-10-30T22:26:51.099Z"});
        app.res.senders.lookup(1, "IDK", 0);
        app.res.senders.put(1, crate::heatsync::senders::parse_set(&serde_json::json!([row])));
      }
      (app.complete, app.line) = (None, Default::default());
      app.line.insert("lol IDK");
      app.tab_complete(false);
      let epoch = match asked(&mut rx).as_slice() {
        [Cmd::Catalog { epoch, .. }] => *epoch,
        _ => panic!("one catalog ask"),
      };
      let before: Vec<Box<str>> = app.complete.as_ref().unwrap().matches.iter().map(|p| p.name.clone()).collect();
      app.on_net(NetEvent::Catalog {
        epoch,
        stv: Some(page(vec![hit("IDK", "CHAN"), hit("IDK", "OTHER"), hit("IDKnew", "NEW")], false)),
        ffz: None,
      });
      let names: Vec<&str> = app.complete.as_ref().unwrap().matches.iter().map(|p| &*p.name).collect();
      assert_eq!(names.len(), before.len() + 1, "only the name nothing here answers joined: {names:?}");
      assert_eq!(names.last(), Some(&"IDKnew"));
      // the picker's tile of the other IDK: the box shows it
      (app.complete, app.line) = (None, Default::default());
      app.insert_pick("IDK", Rec::of(&hit("IDK", "OTHER")));
      assert_eq!(app.box_url("IDK").as_deref(), Some(&*url("OTHER")));
      app.send();
      let sent = asked(&mut rx);
      let cmd =
        sent.iter().find_map(|c| if let Cmd::Emote(e) = c { Some(e) } else { None }).expect("an inventory change");
      match (swap, cmd) {
        (false, EmoteCmd::Add { name, url: u, .. }) | (true, EmoteCmd::Swap { name, url: u, .. }) => {
          assert_eq!((&**name, &**u), ("IDK", &*url("OTHER")))
        }
        _ => panic!("wrong inventory change (swap expected: {swap})"),
      }
    }
  }

  // tab with nothing local asks the catalogs, a stale answer is dropped, the live one fills the cycle
  // (box only), and sending the tabbed name adds it to your inventory exactly once
  #[test]
  fn catalog_tab_search_cycle_and_the_pick_is_added_on_send() {
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
    // end the cycle on zzcatLol and send: it is the one that gets added, once. zzcatWide was only passed
    // on the way round, so typed by hand it is a word (a step you only passed is no pick)
    app.tab_complete(false);
    app.complete = None;
    assert!(app.line.picks.get("zzcatLol").is_some());
    assert!(app.line.picks.get("zzcat").is_none() && app.line.picks.get("zzcatWide").is_none());
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
      [("zzcatLol", "https://cdn.7tv.app/emote/A1/1x.webp", true, false)],
      "only the name the cycle ended on, once"
    );
    assert!(sent.iter().any(|c| matches!(c, Cmd::Send { text, .. } if text.contains("zzcatLol hello"))));
    // rule 7: the line never waits on those adds, and the room is told what was picked, once per room
    let vouched: Vec<_> = sent
      .iter()
      .filter_map(|c| match c {
        Cmd::Hs(crate::net::hs_ws::Cmd::Used(p, chan, v)) => {
          Some((*p, chan.to_string(), v.iter().map(|e| (e.name.to_string(), e.provider)).collect::<Vec<_>>()))
        }
        _ => None,
      })
      .collect();
    assert_eq!(
      vouched,
      [(crate::model::message::Platform::Twitch, "forsen".to_string(), vec![("zzcatLol".to_string(), "7tv")])]
    );
    assert!(app.line.picks.is_empty(), "the picks left with their message");
    app.line.insert("zzcatLol");
    app.send();
    let again = asked(&mut rx);
    assert!(
      again.iter().all(|c| !matches!(c, Cmd::Emote(_) | Cmd::Hs(crate::net::hs_ws::Cmd::Used(..)))),
      "added once, vouched once"
    );
  }

  // a name you own, block or get as a global is never added; logged out adds nothing; blocked names
  // never surface from the catalog
  #[test]
  fn a_sent_pick_skips_owned_blocked_and_logged_out() {
    use crate::emotes::block::Scope;
    let (net, mut rx) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    let url = |i: &str| format!("https://cdn.7tv.app/emote/{i}/1x.webp");
    let mut picks = crate::model::catalog::Registry::default();
    for (n, i) in [("aaown", "1"), ("aablk", "2"), ("aaok", "3")] {
      picks.track(&hit(n, i));
    }
    assert!(app.ship_picks("aaok", &picks).is_empty());
    assert!(asked(&mut rx).is_empty(), "logged out");
    signed_in(&mut app);
    let row = serde_json::json!({"custom_name": "aaown", "url": url("1"), "width": 32, "height": 32,
      "added_at": "2026-10-30T22:26:51.099Z"});
    app.res.senders.lookup(1, "aaown", 0);
    app.res.senders.put(1, crate::heatsync::senders::parse_set(&serde_json::json!([row])));
    app.res.blocked.block("aablk", &url("2"), Scope::Hs);
    let vouched = app.ship_picks("aaown aablk aaok", &picks);
    assert_eq!(
      vouched.iter().map(|v| &*v.name).collect::<Vec<_>>(),
      ["aaok"],
      "the pick of what the word already is says nothing"
    );
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
    cy.ask(1, 0, true);
    let blocked = |n: &str, u: &str| app.res.blocked.contains(n, u);
    cy.absorb(Some(page(vec![hit("aablk", "2"), hit("aafine", "4")], false)), None, &mut m, blocked);
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
      stv: Some(page(vec![hit("qqone", "L1"), hit("qqthree", "R2")], true)),
      ffz: None,
    });
    assert_eq!(app.complete.as_ref().unwrap().matches.len(), 3, "the very emote a local already is, is dropped");
    assert_eq!(app.line.text, "qqtwo ", "the pick under the cursor did not move");
    app.tab_complete(false);
    assert_eq!(app.line.text, "qqthree ");
    // the next trigger asks page 2, from 7tv only (ffz had nothing / failed)
    let next = asked(&mut rx);
    assert!(matches!(next.as_slice(), [Cmd::Catalog { page: 2, stv: true, ffz: false, .. }]), "{}", next.len());
  }

  fn url7(id: &str) -> String {
    format!("https://cdn.7tv.app/emote/{id}/1x.webp")
  }

  // the same word W in every source at once, for the composer rules below: the channel's ffz, a global 7tv,
  // and (when `own`) W in your own inventory
  fn w_everywhere(app: &mut App, own: bool) {
    everywhere(app, "W", own);
  }

  // colonListOrder: "emoji first" puts every emoji above the emotes in the `:` list only. Tab ignores it,
  // and inside each part the law's order stays (the room's emote still leads the emotes)
  #[test]
  fn emoji_first_reorders_the_colon_list_and_never_tab() {
    use crate::emotes::{Emote, Provider};
    use crate::model::settings::ColonList;
    use crate::model::suggest::Kind;
    let (net, _rx) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    let e = Emote { url: url7("CATJAM").into(), w: 28, h: 28, zero_width: false };
    app.res.emotes.set(Some("forsen"), Provider::Ffz, [("catJAM".into(), e)].into());
    let names = |app: &App, emoji_first: bool| -> Vec<String> {
      app.candidates_in(":cat", false, emoji_first).iter().map(|p| p.name.to_string()).collect()
    };
    let (emotes_first, emoji_first) = (names(&app, false), names(&app, true));
    assert_eq!(emotes_first[0], "catJAM", "emotes first: the room's emote leads");
    assert_eq!(emoji_first[0], "🐱", "emoji first: the cat leads");
    let (j, c) =
      (emoji_first.iter().position(|n| n == "catJAM").unwrap(), emoji_first.iter().position(|n| n == "🐱").unwrap());
    assert!(c < j && emoji_first[..j].iter().all(|n| n != "catJAM"), "{emoji_first:?}");
    let mut a = emotes_first.clone();
    let mut b = emoji_first.clone();
    a.sort();
    b.sort();
    assert_eq!(a, b, "the same candidates, reordered");
    // the list reads the setting; Tab's candidates never do
    app.set.colon_list = ColonList::Emoji;
    app.line.insert(":cat");
    app.sync_suggest();
    let s = app.sug.list.as_ref().expect("the `:` list is open");
    assert_eq!((s.kind, &*s.items[0].name), (Kind::Emote, "🐱"));
    assert_eq!(&*app.candidates(":cat", false)[0].name, "catJAM", "tab keeps the law's order");
  }

  fn everywhere(app: &mut App, name: &str, own: bool) {
    use crate::emotes::{Emote, Provider};
    let e = |id: &str| Emote { url: url7(id).into(), w: 28, h: 28, zero_width: false };
    app.res.emotes.set(Some("forsen"), Provider::Ffz, [(name.into(), e("CHANFFZ"))].into());
    app.res.emotes.set(None, Provider::SevenTv, [(name.into(), e("GLOBAL7"))].into());
    if own {
      let row = serde_json::json!({"custom_name": name, "url": url7("INV"), "width": 32, "height": 32,
        "added_at": "2026-10-30T22:26:51.099Z"});
      app.res.senders.lookup(1, name, 0);
      app.res.senders.put(1, crate::heatsync::senders::parse_set(&serde_json::json!([row])));
    }
  }

  // the emote law's send order in the box: the chip is exactly what sends. a pick, else the channel's, else
  // yours, else a global: a typed word is the room's emote even where you own a copy of the name, and tab
  // offers that one picture of the name, not yours or the global's
  #[test]
  fn the_chip_shows_what_sends_pick_then_channel_then_yours_then_globals() {
    let (net, _rx) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    signed_in(&mut app);
    w_everywhere(&mut app, true);
    let chip = |app: &App| app.box_url("W").map(|u| u.to_string());
    app.line.picks.track(&hit("W", "PICK"));
    assert_eq!(chip(&app), Some(url7("PICK")), "the pick");
    app.line.picks.put("W", None);
    assert_eq!(chip(&app), Some(url7("CHANFFZ")), "hand-typed: the channel's, over your own copy");
    let copies: Vec<(&str, String)> = app
      .candidates("W", false)
      .iter()
      .filter(|p| &*p.name == "W")
      .map(|p| (p.what, p.pic.as_ref().unwrap().url.to_string()))
      .collect();
    assert_eq!(copies, [("channel · ffz users", url7("CHANFFZ"))], "tab's W is the one typing it sends, once");
    app.res.emotes.set(Some("forsen"), crate::emotes::Provider::Ffz, Default::default());
    assert_eq!(chip(&app), Some(url7("INV")), "the channel has none: yours");
    app.res.senders.stale(1, Some("W"));
    assert_eq!(chip(&app), Some(url7("GLOBAL7")), "then the globals");
    assert_eq!(chip_case(&app, "w"), None, "case is exact");
  }

  // the law's headline: the chip in the box and the row that comes back are one answer. hand-typed, both
  // are the channel's emote (never your own copy of the name); picked, both are the pick, which the row
  // gets from the vouch
  #[test]
  fn the_chip_and_the_row_that_comes_back_are_the_same_picture() {
    use crate::model::message::{Kind, Message, Platform, flag};
    let (net, _rx) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    signed_in(&mut app);
    w_everywhere(&mut app, true);
    let row = |app: &mut App, pick: Option<&str>| {
      let mut m = Message::system(now_ms(), Platform::Twitch, "W");
      (m.kind, m.user_id, m.login) = (Kind::Chat, "1".into(), "mellen".into());
      m.flags |= flag::OWN;
      if let Some(u) = pick {
        m.extra().picks = vec![("W".into(), u.into(), false)].into();
      }
      crate::emotes::resolve::find_word(&mut app.res.sources(), "forsen", &m, 0, 1, "W").map(|f| f.0.to_string())
    };
    let chip = app.box_url("W").map(|u| u.to_string());
    assert_eq!(chip, Some(url7("CHANFFZ")));
    assert_eq!(row(&mut app, None), chip, "hand-typed: the row is the chip");
    app.line.picks.track(&hit("W", "PICK"));
    let chip = app.box_url("W").map(|u| u.to_string());
    assert_eq!(chip, Some(url7("PICK")));
    assert_eq!(row(&mut app, chip.as_deref()), chip, "picked: the vouched row is the chip");
  }

  // tab offers one picture of a name, the one typing it sends, so there is no second copy to step onto. a
  // picture of that name put in on purpose (an emote clicked in a chat line, a favorite) is the draft's pick:
  // the box shows it and the room is told. a pick whose word left the box is over once anything is typed
  #[test]
  fn tab_offers_the_one_wide_and_a_picture_put_in_on_purpose_is_vouched() {
    let (net, mut rx) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    signed_in(&mut app);
    everywhere(&mut app, "Wide", true);
    app.line.insert("Wide");
    app.tab_complete(false);
    assert_eq!(app.box_url("Wide").as_deref(), Some(&*url7("CHANFFZ")));
    app.tab_complete(false);
    assert_eq!(app.box_url("Wide").as_deref(), Some(&*url7("CHANFFZ")), "yours and the global's are not offered");
    let pick = app.line.picks.get("Wide").map(|r| r.url.to_string());
    assert_eq!(pick, Some(url7("CHANFFZ")), "tab onto the one Wide: its pick, which is what the word sends anyway");
    app.complete = None;
    app.line.take();
    let rec = app.pick_of("Wide", &url7("INV"));
    app.insert_pick("Wide", rec);
    assert_eq!(app.box_url("Wide").as_deref(), Some(&*url7("INV")), "yours, put in on purpose");
    app.send();
    let sent = asked(&mut rx);
    let vouched: Vec<String> = sent
      .iter()
      .filter_map(|c| match c {
        Cmd::Hs(crate::net::hs_ws::Cmd::Used(_, _, v)) => Some(v.iter().map(|e| e.url.to_string()).collect::<Vec<_>>()),
        _ => None,
      })
      .flatten()
      .collect();
    assert_eq!(vouched, [url7("INV")], "the room is told which Wide this line means");
    assert!(sent.iter().all(|c| !matches!(c, Cmd::Emote(_))), "it is already yours: nothing is added or swapped");
    // the channel's own Wide, picked: it is what the word sends anyway, so nothing is said
    app.line.insert("Wide");
    app.tab_complete(false);
    app.complete = None;
    asked(&mut rx);
    app.send();
    assert!(asked(&mut rx).iter().all(|c| matches!(c, Cmd::Send { .. } | Cmd::Hs(crate::net::hs_ws::Cmd::Origin(..)))));
    // a pick whose word left the box ends with the next thing typed
    app.line.picks.track(&hit("W", "PICK"));
    app.line.picks.prune("hello ");
    assert!(app.line.picks.is_empty());
  }

  // people are not emotes: the law ranks the emotes and says where its groups end. the person typed whole
  // follows the emote of that exact name, the other recent talkers follow the room's emotes, and what is
  // only yours or global comes after them
  #[test]
  fn people_join_tab_after_the_exact_name_and_after_the_rooms_emotes() {
    use crate::emotes::{Emote, Provider};
    use crate::model::message::{Kind, Message, Platform};
    let (net, _rx) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    signed_in(&mut app);
    let e = |id: &str| Emote { url: url7(id).into(), w: 28, h: 28, zero_width: false };
    app.res.emotes.set(
      Some("forsen"),
      Provider::SevenTv,
      [("zorp".into(), e("Z1")), ("zorpRoom".into(), e("Z2"))].into(),
    );
    app.res.emotes.set(None, Provider::Bttv, [("zorpGlobal".into(), e("Z3"))].into());
    for nick in ["Zorp", "zorpTalker"] {
      let mut m = Message::system(now_ms(), Platform::Twitch, "hi");
      m.kind = Kind::Chat;
      (m.login, m.nick) = (nick.to_lowercase().into(), nick.into());
      let at = app.active;
      app.tabs[at].view.push(m);
    }
    let names: Vec<String> = app.candidates("zorp", false).iter().map(|p| p.name.to_string()).collect();
    assert_eq!(names, ["zorp", "Zorp", "zorpRoom", "zorpTalker", "zorpGlobal"]);
    let at: Vec<String> = app.candidates("@zorp", false).iter().map(|p| p.name.to_string()).collect();
    assert!(at.iter().all(|n| n.starts_with('@')) && at.len() == 2, "@ is people only: {at:?}");
  }

  fn chip_case(app: &App, w: &str) -> Option<String> {
    app.box_url(w).map(|u| u.to_string())
  }

  // rule 7: a pick you don't own is added and the message goes out in the same breath (no hold, the room is
  // vouched for it); a pick on top of a different W you own swaps on purpose; a W you own and did not pick
  // again is never touched
  #[test]
  fn rule_7_a_pick_joins_the_inventory_without_holding_the_message() {
    let (net, mut rx) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    signed_in(&mut app);
    // no W of mine: the pick is an add, the line goes, the room is vouched
    app.line.picks.track(&hit("W", "PICK"));
    app.line.insert("W");
    app.send();
    let sent = asked(&mut rx);
    let kinds: Vec<&str> = sent
      .iter()
      .map(|c| match c {
        Cmd::Emote(EmoteCmd::Add { .. }) => "add",
        Cmd::Emote(EmoteCmd::Swap { .. }) => "swap",
        Cmd::Send { .. } => "send",
        Cmd::Hs(crate::net::hs_ws::Cmd::Used(..)) => "vouch",
        Cmd::Hs(crate::net::hs_ws::Cmd::Origin(..)) => "origin",
        _ => "other",
      })
      .collect();
    assert_eq!(kinds, ["add", "send", "origin", "vouch"]);
    // a different W of mine: the pick replaces it on purpose, in one swap
    w_everywhere(&mut app, true);
    app.line.picks.track(&hit("W", "PICK"));
    app.line.insert("W");
    app.send();
    assert!(asked(&mut rx).iter().any(|c| matches!(c, Cmd::Emote(EmoteCmd::Swap { name, .. }) if &**name == "W")));
    // the same W again, no pick: nothing is added, swapped or vouched
    app.line.insert("W");
    app.send();
    let again = asked(&mut rx);
    assert!(
      again.iter().all(|c| matches!(c, Cmd::Send { .. } | Cmd::Hs(crate::net::hs_ws::Cmd::Origin(..)))),
      "just the line, and the word to heatsync that it went"
    );
  }
}
