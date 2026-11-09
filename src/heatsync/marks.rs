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

// `badged`: they show a picked heatsync badge, which takes the flame's place (one heatsync mark per name,
// ext 6bea826e). `known`: we have seen heatsync content from this sender. `mine`: it is your own line, which counts as
// known even before your emote set loads (ext name-marks.js hsMarksWear). `heat`: their score, 0 while unknown
pub fn wear(flame_on: bool, bars_on: bool, known: bool, mine: bool, badged: bool, heat: u32) -> Wear {
  let bar = if bars_on { bar(heat) } else { None };
  Wear { flame: flame_on && !badged && (known || mine) && bar.is_none(), bar }
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
              for badged in [false, true] {
                let w = wear(flame_on, bars_on, known, mine, badged, heat);
                assert!(!(w.flame && w.bar.is_some()), "{flame_on} {bars_on} {known} {mine} {heat}");
                assert!(!(w.flame && badged), "a badge takes the flame's place");
              }
            }
          }
        }
      }
    }
  }

  #[test]
  fn who_wears_what() {
    let w = |f, b, k, m, h| wear(f, b, k, m, false, h);
    assert_eq!(w(true, true, true, false, 3), Wear { flame: true, bar: None }, "known, cold: the flame");
    assert!(!w(true, true, true, false, 7).flame && w(true, true, true, false, 7).bar.is_some(), "warm: the bar");
    assert_eq!(w(true, true, false, false, 0), Wear::default(), "a stranger wears nothing");
    assert!(w(true, true, true, true, 3).flame, "your own line wears it too");
    assert!(w(true, true, false, true, 0).flame, "even before your emote set loads");
    assert!(!w(false, true, false, true, 0).flame, "not with the switch off");
    let warm = w(true, true, true, true, 300);
    assert!(warm.bar.is_some() && !warm.flame, "your bar shows, and then no flame");
    assert!(w(true, false, true, false, 300).flame, "bars off: a known user wears the flame again");
    assert_eq!(w(false, false, true, false, 300), Wear::default(), "both off");
    assert!(!wear(true, true, true, true, true, 0).flame, "a badge steps the flame aside, even on your own line");
    assert!(wear(true, true, true, false, true, 300).bar.is_some(), "a badge leaves the heat bar alone");
  }
}
