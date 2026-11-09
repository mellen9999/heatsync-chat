// which of the 34 languages: the os locale mapped to the nearest one, and the plural rule each one counts by
// (the same 34 as the extension's _locales)

pub const CODES: [&str; 34] = [
  "ar", "bg", "cs", "da", "de", "el", "en", "es", "fi", "fr", "he", "hi", "hu", "id", "it", "ja", "ko", "ms", "nl",
  "no", "pl", "pt_BR", "pt_PT", "ro", "ru", "sk", "sv", "th", "tl", "tr", "uk", "vi", "zh_CN", "zh_TW",
];

pub const EN: usize = 6;

// each language in its own words, for the picker
pub const NATIVE: [&str; 34] = [
  "العربية",
  "български",
  "čeština",
  "dansk",
  "deutsch",
  "ελληνικά",
  "english",
  "español",
  "suomi",
  "français",
  "עברית",
  "हिन्दी",
  "magyar",
  "indonesia",
  "italiano",
  "日本語",
  "한국어",
  "melayu",
  "nederlands",
  "norsk",
  "polski",
  "português (br)",
  "português (pt)",
  "română",
  "русский",
  "slovenčina",
  "svenska",
  "ไทย",
  "tagalog",
  "türkçe",
  "українська",
  "tiếng việt",
  "简体中文",
  "繁體中文",
];

pub fn index(code: &str) -> Option<usize> {
  CODES.iter().position(|c| *c == code)
}

// right-to-left scripts: their text runs are shaped and ordered by the text engine; the layout is not mirrored
pub fn rtl(i: usize) -> bool {
  matches!(CODES[i], "ar" | "he")
}

// a locale tag as the os spells it (`de_AT.UTF-8`, `pt-BR`, `zh_Hant_TW`, `nb_NO@euro`) → the nearest language, or
// None when it is none of ours (and for `C`/`POSIX`, which say no language at all)
pub fn map_tag(tag: &str) -> Option<usize> {
  let tag = tag.split(['.', '@']).next().unwrap_or("");
  let lower = tag.to_ascii_lowercase().replace('-', "_");
  let mut parts = lower.split('_').filter(|p| !p.is_empty());
  let lang = parts.next()?;
  let rest: Vec<&str> = parts.collect();
  let region = rest.iter().find(|p| p.len() == 2 || (p.len() == 3 && p.bytes().all(|b| b.is_ascii_digit())));
  let hant = rest.contains(&"hant");
  let code = match lang {
    "pt" => match region.copied() {
      Some("pt" | "ao" | "mz" | "cv" | "gw" | "st" | "tl" | "mo") => "pt_PT",
      _ => "pt_BR",
    },
    "zh" => match region.copied() {
      Some("tw" | "hk" | "mo") => "zh_TW",
      _ if hant => "zh_TW",
      _ => "zh_CN",
    },
    "nb" | "nn" | "no" => "no",
    "fil" => "tl",
    "iw" => "he",
    "in" => "id",
    "c" | "posix" => return None,
    other => return index(other),
  };
  index(code)
}

// the os's own answer: the first of LC_ALL, LC_MESSAGES, LANG that is set and not empty decides (glibc's order). `get` reads a
// variable so a test can hand it any environment. an unmapped answer is english
pub fn from_env(get: impl Fn(&str) -> Option<String>) -> usize {
  for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
    if let Some(v) = get(var).filter(|v| !v.is_empty()) {
      return map_tag(&v).unwrap_or(EN);
    }
  }
  EN
}

#[cfg(unix)]
pub fn os() -> usize {
  from_env(|k| std::env::var(k).ok())
}

// the windows user locale (`de-DE`, `zh-Hans-CN`): GetUserDefaultLocaleName
#[cfg(windows)]
pub fn os() -> usize {
  use windows_sys::Win32::Globalization::GetUserDefaultLocaleName;
  let mut buf = [0u16; 85];
  let n = unsafe { GetUserDefaultLocaleName(buf.as_mut_ptr(), buf.len() as i32) };
  if n <= 1 {
    return EN;
  }
  let tag = String::from_utf16_lossy(&buf[..n as usize - 1]);
  // zh-Hans-CN has a script and a region: map_tag reads the region, `hans` is the default
  map_tag(&tag).unwrap_or(EN)
}

// the plural form a count takes in a language: one of the CLDR names (integers only)
pub fn plural(lang: usize, n: u64) -> &'static str {
  let (m10, m100) = (n % 10, n % 100);
  match CODES[lang] {
    "ja" | "ko" | "zh_CN" | "zh_TW" | "th" | "vi" | "id" | "ms" => "other",
    "fr" | "hi" | "pt_BR" => {
      if n < 2 {
        "one"
      } else {
        "other"
      }
    }
    "ru" | "uk" => {
      if m10 == 1 && m100 != 11 {
        "one"
      } else if (2..=4).contains(&m10) && !(12..=14).contains(&m100) {
        "few"
      } else {
        "many"
      }
    }
    "pl" => {
      if n == 1 {
        "one"
      } else if (2..=4).contains(&m10) && !(12..=14).contains(&m100) {
        "few"
      } else {
        "many"
      }
    }
    "cs" | "sk" => match n {
      1 => "one",
      2..=4 => "few",
      _ => "other",
    },
    "ro" => {
      if n == 1 {
        "one"
      } else if n == 0 || (1..=19).contains(&m100) {
        "few"
      } else {
        "other"
      }
    }
    "ar" => match n {
      0 => "zero",
      1 => "one",
      2 => "two",
      _ if (3..=10).contains(&m100) => "few",
      _ if (11..=99).contains(&m100) => "many",
      _ => "other",
    },
    "he" => match n {
      1 => "one",
      2 => "two",
      _ => "other",
    },
    _ => {
      if n == 1 {
        "one"
      } else {
        "other"
      }
    }
  }
}

// the forms a language needs a string written in, in order (for the completeness test)
pub fn forms(lang: usize) -> &'static [&'static str] {
  match CODES[lang] {
    "ja" | "ko" | "zh_CN" | "zh_TW" | "th" | "vi" | "id" | "ms" => &["other"],
    "ru" | "uk" | "pl" => &["one", "few", "many"],
    "cs" | "sk" | "ro" => &["one", "few", "other"],
    "ar" => &["zero", "one", "two", "few", "many", "other"],
    "he" => &["one", "two", "other"],
    _ => &["one", "other"],
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn code(tag: &str) -> &'static str {
    map_tag(tag).map_or("none", |i| CODES[i])
  }

  #[test]
  fn the_os_tag_maps_to_the_nearest_language() {
    assert_eq!(code("de_AT.UTF-8"), "de");
    assert_eq!(code("pt_BR.UTF-8"), "pt_BR");
    assert_eq!(code("pt"), "pt_BR");
    assert_eq!(code("pt-PT"), "pt_PT");
    assert_eq!(code("pt_AO"), "pt_PT");
    assert_eq!(code("zh_CN.UTF-8"), "zh_CN");
    assert_eq!(code("zh"), "zh_CN");
    assert_eq!(code("zh_TW"), "zh_TW");
    assert_eq!(code("zh_HK.UTF-8"), "zh_TW");
    assert_eq!(code("zh-Hant"), "zh_TW");
    assert_eq!(code("zh-Hant-TW"), "zh_TW");
    assert_eq!(code("zh-Hans-CN"), "zh_CN");
    assert_eq!(code("nb_NO.UTF-8"), "no");
    assert_eq!(code("nn_NO@euro"), "no");
    assert_eq!(code("fil_PH"), "tl");
    assert_eq!(code("iw_IL"), "he");
    assert_eq!(code("ES_mx"), "es");
    assert_eq!(code("sr_RS"), "none");
    assert_eq!(code("C"), "none");
    assert_eq!(code("POSIX"), "none");
    assert_eq!(code(""), "none");
  }

  #[test]
  fn every_code_maps_to_itself() {
    for (i, c) in CODES.iter().enumerate() {
      assert_eq!(map_tag(c), Some(i), "{c}");
      assert_eq!(map_tag(&c.replace('_', "-")), Some(i), "{c}");
    }
    assert_eq!(CODES[EN], "en");
    assert!(CODES.windows(2).all(|w| w[0] < w[1] || w[0].starts_with("pt") || w[0].starts_with("zh")));
  }

  #[test]
  fn the_first_set_variable_decides_and_nothing_is_english() {
    let env = |pairs: &'static [(&'static str, &'static str)]| {
      move |k: &str| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
    };
    assert_eq!(from_env(env(&[("LANG", "fr_FR.UTF-8")])), index("fr").unwrap());
    assert_eq!(from_env(env(&[("LANG", "fr_FR"), ("LC_ALL", "ja_JP")])), index("ja").unwrap());
    assert_eq!(from_env(env(&[("LANG", "fr_FR"), ("LC_MESSAGES", "ko")])), index("ko").unwrap());
    assert_eq!(from_env(env(&[("LC_ALL", ""), ("LANG", "it_IT")])), index("it").unwrap());
    assert_eq!(from_env(env(&[("LC_ALL", "C"), ("LANG", "it_IT")])), EN);
    assert_eq!(from_env(env(&[("LANG", "xx_YY")])), EN);
    assert_eq!(from_env(env(&[])), EN);
  }

  #[test]
  fn counts_take_the_form_their_language_counts_by() {
    let p = |c: &str, n| plural(index(c).unwrap(), n);
    assert_eq!((p("en", 1), p("en", 2), p("en", 0)), ("one", "other", "other"));
    assert_eq!((p("ru", 1), p("ru", 3), p("ru", 5), p("ru", 21), p("ru", 11)), ("one", "few", "many", "one", "many"));
    assert_eq!((p("pl", 1), p("pl", 22), p("pl", 12)), ("one", "few", "many"));
    assert_eq!((p("ja", 1), p("zh_CN", 5)), ("other", "other"));
    assert_eq!(
      (p("ar", 0), p("ar", 2), p("ar", 5), p("ar", 50), p("ar", 100)),
      ("zero", "two", "few", "many", "other")
    );
    assert_eq!((p("fr", 0), p("fr", 1), p("fr", 2)), ("one", "one", "other"));
    for (i, _) in CODES.iter().enumerate() {
      for n in [0, 1, 2, 3, 5, 11, 21, 100, 101] {
        assert!(forms(i).contains(&plural(i, n)) || plural(i, n) == "other", "{} {n}", CODES[i]);
      }
    }
  }
}
