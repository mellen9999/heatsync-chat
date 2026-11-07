// tab-completion for the built-in path prompt: filesystem paths, dirs get a trailing /
use std::path::Path;

use crate::net::upload::expand_as;

const MAX_ENTRIES: usize = 4000;

// (the completed text, the candidates when there are several)
pub fn complete(input: &str, home: Option<&str>) -> (String, Vec<String>) {
  complete_as(input, home, cfg!(windows))
}

// the folder part (up to and with its last separator) and the name being typed. on windows `\` separates too
fn split(input: &str, win: bool) -> (&str, &str) {
  match input.rfind(|c| c == '/' || (win && c == '\\')) {
    Some(i) => (&input[..=i], &input[i + 1..]),
    None => ("", input),
  }
}

// a folder is shown with the separator the person is typing: a backslash path keeps its backslashes
fn sep(dir_part: &str, win: bool) -> char {
  if win && (dir_part.contains('\\') || !dir_part.contains('/')) { '\\' } else { '/' }
}

fn complete_as(input: &str, home: Option<&str>, win: bool) -> (String, Vec<String>) {
  let (dir_part, prefix) = split(input, win);
  let dir = if dir_part.is_empty() { Path::new(".").to_path_buf() } else { expand_as(dir_part, home, win) };
  let end = sep(dir_part, win);
  let Ok(rd) = std::fs::read_dir(&dir) else { return (input.to_owned(), Vec::new()) };
  let mut names: Vec<String> = rd
    .take(MAX_ENTRIES)
    .filter_map(Result::ok)
    .filter_map(|e| {
      let n = e.file_name().into_string().ok()?;
      if !n.starts_with(prefix) || (n.starts_with('.') && !prefix.starts_with('.')) {
        return None;
      }
      let dir = e.path().is_dir();
      Some(if dir { format!("{n}{end}") } else { n })
    })
    .collect();
  names.sort();
  match names.len() {
    0 => (input.to_owned(), Vec::new()),
    1 => (format!("{dir_part}{}", names[0]), Vec::new()),
    _ => (format!("{dir_part}{}", common(&names)), names),
  }
}

fn common(names: &[String]) -> String {
  let first = &names[0];
  let mut end = first.len();
  for n in &names[1..] {
    end = end.min(first.chars().zip(n.chars()).take_while(|(a, b)| a == b).map(|(a, _)| a.len_utf8()).sum());
  }
  first[..end].to_owned()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_windows_path_splits_at_either_slash() {
    assert_eq!(split("C:\\Users\\me\\pi", true), ("C:\\Users\\me\\", "pi"));
    assert_eq!(split("C:/Users\\me/pi", true), ("C:/Users\\me/", "pi"));
    assert_eq!(split("C:\\", true), ("C:\\", ""));
    assert_eq!(split("pi", true), ("", "pi"));
    assert_eq!(split("\\\\nas\\pics\\a", true), ("\\\\nas\\pics\\", "a"));
    // on unix a backslash is part of a name
    assert_eq!(split("/tmp/a\\b", false), ("/tmp/", "a\\b"));
    assert_eq!(split("a\\b", false), ("", "a\\b"));
  }

  #[test]
  fn a_folder_keeps_the_separator_being_typed() {
    assert_eq!(sep("C:\\Users\\", true), '\\');
    assert_eq!(sep("C:/Users/", true), '/');
    assert_eq!(sep("", true), '\\');
    assert_eq!(sep("C:/a\\b/", true), '\\');
    assert_eq!(sep("/tmp/", false), '/');
    assert_eq!(sep("C:\\x\\", false), '/', "unix always");
  }

  #[test]
  fn completes_paths() {
    let d = std::env::temp_dir().join(format!("hsc-pc-{}", std::process::id()));
    std::fs::create_dir_all(d.join("pics")).unwrap();
    std::fs::write(d.join("cat1.png"), b"x").unwrap();
    std::fs::write(d.join("cat2.png"), b"x").unwrap();
    std::fs::write(d.join(".hidden"), b"x").unwrap();
    let base = format!("{}/", d.display());
    assert_eq!(complete(&format!("{base}pi"), None).0, format!("{base}pics/"));
    let (t, many) = complete(&format!("{base}c"), None);
    assert_eq!(t, format!("{base}cat"));
    assert_eq!(many, ["cat1.png", "cat2.png"]);
    assert_eq!(complete(&format!("{base}zz"), None).0, format!("{base}zz"));
    assert!(complete(&base, None).1.iter().all(|n| !n.starts_with('.')));
    assert_eq!(complete(&format!("{base}.h"), None).0, format!("{base}.hidden"));
    let _ = std::fs::remove_dir_all(&d);
  }
}
