// the font lookup on unix: asked of fontconfig's own `fc-match` (the tool every desktop with fonts has). run the way the clipboard tools are (util::spawn): an argument list and never a shell, a
// deadline, a cap on what it may print. it blocks, so callers run it off the ui thread (see net/font.rs).
// fc-match never says no: for a name it doesn't have it hands back the nearest font, so the answer is only
// taken when the font it names is the one that was asked for
use std::time::Duration;

use super::{Found, MAX_NAME};
use crate::util::spawn::{Fail, Tool, run};

// a cold font cache can take a few seconds to build the first time
const WAIT: Duration = Duration::from_secs(5);
const MAX_OUT: usize = 4 << 10;
const PKG: &str = "fontconfig";

#[derive(Debug, PartialEq, Eq)]
pub enum Miss {
  // fc-match isn't installed
  NoTool,
  Slow,
  // no such family; fontconfig's nearest is named
  Absent(String),
  // an answer that isn't the shape fc-match gives
  Odd,
}

impl Miss {
  // one plain line for the status bar
  pub fn said(&self, asked: &str) -> String {
    match self {
      Miss::NoTool => format!("looking up fonts needs fc-match: paru -S {PKG}"),
      Miss::Slow => "the font lookup didn't answer: try again".to_owned(),
      Miss::Absent(near) => format!("no font family called {asked} is installed (the closest is {near})"),
      Miss::Odd => "the font lookup gave an answer i can't read".to_owned(),
    }
  }
}

// a family name as a fontconfig pattern: its four separators escaped, so the whole name is one family (and
// can never start like an option)
fn pattern(name: &str) -> String {
  let mut out = String::with_capacity(name.len() + 4);
  for c in name.chars() {
    if matches!(c, '\\' | '-' | ':' | ',') {
      out.push('\\');
    }
    out.push(c);
  }
  out
}

// file, face number, first family name, every family name
const FORMAT: &str = "%{file}\t%{index}\t%{family[0]}\t%{family}\n";

fn tool(name: &str) -> Tool {
  Tool::new("fc-match", &["-f", FORMAT, &pattern(name)])
}

// what fc-match printed for `asked`: the font, if it is that family; else its refusal
fn parse(out: &str, asked: &str) -> Result<Found, Miss> {
  let line = out.trim_end_matches('\n');
  let mut f = line.split('\t');
  let (Some(file), Some(index), Some(first), Some(all), None) = (f.next(), f.next(), f.next(), f.next(), f.next())
  else {
    return Err(Miss::Odd);
  };
  let path = std::path::Path::new(file);
  if !path.is_absolute() || file.chars().any(char::is_control) || index.parse::<u32>().is_err() || first.is_empty() {
    return Err(Miss::Odd);
  }
  let want = asked.trim().to_lowercase();
  // the family list is comma-joined: the asked name may be any of the font's names
  if first.to_lowercase() == want || all.split(',').any(|n| n.trim().to_lowercase() == want) {
    // the family is kept as a setting afterwards: one that couldn't be (too long, a control character) is no answer
    if first.len() > MAX_NAME || first.chars().any(char::is_control) {
      return Err(Miss::Odd);
    }
    Ok(Found { family: first.to_owned(), file: Some(path.to_owned()) })
  } else {
    Err(Miss::Absent(first.chars().filter(|c| !c.is_control()).take(MAX_NAME).collect()))
  }
}

// blocks for up to a few seconds; the answer or one plain sentence for the status bar
pub fn find(name: &str) -> Result<Found, String> {
  lookup(name).map_err(|m| m.said(name))
}

fn lookup(name: &str) -> Result<Found, Miss> {
  let ran = run(&tool(name), None, Some(MAX_OUT), WAIT, PKG).map_err(|e| match e {
    Fail::Missing(_) => Miss::NoTool,
    Fail::Timeout => Miss::Slow,
    Fail::TooBig(_) | Fail::Tool(_) => Miss::Odd,
  })?;
  if !ran.ok {
    return Err(Miss::Odd);
  }
  parse(&String::from_utf8_lossy(&ran.out), name)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn separators_are_escaped_so_a_name_stays_one_family() {
    assert_eq!(pattern("Fira Code"), "Fira Code");
    assert_eq!(pattern("Terminus (TTF)"), "Terminus (TTF)");
    assert_eq!(pattern("Foo-Bar"), "Foo\\-Bar", "a hyphen would start a size");
    assert_eq!(pattern("a,b:c\\d"), "a\\,b\\:c\\\\d");
    assert_eq!(pattern("-o"), "\\-o", "never looks like an option");
    assert_eq!(pattern("日本語 Mono"), "日本語 Mono");
  }

  #[test]
  fn the_command_is_an_argument_list_with_the_name_as_one_argument() {
    let t = tool("x; rm -rf ~ $(id)");
    assert_eq!(t.prog, "fc-match");
    assert_eq!(t.args, ["-f", FORMAT, "x; rm \\-rf ~ $(id)"]);
    assert_eq!(t.args.len(), 3);
  }

  #[cfg(unix)]
  const OUT: &str = "/usr/share/fonts/TTF/DejaVuSansMono.ttf\t0\tDejaVu Sans Mono\tDejaVu Sans Mono\n";

  #[cfg(unix)]
  #[test]
  fn the_font_asked_for_is_found_whatever_the_case() {
    let want =
      Found { family: "DejaVu Sans Mono".into(), file: Some("/usr/share/fonts/TTF/DejaVuSansMono.ttf".into()) };
    assert_eq!(parse(OUT, "DejaVu Sans Mono"), Ok(want.clone()));
    assert_eq!(parse(OUT, "dejavu sans mono"), Ok(want.clone()));
    assert_eq!(parse(OUT, "  DEJAVU SANS MONO "), Ok(want));
  }

  #[cfg(unix)]
  #[test]
  fn any_of_a_fonts_names_finds_it_under_its_first() {
    let out = "/usr/share/fonts/a.ttc\t2\tNoto Sans Mono CJK JP\tNoto Sans Mono CJK JP,Noto Sans Mono CJK JP Regular\n";
    let found = parse(out, "noto sans mono cjk jp regular").unwrap();
    assert_eq!(found.family, "Noto Sans Mono CJK JP", "named the way the font files name it");
  }

  #[cfg(unix)]
  #[test]
  fn another_font_handed_back_is_a_miss_that_names_it() {
    // fc-match for a name it hasn't got: its default
    let out = "/usr/share/fonts/noto/NotoSans-Regular.ttf\t0\tNoto Sans\tNoto Sans\n";
    assert_eq!(parse(out, "Fira Code"), Err(Miss::Absent("Noto Sans".into())));
    assert_eq!(parse(out, "Noto"), Err(Miss::Absent("Noto Sans".into())), "a part of a name is not the name");
    assert_eq!(parse(out, "Courier"), Err(Miss::Absent("Noto Sans".into())));
    assert_eq!(
      Miss::Absent("Noto Sans".into()).said("Fira Code"),
      "no font family called Fira Code is installed (the closest is Noto Sans)"
    );
  }

  #[test]
  fn an_answer_of_the_wrong_shape_is_odd() {
    for bad in [
      "",
      "\n",
      "nonsense",
      "a\tb\tc",
      "relative/font.ttf\t0\tX\tX\n",
      "/f.ttf\tzero\tX\tX\n",
      "/f.ttf\t-1\tX\tX\n",
      "/f.ttf\t0\t\tX\n",
      "/f\u{7}.ttf\t0\tX\tX\n",
      "/f.ttf\t0\tX\tX\textra\n",
    ] {
      assert_eq!(parse(bad, "X"), Err(Miss::Odd), "{bad:?}");
    }
  }

  #[cfg(unix)]
  #[test]
  fn a_hostile_nearest_name_is_cleaned_and_cut() {
    let out = format!("/f.ttf\t0\t{}\tz\n", "A\u{1b}[31m".repeat(100));
    let Err(Miss::Absent(n)) = parse(&out, "q") else { panic!("a miss") };
    assert!(n.chars().count() <= MAX_NAME && !n.chars().any(char::is_control), "{n:?}");
  }

  #[cfg(unix)]
  #[test]
  fn a_name_that_could_not_be_kept_is_no_answer() {
    assert_eq!(parse("/f.ttf\t0\tX\u{1b}[0m\tX\u{1b}[0m\n", "x\u{1b}[0m"), Err(Miss::Odd));
    let long = "a".repeat(MAX_NAME + 1);
    assert_eq!(parse(&format!("/f.ttf\t0\t{long}\t{long}\n"), &long), Err(Miss::Odd));
    let fits = "a".repeat(MAX_NAME);
    assert!(parse(&format!("/f.ttf\t0\t{fits}\t{fits}\n"), &fits).is_ok());
  }

  #[test]
  fn every_miss_says_something_plain() {
    for m in [Miss::NoTool, Miss::Slow, Miss::Absent("x".into()), Miss::Odd] {
      let s = m.said("y");
      assert!(!s.is_empty() && !s.ends_with('.') && !s.chars().any(char::is_control), "{s}");
    }
    assert!(Miss::NoTool.said("x").ends_with("paru -S fontconfig"));
  }

  // asks the real fc-match, skipped where there is none: the font behind `monospace` is found by its own
  // name, and a name nobody has is a miss
  #[test]
  fn live_fc_match_finds_an_installed_family_and_misses_a_made_up_one() {
    let Ok(ran) = run(&Tool::new("fc-match", &["-f", "%{family[0]}", "monospace"]), None, Some(256), WAIT, PKG) else {
      return;
    };
    let family = String::from_utf8_lossy(&ran.out).into_owned();
    if family.is_empty() {
      return;
    }
    let found = find(&family).unwrap_or_else(|e| panic!("{family}: {e:?}"));
    assert_eq!(found.family, family);
    assert!(found.file.as_deref().is_some_and(std::path::Path::is_file), "{:?}", found.file);
    assert!(matches!(lookup("hsc no such family 9"), Err(Miss::Absent(_))));
    assert!(
      matches!(lookup("hsc-no-such, family: -x"), Err(Miss::Absent(_))),
      "separators and a dash are just letters"
    );
  }
}
