// feed-post reactions (ext feed-engage.js feedReactionsHtml / feedFrameApply): the chips under a post. a chip
// is the server's own emote for that post (its url, its name), never a word the chat could turn into an
// emote: nothing here touches emotes::resolve, and the picture comes only from the chip's own url
use serde_json::Value;

use crate::images::url::allowed;

// chips a post shows at most (the picker holds far fewer; a hostile list is cut)
pub const MAX_CHIPS: usize = 32;
const MAX_NAME: usize = 64;
// a count past this is a lie or a bug: it reads as this
const MAX_COUNT: u32 = 999_999;
// an emote id is a database row number
const MAX_ID: u64 = 1 << 48;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reaction {
  pub id: u64,
  pub name: Box<str>,
  // None = name only (no url, a url off the allowlist, or an emote flagged nsfw / content-warned)
  pub url: Option<Box<str>>,
  pub count: u32,
  // you reacted with it
  pub mine: bool,
}

fn id_of(v: Option<&Value>) -> Option<u64> {
  let n = match v? {
    Value::Number(n) => n.as_u64()?,
    Value::String(s) => s.parse().ok()?,
    _ => return None,
  };
  (1..MAX_ID).contains(&n).then_some(n)
}

fn name_of(v: &Value) -> String {
  let s = v.get("emote_name").and_then(Value::as_str).unwrap_or("");
  s.chars().filter(|c| !c.is_control()).take(MAX_NAME).collect::<String>().trim().to_owned()
}

// the picture, unless flagged: a flagged emote is shown as its name only, whatever the viewer's switches say
fn url_of(v: &Value) -> Option<Box<str>> {
  let gated = v.get("nsfw").and_then(Value::as_bool) == Some(true)
    || v.get("cw_cats").and_then(Value::as_array).is_some_and(|a| !a.is_empty());
  let u = v.get("emote_url").and_then(Value::as_str)?;
  (!gated && allowed(u)).then(|| u.into())
}

fn make(v: &Value, count: u32, mine: bool) -> Option<Reaction> {
  let (id, name, url) = (id_of(v.get("emote_id"))?, name_of(v), url_of(v));
  (!name.is_empty() || url.is_some()).then(|| Reaction { id, name: name.into(), url, count, mine })
}

// a post's `reactions: [{emote_id, emote_name, emote_url, count, reacted}]`. chips with no count are not chips;
// the same emote twice is one chip
pub fn parse_list(v: Option<&Value>) -> Box<[Reaction]> {
  let Some(list) = v.and_then(Value::as_array).filter(|a| !a.is_empty()) else { return Box::new([]) };
  let mut out: Vec<Reaction> = Vec::new();
  for e in list.iter().take(MAX_CHIPS * 2) {
    let count = e.get("count").and_then(Value::as_u64).map_or(0, |n| n.min(MAX_COUNT as u64) as u32);
    if count == 0 || out.len() >= MAX_CHIPS {
      continue;
    }
    let Some(r) = make(e, count, e.get("reacted").and_then(Value::as_bool) == Some(true)) else { continue };
    if !out.iter().any(|x| x.id == r.id) {
      out.push(r);
    }
  }
  out.into()
}

#[derive(Debug)]
pub enum Change {
  // someone reacted: the chip to add to (or start)
  Added(Reaction),
  Removed(u64),
}

// a `reaction:added` / `reaction:removed` frame: which post, who, what
#[derive(Debug)]
pub struct Frame {
  pub post: Box<str>,
  pub user: Box<str>,
  pub change: Change,
}

pub fn parse_frame(v: &Value, added: bool) -> Option<Frame> {
  let post = v.get("message_id").and_then(Value::as_str).filter(|s| super::post::id_ok(s))?;
  let user = match v.get("user_id") {
    Some(Value::String(s)) => s.clone(),
    Some(Value::Number(n)) => n.to_string(),
    _ => String::new(),
  };
  let user = if user.len() <= 64 { user } else { String::new() };
  let change = if added { Change::Added(make(v, 1, false)?) } else { Change::Removed(id_of(v.get("emote_id"))?) };
  Some(Frame { post: post.to_ascii_lowercase().into(), user: user.into(), change })
}

// one more of this emote (a new chip when none, while there is room)
pub fn add(list: &mut Vec<Reaction>, r: &Reaction) {
  if let Some(x) = list.iter_mut().find(|x| x.id == r.id) {
    x.count = (x.count + 1).min(MAX_COUNT);
  } else if list.len() < MAX_CHIPS {
    list.push(Reaction { count: 1, mine: false, ..r.clone() });
  }
}

// one fewer; a chip at nothing is gone
pub fn sub(list: &mut Vec<Reaction>, id: u64) {
  if let Some(x) = list.iter_mut().find(|x| x.id == id) {
    x.count = x.count.saturating_sub(1);
  }
  list.retain(|x| x.count > 0);
}

// your own reaction on or off, counted as it goes. false = it already was that way
pub fn set_mine(list: &mut Vec<Reaction>, r: &Reaction, on: bool) -> bool {
  let was = list.iter().find(|x| x.id == r.id).is_some_and(|x| x.mine);
  if was == on {
    return false;
  }
  if on {
    add(list, r);
  } else {
    sub(list, r.id);
  }
  if let Some(x) = list.iter_mut().find(|x| x.id == r.id) {
    x.mine = on;
  }
  true
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  fn chip(id: u64, count: u32, mine: bool) -> Reaction {
    Reaction { id, name: format!("e{id}").into(), url: None, count, mine }
  }

  #[test]
  fn a_list_keeps_the_good_chips_and_cuts_the_rest() {
    let v = json!([
      {"emote_id": 1, "emote_name": "catJAM", "emote_url": "https://cdn.7tv.app/a.webp", "count": 3, "reacted": true},
      {"emote_id": 2, "emote_name": "off", "emote_url": "https://evil.example/a.png", "count": 2, "reacted": false},
      {"emote_id": 3, "emote_name": "zero", "count": 0},
      {"emote_id": "4", "emote_name": "str", "count": 5},
      {"emote_id": 0, "emote_name": "noid", "count": 1},
      {"emote_id": 1, "emote_name": "dupe", "count": 9},
      {"emote_name": "noid2", "count": 1},
      {"emote_id": 5, "emote_name": "", "count": 1},
      {"emote_id": 6, "emote_name": "nsfw", "emote_url": "https://cdn.7tv.app/n.webp", "count": 1, "nsfw": true},
      {"emote_id": 7, "emote_name": "cw", "emote_url": "https://cdn.7tv.app/c.webp", "count": 1, "cw_cats": ["gore"]},
      {"emote_id": 8, "emote_name": "big", "count": 99999999999u64},
      "junk", 5, null,
    ]);
    let got = parse_list(Some(&v));
    let ids: Vec<u64> = got.iter().map(|r| r.id).collect();
    assert_eq!(ids, [1, 2, 4, 6, 7, 8]);
    assert_eq!((got[0].url.as_deref(), got[0].count, got[0].mine), (Some("https://cdn.7tv.app/a.webp"), 3, true));
    assert_eq!(got[1].url, None, "off the allowlist: its name only");
    assert_eq!((got[3].url.as_deref(), &*got[3].name), (None, "nsfw"), "flagged: its name only");
    assert_eq!(got[4].url, None);
    assert_eq!(got[5].count, MAX_COUNT);
  }

  #[test]
  fn a_hostile_list_is_bounded() {
    let many: Vec<Value> = (1..500).map(|i| json!({"emote_id": i, "emote_name": "x", "count": 1})).collect();
    assert_eq!(parse_list(Some(&json!(many))).len(), MAX_CHIPS);
    let long = json!([{"emote_id": 1, "emote_name": format!("a\u{7}{}", "b".repeat(500)), "count": 1}]);
    let r = &parse_list(Some(&long))[0];
    assert_eq!(r.name.chars().count(), MAX_NAME);
    assert!(!r.name.contains('\u{7}'));
    for bad in [json!(null), json!("x"), json!({}), json!([])] {
      assert!(parse_list(Some(&bad)).is_empty());
    }
    assert!(parse_list(None).is_empty());
  }

  #[test]
  fn frames_are_read_as_hostile() {
    let add = json!({"type": "reaction:added", "message_id": "00ABC1", "user_id": 7, "emote_id": 4,
      "emote_name": "kek", "emote_url": "https://cdn.7tv.app/k.webp"});
    let f = parse_frame(&add, true).unwrap();
    assert_eq!((&*f.post, &*f.user), ("00abc1", "7"));
    let Change::Added(r) = f.change else { panic!() };
    assert_eq!((r.id, &*r.name, r.url.as_deref(), r.count), (4, "kek", Some("https://cdn.7tv.app/k.webp"), 1));
    let gated = json!({"message_id": "ab", "user_id": "u", "emote_id": 4, "emote_name": "k",
      "emote_url": "https://cdn.7tv.app/k.webp", "nsfw": true});
    let Change::Added(r) = parse_frame(&gated, true).unwrap().change else { panic!() };
    assert_eq!(r.url, None);
    let cw = json!({"message_id": "ab", "emote_id": 4, "emote_name": "k", "emote_url": "https://cdn.7tv.app/k.webp",
      "cw_cats": ["gore"]});
    let Change::Added(r) = parse_frame(&cw, true).unwrap().change else { panic!() };
    assert_eq!(r.url, None);
    // the server may drop the image for an unsafe emote: the name stays
    let nourl = json!({"message_id": "ab", "emote_id": 4, "emote_name": "k"});
    assert!(matches!(parse_frame(&nourl, true).unwrap().change, Change::Added(r) if r.url.is_none()));
    let rm = json!({"message_id": "ab", "user_id": "9", "emote_id": 4});
    assert!(matches!(parse_frame(&rm, false).unwrap().change, Change::Removed(4)));
    for bad in [
      json!({"message_id": "not an id", "emote_id": 4, "emote_name": "k"}),
      json!({"emote_id": 4, "emote_name": "k"}),
      json!({"message_id": "ab", "emote_id": -4, "emote_name": "k"}),
      json!({"message_id": "ab", "emote_name": "k"}),
      json!({"message_id": "ab", "emote_id": 4}),
    ] {
      assert!(parse_frame(&bad, true).is_none(), "{bad}");
    }
    assert!(parse_frame(&json!({"message_id": "ab", "emote_id": 4}), false).is_some());
  }

  #[test]
  fn counts_move_and_a_chip_at_nothing_is_gone() {
    let mut l = vec![chip(1, 1, false)];
    add(&mut l, &chip(1, 1, false));
    add(&mut l, &chip(2, 1, false));
    assert_eq!((l[0].count, l.len()), (2, 2));
    sub(&mut l, 2);
    sub(&mut l, 99);
    assert_eq!(l.len(), 1);
    sub(&mut l, 1);
    sub(&mut l, 1);
    assert!(l.is_empty());
    let mut full: Vec<Reaction> = (1..=MAX_CHIPS as u64).map(|i| chip(i, 1, false)).collect();
    add(&mut full, &chip(900, 1, false));
    assert_eq!(full.len(), MAX_CHIPS, "no room: no new chip");
  }

  #[test]
  fn your_reaction_flips_and_counts() {
    let mut l = vec![chip(1, 2, false)];
    assert!(set_mine(&mut l, &chip(1, 2, false), true));
    assert_eq!((l[0].count, l[0].mine), (3, true));
    assert!(!set_mine(&mut l, &chip(1, 2, false), true), "already on");
    assert!(set_mine(&mut l, &chip(1, 2, false), false));
    assert_eq!((l[0].count, l[0].mine), (2, false));
    // the only reactor leaving drops the chip; putting it back brings it at one
    let mut one = vec![chip(5, 1, true)];
    assert!(set_mine(&mut one, &chip(5, 1, true), false));
    assert!(one.is_empty());
    assert!(set_mine(&mut one, &chip(5, 1, true), true));
    assert_eq!((one[0].count, one[0].mine), (1, true));
  }
}
