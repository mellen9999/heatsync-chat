// THE render chokepoint: the one place a word in a chat line becomes an emote. privacy invariant: a
// word renders as a heatsync emote iff the SENDER's own inventory has it (or the line's own
// server-verified refs say so), and as a 7tv personal emote iff 7tv's entitlements name the SENDER's own
// twitch id as owner of the set. catalog and search caches never feed this path, and the only inputs it
// can see are the ones in `Sources` — change what a heatsync or personal emote is and it is changed here,
// nowhere else
use super::block::Scope;
use super::personal::Personal;
use super::{Emote, Emotes};
use crate::heatsync::senders::Senders;
use crate::model::message::{Message, Platform, flag};

// what a word may become an emote from: the channel/global catalogs, the senders' own inventories, the
// renderInventoryEmotes switch, and the senders' own 7tv personal sets (empty while their switch is off).
// built by `Res::sources`, so nothing else can reach the render path
pub struct Sources<'a> {
  pub emotes: &'a Emotes,
  pub senders: &'a mut Senders,
  pub personal: &'a mut Personal,
  pub inventory: bool,
}

// url, native size (0 = unknown), zero-width overlay, where it came from
pub type Found = (Box<str>, (u16, u16), bool, Scope);

// the word at body bytes a..b as an emote: twitch's own by position, else by name
pub fn find_word(src: &mut Sources, chan: &str, m: &Message, a: usize, b: usize, word: &str) -> Option<Found> {
  let sender = (m.platform == Platform::Twitch).then(|| m.user_id.parse::<u64>().ok()).flatten();
  // a placed emote is the platform's own (twitch tags, kick tokens, an archive's provider mirrors). a
  // heatsync inventory image never is, whoever placed it: it goes through `resolve` like any other word
  let placed = m.emotes.iter().find(|(r, _)| r.start as usize == a && r.end as usize == b);
  match placed.filter(|(_, url)| !crate::heatsync::inventory_url(url)) {
    Some((_, url)) => Some((url.clone(), (0, 0), false, Scope::Native)),
    None if word.is_empty() => None,
    None => resolve(src, chan, m, sender, word),
  }
}

// order: the line's own server-verified refs, the sender's heatsync inventory (the ext's refs carry
// exactly that, ahead of the channel), the sender's 7tv personal set, the channel catalog, then globals.
// the sender's own sets are the only heatsync / personal sources (privacy invariant).
// personal vs channel: the personal set wins, as in chatterino7 and the 7tv extension: it is the one the
// sender picked, and the same name stays the channel's for everyone else's lines. heatsync vs personal:
// heatsync wins, as in the ext, which lays the sender's heatsync set over the 7tv personal one it merges
fn resolve(src: &mut Sources, chan: &str, m: &Message, sender: Option<u64>, word: &str) -> Option<Found> {
  let e = |e: &Emote, sc| (e.url.clone(), (e.w, e.h), e.zero_width, sc);
  // the one gate for renderInventoryEmotes: off, nothing from a heatsync inventory ever draws
  let inventory = src.inventory;
  if inventory && let Some((_, u, zw)) = m.hs.iter().find(|(w, _, _)| **w == *word) {
    return Some((u.clone(), (0, 0), *zw, Scope::Hs));
  }
  // the sender's own emote of this name beats the channel's: it is the one they picked (heatsync's
  // own per-line refs say the same for relayed lines, above)
  if let Some(x) = sender.filter(|_| inventory).and_then(|id| src.senders.lookup(id, word, m.ts)) {
    return Some((x.url.clone(), (x.w, x.h), x.zero_width, Scope::Hs));
  }
  // a 7tv personal emote is a 7tv emote: a block files it under its 7tv id, like a channel's
  if let Some(x) = sender.and_then(|id| src.personal.lookup(id, word)) {
    return Some(e(x, Scope::Channel));
  }
  if let Some(x) = src.emotes.channel(chan, word) {
    return Some(e(x, Scope::Channel));
  }
  // our own echoed line carries no emote positions: twitch's globals and our sub emotes by name
  if m.has(flag::OWN)
    && let Some(x) = src.emotes.twitch_global(word).or_else(|| src.emotes.sub(word))
  {
    return Some(e(x, Scope::Global));
  }
  src.emotes.global(word).map(|x| e(x, Scope::Global))
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::emotes::Provider;
  use crate::model::message::Kind;

  struct World {
    emotes: Emotes,
    senders: Senders,
    personal: Personal,
    inventory: bool,
  }

  impl World {
    fn new() -> Self {
      let mut personal = Personal::default();
      personal.set_on(true);
      Self { emotes: Emotes::default(), senders: Senders::default(), personal, inventory: true }
    }

    fn sources(&mut self) -> Sources<'_> {
      Sources {
        emotes: &self.emotes,
        senders: &mut self.senders,
        personal: &mut self.personal,
        inventory: self.inventory,
      }
    }

    // 7tv says these twitch accounts own the set `id`, and the set holds these emotes (name → 7tv id)
    fn own(&mut self, users: &[u64], id: &str, emotes: &[(&str, &str)]) {
      use crate::emotes::personal::Ev;
      self.personal.apply(Ev::Grant { users: users.to_vec(), set: id.into() });
      let set = emotes.iter().map(|(n, i)| ((*n).into(), cdn(i))).collect();
      self.personal.apply(Ev::Set(id.into(), set));
    }
  }

  fn cdn(id: &str) -> Emote {
    Emote { url: format!("https://cdn.7tv.app/emote/{id}/1x.webp").into(), w: 28, h: 28, zero_width: false }
  }

  fn plain(id: &str, body: &str) -> Message {
    let mut m = Message::system(1, Platform::Twitch, "x");
    m.kind = Kind::Chat;
    m.id = id.into();
    m.nick = "bob".into();
    m.body = body.into();
    m
  }

  // a channel's IDK and the sender's own, different IDK: the sender's is drawn (it is the one they
  // picked); anyone else's IDK is still the channel's
  #[test]
  fn the_senders_own_emote_beats_the_channels_of_the_same_name() {
    let mut w = World::new();
    let url = |id: &str| format!("https://cdn.7tv.app/emote/{id}/1x.webp");
    let e = Emote { url: url("CHAN").into(), w: 28, h: 28, zero_width: false };
    w.emotes.set(Some("c"), Provider::SevenTv, [("IDK".into(), e)].into());
    let ts = 1_793_399_211_099;
    let row = serde_json::json!({"custom_name": "IDK", "url": url("MINE"), "width": 32, "height": 32,
      "added_at": "2026-10-30T22:26:51.099Z"});
    w.senders.lookup(42, "IDK", ts);
    w.senders.put(42, crate::heatsync::senders::parse_set(&serde_json::json!([row])));
    let mut m = plain("a", "IDK");
    (m.user_id, m.ts) = ("42".into(), ts);
    let got = |w: &mut World, m: &Message, who: Option<u64>| {
      resolve(&mut w.sources(), "c", m, who, "IDK").map(|f| f.0.to_string())
    };
    assert_eq!(got(&mut w, &m, Some(42)), Some(url("MINE")));
    assert_eq!(got(&mut w, &m, Some(7)), Some(url("CHAN")), "someone without their own IDK");
    w.inventory = false;
    assert_eq!(got(&mut w, &m, Some(42)), Some(url("CHAN")), "inventory emotes off: the channel's");
  }

  // our own echoed line has no emote positions: our sub emotes draw by name there, nowhere else
  #[test]
  fn sub_emotes_draw_in_our_own_lines_only() {
    let mut w = World::new();
    let url = "https://static-cdn.jtvnw.net/emoticons/v2/emotesv2_x/default/dark/1.0";
    w.emotes.set_subs([("delloreat".into(), Emote { url: url.into(), w: 28, h: 28, zero_width: false })].into());
    let mut own = plain("a", "delloreat");
    own.flags |= flag::OWN;
    assert_eq!(resolve(&mut w.sources(), "c", &own, None, "delloreat").map(|f| f.0.to_string()).as_deref(), Some(url));
    assert!(
      resolve(&mut w.sources(), "c", &plain("b", "delloreat"), None, "delloreat").is_none(),
      "someone else's line"
    );
  }

  // a heatsync image placed on a line by position (any producer) is not the platform's own: it goes
  // the sender's-inventory road, so a sender without it gets the plain word. a provider mirror is native
  #[test]
  fn a_placed_heatsync_image_is_not_native() {
    let mut w = World::new();
    let mut m = plain("p", "kek hi");
    m.user_id = "44".into();
    m.emotes = Box::new([(0..3, "https://heatsync.org/e/hs/abc".into())]);
    assert!(find_word(&mut w.sources(), "c", &m, 0, 3, "kek").is_none(), "not 44's");
    m.emotes = Box::new([(0..3, "https://heatsync.org/e/7tv/abc".into())]);
    assert!(matches!(find_word(&mut w.sources(), "c", &m, 0, 3, "kek"), Some((_, _, false, Scope::Native))));
  }

  fn word_of(w: &mut World, chan: &str, who: &str, kind: Platform, word: &str) -> Option<(String, Scope)> {
    let mut m = plain("p", word);
    (m.user_id, m.platform) = (who.into(), kind);
    find_word(&mut w.sources(), chan, &m, 0, word.len(), word).map(|f| (f.0.to_string(), f.3))
  }

  // PRIVACY INVARIANT: a personal emote draws on its OWNER's lines and on nobody else's: a different sender
  // typing the same word gets the plain word (or the channel's emote of that name, if it has one)
  #[test]
  fn a_personal_emote_draws_for_its_owner_only() {
    let mut w = World::new();
    w.own(&[42], "SET", &[("DonkMonk", "DONK")]);
    let on = |w: &mut World, who| word_of(w, "c", who, Platform::Twitch, "DonkMonk");
    assert_eq!(on(&mut w, "42"), Some((cdn("DONK").url.to_string(), Scope::Channel)));
    assert!(on(&mut w, "7").is_none(), "another sender, plain text");
    assert!(on(&mut w, "").is_none() && on(&mut w, "x").is_none(), "no usable sender id");
    // the channel has a DonkMonk of its own: everyone else still gets that one, never the owner's
    let chan = Emote { url: cdn("CHAN").url, w: 28, h: 28, zero_width: false };
    w.emotes.set(Some("c"), Provider::SevenTv, [("DonkMonk".into(), chan)].into());
    assert_eq!(on(&mut w, "7").map(|f| f.0), Some(cdn("CHAN").url.to_string()));
    assert_eq!(on(&mut w, "42").map(|f| f.0), Some(cdn("DONK").url.to_string()), "the owner's beats the channel's");
    // in another channel too: a personal set goes where its owner goes
    let there = word_of(&mut w, "elsewhere", "42", Platform::Twitch, "DonkMonk");
    assert_eq!(there.map(|f| f.0), Some(cdn("DONK").url.to_string()));
    // exact case, like every emote
    assert!(word_of(&mut w, "elsewhere", "42", Platform::Twitch, "donkmonk").is_none());
  }

  // only a twitch line has a sender id we look up by: a kick or youtube chatter whose id happens to equal a
  // twitch owner's gets nothing
  #[test]
  fn a_personal_emote_never_draws_on_a_kick_or_youtube_line() {
    let mut w = World::new();
    w.own(&[42], "SET", &[("DonkMonk", "DONK")]);
    for p in [Platform::Kick, Platform::Youtube] {
      assert!(word_of(&mut w, "c", "42", p, "DonkMonk").is_none(), "{p:?}");
    }
  }

  #[test]
  fn the_switch_off_draws_none() {
    let mut w = World::new();
    w.own(&[42], "SET", &[("DonkMonk", "DONK")]);
    w.personal.set_on(false);
    assert!(word_of(&mut w, "c", "42", Platform::Twitch, "DonkMonk").is_none());
  }

  // the twitch emotes a line carries by position are twitch's, whatever a personal set calls its own
  #[test]
  fn a_placed_twitch_emote_is_not_overridden_by_a_personal_one() {
    let mut w = World::new();
    w.own(&[42], "SET", &[("Kappa", "MINE")]);
    let mut m = plain("p", "Kappa");
    m.user_id = "42".into();
    m.emotes = Box::new([(0..5, "https://static-cdn.jtvnw.net/emoticons/v2/25/default/dark/1.0".into())]);
    let got = find_word(&mut w.sources(), "c", &m, 0, 5, "Kappa").unwrap();
    assert_eq!((got.0.contains("jtvnw"), got.3), (true, Scope::Native));
  }

  // order: the line's own refs, the sender's heatsync inventory, their personal set, the channel, globals
  #[test]
  fn heatsync_beats_personal_beats_channel_beats_global() {
    let mut w = World::new();
    let ts = 1_793_399_211_099;
    w.emotes.set(None, Provider::Bttv, [("IDK".into(), cdn("GLOBAL"))].into());
    let id = |w: &mut World| word_of(w, "c", "42", Platform::Twitch, "IDK").map(|f| f.0);
    assert_eq!(id(&mut w), Some(cdn("GLOBAL").url.to_string()));
    w.emotes.set(Some("c"), Provider::SevenTv, [("IDK".into(), cdn("CHAN"))].into());
    assert_eq!(id(&mut w), Some(cdn("CHAN").url.to_string()));
    w.own(&[42], "SET", &[("IDK", "PERSONAL")]);
    assert_eq!(id(&mut w), Some(cdn("PERSONAL").url.to_string()));
    let row = serde_json::json!({"custom_name": "IDK", "url": cdn("HS").url, "width": 32, "height": 32,
      "added_at": "2026-10-30T22:26:51.099Z"});
    w.senders.lookup(42, "IDK", ts);
    w.senders.put(42, crate::heatsync::senders::parse_set(&serde_json::json!([row])));
    let mut m = plain("a", "IDK");
    (m.user_id, m.ts) = ("42".into(), ts);
    let got = find_word(&mut w.sources(), "c", &m, 0, 3, "IDK").map(|f| f.0.to_string());
    assert_eq!(got, Some(cdn("HS").url.to_string()));
    // heatsync emotes off: the personal one is next
    w.inventory = false;
    let got = find_word(&mut w.sources(), "c", &m, 0, 3, "IDK").map(|f| f.0.to_string());
    assert_eq!(got, Some(cdn("PERSONAL").url.to_string()));
  }
}
