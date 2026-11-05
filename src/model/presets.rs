// your own settings presets (ext customPresets): the switches and choices that differ from the defaults,
// saved under a name, applied later, deleted. a preset only touches the rows it names, so it composes with
// whatever else you changed. kept in one small file (`presets`, owner-only): a json list of
// {name, diff: {row id: true | "choice"}}. the file is hostile like any other: unknown rows and wrong
// types are dropped, the list and the file are bounded, an unreadable file is moved to presets.bak
use std::cell::RefCell;

use serde_json::{Map, Value, json};

use crate::model::schema::{self, DEFS, Def};
use crate::model::settings::Settings;

// the ext keeps the last 8, names cut at 24
pub const MAX: usize = 8;
pub const NAME_MAX: usize = 24;
// a full preset is a few KB: a bigger file is not ours
const FILE_MAX: usize = 16 * 1024;

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Val {
  On(bool),
  Is(String),
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Custom {
  pub name: String,
  pub diff: Vec<(String, Val)>,
}

// one line, no control chars, cut to the cap
pub fn clean(name: &str) -> String {
  let flat: String = name.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
  flat.trim().chars().take(NAME_MAX).collect::<String>().trim_end().to_owned()
}

fn same(a: &str, b: &str) -> bool {
  a.trim().eq_ignore_ascii_case(b.trim())
}

// the row's value now as a preset holds it; None = not a switch or a choice
fn value(d: &Def, s: &Settings) -> Option<Val> {
  d.on(s).map(Val::On).or_else(|| d.picked(s).map(|n| Val::Is(n.to_owned())))
}

// every switch and choice that is not at its default
pub fn snapshot(s: &Settings) -> Vec<(String, Val)> {
  let base = Settings::default();
  DEFS
    .iter()
    .filter_map(|d| {
      let v = value(d, s)?;
      (Some(&v) != value(d, &base).as_ref()).then(|| (d.id.to_owned(), v))
    })
    .collect()
}

impl Custom {
  // every row it names reads as it says (it names at least one)
  pub fn active(&self, s: &Settings) -> bool {
    !self.diff.is_empty()
      && self.diff.iter().all(|(id, v)| schema::def(id).and_then(|d| value(d, s)).as_ref() == Some(v))
  }

  // write its values into `s`; the rows that changed (the app redoes what each one needs)
  pub fn apply(&self, s: &mut Settings) -> Vec<&'static Def> {
    let mut moved = Vec::new();
    for (id, v) in &self.diff {
      let Some(d) = schema::def(id) else { continue };
      if value(d, s).as_ref() == Some(v) {
        continue;
      }
      let ok = match v {
        Val::On(on) => d.put(s, *on),
        Val::Is(n) => d.pick(s, n),
      };
      if ok {
        moved.push(d);
      }
    }
    moved
  }

  fn to_json(&self) -> Value {
    let diff: Map<String, Value> = self
      .diff
      .iter()
      .map(|(k, v)| {
        (
          k.clone(),
          match v {
            Val::On(b) => json!(b),
            Val::Is(n) => json!(n),
          },
        )
      })
      .collect();
    json!({ "name": self.name, "diff": diff })
  }

  // None = not a preset; rows we have no switch for, or of the wrong type, are left out
  fn from_json(v: &Value) -> Option<Custom> {
    let name = clean(v.get("name")?.as_str()?);
    let diff = v.get("diff")?.as_object()?;
    if name.is_empty() {
      return None;
    }
    let diff = diff
      .iter()
      .take(DEFS.len())
      .filter_map(|(id, v)| {
        let d = schema::def(id)?;
        let v = match v {
          Value::Bool(b) if d.on(&Settings::default()).is_some() => Val::On(*b),
          Value::String(n) if d.picked(&Settings::default()).is_some() => {
            let mut t = Settings::default();
            d.pick(&mut t, n).then(|| Val::Is(n.trim().to_owned()))?
          }
          _ => return None,
        };
        Some((id.clone(), v))
      })
      .collect();
    Some(Custom { name, diff })
  }
}

#[derive(Default, Debug)]
pub struct Customs {
  list: Vec<Custom>,
}

impl Customs {
  pub fn all(&self) -> &[Custom] {
    &self.list
  }

  pub fn find(&self, name: &str) -> Option<&Custom> {
    self.list.iter().find(|p| same(&p.name, name))
  }

  // Err = why it was not kept. the same name again replaces it; past MAX the oldest goes (the ext's slice(-8))
  pub fn add(&mut self, name: &str, diff: Vec<(String, Val)>) -> Result<(), &'static str> {
    let name = clean(name);
    if name.is_empty() {
      return Err("preset needs a name");
    }
    if schema::PRESETS.iter().any(|p| same(p.name, &name)) {
      return Err("that name is a built-in preset");
    }
    if diff.is_empty() {
      return Err("nothing to save: your settings are the defaults");
    }
    let mut next: Vec<Custom> = self.list.iter().filter(|p| !same(&p.name, &name)).cloned().collect();
    next.push(Custom { name, diff });
    if next.len() > MAX {
      next.remove(0);
    }
    if render(&next).len() > FILE_MAX {
      return Err("presets storage full: delete one first");
    }
    self.list = next;
    Ok(())
  }

  pub fn remove(&mut self, name: &str) -> bool {
    let n = self.list.len();
    self.list.retain(|p| !same(&p.name, name));
    self.list.len() != n
  }

  pub fn parse(s: &str) -> Option<Self> {
    let Value::Array(a) = serde_json::from_str::<Value>(s).ok()? else { return None };
    let mut list: Vec<Custom> = Vec::new();
    for p in a.iter().filter_map(Custom::from_json) {
      if list.len() < MAX && !list.iter().any(|o| same(&o.name, &p.name)) {
        list.push(p);
      }
    }
    Some(Customs { list })
  }

  pub fn load() -> Self {
    let Some(s) = crate::store::read_capped("presets", FILE_MAX as u64) else { return Self::default() };
    Self::parse(&s).unwrap_or_else(|| {
      crate::store::set_aside("presets", None);
      Self::default()
    })
  }

  pub fn save(&self) {
    if let Err(e) = crate::store::write("presets", &render(&self.list)) {
      eprintln!("save presets: {e}");
    }
  }
}

fn render(list: &[Custom]) -> String {
  Value::Array(list.iter().map(Custom::to_json).collect()).to_string()
}

// the ui thread owns them; the settings screen reads them on every draw
thread_local! {
  static LIST: RefCell<Customs> = RefCell::new(Customs::default());
}

pub fn with<R>(f: impl FnOnce(&Customs) -> R) -> R {
  LIST.with(|l| f(&l.borrow()))
}

pub fn edit<R>(f: impl FnOnce(&mut Customs) -> R) -> R {
  LIST.with(|l| f(&mut l.borrow_mut()))
}

#[derive(PartialEq, Eq, Debug)]
pub enum Cmd {
  // name them
  List,
  Save(String),
  Delete(String),
  // a built-in or a saved one
  Apply(String),
  Usage(&'static str),
}

const USAGE: &str = "usage: /preset [name] | save <name> | del <name>";

pub fn command(text: &str) -> Option<Cmd> {
  let t = text.trim();
  let (cmd, rest) = t.split_once(char::is_whitespace).unwrap_or((t, ""));
  if !cmd.eq_ignore_ascii_case("/preset") {
    return None;
  }
  let rest = rest.trim();
  let (verb, tail) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
  let tail = tail.trim();
  Some(match verb.to_ascii_lowercase().as_str() {
    "" => Cmd::List,
    "save" if !tail.is_empty() => Cmd::Save(tail.to_owned()),
    "del" | "rm" | "delete" if !tail.is_empty() => Cmd::Delete(tail.to_owned()),
    "save" | "del" | "rm" | "delete" => Cmd::Usage(USAGE),
    _ => Cmd::Apply(rest.to_owned()),
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  fn tweaked() -> Settings {
    let mut s = Settings::default();
    schema::def("vi").unwrap().put(&mut s, true);
    schema::def("emote_size").unwrap().pick(&mut s, "4x");
    s
  }

  #[test]
  fn a_snapshot_holds_only_what_differs_and_applies_back() {
    assert!(snapshot(&Settings::default()).is_empty());
    let diff = snapshot(&tweaked());
    assert_eq!(diff.len(), 2);
    let p = Custom { name: "mine".into(), diff };
    let mut s = Settings::default();
    assert!(!p.active(&s));
    assert_eq!(p.apply(&mut s).len(), 2);
    assert!(p.active(&s) && s.vi);
    assert!(p.apply(&mut s).is_empty(), "already matching changes nothing");
    // sparse: another row you changed stays
    schema::def("zebra").unwrap().put(&mut s, false);
    let q = Custom { name: "q".into(), diff: vec![("vi".into(), Val::On(false))] };
    q.apply(&mut s);
    assert!(!s.vi && !s.zebra);
  }

  #[test]
  fn names_are_flat_capped_and_unique_ignoring_case() {
    let mut c = Customs::default();
    let d = snapshot(&tweaked());
    assert!(c.add("  ", d.clone()).is_err());
    assert!(c.add("Normal", d.clone()).is_err(), "built-ins keep their names");
    assert!(c.add("x", Vec::new()).is_err());
    assert!(c.add("My\nBuild", d.clone()).is_ok());
    assert!(c.add("my build", d.clone()).is_ok());
    assert_eq!(c.all().len(), 1);
    assert_eq!(c.find("MY BUILD").unwrap().name, "my build");
    assert_eq!(clean(&"é".repeat(40)).chars().count(), NAME_MAX);
    assert!(c.remove("My Build") && !c.remove("my build"));
  }

  #[test]
  fn the_list_keeps_the_last_eight() {
    let mut c = Customs::default();
    for i in 0..MAX + 3 {
      c.add(&format!("p{i}"), snapshot(&tweaked())).unwrap();
    }
    assert_eq!(c.all().len(), MAX);
    assert!(c.find("p0").is_none() && c.find("p10").is_some());
  }

  #[test]
  fn the_file_round_trips_and_survives_garbage() {
    let mut c = Customs::default();
    c.add("mine", snapshot(&tweaked())).unwrap();
    let back = Customs::parse(&render(c.all())).unwrap();
    assert_eq!(back.all(), c.all());
    // unknown rows, wrong types and bad entries are dropped, the good ones stay
    let junk = r#"[{"name":"a","diff":{"vi":true,"nope":true,"zebra":"x","emote_size":"9x","emoji_size":"2x"}},
      {"name":"","diff":{}}, {"diff":{}}, 5, {"name":"a","diff":{}}, {"name":"b","diff":7}]"#;
    let g = Customs::parse(junk).unwrap();
    assert_eq!(g.all().len(), 1);
    assert_eq!(g.all()[0].diff, [("emoji_size".to_owned(), Val::Is("2x".into())), ("vi".to_owned(), Val::On(true))]);
    assert!(Customs::parse("{not json").is_none() && Customs::parse("{}").is_none());
  }

  #[test]
  fn the_commands() {
    assert_eq!(command("/preset"), Some(Cmd::List));
    assert_eq!(command("/PRESET save My Build "), Some(Cmd::Save("My Build".into())));
    assert_eq!(command("/preset del mine"), Some(Cmd::Delete("mine".into())));
    assert_eq!(command("/preset save"), Some(Cmd::Usage(USAGE)));
    assert_eq!(command("/preset low ram"), Some(Cmd::Apply("low ram".into())));
    assert_eq!(command("/presets"), None);
  }
}
