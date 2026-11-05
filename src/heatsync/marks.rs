// what a twitch line wears before the name besides badges, in the contract's order: the sub flair (a
// badge, drawn by the layout), then 🔥 for a known heatsync user, then the heat bar.
// the two heat marks never sit together: someone whose bar shows is plainly on heatsync, so the 🔥 steps
// aside (never a 🔥 on or beside a heat score). a 🔥 is only for those whose heat is too low for a bar
use super::heat::{Bar, bar};

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Wear {
  pub flame: bool,
  pub bar: Option<Bar>,
}

// `known`: we have seen heatsync content from this sender. `mine`: it is your own line (you know it is
// you, so no 🔥, like the chatterino plugin). `heat`: their score, 0 while unknown
pub fn wear(flame_on: bool, bars_on: bool, known: bool, mine: bool, heat: u32) -> Wear {
  let bar = if bars_on { bar(heat) } else { None };
  Wear { flame: flame_on && known && !mine && bar.is_none(), bar }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_flame_never_sits_with_a_bar() {
    for flame_on in [false, true] {
      for bars_on in [false, true] {
        for known in [false, true] {
          for mine in [false, true] {
            for heat in [0, 4, 5, 49, 50, 999, 1000, 5000] {
              let w = wear(flame_on, bars_on, known, mine, heat);
              assert!(!(w.flame && w.bar.is_some()), "{flame_on} {bars_on} {known} {mine} {heat}");
            }
          }
        }
      }
    }
  }

  #[test]
  fn who_wears_what() {
    let w = |f, b, k, m, h| wear(f, b, k, m, h);
    assert_eq!(w(true, true, true, false, 3), Wear { flame: true, bar: None }, "known, cold: the flame");
    assert!(!w(true, true, true, false, 7).flame && w(true, true, true, false, 7).bar.is_some(), "warm: the bar");
    assert_eq!(w(true, true, false, false, 0), Wear::default(), "a stranger wears nothing");
    assert!(!w(true, true, true, true, 3).flame, "no flame on your own line");
    assert!(w(true, true, true, true, 300).bar.is_some(), "but your bar shows");
    assert!(w(true, false, true, false, 300).flame, "bars off: a known user wears the flame again");
    assert_eq!(w(false, false, true, false, 300), Wear::default(), "both off");
  }
}
