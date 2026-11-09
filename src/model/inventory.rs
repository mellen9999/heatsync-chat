// your heatsync emote inventory as the manage screen (app::inventory) holds it: the slots, the recently
// removed, the saved sets. these are lists to look at and act on, never to draw chat from: the render path
// takes its inventory only through the sender sets (emotes::resolve). server rows are hostile: bounded,
// names clean of control bytes, ids checked
use super::ident;
use crate::i18n::t;
use serde_json::Value;

// the server's inventory cap, and so the first slot number it refuses
pub const MAX_SLOTS: u32 = 5000;
// removed rows (the server sends 100) and saved sets (it sends 50)
pub const MAX_GONE: usize = 100;
pub const MAX_SAVED: usize = 50;
// a slot number the server could have stored (its delete/rename bound)
const MAX_READ_SLOT: u64 = 50_000;
const MAX_NAME: usize = crate::heatsync::MAX_NAME;
// the longest channel the import takes
pub const MAX_CHANNEL: usize = 50;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Slot {
  pub slot: u32,
  pub name: Box<str>,
  pub source: Box<str>,
  pub uses: u64,
  pub upload: bool,
  pub zero_width: bool,
}

// a removed emote that can still be restored
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gone {
  pub id: u64,
  pub name: Box<str>,
  pub slot: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Saved {
  pub id: u64,
  pub name: Box<str>,
  pub count: u32,
  pub starred: bool,
}

// a name as shown: no control bytes, cut to the server's length, None when nothing is left
fn clean(s: &str) -> Option<Box<str>> {
  let t: String = s.chars().filter(|c| !c.is_control()).take(MAX_NAME).collect();
  let t = t.trim();
  (!t.is_empty()).then(|| t.into())
}

fn text<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
  v.get(k).and_then(Value::as_str)
}

// `GET /api/user/emotes` → `emotes[]`, by slot. a row without a slot or a name is dropped, and so is a
// second row on a slot
pub fn slots(v: &Value) -> Vec<Slot> {
  let rows = v.get("emotes").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
  let mut out: Vec<Slot> = rows
    .iter()
    .take(MAX_SLOTS as usize)
    .filter_map(|r| {
      let slot = r.get("slot_number")?.as_u64().filter(|&s| s <= MAX_READ_SLOT)? as u32;
      let name = clean(text(r, "custom_name").or_else(|| text(r, "name"))?)?;
      let source = text(r, "source").filter(|s| ident::slug(s, 32)).unwrap_or("");
      Some(Slot {
        slot,
        name,
        source: source.to_ascii_lowercase().into(),
        uses: r.get("usage_count").and_then(Value::as_u64).unwrap_or(0),
        upload: r.get("own_upload").and_then(Value::as_bool) == Some(true),
        zero_width: r.get("zero_width").and_then(Value::as_bool) == Some(true),
      })
    })
    .collect();
  out.sort_by_key(|s| s.slot);
  out.dedup_by_key(|s| s.slot);
  out
}

// `GET /api/user/emotes/removed` → `emotes[]`, newest first as sent
pub fn gone(v: &Value) -> Vec<Gone> {
  let rows = v.get("emotes").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
  rows
    .iter()
    .take(MAX_GONE)
    .filter_map(|r| {
      Some(Gone {
        id: r.get("id")?.as_u64().filter(|&i| i > 0)?,
        name: clean(text(r, "custom_name").or_else(|| text(r, "name"))?)?,
        slot: r.get("slot_number").and_then(Value::as_u64).filter(|&s| s <= MAX_READ_SLOT).unwrap_or(0) as u32,
      })
    })
    .collect()
}

// `GET /api/user/sets` → `sets[]`, starred first as sent
pub fn saved(v: &Value) -> Vec<Saved> {
  let rows = v.get("sets").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
  rows
    .iter()
    .take(MAX_SAVED)
    .filter_map(|r| {
      Some(Saved {
        id: r.get("id")?.as_u64().filter(|&i| i > 0)?,
        name: clean(text(r, "name")?)?,
        count: r.get("emote_count").and_then(Value::as_u64).unwrap_or(0).min(MAX_SLOTS as u64) as u32,
        starred: r.get("starred").and_then(Value::as_bool) == Some(true),
      })
    })
    .collect()
}

// what a rename may become: one word like a chat emote (the server would keep spaces, but a spaced name
// can never be typed as one), not a command, and not another slot's name (the server's 409, caught here)
pub fn rename_ok(list: &[Slot], slot: u32, typed: &str) -> Result<Box<str>, &'static str> {
  let name = typed.trim();
  if name.is_empty() {
    return Err(t("chat1.inventory.err.name_empty"));
  }
  if name.chars().count() > MAX_NAME {
    return Err(t("chat1.inventory.err.name_long"));
  }
  if !crate::heatsync::valid_name(name) {
    return Err(t("chat1.inventory.err.name_form"));
  }
  if list.iter().any(|s| s.slot != slot && *s.name == *name) {
    return Err(t("chat1.inventory.err.name_taken"));
  }
  Ok(name.into())
}

// a slot number as typed: 0 up to the cap
pub fn slot_number(typed: &str) -> Result<u32, &'static str> {
  match typed.trim().parse::<u32>() {
    Ok(n) if n < MAX_SLOTS => Ok(n),
    _ => Err(t("chat1.inventory.err.slot")),
  }
}

// a channel for the import: the server's rule, 1-50 of a-z A-Z 0-9 _ -
pub fn channel(typed: &str) -> Result<Box<str>, &'static str> {
  let c = typed.trim().trim_start_matches('#');
  if ident::slug(c, MAX_CHANNEL) { Ok(c.to_ascii_lowercase().into()) } else { Err(t("chat1.inventory.err.channel")) }
}

// the slot that swapping row `at` with the one below (down) or above it means: the neighbour's slot
pub fn neighbour(list: &[Slot], at: usize, down: bool) -> Option<u32> {
  let other = if down { at.checked_add(1)? } else { at.checked_sub(1)? };
  list.get(other).map(|s| s.slot)
}

// what a move did to the list: onto a free slot it goes there, onto a taken one the two trade places
pub fn moved(list: &mut [Slot], from: u32, to: u32) {
  if from == to || !list.iter().any(|s| s.slot == from) {
    return;
  }
  for s in list.iter_mut() {
    if s.slot == from {
      s.slot = to;
    } else if s.slot == to {
      s.slot = from;
    }
  }
  list.sort_by_key(|s| s.slot);
}

pub fn renamed(list: &mut [Slot], slot: u32, name: &str) {
  if let Some(s) = list.iter_mut().find(|s| s.slot == slot) {
    s.name = name.into();
  }
}

pub fn dropped(list: &mut Vec<Slot>, slot: u32) -> Option<Slot> {
  let i = list.iter().position(|s| s.slot == slot)?;
  Some(list.remove(i))
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  fn s(slot: u32, name: &str) -> Slot {
    Slot { slot, name: name.into(), source: "7tv".into(), uses: 0, upload: false, zero_width: false }
  }

  #[test]
  fn rows_are_read_bounded_sorted_and_clean() {
    let v = json!({"emotes": [
      {"slot_number": 9, "custom_name": "b", "source": "7TV", "usage_count": 4, "own_upload": true},
      {"slot_number": 2, "name": "a\u{7}x", "zero_width": true},
      {"slot_number": 2, "custom_name": "dupe"},
      {"slot_number": 99999999, "custom_name": "far"},
      {"slot_number": 3},
      {"slot_number": 4, "custom_name": "\u{7}\u{8}"},
      {"slot_number": 5, "custom_name": "x", "source": "a b"},
      "junk"
    ]});
    let l = slots(&v);
    let names: Vec<_> = l.iter().map(|s| (s.slot, &*s.name, &*s.source)).collect();
    assert_eq!(names, [(2, "ax", ""), (5, "x", ""), (9, "b", "7tv")]);
    assert!(l[2].upload && l[0].zero_width && l[2].uses == 4);
    for junk in [json!(null), json!({"emotes": "x"}), json!([])] {
      assert!(slots(&junk).is_empty());
    }
    let long = json!({"emotes": [{"slot_number": 1, "custom_name": "x".repeat(300)}]});
    assert_eq!(slots(&long)[0].name.len(), 100);
  }

  #[test]
  fn removed_and_saved_rows_need_a_real_id() {
    let g = gone(&json!({"emotes": [
      {"id": 7, "custom_name": "pog", "slot_number": 3}, {"id": 0, "custom_name": "x"}, {"custom_name": "y"}
    ]}));
    assert_eq!(g, [Gone { id: 7, name: "pog".into(), slot: 3 }]);
    let many = json!({"emotes": (1..300).map(|i| json!({"id": i, "custom_name": "e"})).collect::<Vec<_>>()});
    assert_eq!(gone(&many).len(), MAX_GONE);
    let sv = saved(&json!({"sets": [
      {"id": 4, "name": "my pogs", "emote_count": 12, "starred": true}, {"id": -1, "name": "x"}, {"id": 5}
    ]}));
    assert_eq!(sv, [Saved { id: 4, name: "my pogs".into(), count: 12, starred: true }]);
  }

  #[test]
  fn a_rename_is_one_word_and_not_taken() {
    let l = vec![s(1, "Kappa"), s(2, "pog")];
    assert_eq!(rename_ok(&l, 2, "  PogU ").as_deref(), Ok("PogU"));
    // the same name on the same slot is fine (a case change keeps its slot)
    assert_eq!(rename_ok(&l, 2, "pog").as_deref(), Ok("pog"));
    // names are exact-case, as the server compares them
    assert_eq!(rename_ok(&l, 2, "kappa").as_deref(), Ok("kappa"));
    assert_eq!(rename_ok(&l, 2, "Kappa"), Err("you already have an emote by that name"));
    assert_eq!(rename_ok(&l, 2, "  "), Err("type a name"));
    for bad in ["a b", "/me", ".x", "x\u{7}"] {
      assert_eq!(rename_ok(&l, 2, bad), Err("one word, no spaces, not starting with / or ."), "{bad:?}");
    }
    assert_eq!(rename_ok(&l, 2, &"x".repeat(101)), Err("a name is 100 characters at most"));
    assert!(rename_ok(&l, 2, &"x".repeat(100)).is_ok());
  }

  #[test]
  fn slot_numbers_and_channels_are_checked_like_the_server() {
    assert_eq!(slot_number(" 0 "), Ok(0));
    assert_eq!(slot_number("4999"), Ok(4999));
    for bad in ["5000", "-1", "x", "", "1.5", "99999999999"] {
      assert!(slot_number(bad).is_err(), "{bad}");
    }
    assert_eq!(channel("#Forsen").as_deref(), Ok("forsen"));
    assert!(channel("a/b").is_err() && channel("").is_err() && channel(&"x".repeat(51)).is_err());
    assert!(channel(&"x".repeat(50)).is_ok());
  }

  #[test]
  fn a_swap_trades_slots_and_a_free_slot_is_taken() {
    let mut l = vec![s(1, "a"), s(2, "b"), s(7, "c")];
    assert_eq!(neighbour(&l, 0, true), Some(2));
    assert_eq!(neighbour(&l, 0, false), None);
    assert_eq!(neighbour(&l, 2, true), None);
    assert_eq!(neighbour(&l, 2, false), Some(2));
    moved(&mut l, 1, 2);
    assert_eq!(l.iter().map(|s| (s.slot, &*s.name)).collect::<Vec<_>>(), [(1, "b"), (2, "a"), (7, "c")]);
    moved(&mut l, 2, 40);
    assert_eq!(l.iter().map(|s| (s.slot, &*s.name)).collect::<Vec<_>>(), [(1, "b"), (7, "c"), (40, "a")]);
    // an empty source or the same slot changes nothing
    let before = l.clone();
    moved(&mut l, 3, 4);
    moved(&mut l, 7, 7);
    assert_eq!(l, before);
    renamed(&mut l, 7, "cc");
    assert_eq!(&*l[1].name, "cc");
    assert_eq!(dropped(&mut l, 7).map(|s| s.name), Some("cc".into()));
    assert!(dropped(&mut l, 7).is_none() && l.len() == 2);
  }
}
