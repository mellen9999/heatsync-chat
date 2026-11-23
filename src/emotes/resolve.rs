// THE render chokepoint: the one place a word in a chat line becomes an emote. privacy invariant: a
// word renders as a heatsync emote iff the SENDER's own inventory has it (or the line's own
// server-verified refs say so) or the line's room's STREAMER has it (their set is the room's, public by
// the owner route), and as a 7tv personal emote iff 7tv's entitlements name the SENDER's own twitch id as
// owner of the set. catalog and search caches never feed this path, and the only inputs it can see are the
// ones in `Sources` — change what a heatsync or personal emote is and it is changed here, nowhere else
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

// the emote law's read order (the site's docs/emote-law.md; src/law is the same rule as one pure module,
// and `law::tests` runs the site's read cases through this function). first hit wins:
//   1. on our own twitch line, twitch's own emote of the name: twitch marks it on the line it delivers
//      whatever else we hold (a platform token by position is `find_word`'s, above)
//   2. the line's pick: what the sender's app said the word is
//   3. the sender's 7tv personal set (as chatterino7 and the 7tv extension read it)
//   4. the room, the line's own platform's: the streamer's own heatsync emotes, then the channel's ffz,
//      7tv, bttv, and on a kick line the tab's kick channel's own emotes (a twitch or youtube line never
//      reads those)
//   5. the sender's own heatsync emote of the name, on the sender's lines only: the server's name-match
//      on the line, then their set
//   6. the globals
// the sender's own sets are the only heatsync / personal sources (privacy invariant): nothing of the
// viewer's can reach someone else's line, because nothing of the viewer's is passed in
fn resolve(src: &mut Sources, chan: &str, m: &Message, sender: Option<u64>, word: &str) -> Option<Found> {
  let e = |e: &Emote, sc| (e.url.clone(), (e.w, e.h), e.zero_width, sc);
  // the one gate for renderInventoryEmotes: off, nothing that came through heatsync ever draws
  let inventory = src.inventory;
  // our own echoed line carries no emote positions: twitch's globals and our sub emotes by name, there only
  if m.has(flag::OWN)
    && m.platform == Platform::Twitch
    && let Some(x) = src.emotes.twitch_global(word).or_else(|| src.emotes.sub(word))
  {
    return Some(e(x, Scope::Global));
  }
  if inventory && let Some((_, u, zw)) = m.x.as_ref().and_then(|x| x.picks.iter().find(|(w, _, _)| **w == *word)) {
    // a pick that was taken down is text, never a look-alike of the same name
    return (!u.is_empty()).then(|| (u.clone(), (0, 0), *zw, Scope::Hs));
  }
  // a 7tv personal emote is a 7tv emote: a block files it under its 7tv id, like a channel's
  if let Some(x) = sender.and_then(|id| src.personal.lookup(id, word)) {
    return Some(e(x, Scope::Channel));
  }
  if inventory && let Some(x) = src.emotes.owner_on(chan, m.platform, word) {
    return Some(e(x, Scope::Hs));
  }
  if let Some(x) = src.emotes.channel_on(chan, m.platform == Platform::Kick, word) {
    return Some(e(x, Scope::Channel));
  }
  if inventory && let Some((_, u, zw)) = m.hs.iter().find(|(w, _, _)| **w == *word) {
    return Some((u.clone(), (0, 0), *zw, Scope::Hs));
  }
  if let Some(x) = sender.filter(|_| inventory).and_then(|id| src.senders.lookup(id, word, m.ts)) {
    return Some((x.url.clone(), (x.w, x.h), x.zero_width, Scope::Hs));
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

  // a channel's IDK and the sender's own, different IDK: a typed IDK is the channel's for everyone (the
  // law: in a room a word is the room's emote). the sender's own draws where the channel has none, and
  // over the channel's only when their app said so (a pick)
  #[test]
  fn a_typed_word_is_the_channels_and_the_senders_own_draws_where_the_channel_has_none() {
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
    let got = |w: &mut World, m: &Message, chan: &str, who: Option<u64>| {
      resolve(&mut w.sources(), chan, m, who, "IDK").map(|f| f.0.to_string())
    };
    assert_eq!(got(&mut w, &m, "c", Some(42)), Some(url("CHAN")), "typed: the channel's, their own IDK or not");
    assert_eq!(got(&mut w, &m, "c", Some(7)), Some(url("CHAN")), "someone without their own IDK");
    assert_eq!(got(&mut w, &m, "elsewhere", Some(42)), Some(url("MINE")), "a channel with no IDK: theirs");
    // the server's name-match on the line is the same emote of theirs: no more than their set
    m.hs = Box::new([("IDK".into(), url("MINE").into(), false)]);
    assert_eq!(got(&mut w, &m, "c", Some(42)), Some(url("CHAN")), "a name-match is not a pick");
    // their app said this line's IDK is theirs: that is the line's emote
    m.extra().picks = Box::new([("IDK".into(), url("MINE").into(), false)]);
    assert_eq!(got(&mut w, &m, "c", Some(42)), Some(url("MINE")), "picked: theirs, over the channel's");
    w.inventory = false;
    assert_eq!(got(&mut w, &m, "c", Some(42)), Some(url("CHAN")), "heatsync emotes off: the channel's");
    assert_eq!(got(&mut w, &m, "elsewhere", Some(42)), None, "and nothing of theirs anywhere");
  }

  // the streamer's own heatsync emote is the room's first answer: over the channel's ffz, 7tv and bttv,
  // under a pick and the sender's 7tv personal emote, and on its own platform's lines only
  #[test]
  fn the_streamers_own_emote_leads_the_room_on_its_platforms_lines() {
    let mut w = World::new();
    let url = |id: &str| format!("https://cdn.7tv.app/emote/{id}/1x.webp");
    for (p, id) in [(Provider::Ffz, "FFZ"), (Provider::SevenTv, "7TV"), (Provider::Bttv, "BTTV")] {
      w.emotes.set(Some("c"), p, [("KEKW".into(), cdn(id))].into());
    }
    let leg = [(Platform::Twitch, Box::from("c"))];
    w.emotes.route_owner([("c", &leg[..])]);
    let [leg] = leg;
    assert!(w.emotes.set_owner(leg, [("KEKW".into(), cdn("OWNER")), ("Theirs".into(), cdn("ONLY"))].into()));
    let got = |w: &mut World, m: &Message, word: &str| {
      let who = m.user_id.parse().ok();
      resolve(&mut w.sources(), "c", m, who, word).map(|f| (f.0.to_string(), f.3))
    };
    let mut m = plain("a", "KEKW Theirs");
    m.user_id = "42".into();
    assert_eq!(got(&mut w, &m, "KEKW"), Some((url("OWNER"), Scope::Hs)));
    assert_eq!(got(&mut w, &m, "Theirs"), Some((url("ONLY"), Scope::Hs)), "a name only the streamer has");
    assert_eq!(got(&mut w, &m, "kekw"), None, "case is exact");
    let mut kick = m.clone();
    kick.platform = Platform::Kick;
    assert_eq!(got(&mut w, &kick, "KEKW").map(|f| f.0), Some(url("FFZ")), "a kick line: not the twitch streamer's");
    // the sender's 7tv personal emote and a pick are facts about the line: both stay above the room
    w.own(&[42], "SET", &[("KEKW", "PERSONAL")]);
    assert_eq!(got(&mut w, &m, "KEKW").map(|f| f.0), Some(url("PERSONAL")));
    m.extra().picks = Box::new([("KEKW".into(), url("PICK").into(), false)]);
    assert_eq!(got(&mut w, &m, "KEKW").map(|f| f.0), Some(url("PICK")));
    w.inventory = false;
    assert_eq!(got(&mut w, &plain("b", "KEKW"), "KEKW").map(|f| f.0), Some(url("FFZ")), "heatsync emotes off");
    assert_eq!(got(&mut w, &plain("b", "Theirs"), "Theirs"), None);
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
    // a kick line of ours: kick shows the word, so does the row
    let mut on_kick = own.clone();
    on_kick.platform = Platform::Kick;
    assert!(resolve(&mut w.sources(), "c", &on_kick, None, "delloreat").is_none(), "our kick line");
  }

  // twitch marks its own emote on the line it delivers, whatever the channel or our pick calls that name:
  // our own twitch row draws what everyone else will see
  #[test]
  fn on_our_own_twitch_line_twitchs_emote_of_a_name_leads() {
    let mut w = World::new();
    let tw = "https://static-cdn.jtvnw.net/emoticons/v2/25/default/dark/1.0";
    w.emotes.set(
      None,
      Provider::Twitch,
      [("Kappa".into(), Emote { url: tw.into(), w: 28, h: 28, zero_width: false })].into(),
    );
    w.emotes.set(Some("c"), Provider::SevenTv, [("Kappa".into(), cdn("CHAN"))].into());
    let mut own = plain("a", "Kappa");
    own.flags |= flag::OWN;
    own.extra().picks = Box::new([("Kappa".into(), cdn("PICK").url, false)]);
    let got = |w: &mut World, m: &Message| resolve(&mut w.sources(), "c", m, None, "Kappa").map(|f| f.0.to_string());
    assert_eq!(got(&mut w, &own).as_deref(), Some(tw));
    // someone else's untagged Kappa is an ordinary word: the channel's
    assert_eq!(got(&mut w, &plain("b", "Kappa")), Some(cdn("CHAN").url.to_string()));
    // on kick nothing marks it: our pick is what ships
    own.platform = Platform::Kick;
    assert_eq!(got(&mut w, &own), Some(cdn("PICK").url.to_string()));
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

  // order: the line's pick, the sender's 7tv personal set, the channel, the sender's own heatsync emote, globals
  #[test]
  fn pick_beats_personal_beats_channel_beats_the_senders_own_beats_global() {
    let mut w = World::new();
    let ts = 1_793_399_211_099;
    let mut m = plain("a", "IDK");
    (m.user_id, m.ts) = ("42".into(), ts);
    let id = |w: &mut World, m: &Message| find_word(&mut w.sources(), "c", m, 0, 3, "IDK").map(|f| f.0.to_string());
    w.emotes.set(None, Provider::Bttv, [("IDK".into(), cdn("GLOBAL"))].into());
    assert_eq!(id(&mut w, &m), Some(cdn("GLOBAL").url.to_string()));
    let row = serde_json::json!({"custom_name": "IDK", "url": cdn("HS").url, "width": 32, "height": 32,
      "added_at": "2026-10-30T22:26:51.099Z"});
    w.senders.lookup(42, "IDK", ts);
    w.senders.put(42, crate::heatsync::senders::parse_set(&serde_json::json!([row])));
    assert_eq!(id(&mut w, &m), Some(cdn("HS").url.to_string()), "their own, over a global");
    w.emotes.set(Some("c"), Provider::SevenTv, [("IDK".into(), cdn("CHAN"))].into());
    assert_eq!(id(&mut w, &m), Some(cdn("CHAN").url.to_string()), "the channel's, over their own");
    w.own(&[42], "SET", &[("IDK", "PERSONAL")]);
    assert_eq!(id(&mut w, &m), Some(cdn("PERSONAL").url.to_string()), "their 7tv personal, over the channel's");
    m.extra().picks = Box::new([("IDK".into(), cdn("PICK").url, false)]);
    assert_eq!(id(&mut w, &m), Some(cdn("PICK").url.to_string()), "the line's pick, over everything");
    // heatsync emotes off: the pick and their own set are gone, the rest stands
    w.inventory = false;
    assert_eq!(id(&mut w, &m), Some(cdn("PERSONAL").url.to_string()));
  }

  // a pick whose emote was taken down is text: no look-alike of the name steps in
  #[test]
  fn a_taken_down_pick_is_text() {
    let mut w = World::new();
    w.emotes.set(Some("c"), Provider::SevenTv, [("IDK".into(), cdn("CHAN"))].into());
    let mut m = plain("a", "IDK");
    m.extra().picks = Box::new([("IDK".into(), "".into(), false)]);
    assert!(find_word(&mut w.sources(), "c", &m, 0, 3, "IDK").is_none());
  }

  // ---- the emote rules (docs/planning/emote-rules-2026-11-07.md), each staged with the same word W in
  // every source at once, the way the site's gate stages it ----

  const TS: i64 = 1_793_399_211_099;

  // W in the channel (ffz, 7tv, bttv), in the globals (7tv, bttv, ffz, twitch) and in `inventory` ids' own
  // sets: every one a different picture, named by where it came from
  fn collide(w: &mut World, inventory: &[u64]) {
    for (chan, p, tag) in [
      (Some("c"), Provider::Ffz, "c-ffz"),
      (Some("c"), Provider::SevenTv, "c-7tv"),
      (Some("c"), Provider::Bttv, "c-bttv"),
      (None, Provider::SevenTv, "g-7tv"),
      (None, Provider::Bttv, "g-bttv"),
      (None, Provider::Ffz, "g-ffz"),
      (None, Provider::Twitch, "g-twitch"),
    ] {
      w.emotes.set(chan, p, [("W".into(), cdn(tag))].into());
    }
    for &id in inventory {
      let row = serde_json::json!({"custom_name": "W", "url": cdn(&format!("inv{id}")).url, "width": 32,
        "height": 32, "added_at": "2026-10-30T22:26:51.099Z"});
      w.senders.lookup(id, "W", TS);
      w.senders.put(id, crate::heatsync::senders::parse_set(&serde_json::json!([row])));
    }
  }

  fn line(who: &str, body: &str) -> Message {
    let mut m = plain("r", body);
    (m.user_id, m.ts) = (who.into(), TS);
    m
  }

  fn drawn(w: &mut World, m: &Message, word: &str) -> Option<String> {
    find_word(&mut w.sources(), "c", m, 0, word.len(), word).map(|f| {
      let id = f.0.rsplit('/').nth(1).unwrap_or("").to_owned();
      if id.is_empty() { f.0.to_string() } else { id }
    })
  }

  // rule 1: a platform token is exact by position. nothing with the same name replaces it: not the sender's
  // inventory, not their line's refs, not the channel's or a global
  #[test]
  fn rule_1_a_platform_token_is_exact_by_position() {
    let mut w = World::new();
    collide(&mut w, &[42]);
    let tw = "https://static-cdn.jtvnw.net/emoticons/v2/25/default/dark/1.0";
    let mut m = line("42", "W W");
    m.emotes = Box::new([(0..1, tw.into())]);
    m.extra().picks = Box::new([("W".into(), cdn("ref").url, false)]);
    let first = find_word(&mut w.sources(), "c", &m, 0, 1, "W").unwrap();
    assert_eq!((&*first.0, first.3), (tw, Scope::Native), "the token at 0..1");
    // the same word at a place no token covers is an ordinary word
    let second = find_word(&mut w.sources(), "c", &m, 2, 3, "W").unwrap();
    assert_eq!(second.3, Scope::Hs, "the line's own pick, not the token's picture");
    // a kick token is the same
    let mut k = line("42", "W");
    k.platform = Platform::Kick;
    k.emotes = Box::new([(0..1, "https://files.kick.com/emotes/777/fullsize".into())]);
    assert_eq!(find_word(&mut w.sources(), "c", &k, 0, 1, "W").unwrap().3, Scope::Native);
  }

  // rule 2: the sender's pick on the line is the line's pick, for any viewer: over their inventory's other W,
  // the channel's W and every global W
  #[test]
  fn rule_2_the_senders_ref_on_the_line_wins() {
    let mut w = World::new();
    collide(&mut w, &[42]);
    let mut m = line("42", "W");
    m.extra().picks = Box::new([("W".into(), cdn("pick").url, false)]);
    assert_eq!(drawn(&mut w, &m, "W").as_deref(), Some("pick"));
    // a sender with no inventory at all (a kick chatter, a relayed line) still has the pick honoured
    m.user_id = "".into();
    m.platform = Platform::Kick;
    assert_eq!(drawn(&mut w, &m, "W").as_deref(), Some("pick"));
    // the viewer's own W changes nothing about someone else's line
    collide(&mut w, &[1]);
    assert_eq!(drawn(&mut w, &m, "W").as_deref(), Some("pick"));
  }

  // rule 3: a bare word with no ref walks the channel (ffz, 7tv, bttv) then the globals (7tv, bttv, ffz,
  // twitch): each one that is taken away lets the next step up
  #[test]
  fn rule_3_bare_word_walks_channel_then_globals_in_fixed_order() {
    let mut w = World::new();
    collide(&mut w, &[]);
    let m = line("7", "W");
    let mut walk = Vec::new();
    for (chan, p) in [
      (Some("c"), Provider::Ffz),
      (Some("c"), Provider::SevenTv),
      (Some("c"), Provider::Bttv),
      (None, Provider::SevenTv),
      (None, Provider::Bttv),
      (None, Provider::Ffz),
      (None, Provider::Twitch),
    ] {
      walk.push(drawn(&mut w, &m, "W"));
      w.emotes.set(chan, p, Default::default());
    }
    walk.push(drawn(&mut w, &m, "W"));
    let want = ["c-ffz", "c-7tv", "c-bttv", "g-7tv", "g-bttv", "g-ffz", "g-twitch"];
    assert_eq!(walk[..7].iter().map(|x| x.as_deref()).collect::<Vec<_>>(), want.map(Some));
    assert_eq!(walk[7], None, "nothing left: the plain word");
  }

  // rule 4: your inventory never draws someone else's bare word, and owning W never hides the channel's or a
  // global W on their lines: it falls through to rule 3
  #[test]
  fn rule_4_my_inventory_never_draws_another_senders_word() {
    let mut w = World::new();
    collide(&mut w, &[1]);
    w.senders.set_me(Some(1));
    let theirs = line("7", "W");
    assert_eq!(drawn(&mut w, &theirs, "W").as_deref(), Some("c-ffz"), "the channel's, not mine");
    for p in [Provider::Ffz, Provider::SevenTv, Provider::Bttv] {
      w.emotes.set(Some("c"), p, Default::default());
    }
    assert_eq!(drawn(&mut w, &theirs, "W").as_deref(), Some("g-7tv"), "then the global");
    w.emotes.set(None, Provider::Twitch, Default::default());
    for p in [Provider::SevenTv, Provider::Bttv, Provider::Ffz] {
      w.emotes.set(None, p, Default::default());
    }
    assert_eq!(drawn(&mut w, &theirs, "W"), None, "nothing left for them: mine is still not theirs");
    // mine draws on my own line only, where no room emote takes the name
    let mine = line("1", "W");
    assert_eq!(drawn(&mut w, &mine, "W").as_deref(), Some("inv1"));
    // a second chatter who owns a different W draws their own, never mine
    collide(&mut w, &[2]);
    for p in [Provider::Ffz, Provider::SevenTv, Provider::Bttv] {
      w.emotes.set(Some("c"), p, Default::default());
    }
    assert_eq!(drawn(&mut w, &line("2", "W"), "W").as_deref(), Some("inv2"));
    // with the channel's W back, every typed W is the channel's: mine, theirs, anyone's
    collide(&mut w, &[]);
    for who in ["1", "2", "7"] {
      assert_eq!(drawn(&mut w, &line(who, "W"), "W").as_deref(), Some("c-ffz"), "{who}");
    }
  }

  // rule 5: case is exact in every source: two emotes whose names differ by case stay two
  #[test]
  fn rule_5_case_is_exact_in_every_source() {
    let mut w = World::new();
    w.emotes.set(Some("c"), Provider::Ffz, [("KEKW".into(), cdn("c-up")), ("kekw".into(), cdn("c-low"))].into());
    w.emotes.set(None, Provider::SevenTv, [("Kekw".into(), cdn("g-mix"))].into());
    let m = line("7", "x");
    let at = |w: &mut World, word: &str| drawn(w, &m, word);
    assert_eq!(at(&mut w, "KEKW").as_deref(), Some("c-up"));
    assert_eq!(at(&mut w, "kekw").as_deref(), Some("c-low"));
    assert_eq!(at(&mut w, "Kekw").as_deref(), Some("g-mix"));
    assert_eq!(at(&mut w, "kEKW"), None);
    // the sender's own and the line's ref are exact too: their kekw is not a KEKW
    let row = serde_json::json!({"custom_name": "kekw", "url": cdn("inv").url, "width": 32, "height": 32,
      "added_at": "2026-10-30T22:26:51.099Z"});
    w.senders.lookup(42, "kekw", TS);
    w.senders.put(42, crate::heatsync::senders::parse_set(&serde_json::json!([row])));
    let mut theirs = line("42", "x");
    theirs.extra().picks = Box::new([("Kekw".into(), cdn("ref").url, false)]);
    assert_eq!(drawn(&mut w, &theirs, "kekw").as_deref(), Some("c-low"), "the channel's kekw, over their own");
    assert_eq!(drawn(&mut w, &theirs, "KEKW").as_deref(), Some("c-up"), "not their kekw");
    assert_eq!(drawn(&mut w, &theirs, "Kekw").as_deref(), Some("ref"), "their pick of Kekw, exact");
    w.emotes.set(Some("c"), Provider::Ffz, [("KEKW".into(), cdn("c-up"))].into());
    assert_eq!(drawn(&mut w, &theirs, "kekw").as_deref(), Some("inv"), "no channel kekw: theirs, exact");
    assert_eq!(drawn(&mut w, &theirs, "kEKW"), None);
  }
}
