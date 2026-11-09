// changing a rule that is already in the list (the ext's edit-in-place, notify switch and per-rule sound), and
// trying a rule on the lines you already have before it is saved (the form's "matches n of the last 200").
// every change rebuilds the rule's compiled form from its raw json, and a change that would leave the rule
// unusable, or the list too big to sync, is refused and leaves everything as it was
use serde_json::{Value, json};

use super::{Line, MAX_JSON, Rules, compile};
use crate::model::message::Message;
use crate::util::sound::Preset;

// what a rule's sound can be set to
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Pick {
  Off,
  Preset(Preset),
  // a wav file of yours: only its name travels with the rule, the file stays on this device
  File(String),
}

// one rule's fields, as the list screen reads them
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct View {
  pub action: String,
  pub ty: String,
  pub value: String,
  pub case: bool,
  // the sound's name: a preset, or "custom"
  pub sound: Option<String>,
  // a custom sound's file name
  pub file: Option<String>,
  pub volume: Option<u8>,
  pub notify: bool,
}

// longest file name a rule carries (the ext's `soundName`)
const MAX_NAME: usize = 100;

impl Rules {
  // `key` (a number or id) → its place
  pub fn at(&self, key: &str) -> Option<usize> {
    self.find(key)
  }

  // the verdict the rule at `at` gives a line it takes: its sound, volume and notify. None = off or unusable
  pub fn judged(&self, at: usize) -> Option<super::Judged> {
    let c = self.list.get(at)?.c.as_ref()?;
    Some(super::Judged {
      verdict: if c.hide { super::Verdict::Hide } else { super::Verdict::Mark },
      ink: c.ink,
      sound: c.sound,
      custom: c.custom,
      volume: c.volume,
      notify: c.notify,
      at,
    })
  }

  // the rule's id, for the things kept beside the list (a sound file)
  pub fn id_at(&self, at: usize) -> Option<&str> {
    self.list.get(at)?.raw.get("id")?.as_str()
  }

  // one place's raw json replaced by `change`'s result; refused (Err) when the rule would no longer compile or
  // the list no longer fits the setting
  fn amend(&mut self, key: &str, change: impl FnOnce(&mut serde_json::Map<String, Value>)) -> Result<(), &'static str> {
    let i = self.find(key).ok_or("no such rule. /rule list shows the numbers")?;
    let mut raw = self.list[i].raw.clone();
    change(raw.as_object_mut().ok_or("that rule isn't valid")?);
    // an off rule compiles to nothing; the check is on the same rule switched on
    let mut probe = raw.clone();
    probe["enabled"] = json!(true);
    if compile(&probe).is_none() {
      return Err("that rule isn't valid");
    }
    let old = std::mem::replace(&mut self.list[i].raw, raw);
    if self.json().len() > MAX_JSON {
      self.list[i].raw = old;
      return Err("rules are full");
    }
    self.list[i].c = compile(&self.list[i].raw);
    Ok(())
  }

  fn is_highlight(&self, key: &str) -> Result<(), &'static str> {
    let i = self.find(key).ok_or("no such rule. /rule list shows the numbers")?;
    match self.list[i].raw.get("action").and_then(Value::as_str) {
      Some("highlight") => Ok(()),
      _ => Err("only a highlight rule has that"),
    }
  }

  // /rule edit: the rule keeps its id, place, scope, case and switch; its match and action change. a rule turned
  // to hide lets go of what only a highlight owns, one turned to highlight gets the plain colour
  pub fn edit(&mut self, key: &str, action: &str, ty: &str, value: &str) -> Result<(), &'static str> {
    if !matches!(action, "hide" | "highlight") {
      return Err("a rule hides or highlights");
    }
    if !matches!(ty, "keyword" | "regex" | "expr" | "user" | "badge" | "msgtype") {
      return Err("type is keyword, regex, user, badge, msgtype or expr");
    }
    let value = value.trim();
    self
      .amend(key, |o| {
        let case = o.get("match").and_then(|m| m.get("caseSensitive")).cloned().unwrap_or(json!(false));
        o.insert("match".into(), json!({ "type": ty, "value": value, "caseSensitive": case }));
        o.insert("action".into(), json!(action));
        if action == "hide" {
          for k in ["color", "sound", "soundName", "soundVolume", "notify"] {
            o.remove(k);
          }
        } else if !o.contains_key("color") {
          o.insert("color".into(), json!("#ffff00"));
        }
      })
      .map_err(|e| if e == "that rule isn't valid" && ty == "expr" { "that expression doesn't parse" } else { e })
  }

  // a hit also lands in the mentions tab (the ext's rule `notify`). highlights only
  pub fn set_notify(&mut self, key: &str, on: bool) -> Result<(), &'static str> {
    self.is_highlight(key)?;
    self.amend(key, |o| {
      if on {
        o.insert("notify".into(), json!(true));
      } else {
        o.remove("notify");
      }
    })
  }

  // the rule's sound. a file keeps its name only; off also drops the volume, which means nothing without a sound
  pub fn set_sound(&mut self, key: &str, pick: &Pick) -> Result<(), &'static str> {
    self.is_highlight(key)?;
    let name: String = match pick {
      Pick::File(n) => n.chars().filter(|c| !c.is_control()).take(MAX_NAME).collect(),
      _ => String::new(),
    };
    self.amend(key, |o| {
      for k in ["sound", "soundName"] {
        o.remove(k);
      }
      match pick {
        Pick::Off => {
          o.remove("soundVolume");
        }
        Pick::Preset(p) => {
          o.insert("sound".into(), json!(p.name()));
        }
        Pick::File(_) => {
          o.insert("sound".into(), json!("custom"));
          o.insert("soundName".into(), json!(name));
        }
      }
    })
  }

  // the rule's own volume, 0..100 (None: the shared mention volume again). needs a sound to apply to
  pub fn set_volume(&mut self, key: &str, pct: Option<u8>) -> Result<(), &'static str> {
    self.is_highlight(key)?;
    let i = self.find(key).ok_or("no such rule. /rule list shows the numbers")?;
    if pct.is_some() && self.list[i].raw.get("sound").and_then(Value::as_str).is_none() {
      return Err("give it a sound first: /rule sound <number> chime");
    }
    self.amend(key, |o| match pct {
      Some(p) => _ = o.insert("soundVolume".into(), json!(p.min(100))),
      None => _ = o.remove("soundVolume"),
    })
  }

  // a rule's fields as the list screen shows and edits them
  pub fn view(&self, at: usize) -> Option<View> {
    let raw = &self.list.get(at)?.raw;
    let text = |v: Option<&Value>| v.and_then(Value::as_str).unwrap_or_default().to_owned();
    let m = raw.get("match");
    Some(View {
      action: text(raw.get("action")),
      ty: text(m.and_then(|m| m.get("type"))),
      value: text(m.and_then(|m| m.get("value"))),
      case: m.and_then(|m| m.get("caseSensitive")) == Some(&Value::Bool(true)),
      sound: Some(text(raw.get("sound"))).filter(|s| !s.is_empty()),
      file: Some(text(raw.get("soundName"))).filter(|s| !s.is_empty()),
      volume: raw
        .get("soundVolume")
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite())
        .map(|v| v.round().clamp(0.0, 100.0) as u8),
      notify: raw.get("notify") == Some(&Value::Bool(true)),
    })
  }

  // what a highlight row says beside its words: what it plays, how loud, whether it notifies
  pub fn extras(&self, at: usize) -> String {
    let Some(v) = self.view(at).filter(|v| v.action == "highlight") else { return String::new() };
    let mut out = Vec::new();
    if let Some(s) = &v.sound {
      let what: String =
        if s == "custom" { v.file.clone().unwrap_or_else(|| "file".into()) } else { s.chars().take(12).collect() };
      out.push(match v.volume {
        Some(n) => format!("\u{266a} {what} {n}"),
        None => format!("\u{266a} {what}"),
      });
    }
    if v.notify {
      out.push("notify".to_owned());
    }
    out.join(" \u{b7} ")
  }

  // how many of `rows` a rule built from these parts would take, with how many were tried: (hits, tried).
  // None = the parts make no usable rule (an empty value, a pattern that will not compile). it is the real
  // matcher, a rule for every channel; your own lines are left out, a rule never touches them
  pub fn preview<'a>(
    action: &str,
    ty: &str,
    value: &str,
    case: bool,
    rows: impl Iterator<Item = &'a Message>,
    chan: &str,
  ) -> Option<(usize, usize)> {
    let raw = json!({
      "id": "preview", "enabled": true, "scope": "all", "action": action,
      "match": { "type": ty, "value": value.trim(), "caseSensitive": case },
    });
    let c = compile(&raw)?;
    let (mut hits, mut tried) = (0, 0);
    for m in rows.filter(|m| m.kind.said() && !m.has(crate::model::message::flag::OWN)) {
      tried += 1;
      hits += usize::from(c.hits(&Line::new(m, chan)));
    }
    Some((hits, tried))
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::model::message::Platform;

  fn rules(list: &[Value]) -> Rules {
    Rules::parse(&Value::Array(list.to_vec()).to_string()).unwrap()
  }

  fn hl(id: &str, value: &str) -> Value {
    json!({"id": id, "enabled": true, "scope": "all", "action": "highlight", "color": "#00ff00",
      "match": {"type": "keyword", "value": value, "caseSensitive": true}})
  }

  fn line(body: &str) -> Message {
    let mut m = Message::system(0, Platform::Twitch, body);
    m.kind = crate::model::message::Kind::Chat;
    m.login = "bob".into();
    m
  }

  fn raw(r: &Rules, i: usize) -> Value {
    serde_json::from_str::<Vec<Value>>(&r.json()).unwrap().remove(i)
  }

  #[test]
  fn an_edit_keeps_the_id_the_place_and_the_rest() {
    let mut r = rules(&[hl("a", "one"), hl("b", "two"), hl("c", "three")]);
    r.edit("2", "highlight", "keyword", "deux").unwrap();
    assert_eq!(raw(&r, 1)["id"], "b", "same id");
    assert_eq!(raw(&r, 1)["match"]["value"], "deux");
    assert_eq!(raw(&r, 1)["match"]["caseSensitive"], true, "case kept");
    assert_eq!(raw(&r, 1)["color"], "#00ff00", "colour kept");
    assert_eq!(raw(&r, 0)["match"]["value"], "one");
    assert_eq!(raw(&r, 2)["match"]["value"], "three");
    // and it matches by the new words from then on
    assert_eq!(r.judge(&Line::new(&line("deux"), "")).verdict, super::super::Verdict::Mark);
    assert_eq!(r.judge(&Line::new(&line("two"), "")).verdict, super::super::Verdict::Pass);
  }

  #[test]
  fn a_bad_edit_changes_nothing() {
    let mut r = rules(&[hl("a", "one")]);
    let before = r.json();
    assert_eq!(r.edit("1", "highlight", "expr", "&& &&").unwrap_err(), "that expression doesn't parse");
    assert!(r.edit("1", "highlight", "keyword", "   ").is_err(), "an empty value is no rule");
    assert!(r.edit("1", "explode", "keyword", "x").is_err());
    assert!(r.edit("1", "highlight", "nonsense", "x").is_err());
    assert!(r.edit("9", "hide", "keyword", "x").is_err());
    assert_eq!(r.json(), before);
    // a value that would burst the setting's size is refused too
    let big = "x".repeat(super::super::MAX_JSON);
    assert!(r.edit("1", "highlight", "keyword", &big).is_err());
    assert_eq!(r.json(), before);
  }

  #[test]
  fn hiding_lets_go_of_what_only_a_highlight_has_and_back_gets_the_plain_colour() {
    let mut r = rules(&[hl("a", "one")]);
    r.set_sound("1", &Pick::Preset(Preset::Chime)).unwrap();
    r.set_volume("1", Some(40)).unwrap();
    r.set_notify("1", true).unwrap();
    r.edit("1", "hide", "keyword", "one").unwrap();
    let h = raw(&r, 0);
    for k in ["color", "sound", "soundName", "soundVolume", "notify"] {
      assert!(h.get(k).is_none(), "{k}");
    }
    r.edit("1", "highlight", "keyword", "one").unwrap();
    assert_eq!(raw(&r, 0)["color"], "#ffff00");
    // a hide rule has none of those to set
    r.edit("1", "hide", "keyword", "one").unwrap();
    assert!(
      r.set_notify("1", true).is_err() && r.set_sound("1", &Pick::Off).is_err() && r.set_volume("1", None).is_err()
    );
  }

  #[test]
  fn sound_volume_and_notify_ride_the_rule_and_reach_the_verdict() {
    let mut r = rules(&[hl("a", "gg")]);
    let judged = |r: &Rules| r.judge(&Line::new(&line("gg"), ""));
    assert_eq!((judged(&r).sound, judged(&r).volume, judged(&r).notify), (None, None, false));
    // a volume with no sound has nothing to apply to
    assert!(r.set_volume("1", Some(50)).is_err());
    r.set_sound("1", &Pick::Preset(Preset::Knock)).unwrap();
    r.set_volume("1", Some(250)).unwrap();
    r.set_notify("a", true).unwrap();
    let j = judged(&r);
    assert_eq!((j.sound, j.volume, j.notify, j.custom, j.at), (Some(Preset::Knock), Some(100), true, false, 0));
    // a file: only its name is in the rule, the sound is custom, the bounds hold
    r.set_sound("1", &Pick::File(format!("my\nping{}.wav", "x".repeat(300)))).unwrap();
    let f = raw(&r, 0);
    assert_eq!(f["sound"], "custom");
    assert!(
      f["soundName"].as_str().unwrap().chars().count() <= MAX_NAME && !f["soundName"].as_str().unwrap().contains('\n')
    );
    let j = judged(&r);
    assert!(j.custom && j.sound.is_none() && j.volume == Some(100));
    // off drops the sound and its volume
    r.set_sound("1", &Pick::Off).unwrap();
    let j = judged(&r);
    assert_eq!((j.sound, j.custom, j.volume), (None, false, None));
    assert!(raw(&r, 0).get("soundVolume").is_none());
    assert!(r.set_notify("1", false).is_ok() && !judged(&r).notify);
    assert_eq!(r.extras(0), "");
  }

  #[test]
  fn a_junk_volume_in_a_synced_rule_follows_the_shared_one() {
    let mut a = hl("a", "gg");
    a["sound"] = json!("ping");
    for bad in [json!("loud"), json!(null), json!([1]), json!({"x": 1})] {
      a["soundVolume"] = bad;
      let r = rules(&[a.clone()]);
      assert_eq!(r.judge(&Line::new(&line("gg"), "")).volume, None);
    }
    a["soundVolume"] = json!(-5);
    assert_eq!(rules(&[a.clone()]).judge(&Line::new(&line("gg"), "")).volume, Some(0));
    a["soundVolume"] = json!(33.6);
    assert_eq!(rules(&[a.clone()]).judge(&Line::new(&line("gg"), "")).volume, Some(34));
    // a volume with no sound is ignored
    a.as_object_mut().unwrap().remove("sound");
    assert_eq!(rules(&[a]).judge(&Line::new(&line("gg"), "")).volume, None);
    // notify only counts as the boolean true, and only on a highlight
    let mut n = hl("n", "gg");
    n["notify"] = json!("yes");
    assert!(!rules(&[n.clone()]).judge(&Line::new(&line("gg"), "")).notify);
    n["notify"] = json!(true);
    assert!(rules(&[n]).judge(&Line::new(&line("gg"), "")).notify);
  }

  #[test]
  fn extras_say_what_the_row_plays() {
    let mut r = rules(&[hl("a", "gg")]);
    r.set_sound("1", &Pick::Preset(Preset::Chime)).unwrap();
    assert_eq!(r.extras(0), "\u{266a} chime");
    r.set_volume("1", Some(40)).unwrap();
    r.set_notify("1", true).unwrap();
    assert_eq!(r.extras(0), "\u{266a} chime 40 \u{b7} notify");
    r.set_sound("1", &Pick::File("ding.wav".into())).unwrap();
    assert_eq!(r.extras(0), "\u{266a} ding.wav 40 \u{b7} notify");
    assert_eq!(r.extras(7), "");
  }

  #[test]
  fn the_preview_counts_with_the_real_matcher_and_skips_your_own_lines() {
    let rows: Vec<Message> = ["gg wp", "no", "GG", "ggwp", "gg"].iter().map(|b| line(b)).collect();
    let mut own = line("gg");
    own.flags |= crate::model::message::flag::OWN;
    let all: Vec<&Message> = rows.iter().chain([&own]).collect();
    let n = |ty: &str, v: &str, cs: bool| Rules::preview("highlight", ty, v, cs, all.iter().copied(), "");
    assert_eq!(n("keyword", "gg", false), Some((3, 5)), "whole words, any case, not your own line");
    assert_eq!(n("keyword", "gg", true), Some((2, 5)));
    assert_eq!(n("regex", "^gg", false), Some((4, 5)));
    assert_eq!(n("user", "bob", false), Some((5, 5)));
    assert_eq!(n("expr", "&& &&", false), None, "an expression that will not parse");
    assert_eq!(n("keyword", "  ", false), None);
    assert_eq!(n("expr", "&&", false), None);
    // the system lines are not messages to try a rule on
    let sys = Message::system(0, Platform::Twitch, "gg");
    assert_eq!(Rules::preview("hide", "keyword", "gg", false, [&sys].into_iter(), ""), Some((0, 0)));
  }
}
