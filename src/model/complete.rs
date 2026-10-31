// tab-complete: candidates for the word before the cursor, in the ext's order
// (channel emotes, then your own heatsync inventory, then globals; `@` completes chatters)
const MAX: usize = 200;

pub struct Complete {
  // byte range in the line the candidate replaces
  pub start: usize,
  pub end: usize,
  pub matches: Vec<Box<str>>,
  pub at: usize,
}

// groups are already in priority order; inside one, exact-case prefix first, then shorter, then a-z
pub fn rank<'a>(prefix: &str, groups: impl IntoIterator<Item = Vec<&'a str>>) -> Vec<Box<str>> {
  if prefix.is_empty() {
    return Vec::new();
  }
  let low = prefix.to_lowercase();
  let mut out: Vec<Box<str>> = Vec::new();
  for g in groups {
    let mut hits: Vec<&str> = g.into_iter().filter(|n| n.to_lowercase().starts_with(&low)).collect();
    hits.sort_unstable_by(|a, b| (!a.starts_with(prefix), a.len(), *a).cmp(&(!b.starts_with(prefix), b.len(), *b)));
    for h in hits {
      if out.len() == MAX {
        return out;
      }
      if !out.iter().any(|o| **o == *h) {
        out.push(h.into());
      }
    }
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn order() {
    let channel = vec!["PogU", "pog", "Pogger"];
    let own = vec!["PogBones", "PogU"];
    let global = vec!["PogChamp", "lul"];
    let r = rank("Pog", [channel, own, global]);
    let r: Vec<&str> = r.iter().map(|b| &**b).collect();
    assert_eq!(r, ["PogU", "Pogger", "pog", "PogBones", "PogChamp"]);
    assert!(rank("", [vec!["a"]]).is_empty());
  }
}
