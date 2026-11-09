// what sensitive pictures show (ext Filters › Content, viewer_show_*): one rule for feed media and
// heatsync emotes. five switches, all off by default. the net threads parse emotes, so the switches
// live here as one atomic the app keeps in step with its settings
use std::sync::atomic::{AtomicU8, Ordering};

use serde_json::Value;

use crate::i18n::t;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cat {
  Sexual,
  Violence,
  Weapons,
  Drugs,
  Hate,
}

impl Cat {
  // what the hidden box says (ext HS_MEDIA_SETTING labels)
  pub fn label(self) -> &'static str {
    match self {
      Cat::Sexual => t("misc.content.nsfw"),
      Cat::Violence => t("misc.content.violence"),
      Cat::Weapons => t("misc.content.weapons"),
      Cat::Drugs => t("misc.content.drugs"),
      Cat::Hate => t("misc.content.hate"),
    }
  }

  // the server's names: media moderation (nsfw violence weapons drugs hate) and emote cw_cats
  // (sexual gore weapon drug hate)
  pub fn of(name: &str) -> Option<Cat> {
    Some(match name {
      "nsfw" | "sexual" => Cat::Sexual,
      "violence" | "gore" => Cat::Violence,
      "weapons" | "weapon" => Cat::Weapons,
      "drugs" | "drug" => Cat::Drugs,
      "hate" => Cat::Hate,
      _ => return None,
    })
  }

  fn bit(self) -> u8 {
    1 << self as u8
  }
}

#[cfg(not(test))]
static SHOWN: AtomicU8 = AtomicU8::new(0);
// each test runs on its own thread: its own switches, so parallel tests never see each other's
#[cfg(test)]
thread_local!(static SHOWN: AtomicU8 = const { AtomicU8::new(0) });

fn mask() -> u8 {
  #[cfg(not(test))]
  return SHOWN.load(Ordering::Relaxed);
  #[cfg(test)]
  SHOWN.with(|s| s.load(Ordering::Relaxed))
}

// the app's switches, after a load or a flip
pub fn set_shown(on: impl IntoIterator<Item = (Cat, bool)>) {
  let m = on.into_iter().filter(|x| x.1).fold(0, |m, (c, _)| m | c.bit());
  #[cfg(not(test))]
  SHOWN.store(m, Ordering::Relaxed);
  #[cfg(test)]
  SHOWN.with(|s| s.store(m, Ordering::Relaxed));
}

pub fn shown(c: Cat) -> bool {
  mask() & c.bit() != 0
}

// a feed picture the viewer has not asked to see: what the box says, and whether `show` can lift it.
// csam never shows; a category we don't know is held too (ext hsHeldMediaCat: held unless switched on)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hold {
  pub label: Box<str>,
  pub can_show: bool,
}

pub fn held<'a>(cats: impl IntoIterator<Item = &'a str>) -> Option<Hold> {
  let mut first = None;
  for c in cats {
    if c == "csam" {
      return Some(Hold { label: t("misc.content.blocked").into(), can_show: false });
    }
    if first.is_none() && Cat::of(c).is_none_or(|k| !shown(k)) {
      let label: &str = match Cat::of(c) {
        Some(k) => k.label(),
        None => c,
      };
      first = Some(Hold { label: label.chars().filter(|c| !c.is_control()).take(32).collect(), can_show: true });
    }
  }
  first
}

// an emote the viewer hides: flagged (`nsfw`, or a `cw_cats` category) with that category switched off.
// a category we don't know shows (ext hsOwnCwHiddenCat: hidden only when the switch is off)
pub fn emote_hidden(e: &Value) -> bool {
  let nsfw = (e.get("nsfw").and_then(Value::as_bool) == Some(true)).then_some(Cat::Sexual);
  let cats = e.get("cw_cats").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str);
  nsfw.into_iter().chain(cats.filter_map(Cat::of)).any(|c| !shown(c))
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn the_switches_decide() {
    set_shown([]);
    assert_eq!(held(["violence"]), Some(Hold { label: "violence".into(), can_show: true }));
    assert_eq!(held(["sexual"]).map(|h| h.label), Some("nsfw".into()));
    assert_eq!(held(["csam", "violence"]).map(|h| h.can_show), Some(false));
    assert_eq!(held(["mystery"]).map(|h| h.label), Some("mystery".into()), "unknown: held");
    assert_eq!(held([]), None);
    assert!(emote_hidden(&json!({"nsfw": true})));
    assert!(emote_hidden(&json!({"cw_cats": ["weapon"]})));
    assert!(!emote_hidden(&json!({"cw_cats": ["mystery"]})), "unknown: shown");
    assert!(!emote_hidden(&json!({})));
    set_shown([(Cat::Violence, true), (Cat::Sexual, true)]);
    assert_eq!(held(["gore"]), None);
    assert_eq!(held(["violence", "drugs"]).map(|h| h.label), Some("drugs".into()), "the first held one names it");
    assert_eq!(held(["csam"]).map(|h| h.can_show), Some(false), "csam never shows");
    assert!(!emote_hidden(&json!({"nsfw": true, "cw_cats": ["gore"]})));
    assert!(emote_hidden(&json!({"cw_cats": ["sexual", "hate"]})));
  }
}
