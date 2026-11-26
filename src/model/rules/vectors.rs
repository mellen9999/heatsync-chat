// the filter rules engine as data: `filter-rules.vectors.json` is the site's shared/filter-rules.vectors.json, vendored
// byte for byte (scripts/sync-emote-vectors.sh) and run here case by case, so a rule hides, lights or sounds the
// same in every app. a field or a verdict this runner does not know is a failure, never a skip
use super::*;
use serde_json::Map;

const FILE: &str = include_str!("../filter-rules.vectors.json");
const MSG_FIELDS: [&str; 8] = ["text", "user", "badges", "platform", "isFirstMsg", "isAction", "bits", "replyTo"];

fn file() -> Map<String, Value> {
  match serde_json::from_str::<Value>(FILE).expect("the vector file is json") {
    Value::Object(m) => m,
    _ => panic!("the vector file is an object"),
  }
}

fn word(o: &Map<String, Value>, k: &str, default: &'static str) -> String {
  o.get(k).map_or(default, |v| v.as_str().unwrap_or_else(|| panic!("{k} is not text"))).to_owned()
}

// a vector line as this app holds it
fn message(o: &Map<String, Value>) -> Message {
  assert!(o.keys().all(|k| MSG_FIELDS.contains(&k.as_str())), "a msg field this runner does not know");
  let platform = match word(o, "platform", "twitch").as_str() {
    "twitch" => Platform::Twitch,
    "kick" => Platform::Kick,
    "youtube" => Platform::Youtube,
    p => panic!("a platform this runner does not know: {p}"),
  };
  let mut m = Message::system(0, platform, "");
  m.kind = if o.get("isAction").and_then(Value::as_bool) == Some(true) { Kind::Action } else { Kind::Chat };
  let user = word(o, "user", "alice");
  m.login = user.as_str().into();
  m.nick = user.into();
  m.body = word(o, "text", "").into();
  if o.get("isFirstMsg").and_then(Value::as_bool) == Some(true) {
    m.flags |= flag::FIRST;
  }
  // twitch and kick hold "set/ver,set/ver"; youtube holds a list of names
  let pair = |s: &str| {
    let (set, ver) = s.split_once('/').unwrap_or((s, ""));
    (Box::<str>::from(set), Box::<str>::from(ver))
  };
  m.badges = match o.get("badges") {
    None => Box::new([]),
    Some(Value::String(s)) => s.split(',').filter(|b| !b.is_empty()).map(pair).collect(),
    Some(Value::Array(a)) => a.iter().map(|b| pair(b.as_str().expect("a badge is text"))).collect(),
    Some(b) => panic!("a badges shape this runner does not know: {b}"),
  };
  match o.get("replyTo") {
    None | Some(Value::Null) => {}
    Some(r) => {
      let to = r.get("user").and_then(Value::as_str).expect("replyTo.user");
      // the ext reads a reply that names nobody as no reply
      if !to.is_empty() {
        m.reply_to = Some(to.into());
      }
    }
  }
  if let Some(b) = o.get("bits") {
    m.extra().bits = b.as_u64().expect("bits is a count") as u32;
  }
  m
}

// the file is the site's, byte for byte: its hash is kept beside it and compared across the apps
#[test]
fn the_vendored_vectors_are_the_ones_synced() {
  let sum = ring::digest::digest(&ring::digest::SHA256, FILE.as_bytes());
  let hex: String = sum.as_ref().iter().map(|b| format!("{b:02x}")).collect();
  assert_eq!(hex, include_str!("../filter-rules.vectors.sha256").trim(), "run scripts/sync-emote-vectors.sh");
}

#[test]
fn every_line_is_judged_as_the_vectors_say() {
  let f = file();
  assert!(f.keys().all(|k| k == "about" || k == "cases"), "a field this runner does not know");
  let cases = f["cases"].as_array().expect("cases");
  let mut misses: Vec<String> = Vec::new();
  assert!(cases.len() >= 100, "only {} cases", cases.len());
  for c in cases {
    let o = c.as_object().expect("a case is an object");
    let name = o["name"].as_str().expect("name");
    assert!(
      o.keys().all(|k| ["name", "rules", "msg", "channel", "want"].contains(&k.as_str())),
      "{name}: a field this runner does not know"
    );
    // the engine reads the setting's text, as the ext writes it
    let text = o["rules"].to_string();
    let rules = Rules::parse(&text).unwrap_or_else(|| panic!("{name}: the rules do not parse"));
    let m = message(o["msg"].as_object().expect("msg"));
    let got = rules.judge(&Line::new(&m, o.get("channel").and_then(Value::as_str).unwrap_or("")));
    let want = o["want"].as_object().expect("want");
    assert!(
      want.keys().all(|k| ["hide", "color", "notify", "sound", "soundVolume"].contains(&k.as_str())),
      "{name}: a verdict this runner does not know"
    );
    let mut miss = |what: &str, ok: bool| {
      if !ok {
        misses.push(format!("{name}: {what}"));
      }
    };
    miss("hide", (got.verdict == Verdict::Hide) == (want["hide"] == true));
    if want["hide"] == true {
      continue;
    }
    // this app paints the seven ansi colours: a rule's colour is the nearest of them, so what is held is that there is one
    miss("colour", got.ink.is_some() != want["color"].is_null());
    miss("notify", got.notify == (want["notify"] == true));
    let (sound, custom) = match want["sound"].as_str() {
      None => (None, false),
      Some("custom") => (None, true),
      Some(s) => {
        (Some(Preset::parse(s).unwrap_or_else(|| panic!("{name}: a sound this runner does not know: {s}"))), false)
      }
    };
    miss("sound", (got.sound, got.custom) == (sound, custom));
    miss("volume", got.volume.map(u64::from) == want["soundVolume"].as_u64());
  }
  assert!(misses.is_empty(), "{} of {} cases differ from the site:\n{}", misses.len(), cases.len(), misses.join("\n"));
}
