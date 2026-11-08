//! god-files only grow. the 6 biggest source files each have a ceiling in
//! tests/loc-ceilings.txt (line = "<max> <path>  # why"). lines = newline count.
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

const LIST: &str = "tests/loc-ceilings.txt";

fn walk(dir: &Path, root: &Path, out: &mut BTreeMap<String, usize>) {
  for e in fs::read_dir(dir).unwrap().flatten() {
    let p = e.path();
    if p.is_dir() {
      walk(&p, root, out);
    } else if p.extension().is_some_and(|x| x == "rs") && p.file_name().is_some_and(|n| n != "tests.rs") {
      let n = fs::read(&p).unwrap().iter().filter(|&&b| b == b'\n').count();
      out.insert(p.strip_prefix(root).unwrap().to_string_lossy().into(), n);
    }
  }
}

#[test]
fn loc_ceilings() {
  let root = Path::new(env!("CARGO_MANIFEST_DIR"));
  let mut lines = BTreeMap::new();
  walk(&root.join("src"), root, &mut lines);
  let mut ceilings = BTreeMap::new();
  for l in fs::read_to_string(root.join(LIST)).unwrap().lines() {
    let mut w = l.split_whitespace();
    if let (Some(n), Some(p)) = (w.next().and_then(|n| n.parse::<usize>().ok()), w.next()) {
      ceilings.insert(p.to_string(), n);
    }
  }
  let mut bad = vec![];
  if ceilings.len() != 6 {
    bad.push(format!("{LIST} must list exactly 6 files, has {}", ceilings.len()));
  }
  for (p, max) in &ceilings {
    match lines.get(p) {
      None => bad.push(format!("{p}: listed but gone; remove or replace it in {LIST}")),
      Some(n) if n > max => bad.push(format!(
        "{p}: {n} lines > ceiling {max}. split the file, or raise its number in {LIST} in the same commit with a one-line \"# reason\" next to it"
      )),
      Some(n) if (*n as f64) < *max as f64 * 0.9 => {
        println!("hint: {p} is {n} lines, ceiling {max} — lower it in {LIST}")
      }
      _ => {}
    }
  }
  let floor = ceilings.values().copied().min().unwrap_or(0);
  for (p, n) in &lines {
    if !ceilings.contains_key(p) && *n > floor {
      bad.push(format!(
        "{p}: {n} lines > smallest ceiling {floor}. a new god-file: split it, or add \"{n} {p}  # reason\" to {LIST} (and drop the smallest entry)"
      ));
    }
  }
  assert!(bad.is_empty(), "\n{}", bad.join("\n"));
}
