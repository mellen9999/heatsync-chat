//! runs src/law/emote-mark.vectors.json against the tile mark (src/law/mark.rs).
//!
//! the second contract file the site vendors: who sees a picker tile's picture if you send it from this
//! room, and whether the tile is offered at all. same rules as the law's file: byte-for-byte, pinned by
//! sha256, every case under five seeded shuffles of every list in its world, a kind or a field this runner
//! does not know is a failure, never a skip.

use super::*;
use crate::law::mark::{self, tile_says};

const MARKS: &str = include_str!("../emote-mark.vectors.json");
const CASE: [&str; 10] = ["kind", "name", "world", "room", "legs", "who", "tile", "tier", "fold", "expect"];
const SAYS: [&str; 5] = ["offered", "mark", "kind", "as", "tier"];

fn marks() -> Map<String, Value> {
  match serde_json::from_str::<Value>(MARKS).expect("the mark vector file is json") {
    Value::Object(m) => m,
    _ => panic!("the mark vector file is an object"),
  }
}

fn run(file: &Map<String, Value>, c: &Value, seed: u32) -> Res<Value> {
  let c = obj(c, "case")?;
  let name = str_of(c, "name").unwrap_or("?");
  known(c, &CASE, name)?;
  let kind = str_of(c, "kind").ok_or("a case without a kind")?;
  if kind != "tile" {
    return Err(format!("case kind \"{kind}\" is not one this runner understands"));
  }
  known(obj(&c["expect"], "expect")?, &SAYS, &format!("{name} expect"))?;
  let world = str_of(c, "world").ok_or("a case without a world")?;
  let raw = obj(file["worlds"].get(world).ok_or_else(|| format!("unknown world: {world}"))?, world)?;
  let w = build_world(file, raw, &mut Rng(seed))?;
  let own = w.person(str_of(c, "who"))?.ok_or("a tile needs a person")?;
  let legs: Vec<&Room<Em>> =
    strs(c.get("legs")).iter().map(|id| w.room(Some(id)).map(|r| r.expect("a leg is a room"))).collect::<Res<_>>()?;
  let ctx = SendCtx {
    room: w.room(str_of(c, "room"))?,
    legs: &legs,
    own,
    globals: &w.globals,
    picks: None,
    fold: c.get("fold").and_then(Value::as_bool).unwrap_or(false),
  };
  let tile = w.emote(str_of(c, "tile").ok_or("a tile case without a tile")?)?;
  let says = tile_says(&ctx, tile, str_of(c, "tier").ok_or("a tile case without a tier")?);
  Ok(json!({
    "offered": says.offered,
    "mark": says.mark.map(String::from).unwrap_or_default(),
    "kind": says.kind.as_str(),
    "as": says.as_,
    "tier": says.tier,
  }))
}

#[test]
fn the_vendored_file_has_a_known_hash() {
  let got = sha256(MARKS.as_bytes());
  println!("emote-mark.vectors.json sha256 {got}");
  assert_eq!(
    got,
    include_str!("../emote-mark.vectors.sha256").trim(),
    "the vendored mark vectors were edited: refresh them with scripts/sync-emote-vectors.sh"
  );
}

#[test]
fn is_the_version_this_code_implements() {
  assert_eq!(marks()["version"], json!(mark::MARK_VERSION));
}

#[test]
fn names_every_case_once_and_only_asks_what_this_runner_knows() {
  let f = marks();
  known(&f, &["version", "about", "emotes", "worlds", "cases"], "the mark vector file").unwrap();
  let cases = f["cases"].as_array().unwrap();
  let names: HashSet<&str> = cases.iter().filter_map(|c| c["name"].as_str()).collect();
  assert_eq!(names.len(), cases.len(), "two cases share a name");
  assert!(cases.iter().all(|c| c["kind"] == "tile"));
}

#[test]
fn every_case_passes_under_every_shuffle() {
  let f = marks();
  let cases = f["cases"].as_array().unwrap();
  let mut failed = Vec::new();
  for c in cases {
    for seed in SHUFFLES {
      match run(&f, c, seed) {
        Ok(got) if got == c["expect"] => {}
        Ok(got) => {
          failed.push(format!("{} (shuffle {seed})\n  want {}\n  got  {got}", c["name"], c["expect"]));
          break;
        }
        Err(e) => {
          failed.push(format!("{} (shuffle {seed}): {e}", c["name"]));
          break;
        }
      }
    }
  }
  println!(
    "emote marks: {}/{} cases pass under {} shuffles each",
    cases.len() - failed.len(),
    cases.len(),
    SHUFFLES.len()
  );
  assert!(failed.is_empty(), "{} case(s) failed:\n{}", failed.len(), failed.join("\n"));
}

#[test]
fn refuses_a_case_it_does_not_understand_instead_of_skipping_it() {
  let f = marks();
  let err = |c: Value| run(&f, &c, 1).unwrap_err();
  let tile = json!({ "kind": "tile", "name": "x", "world": "mark", "room": null, "who": "me", "tile": "mine_only", "tier": "own", "expect": {} });
  assert!(run(&f, &tile, 1).is_ok());
  let with = |k: &str, v: Value| {
    let mut c = tile.clone();
    c[k] = v;
    c
  };
  assert!(err(with("kind", json!("rank"))).contains("not one this runner understands"));
  assert!(err(with("viewer", json!("me"))).contains("field \"viewer\""));
  assert!(err(with("expect", json!({ "colour": 1 }))).contains("field \"colour\""));
  assert!(err(with("tile", json!("nobody_made_this"))).contains("unknown emote alias"));
  assert!(err(with("legs", json!(["twitch:nowhere"]))).contains("unknown room"));
}
