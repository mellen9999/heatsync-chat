// every word the app shows, in the language of the machine (or the one picked in settings).
// `t("area.key")` looks a string up; `f` fills `{name}` holes; `tn` picks the plural form for a count.
// the strings live in `locales/<code>.json`, embedded, flat `key → text`. a key a language lacks falls back to
// english, and one english lacks is a bug a test catches: it draws as nothing, never as the raw key.
// only english and the language in use are ever parsed
pub mod lang;

use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

type Table = HashMap<String, String>;

static CURRENT: AtomicUsize = AtomicUsize::new(lang::EN);
static TABLES: [OnceLock<Table>; 34] = [const { OnceLock::new() }; 34];

mod locales;
#[cfg(test)]
mod tests;
use locales::SOURCES;

fn table(i: usize) -> &'static Table {
  TABLES[i].get_or_init(|| serde_json::from_str(SOURCES[i]).unwrap_or_default())
}

// the language in use: an index into `lang::CODES`
pub fn current() -> usize {
  CURRENT.load(Ordering::Relaxed)
}

// switch to language `i` (out of range = english)
pub fn set(i: usize) {
  CURRENT.store(if i < lang::CODES.len() { i } else { lang::EN }, Ordering::Relaxed);
}

// settings value → the language in use: 0 is `auto` (the os's), then `lang::CODES` in order
pub fn apply(setting: usize) {
  // tests read english whatever the machine says: only `set` moves them
  set(resolve(setting, if cfg!(test) { lang::EN } else { lang::os() }));
}

// what the setting picks: itself, or the os's language when it is `auto`. the setting wins over the os
pub fn resolve(setting: usize, os: usize) -> usize {
  if setting == 0 { os } else { setting - 1 }
}

fn lookup(key: &str) -> Option<&'static str> {
  lookup_in(current(), key)
}

// the text for `key` in language `l`, english where that language has none
pub fn lookup_in(l: usize, key: &str) -> Option<&'static str> {
  table(l).get(key).or_else(|| table(lang::EN).get(key)).map(String::as_str).filter(|s| !s.is_empty())
}

// the text for `key`
pub fn t(key: &str) -> &'static str {
  match lookup(key) {
    Some(s) => s,
    None => {
      debug_assert!(false, "no english text for the key {key}");
      ""
    }
  }
}

// the text for `key` with each `{name}` replaced by its value
pub fn f(key: &str, args: &[(&str, &str)]) -> String {
  fill(t(key), args)
}

pub fn fill(text: &str, args: &[(&str, &str)]) -> String {
  let mut out = text.to_owned();
  for (name, value) in args {
    out = out.replace(&format!("{{{name}}}"), value);
  }
  out
}

// the text for the plural form of `key` that count `n` takes (`key.one`, `key.few`, … then `key.other`), `{n}`
// filled in, and any other holes from `args`
pub fn tn(key: &str, n: u64, args: &[(&str, &str)]) -> String {
  let form = lang::plural(current(), n);
  let text = lookup(&format!("{key}.{form}")).or_else(|| lookup(&format!("{key}.other"))).unwrap_or_else(|| {
    debug_assert!(false, "no english text for the key {key}");
    ""
  });
  let mut out = fill(text, args);
  out = out.replace("{n}", &n.to_string());
  out
}

// every key a language has, for the tests
#[cfg(test)]
pub fn keys(i: usize) -> Vec<&'static str> {
  table(i).keys().map(String::as_str).collect()
}

#[cfg(test)]
pub fn raw(i: usize, key: &str) -> Option<&'static str> {
  table(i).get(key).map(String::as_str)
}
