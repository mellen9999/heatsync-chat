//! big emote-only lines: a line of nothing but pictures draws them twice as big. this is the site's
//! shared/emote-only.ts, and `emote-only.vectors.json` is the rule as data, vendored byte for byte from the
//! site (scripts/sync-emote-vectors.sh) and run below: a line reads the same in every app. the rule reads
//! what the row already drew, one `Kind` per word, never the text, so there is no second resolver here

/// the most pictures a big line holds; one more and the line is a wall
pub const BIG_MAX: usize = 5;
/// big = this many times the line's normal emote height
pub const BIG_SCALE: u16 = 2;

/// what a word of the line was drawn as
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
  /// a picture the row drew: a room, user, global or platform-native emote
  Emote,
  /// one unicode emoji (a word of three is three of these)
  Emoji,
  /// a zero-width emote stacked on the picture before it
  Overlay,
  /// a modifier word bound to a picture
  Mod,
  /// anything else: a word, a link, a mention, a name the room does not know, a blocked or taken-down emote
  Text,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Verdict {
  pub big: bool,
  /// emotes and emoji, never an overlay or a modifier
  pub pictures: usize,
}

/// a line read one word at a time, as the row draws it
#[derive(Clone, Copy, Default)]
pub struct Read {
  pictures: usize,
  text: bool,
  // a picture is on the line for an overlay to sit on
  open: bool,
}

impl Read {
  pub fn push(&mut self, k: Kind) {
    match k {
      Kind::Text => self.text = true,
      Kind::Emote | Kind::Emoji => (self.pictures, self.open) = (self.pictures + 1, true),
      // an overlay with no picture before it draws inline, so it is one
      Kind::Overlay if !self.open => (self.pictures, self.open) = (self.pictures + 1, true),
      Kind::Overlay | Kind::Mod => {}
    }
  }

  pub fn verdict(self) -> Verdict {
    Verdict { big: !self.text && (1..=BIG_MAX).contains(&self.pictures), pictures: self.pictures }
  }
}

/// the whole line at once: the rule the vectors run (the row itself reads a word at a time)
#[cfg(test)]
pub fn emote_only(read: &[Kind]) -> Verdict {
  let mut r = Read::default();
  read.iter().for_each(|&k| r.push(k));
  r.verdict()
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::Value;

  const FILE: &str = include_str!("emote-only.vectors.json");

  fn file() -> serde_json::Map<String, Value> {
    match serde_json::from_str::<Value>(FILE).expect("the vector file is json") {
      Value::Object(m) => m,
      _ => panic!("the vector file is an object"),
    }
  }

  fn kind(k: &Value) -> Kind {
    match k.as_str() {
      Some("emote") => Kind::Emote,
      Some("emoji") => Kind::Emoji,
      Some("overlay") => Kind::Overlay,
      Some("mod") => Kind::Mod,
      Some("text") => Kind::Text,
      _ => panic!("a kind this runner does not know: {k}"),
    }
  }

  // the file is the site's, byte for byte: its hash is kept beside it and compared across the apps
  #[test]
  fn the_vendored_vectors_are_the_ones_synced() {
    let sum = ring::digest::digest(&ring::digest::SHA256, FILE.as_bytes());
    let hex: String = sum.as_ref().iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(hex, include_str!("emote-only.vectors.sha256").trim(), "run scripts/sync-emote-vectors.sh");
  }

  #[test]
  fn the_constants_are_the_rules_of_the_file() {
    let f = file();
    assert!(f.keys().all(|k| ["about", "rules", "cases"].contains(&k.as_str())), "a field this runner does not know");
    let rules = f["rules"].as_object().expect("rules");
    assert!(rules.keys().all(|k| k == "max" || k == "scale"), "a rule this runner does not know");
    assert_eq!(rules["max"], BIG_MAX, "rules.max");
    assert_eq!(rules["scale"], BIG_SCALE, "rules.scale");
  }

  #[test]
  fn every_line_reads_as_the_vectors_say() {
    let f = file();
    let cases = f["cases"].as_array().expect("cases");
    assert!(cases.len() >= 40, "only {} cases", cases.len());
    for c in cases {
      let o = c.as_object().expect("a case is an object");
      let name = o["name"].as_str().expect("name");
      assert!(
        o.keys().all(|k| ["name", "line", "read", "want", "pictures"].contains(&k.as_str())),
        "{name}: a field this runner does not know"
      );
      let read: Vec<Kind> = o["read"].as_array().expect("read").iter().map(kind).collect();
      let want = match o["want"].as_str() {
        Some("big") => true,
        Some("normal") => false,
        w => panic!("{name}: a size this runner does not know: {w:?}"),
      };
      let got = emote_only(&read);
      assert_eq!(got.big, want, "{name}: {:?}", o["line"]);
      assert_eq!(o["pictures"], got.pictures, "{name}: pictures");
    }
  }

  // the row feeds the rule a word at a time; the whole line at once says the same
  #[test]
  fn a_line_read_a_word_at_a_time_says_what_the_whole_line_does() {
    let f = file();
    for c in f["cases"].as_array().unwrap() {
      let read: Vec<Kind> = c["read"].as_array().unwrap().iter().map(kind).collect();
      let mut r = Read::default();
      for (i, &k) in read.iter().enumerate() {
        r.push(k);
        assert_eq!(r.verdict(), emote_only(&read[..=i]), "{} at {i}", c["name"]);
      }
    }
  }
}
