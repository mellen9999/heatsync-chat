// tab / shift-tab in the message box: the first press gathers candidates for the word around the
// cursor, the next ones cycle them in place, and a tooltip over the box names the shown one
use super::App;
use crate::emotes::Provider;
use crate::model::complete::{Cand, Complete, MIN_WORD, Pick, commands, emotes, people};
use crate::model::slash;
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
    let chat = |n: Box<str>| Pick { name: n, what: "chatter", emote: false };
    if slash {
      return commands(word, [slash::all().into_iter().map(|c| c.name).collect()]);
    }
    let recent = tab.map_or_else(Vec::new, |t| t.view.recent_chatters(now_ms() - RECENT_MS, RECENT_MAX));
    if let Some(q) = word.strip_prefix('@') {
      let all = self.everyone();
      let found = people(q, &recent, &all, false);
      return found.into_iter().map(|(n, _)| chat(format!("@{}", n.trim_start_matches('@')).into())).collect();
    }
    // `:kek` searches emotes too (the ext's emoji shortcodes are not here)
    let q = word.strip_prefix(':').filter(|q| q.len() >= MIN_WORD).unwrap_or(word);
    let chan = tab.map_or("", |t| &*t.view.channel);
    let now = now_ms();
    let blocked = |n: &str, url: &str| self.res.blocked.contains(n, url);
    let channel = self.res.emotes.channel_names(chan).into_iter().filter(|(n, _, e)| !blocked(n, &e.url));
    let channel = channel.map(|(name, p, _)| Cand { name, tier: 0, what: what(true, p) });
    let own = self.own_id().map_or_else(Vec::new, |id| self.res.senders.entries(id, now));
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
      if matches.is_empty() {
        self.status = format!("no match for {word}");
        self.bar_dirty = true;
        return self.wake();
      }
      self.complete = Some(Complete { start: r.start, end: r.end, matches, at: usize::MAX, chipped: None });
    }
    let Some(c) = &mut self.complete else { return };
    let n = c.matches.len();
    c.at = match (c.at, back) {
      (usize::MAX, false) => 0,
      (usize::MAX, true) => n - 1,
      (i, false) => (i + 1) % n,
      (i, true) => (i + n - 1) % n,
    };
    self.show_pick();
  }

  // put the cycle's current candidate in the box, followed by one space to keep typing (kept, not
  // doubled, while cycling). an emote shows as its picture; the one it replaces stops showing
  pub(super) fn show_pick(&mut self) {
    let Some(c) = &mut self.complete else { return };
    let Some(pick) = c.matches.get(c.at).map(|m| m.name.clone()) else { return };
    if let Some(old) = c.chipped.take() {
      self.line.chips.remove(&old);
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
