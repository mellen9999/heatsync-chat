// what the emote law says of a picker tile (law::mark): the app asks about every tile once, and a tile the room
// answers with another picture is not drawn
use super::{Item, Picker, Sec, Tab};

// what the law says of a tile for the room the box writes to, asked of the app once
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Verdict {
  // not asked yet: drawn plain
  #[default]
  Unjudged,
  // the room answers this tile's name with another picture: there is no such tile
  Hidden,
  // a tile, and the one corner letter for who sees it if sent from here (none = everyone)
  Shown(Option<char>),
}

impl Verdict {
  pub fn mark(self) -> Option<char> {
    match self {
      Verdict::Shown(m) => m,
      _ => None,
    }
  }
}

impl Picker {
  // where a tile sits as the law's offer knows it: yours, the room's, a global, or a search answer. a second
  // tile of a picture that has its own place (a favorite, a recent) sits where that one does; one nothing
  // here holds is a search answer
  fn tier(&self, i: &Item) -> &'static str {
    match i.sec {
      Sec::Mine => "own",
      Sec::Channel => "room",
      Sec::Global => "global",
      Sec::Favorite | Sec::Recent => {
        let own = self.items[..self.n_local].iter().find(|x| !x.sec.again() && x.name == i.name && x.url == i.url);
        own.map_or("catalog", |x| self.tier(x))
      }
      _ => "catalog",
    }
  }

  // the emote tiles nothing has said anything about yet: (index, name, address, tier). the app asks the law
  // about each and answers with `judged`
  pub fn unjudged(&self) -> Vec<(u32, &str, &str, &'static str)> {
    let todo = self.items.iter().enumerate().filter(|(_, i)| i.verdict == Verdict::Unjudged);
    todo.map(|(n, i)| (n as u32, &*i.name, &*i.url, self.tier(i))).collect()
  }

  // the law's answers: a tile the room answers with another picture is gone from the grid, the rest wear
  // their letter. the pick and the scroll stay where they were
  pub fn judged(&mut self, says: &[(u32, Verdict)]) {
    let hid = |items: &[Item]| items.iter().filter(|i| i.verdict == Verdict::Hidden).count();
    let (place, before) = (self.place(), hid(&self.items));
    for &(n, v) in says {
      if let Some(i) = self.items.get_mut(n as usize) {
        i.verdict = v;
      }
    }
    self.rev += 1;
    if self.tab == Tab::Emotes && hid(&self.items) != before {
      self.refilter_to(place);
    }
  }
}
