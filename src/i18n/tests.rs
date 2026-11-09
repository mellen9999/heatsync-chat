use super::*;
use std::collections::BTreeSet;

fn holes(s: &str) -> BTreeSet<String> {
  let mut out = BTreeSet::new();
  let mut rest = s;
  while let Some(i) = rest.find('{') {
    let Some(j) = rest[i..].find('}') else { break };
    out.insert(rest[i + 1..i + j].to_owned());
    rest = &rest[i + j + 1..];
  }
  out
}

// a key's plural base: `x.y.other` → `x.y`
fn base(key: &str) -> Option<&str> {
  ["zero", "one", "two", "few", "many", "other"].iter().find_map(|f| key.strip_suffix(&format!(".{f}")))
}

#[test]
fn english_has_text_for_every_key() {
  let en = keys(lang::EN);
  assert!(en.len() > 500, "english looks empty: {}", en.len());
  for k in en {
    let v = raw(lang::EN, k).unwrap();
    assert!(!v.is_empty() && v != k, "{k}");
  }
}

#[test]
fn every_language_has_every_key() {
  let en: BTreeSet<&str> = keys(lang::EN).into_iter().collect();
  for (i, code) in lang::CODES.iter().enumerate() {
    let have: BTreeSet<&str> = keys(i).into_iter().collect();
    let missing: Vec<_> = en.iter().filter(|k| base(k).is_none() && !have.contains(*k)).take(5).collect();
    assert!(missing.is_empty(), "{code} lacks {missing:?} ({} in all)", en.difference(&have).count());
    let extra: Vec<_> = have.iter().filter(|k| !en.contains(*k) && base(k).is_none()).take(5).collect();
    assert!(extra.is_empty(), "{code} has keys english lacks: {extra:?}");
  }
}

#[test]
fn every_language_has_the_plural_forms_it_counts_by() {
  let bases: BTreeSet<&str> = keys(lang::EN).into_iter().filter_map(base).collect();
  assert!(!bases.is_empty());
  for (i, code) in lang::CODES.iter().enumerate() {
    for b in &bases {
      for form in lang::forms(i) {
        // `other` is the fallback every form may lean on; the rest must be written
        let k = format!("{b}.{form}");
        assert!(raw(i, &k).is_some_and(|v| !v.is_empty()), "{code} lacks {k}");
      }
    }
  }
}

#[test]
fn placeholders_survive_translation() {
  for k in keys(lang::EN) {
    let want = holes(raw(lang::EN, k).unwrap());
    for (i, code) in lang::CODES.iter().enumerate() {
      let Some(v) = raw(i, k) else { continue };
      let got = holes(v);
      // a plural form may drop `{n}` ("one" written as a word), never add or rename holes
      let ok = if base(k).is_some() { got.is_subset(&want) } else { got == want };
      assert!(ok, "{code} {k}: {got:?} vs english {want:?}");
    }
  }
}

#[test]
fn translations_are_never_empty_or_the_raw_key() {
  for (i, code) in lang::CODES.iter().enumerate() {
    for k in keys(i) {
      let v = raw(i, k).unwrap();
      assert!(!v.trim().is_empty() && v != k, "{code} {k}");
    }
  }
}

fn sources(dir: &std::path::Path, out: &mut Vec<String>) {
  for e in std::fs::read_dir(dir).unwrap().flatten() {
    let p = e.path();
    if p.is_dir() {
      if p.file_name().is_some_and(|n| n == "i18n") {
        continue;
      }
      sources(&p, out);
    } else if p.extension().is_some_and(|x| x == "rs") {
      out.push(std::fs::read_to_string(&p).unwrap());
    }
  }
}

// every key the code asks for by literal exists in english, so nothing can draw as a raw key
#[test]
fn every_key_in_the_code_has_english_text() {
  let re = regex::Regex::new(r#"\b(?:t|f|tn)\(\s*"([a-z0-9_]+(?:\.[a-z0-9_]+)+)""#).unwrap();
  let mut files = Vec::new();
  sources(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
  let mut seen = 0;
  let mut bad = Vec::new();
  for src in &files {
    for c in re.captures_iter(src) {
      seen += 1;
      let k = &c[1];
      if lookup_in(lang::EN, k).is_none() && lookup_in(lang::EN, &format!("{k}.other")).is_none() {
        bad.push(k.to_owned());
      }
    }
  }
  assert!(seen > 300, "the scan found only {seen} keys");
  assert!(bad.is_empty(), "keys with no english text: {bad:?}");
}

#[test]
fn the_setting_beats_the_os_language() {
  let (de, fr) = (lang::index("de").unwrap(), lang::index("fr").unwrap());
  // 0 = auto follows the os, any other value is its own language whatever the os says
  assert_eq!(resolve(0, fr), fr);
  assert_eq!(resolve(lang::index("ja").unwrap() + 1, fr), lang::index("ja").unwrap());
  assert_eq!(resolve(de + 1, fr), de);
  assert_eq!(crate::model::settings::Language::ALL.len(), lang::CODES.len() + 1);
  for (i, c) in lang::CODES.iter().enumerate() {
    assert_eq!(crate::model::settings::Language::CHOICE.names[i + 1], c.to_lowercase());
  }
}

#[test]
fn a_missing_key_falls_back_to_english_and_an_unknown_one_to_nothing() {
  assert_eq!(lookup_in(lang::index("de").unwrap(), "no.such.key"), None);
  assert_eq!(f_in("x {a} y", &[("a", "1")]), "x 1 y");
}

fn f_in(text: &str, args: &[(&str, &str)]) -> String {
  fill(text, args)
}
