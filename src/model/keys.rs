// every key the app answers, in plain words, grouped for the `?` screen. the handlers live in
// app/keys.rs (global_key, key_insert, key_normal, pick_key, leader_key) and the layer files beside it
// (picker, img_view, rover, logs, inventory, the screens, the menus); when a binding changes there, change
// its row here. `k` rows work with plain input, `v` rows only with vi mode on.
// the tests keep the list well-formed, free of repeats, and tied to the handlers' source
// one line of the map: the keys, what they do, and whether they need vi mode (the vi keys, normal mode)
pub struct Row {
  pub keys: &'static str,
  // the i18n key of the text, shown through `t`
  pub does: &'static str,
  pub vi: bool,
}

pub struct Group {
  // the i18n key of the heading, shown through `t`
  pub name: &'static str,
  pub rows: &'static [Row],
}

impl Group {
  // every row needs vi mode: the heading says so once instead of every row
  pub fn all_vi(&self) -> bool {
    self.rows.iter().all(|r| r.vi)
  }
}

// works with plain input
const fn k(keys: &'static str, does: &'static str) -> Row {
  Row { keys, does, vi: false }
}

// works only with vi mode on (/vi), in normal mode: esc leaves typing first
const fn v(keys: &'static str, does: &'static str) -> Row {
  Row { keys, does, vi: true }
}

pub static KEYS: [Group; 19] = [
  Group {
    name: "keys.anywhere.name",
    rows: &[
      k("f1", "keys.anywhere.01"),
      k("ctrl-,", "keys.anywhere.02"),
      k("/reset", "keys.anywhere.03"),
      k("ctrl-t", "keys.anywhere.04"),
      k("ctrl-l", "keys.anywhere.05"),
      k("ctrl-shift-l", "keys.anywhere.06"),
      k("/login /logout", "keys.anywhere.07"),
      k("/account", "keys.anywhere.08"),
      k("ctrl-b", "keys.anywhere.09"),
      k("/import", "keys.anywhere.10"),
      k("ctrl-p", "keys.anywhere.11"),
      k("ctrl-o", "keys.anywhere.12"),
      k("ctrl-;", "keys.anywhere.13"),
      k("ctrl-v  shift-insert", "keys.anywhere.14"),
      k("ctrl-= ctrl-- ctrl-0", "keys.anywhere.15"),
      k("ctrl-wheel", "keys.anywhere.16"),
      k("pageup pagedown", "keys.anywhere.17"),
      v("?", "keys.anywhere.18"),
    ],
  },
  Group {
    name: "keys.chat.name",
    rows: &[
      k("click", "keys.chat.01"),
      k("right-click", "keys.chat.02"),
      k("click  ctrl-f", "keys.chat.03"),
      k("ctrl-g", "keys.chat.04"),
      v("ctrl-d ctrl-u", "keys.chat.05"),
      v("space g  space G", "keys.chat.06"),
      v("space r  space R", "keys.chat.07"),
      v("v", "keys.chat.08"),
      v("/  space /", "keys.chat.09"),
      v("n  N  space n  space N", "keys.chat.10"),
      v("space x", "keys.chat.11"),
      v("space p", "keys.chat.12"),
      v("space t", "keys.chat.13"),
      v("space w", "keys.chat.14"),
      v("space s", "keys.chat.15"),
      v("ctrl-w", "keys.chat.16"),
    ],
  },
  Group {
    name: "keys.picked_line.name",
    rows: &[
      v("j k  up down", "keys.picked_line.01"),
      v("r", "keys.picked_line.02"),
      v("t", "keys.picked_line.03"),
      v("m", "keys.picked_line.04"),
      v("c", "keys.picked_line.05"),
      v("o", "keys.picked_line.06"),
      v("ctrl-c", "keys.picked_line.07"),
      v("enter", "keys.picked_line.08"),
      v("u", "keys.picked_line.09"),
      v("x", "keys.picked_line.10"),
      v("h backspace", "keys.picked_line.11"),
      v("v esc", "keys.picked_line.12"),
    ],
  },
  Group {
    name: "keys.chat_logs.name",
    rows: &[
      k("j k  up down  click", "keys.chat_logs.01"),
      k("g G  wheel pageup", "keys.chat_logs.02"),
      k("/", "keys.chat_logs.03"),
      k("a", "keys.chat_logs.04"),
      k("y", "keys.chat_logs.05"),
      k("r", "keys.chat_logs.06"),
      k("esc q h backspace", "keys.chat_logs.07"),
      v("c", "keys.chat_logs.08"),
    ],
  },
  Group {
    name: "keys.typing.name",
    rows: &[
      k("enter", "keys.typing.01"),
      k("tab  shift-tab", "keys.typing.02"),
      k(":ka  @  /", "keys.typing.03"),
      k(":fire:", "keys.typing.04"),
      k("up down", "keys.typing.05"),
      k("ctrl-z  ctrl-y  ctrl-shift-z", "keys.typing.06"),
      k("click", "keys.typing.07"),
      k("ctrl-n", "keys.typing.08"),
      k("alt-t alt-k alt-y", "keys.typing.09"),
      k("ctrl-a ctrl-e  home end", "keys.typing.10"),
      k("ctrl-left ctrl-right", "keys.typing.11"),
      k("ctrl-w  ctrl-backspace", "keys.typing.12"),
      k("backspace  delete", "keys.typing.13"),
      k("ctrl-u ctrl-k", "keys.typing.14"),
      k("esc", "keys.typing.15"),
    ],
  },
  Group {
    name: "keys.editing.name",
    rows: &[
      v("h l w b e", "keys.editing.01"),
      v("0 ^ $", "keys.editing.02"),
      v("f t F T", "keys.editing.03"),
      v("x X D", "keys.editing.04"),
      v("d c y", "keys.editing.05"),
      v("p  P", "keys.editing.06"),
      v("u  ctrl-r", "keys.editing.07"),
      v("i a I A", "keys.editing.08"),
      v("j k", "keys.editing.09"),
      v("enter", "keys.editing.10"),
      v("esc", "keys.editing.11"),
    ],
  },
  Group {
    name: "keys.picker.name",
    rows: &[
      k("h j k l  arrows", "keys.picker.01"),
      k("enter", "keys.picker.02"),
      k("/ i  tab", "keys.picker.03"),
      k("] [  ctrl-tab", "keys.picker.04"),
      k("alt-1 alt-2 alt-3 alt-4", "keys.picker.05"),
      k("alt-u", "keys.picker.06"),
      k("g G", "keys.picker.07"),
      k("ctrl-d ctrl-u  pagedown pageup", "keys.picker.08"),
      k("backspace  ctrl-w", "keys.picker.09"),
      k("esc q ctrl-;", "keys.picker.10"),
    ],
  },
  Group {
    name: "keys.emotes.name",
    rows: &[
      k("j k  up down", "keys.emotes.01"),
      k("] [  tab", "keys.emotes.02"),
      k("n  enter", "keys.emotes.03"),
      k("m", "keys.emotes.04"),
      k("J K", "keys.emotes.05"),
      k("x", "keys.emotes.06"),
      k("y", "keys.emotes.07"),
      k("u  ctrl-r", "keys.emotes.08"),
      k("s S", "keys.emotes.09"),
      k("i", "keys.emotes.10"),
      k("/  R", "keys.emotes.11"),
      k("esc q", "keys.emotes.12"),
    ],
  },
  Group {
    name: "keys.tabs.name",
    rows: &[
      k("alt-1 .. alt-0", "keys.tabs.01"),
      k("ctrl-tab", "keys.tabs.02"),
      k("alt-[ alt-]", "keys.tabs.03"),
      k("alt-r", "keys.tabs.04"),
      k("click", "keys.tabs.05"),
      k("right-click", "keys.tabs.06"),
      k("edit form", "keys.tabs.07"),
      v("gt  gT  space ] [", "keys.tabs.08"),
      v("space 1 .. space 9", "keys.tabs.09"),
    ],
  },
  Group {
    name: "keys.mod.name",
    rows: &[
      k("/ban /timeout /unban", "keys.mod.01"),
      k("/delete /purge", "keys.mod.02"),
      k("/confirmban", "keys.mod.03"),
      k("/modbuttons", "keys.mod.04"),
      k("allow  deny", "keys.mod.05"),
      k("/blocked", "keys.mod.06"),
      k("/warn /shoutout /shield", "keys.mod.07"),
      k("/modtools", "keys.mod.08"),
      k("/status", "keys.mod.09"),
      k("/slow /followers", "keys.mod.10"),
      k("/emoteonly /subscribers /unique", "keys.mod.11"),
      v("m", "keys.mod.12"),
    ],
  },
  Group {
    name: "keys.tab_mode.name",
    rows: &[
      v("j k  up down", "keys.tab_mode.01"),
      v("g G  home end", "keys.tab_mode.02"),
      v("enter l right", "keys.tab_mode.03"),
      v("J K", "keys.tab_mode.04"),
      v("d x", "keys.tab_mode.05"),
      v("a o", "keys.tab_mode.06"),
      v("esc q t", "keys.tab_mode.07"),
    ],
  },
  Group {
    name: "keys.picture.name",
    rows: &[
      k("click", "keys.picture.01"),
      k("click ▶", "keys.picture.02"),
      k("+ - 0", "keys.picture.03"),
      k("h j k l  arrows", "keys.picture.04"),
      k("pageup pagedown space", "keys.picture.05"),
      k("esc q", "keys.picture.06"),
    ],
  },
  Group {
    name: "keys.settings.name",
    rows: &[
      k("j k  up down", "keys.settings.01"),
      k("ctrl-d ctrl-u  pageup pagedown", "keys.settings.02"),
      k("gg G  home end", "keys.settings.03"),
      k("/", "keys.settings.04"),
      k("enter space", "keys.settings.05"),
      k("h l  left right", "keys.settings.06"),
      k("y", "keys.settings.07"),
      k("?", "keys.settings.08"),
      k("esc q", "keys.settings.09"),
    ],
  },
  Group {
    name: "keys.followed.name",
    rows: &[
      k("j k  up down", "keys.followed.01"),
      k("enter", "keys.followed.02"),
      k("w", "keys.followed.03"),
      k("r", "keys.followed.04"),
      k("/", "keys.followed.05"),
      k("esc q", "keys.followed.06"),
    ],
  },
  Group {
    name: "keys.import.name",
    rows: &[
      k("j k  up down", "keys.import.01"),
      k("space", "keys.import.02"),
      k("s", "keys.import.03"),
      k("enter", "keys.import.04"),
      k("r", "keys.import.05"),
      k("/", "keys.import.06"),
      k("esc q", "keys.import.07"),
    ],
  },
  Group {
    name: "keys.mod_tools.name",
    rows: &[
      k("j k  up down", "keys.mod_tools.01"),
      k("enter space", "keys.mod_tools.02"),
      k("h l  left right", "keys.mod_tools.03"),
      k("y x", "keys.mod_tools.04"),
      k("r", "keys.mod_tools.05"),
      k("/", "keys.mod_tools.06"),
      k("esc q", "keys.mod_tools.07"),
    ],
  },
  Group {
    name: "keys.menus.name",
    rows: &[
      k("j k  up down", "keys.menus.01"),
      k("enter", "keys.menus.02"),
      k("esc", "keys.menus.03"),
      k("1 .. 9", "keys.menus.04"),
      k("y", "keys.menus.05"),
      k("q", "keys.menus.06"),
      k("c", "keys.menus.07"),
    ],
  },
  Group {
    name: "keys.lists.name",
    rows: &[
      k("j k  up down", "keys.lists.01"),
      k("enter", "keys.lists.02"),
      k("x delete", "keys.lists.03"),
      k("a", "keys.lists.04"),
      k("e  n  s  +  -  =", "keys.lists.05"),
      k("?", "keys.lists.06"),
      k("esc q", "keys.lists.07"),
    ],
  },
  Group {
    name: "keys.marks.name",
    rows: &[
      k("new here", "keys.marks.01"),
      k("returning", "keys.marks.02"),
      k("first message", "keys.marks.03"),
      k("shared:name", "keys.marks.04"),
      k("fx:name", "keys.marks.05"),
      k("n/limit", "keys.marks.06"),
    ],
  },
];

#[cfg(test)]
mod tests {
  use super::*;
  use crate::i18n::t;
  use std::collections::HashSet;
  use std::path::Path;

  // the mouse, the buttons and the typed triggers are not keys: no handler to find, and they may repeat
  const NOT_KEYS: [&str; 14] = [
    "click",
    "right-click",
    "wheel",
    "ctrl-wheel",
    "arrows",
    "▶",
    "←",
    "back",
    "..",
    ":ka",
    "@",
    ":fire:",
    "allow",
    "deny",
  ];

  // where a group's keys are answered. one key may mean different things in different layers, and in one
  // layer once with vi keys and once without (ctrl-w closes a tab in vi normal mode, deletes a word typing)
  fn layer(g: &Group) -> &'static str {
    match t(g.name) {
      n if n.starts_with("chat logs") => "logs",
      "a picked line" => "picked",
      "picker" => "picker",
      "tab mode (space t)" => "tab mode",
      "picture" => "picture",
      "your emotes (ctrl-p)" => "emotes",
      "menus and cards" => "menus",
      n if n.starts_with("your lists") => "lists",
      n if n.starts_with("little marks") => "marks",
      "settings (ctrl-,)" => "settings",
      "followed (ctrl-b)" => "followed",
      "import (/import)" => "import",
      "mod tools (/modtools)" => "mod tools",
      _ => "chat",
    }
  }

  fn src(file: &str) -> String {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(file);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
  }

  // the files a layer's keys live in (a screen's own file, then the shared screen keys beside it)
  fn files(layer: &str) -> &'static [&'static str] {
    match layer {
      "logs" => &["app/logs.rs", "app/keys.rs"],
      "picked" => &["app/keys.rs", "app/whispers.rs"],
      "picker" => &["app/picker.rs"],
      "tab mode" => &["app/rover.rs"],
      "picture" => &["app/img_view.rs"],
      "emotes" => &["app/inventory.rs", "app/screen.rs"],
      "menus" => &["app/pop.rs", "app/card.rs", "app/tabs.rs", "app/emote.rs"],
      "settings" => &["app/screen.rs"],
      "followed" => &["app/following.rs", "app/screen.rs"],
      "import" => &["app/import.rs", "app/screen.rs"],
      "mod tools" => &["app/modtools.rs", "app/screen.rs"],
      "lists" => &["app/lists.rs", "app/screen.rs"],
      // marks you read, not keys you press
      "marks" => &[],
      // the box and the chat: the key handlers, the vi engine, zoom, paste, the find bar and the lists
      _ => &[
        "app/keys.rs",
        "model/vi.rs",
        "app/zoom.rs",
        "app/clip.rs",
        "app/find.rs",
        "app/suggest.rs",
        "app/complete.rs",
        "app/rover.rs",
      ],
    }
  }

  // a handler file without its tests (their key presses are not handlers)
  fn code(file: &str) -> String {
    let s = src(file);
    match s.find("#[cfg(test)]") {
      Some(i) => s[..i].to_owned(),
      None => s,
    }
  }

  fn handlers(layer: &str) -> String {
    files(layer).iter().map(|f| code(f)).collect()
  }

  // a row's keys one by one: blanks split alternatives, a `space` leader joins the key after it
  fn tokens(keys: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut it = keys.split_whitespace();
    while let Some(t) = it.next() {
      if t == "space"
        && let Some(next) = it.next()
      {
        out.push(format!("space {next}"));
      } else {
        out.push(t.to_owned());
      }
    }
    out.retain(|t| !NOT_KEYS.contains(&t.as_str()));
    out
  }

  // the key as a handler writes it: a named key's variant, a letter's quoted literal
  fn handled(src: &str, key: &str) -> bool {
    let named = |n: &str| src.contains(&format!("NamedKey::{n}")) || src.contains(&format!("\"{n}\""));
    let quoted = |c: char| src.contains(&format!("\"{c}\"")) || src.contains(&format!("'{c}'"));
    let key = key.strip_prefix("space ").unwrap_or(key);
    let base = match key.strip_prefix("ctrl-shift-") {
      Some(k) => k.to_uppercase(),
      None => ["ctrl-", "alt-", "shift-"].iter().find_map(|p| key.strip_prefix(p)).unwrap_or(key).to_owned(),
    };
    match base.as_str() {
      "enter" => named("Enter"),
      "esc" => named("Escape") || named("Esc"),
      "tab" => named("Tab"),
      "up" => named("ArrowUp"),
      "down" => named("ArrowDown"),
      "left" => named("ArrowLeft"),
      "right" => named("ArrowRight"),
      "pageup" => named("PageUp"),
      "pagedown" => named("PageDown"),
      "f1" => named("F1"),
      "backspace" => named("Backspace"),
      "delete" => named("Delete"),
      "home" => named("Home"),
      "end" => named("End"),
      "insert" => named("Insert"),
      "space" => named("Space"),
      d if d.len() == 1 && d.as_bytes()[0].is_ascii_digit() => {
        src.contains("is_ascii_digit") || quoted(d.as_bytes()[0] as char)
      }
      t => t.chars().all(quoted),
    }
  }

  #[test]
  fn every_group_is_named_and_lowercase_with_no_empty_cells() {
    let mut names = HashSet::new();
    for g in &KEYS {
      assert!(!t(g.name).is_empty() && !g.rows.is_empty());
      assert_eq!(t(g.name), t(g.name).to_lowercase());
      assert!(names.insert(t(g.name)), "{} twice", t(g.name));
      for r in g.rows {
        assert!(!r.keys.is_empty() && !t(r.does).is_empty(), "{}", r.keys);
        assert_eq!(t(r.does), t(r.does).to_lowercase(), "{}", t(r.does));
      }
    }
  }

  #[test]
  fn no_key_is_listed_twice_in_one_place() {
    let mut seen = HashSet::new();
    for g in KEYS.iter().filter(|g| layer(g) != "marks") {
      for r in g.rows {
        for t in tokens(r.keys) {
          let place = (layer(g), r.vi);
          let mode = if r.vi { "vi" } else { "plain" };
          assert!(seen.insert((place, t.clone())), "{t} is listed twice in {} ({mode})", place.0);
        }
      }
    }
  }

  #[test]
  fn every_listed_key_is_in_the_handler_that_answers_it() {
    let slash: HashSet<&str> = crate::model::slash::all().iter().map(|s| s.name).collect();
    for g in &KEYS {
      if layer(g) == "marks" {
        continue;
      }
      let (code, gname) = (handlers(layer(g)), t(g.name));
      for r in g.rows {
        for t in tokens(r.keys) {
          let ok = if t.len() > 1 && t.starts_with('/') { slash.contains(t.as_str()) } else { handled(&code, &t) };
          assert!(ok, "{t} ({}) has no handler in the {} layer", gname, layer(g));
        }
      }
    }
  }

  // every `Key::Character("x" | "y")` letter and `NamedKey::Foo` a handler file matches on
  fn matched(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = src;
    while let Some(i) = rest.find("Key::Character(") {
      rest = &rest[i + "Key::Character(".len()..];
      let end = rest.find(')').unwrap_or(rest.len());
      // `c @ ("t" | "k")` has its own parens: take up to the one that closes the pattern
      let span =
        if rest[..end].contains('(') { &rest[..rest[end..].find(')').map_or(end, |j| end + j)] } else { &rest[..end] };
      out.extend(span.split('"').skip(1).step_by(2).map(str::to_owned));
    }
    let mut rest = src;
    while let Some(i) = rest.find("NamedKey::") {
      rest = &rest[i + "NamedKey::".len()..];
      let n: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
      out.push(n);
    }
    out.sort();
    out.dedup();
    out
  }

  // the key as a row words it: the letters and names its tokens spell, modifiers dropped
  fn documented(layer: &str) -> HashSet<String> {
    let mut words = HashSet::new();
    for g in KEYS.iter().filter(|g| self::layer(g) == layer) {
      for r in g.rows {
        let mut it = r.keys.split_whitespace();
        while let Some(t) = it.next() {
          if t == "space" {
            words.insert("space".to_owned());
          }
          let t = if t == "space" { it.next().unwrap_or(t) } else { t };
          // ctrl-shift-l is the capital L; every other modifier leaves the key as written
          let base = match t.strip_prefix("ctrl-shift-") {
            Some(k) => k.to_uppercase(),
            None => ["ctrl-", "alt-", "shift-"].iter().find_map(|p| t.strip_prefix(p)).unwrap_or(t).to_owned(),
          };
          words.insert(base.to_lowercase());
          words.extend(base.chars().map(|c| c.to_string()));
        }
      }
    }
    words
  }

  // keys a handler matches that are not the user's to learn: a letter typed into a box, a repeat of
  // another arm, a guard. (file, key) pairs, each with why
  const INTERNAL: [(&str, &str); 7] = [
    // the shifted twins of keys that have rows (ctrl-v / ctrl-shift-v, ctrl-+ / ctrl-=, ctrl-_ / ctrl--, + / =)
    ("app/clip.rs", "V"),
    ("app/zoom.rs", "+"),
    ("app/zoom.rs", "_"),
    ("app/img_view.rs", "="),
    // a space typed into the picker's search
    ("app/picker.rs", "Space"),
    // a space typed into the emotes screen's rename / move box
    ("app/inventory.rs", "Space"),
    // a space typed into a list screen's add box
    ("app/lists.rs", "Space"),
  ];

  fn named_is_documented(n: &str, words: &HashSet<String>) -> bool {
    let w = |k: &str| words.contains(k);
    match n {
      "Enter" => w("enter"),
      "Escape" => w("esc"),
      "Tab" => w("tab"),
      "Space" => w("space"),
      "Backspace" => w("backspace"),
      "Delete" => w("delete"),
      "Home" => w("home"),
      "End" => w("end"),
      "PageUp" => w("pageup"),
      "PageDown" => w("pagedown"),
      "Insert" => w("insert"),
      "F1" => w("f1"),
      "ArrowUp" => w("up") || w("arrows"),
      "ArrowDown" => w("down") || w("arrows"),
      "ArrowLeft" => w("left") || w("arrows"),
      "ArrowRight" => w("right") || w("arrows"),
      _ => true,
    }
  }

  #[test]
  fn every_key_a_handler_answers_has_a_row() {
    let mut layers: Vec<&str> = KEYS.iter().map(layer).collect();
    layers.sort();
    layers.dedup();
    let mut seen: Vec<&str> = layers.iter().flat_map(|l| files(l).iter().copied()).collect();
    seen.sort();
    seen.dedup();
    let mut missing = Vec::new();
    for f in seen {
      // a file answers for every layer that lists it: the shared screen keys serve all the screens
      let words: HashSet<String> =
        layers.iter().filter(|l| files(l).contains(&f)).flat_map(|l| documented(l)).collect();
      for key in matched(&code(f)) {
        let named = key.len() > 1 && key.chars().next().is_some_and(|c| c.is_ascii_uppercase());
        let ok = if named { named_is_documented(&key, &words) } else { words.contains(&key) };
        if !ok && !INTERNAL.contains(&(f, key.as_str())) {
          missing.push(format!("{key} in {f}"));
        }
      }
    }
    assert!(missing.is_empty(), "keys with no row on the ? screen:\n{}", missing.join("\n"));
  }

  // the marks a person sees on a line are explained on the ? screen, in one group of their own
  #[test]
  fn the_key_map_explains_the_chips_and_marks() {
    let g = KEYS.iter().find(|g| t(g.name).starts_with("little marks")).expect("a group for the marks");
    let keys: Vec<&str> = g.rows.iter().map(|r| r.keys).collect();
    assert_eq!(keys, ["new here", "returning", "first message", "shared:name", "fx:name", "n/limit"]);
    assert!(g.rows.iter().all(|r| !r.vi));
  }

  #[test]
  fn vi_only_rows_are_marked_and_the_plain_ones_come_first() {
    // keys that only exist in vi normal mode carry the mark
    for (group, keys) in
      [("chat", "ctrl-d ctrl-u"), ("chat", "ctrl-w"), ("anywhere", "?"), ("tabs", "gt  gT  space ] [")]
    {
      let row = KEYS.iter().find(|g| t(g.name) == group).and_then(|g| g.rows.iter().find(|r| r.keys == keys));
      assert!(row.expect(keys).vi, "{group}: {keys}");
    }
    // a mixed group lists the plain rows first, then the vi ones
    for g in &KEYS {
      let first_vi = g.rows.iter().position(|r| r.vi).unwrap_or(g.rows.len());
      assert!(g.rows[first_vi..].iter().all(|r| r.vi), "{}: a plain row after the vi ones", t(g.name));
    }
    assert!(KEYS.iter().any(Group::all_vi) && KEYS.iter().any(|g| !g.all_vi() && g.rows.iter().any(|r| r.vi)));
  }
}
