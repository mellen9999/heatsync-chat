// a run of timeouts and bans in one tab, folded into one line like chatterino's "combine timeouts":
// `▸ 5 people timed out`, and a click or enter opens it to say who and for how long. only the mod-action
// rows (a person timed out or banned) join a run; a normal chat line between them ends it. what a run
// keeps is bounded, and the words it came from are cleaned (names are a stranger's)

// lines a run keeps to show when opened; more still count, they just are not listed
pub const MAX_LINES: usize = 200;
// distinct names kept to count people once
const MAX_NAMES: usize = 200;
const LINE_CHARS: usize = 120;

#[derive(Clone, Debug, Default)]
pub struct Run {
  names: Vec<Box<str>>,
  lines: Vec<Box<str>>,
  // people timed out or banned (a name seen twice counts once, while the names fit)
  people: u32,
  bans: u32,
  // listed under the header
  pub open: bool,
}

fn clean(s: &str) -> Box<str> {
  s.chars().filter(|c| !c.is_control()).take(LINE_CHARS).collect::<String>().into()
}

impl Run {
  // one person's timeout (`ban` = no end) and the line that says it
  pub fn new(who: &str, line: &str, ban: bool) -> Self {
    Run {
      names: vec![clean(&who.to_lowercase())],
      lines: vec![clean(line)],
      people: 1,
      bans: u32::from(ban),
      open: false,
    }
  }

  // two or more events: the line is a header now
  pub fn many(&self) -> bool {
    self.lines.len() > 1 || self.people > 1
  }

  // `other` is a run of one: its person joins this one
  pub fn add(&mut self, other: &Run) {
    self.bans += other.bans;
    let name = other.names.first();
    if name.is_some_and(|n| self.names.contains(n)) {
      // the same person again: one more line, not one more person
    } else {
      self.people = self.people.saturating_add(1);
      if let Some(n) = name.filter(|_| self.names.len() < MAX_NAMES) {
        self.names.push(n.clone());
      }
    }
    if self.lines.len() < MAX_LINES {
      self.lines.extend(other.lines.iter().cloned());
    }
  }

  // red once anyone was banned, green while all were timeouts (the single rows' own inks)
  pub fn red(&self) -> bool {
    self.bans > 0
  }

  pub fn header(&self) -> String {
    let who = if self.people == 1 { "1 person".to_owned() } else { format!("{} people", self.people) };
    let did = match (self.bans, self.people) {
      (0, _) => "timed out",
      (b, p) if b >= p => "banned",
      _ => "timed out or banned",
    };
    format!("{who} {did}")
  }

  // what the row says: a lone event is its own line; a run is its header, and when open each event under it
  pub fn text(&self) -> String {
    if !self.many() {
      return self.lines.first().map(|l| l.to_string()).unwrap_or_default();
    }
    if !self.open {
      return format!("▸ {}", self.header());
    }
    let mut s = format!("▾ {}", self.header());
    for l in &self.lines {
      s.push_str("\n  ");
      s.push_str(l);
    }
    if self.lines.len() < self.people as usize {
      s.push_str(&format!("\n  and {} more", self.people as usize - self.lines.len()));
    }
    s
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn timeout(who: &str) -> Run {
    Run::new(who, &format!("{who} timed out for 60s"), false)
  }

  #[test]
  fn one_event_reads_as_itself_and_a_run_as_a_count() {
    let mut r = timeout("bob");
    assert!(!r.many());
    assert_eq!(r.text(), "bob timed out for 60s");
    r.add(&timeout("cat"));
    r.add(&timeout("dan"));
    assert_eq!(r.text(), "▸ 3 people timed out");
    r.open = true;
    assert_eq!(
      r.text(),
      "▾ 3 people timed out\n  bob timed out for 60s\n  cat timed out for 60s\n  dan timed out for 60s"
    );
  }

  #[test]
  fn bans_change_the_words_and_the_ink() {
    let mut r = Run::new("bob", "bob was permanently banned", true);
    r.add(&Run::new("cat", "cat was permanently banned", true));
    assert_eq!(r.header(), "2 people banned");
    r.add(&timeout("dan"));
    assert_eq!(r.header(), "3 people timed out or banned");
    assert!(r.red());
  }

  #[test]
  fn the_same_person_twice_is_one_person() {
    let mut r = timeout("Bob");
    r.add(&timeout("bob"));
    assert_eq!(r.header(), "1 person timed out");
    assert!(r.many(), "two lines still fold");
    assert_eq!(r.text(), "▸ 1 person timed out");
  }

  #[test]
  fn a_storm_is_bounded_and_says_how_many_it_did_not_list() {
    let mut r = timeout("u0");
    for i in 1..5000 {
      r.add(&timeout(&format!("u{i}")));
    }
    assert_eq!(r.people, 5000);
    assert_eq!(r.lines.len(), MAX_LINES);
    assert!(r.names.len() <= MAX_NAMES);
    r.open = true;
    assert!(r.text().ends_with("and 4800 more"));
  }

  #[test]
  fn a_hostile_name_cannot_forge_lines_or_run_long() {
    let r = Run::new("x\ny", &format!("evil\n  bob timed out{}", "z".repeat(10_000)), false);
    assert!(!r.text().contains('\n'));
    assert!(r.text().chars().count() <= LINE_CHARS);
  }
}
