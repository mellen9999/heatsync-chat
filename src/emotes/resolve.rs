// THE render chokepoint: the one place a word in a chat line becomes an emote. privacy invariant: a
// word renders as a heatsync emote iff the SENDER's own inventory has it (or the line's own
// server-verified refs say so). catalog and search caches never feed this path, and the only inputs it
// can see are the three in `Sources` — change what a heatsync emote is and it is changed here, nowhere else
use super::block::Scope;
use super::{Emote, Emotes};
use crate::heatsync::senders::Senders;
use crate::model::message::{Message, Platform, flag};

// what a word may become an emote from: the channel/global catalogs, the senders' own inventories, and
// the renderInventoryEmotes switch. built by `Res::sources`, so nothing else can reach the render path
pub struct Sources<'a> {
  pub emotes: &'a Emotes,
  pub senders: &'a mut Senders,
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
// exactly that, ahead of the channel), the channel catalog, then globals. the sender's set is the only heatsync source (privacy invariant)
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
    inventory: bool,
  }

  impl World {
    fn new() -> Self {
      Self { emotes: Emotes::default(), senders: Senders::default(), inventory: true }
    }

    fn sources(&mut self) -> Sources<'_> {
      Sources { emotes: &self.emotes, senders: &mut self.senders, inventory: self.inventory }
    }
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
}
