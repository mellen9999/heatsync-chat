// tab-completion for the built-in path prompt: filesystem paths, dirs get a trailing /
use std::path::Path;

use crate::net::upload::expand;

const MAX_ENTRIES: usize = 4000;

// (the completed text, the candidates when there are several)
pub fn complete(input: &str, home: Option<&str>) -> (String, Vec<String>) {
  let (dir_part, prefix) = match input.rfind('/') {
    Some(i) => (&input[..=i], &input[i + 1..]),
    None => ("", input),
  };
  let dir = if dir_part.is_empty() { Path::new(".").to_path_buf() } else { expand(dir_part, home) };
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
      Some(if dir { format!("{n}/") } else { n })
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
