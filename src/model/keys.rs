// every key the app answers, in plain words, grouped for the `?` screen. the handlers live in
// app/keys.rs (global_key, key_insert, key_normal, pick_key, leader_key) and the layer files beside it
// (picker, img_view, rover, logs, inventory, the screens, the menus); when a binding changes there, change
// its row here. `k` rows work with plain input, `v` rows only with vi mode on.
// the tests keep the list well-formed, free of repeats, and tied to the handlers' source
// one line of the map: the keys, what they do, and whether they need vi mode (the vi keys, normal mode)
pub struct Row {
  pub keys: &'static str,
  pub does: &'static str,
  pub vi: bool,
}

pub struct Group {
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

pub static KEYS: [Group; 18] = [
  Group {
    name: "anywhere",
    rows: &[
      k("f1", "this screen · the ? button · /keys"),
      k("ctrl-,", "settings · the ⚙ button"),
      k("/preset", "apply a settings preset; /preset save <name> keeps yours (settings: enter applies, x deletes)"),
      k("ctrl-t", "add a channel · the + button"),
      k("ctrl-l", "log in · account menu (link, log out)"),
      k("ctrl-shift-l", "log out"),
      k("/login /logout", "like ctrl-l and ctrl-shift-l"),
      k("/account", "who you're logged in as · log out"),
      k("ctrl-b", "who you follow is live, as a list (also /following, /live)"),
      k("/import", "your twitch follows, live first: space picks, enter adds them as tabs"),
      k("ctrl-p", "your emotes: rename, reorder, remove, undo, sets, share (/emotes)"),
      k("ctrl-o", "upload a picture or video"),
      k("ctrl-;", "emote picker"),
      k("ctrl-v  shift-insert", "paste text or a picture"),
      k("ctrl-= ctrl-- ctrl-0", "zoom in, out, reset"),
      k("ctrl-wheel", "zoom"),
      k("pageup pagedown", "scroll a page"),
      v("?", "this screen"),
    ],
  },
  Group {
    name: "chat",
    rows: &[
      k(
        "click",
        "pick a line; a reply's ↳ line opens its thread; a went-live line opens that channel (its tab, else a temporary one)",
      ),
      k("right-click", "line menu: reply, copy, note, mute, block, mod, pin"),
      k("click  ctrl-f", "filter this tab: type live, /regex/ or @user, enter keeps, esc clears"),
      k("ctrl-g", "next match (shift: previous)"),
      v("ctrl-d ctrl-u", "scroll half a page"),
      v("space g  space G", "oldest, newest"),
      v("v", "pick a line"),
      v("/  space /", "filter this tab (the bar at the top)"),
      v("n  N  space n  space N", "next, previous match"),
      v("space x", "hide the pinned line (your channel or a twitch mod: unpin for everyone)"),
      v("space p", "this twitch channel's prediction (also /predictions, or click its banner)"),
      v("space t", "tab mode: move, reorder, close tabs from the keys"),
      v("space w", "watch this channel's stream (mpv, else the browser)"),
      v("ctrl-w", "close the tab"),
    ],
  },
  Group {
    name: "a picked line",
    rows: &[
      v("j k  up down", "move the pick"),
      v("r", "reply to it (whispers: answer that person)"),
      v("t", "its reply thread, oldest first (again, esc or h goes back; the menu copies it)"),
      v("m", "line menu: copy, note, mute, block, hide; a post: upvote, bookmark, edit, delete; op to feed"),
      v("c", "user card"),
      v("o", "open its first link (a video plays)"),
      v("ctrl-c", "copy the line"),
      v("enter", "open it (mentions, feed); in whispers: answer that person, or retry a failed send"),
      v("u", "upvote (feed)"),
      v("h backspace", "back out of a thread, like the ← back tab"),
      v("v esc", "let go"),
    ],
  },
  Group {
    name: "chat logs (line menu or card → chat logs)",
    rows: &[
      k("j k  up down  click", "pick a line, older / newer"),
      k("g G  wheel pageup", "oldest, newest (the ▼ chip jumps down too); the top loads older lines"),
      k("/", "search their lines (enter finds, esc clears)"),
      k("a", "swap this channel and every channel"),
      k("y", "copy the picked line's public link"),
      k("r", "reload"),
      k("esc q h backspace", "back (esc clears a search first)"),
      v("c", "user card"),
    ],
  },
  Group {
    name: "typing",
    rows: &[
      k("enter", "send (with the list open: pick)"),
      k("tab  shift-tab", "complete emote, name, command; again to cycle"),
      k(":ka  @  /", "list emotes (then emoji), people, commands"),
      k(":fire:", "turns into the emoji when sent"),
      k("up down", "older, newer sent lines (with the list open: move in it)"),
      k("ctrl-z  ctrl-y  ctrl-shift-z", "undo, redo"),
      k("click", "put the cursor there"),
      k("ctrl-n", "merged tab sends to: all feeds, then each alone"),
      k("alt-t alt-k alt-y", "merged tab: send to twitch, kick, youtube on or off"),
      k("ctrl-a ctrl-e  home end", "start, end of line"),
      k("ctrl-left ctrl-right", "jump a word"),
      k("ctrl-w  ctrl-backspace", "delete the word before"),
      k("backspace  delete", "erase the letter before, after the cursor"),
      k("ctrl-u ctrl-k", "delete to start, to end"),
      k("esc", "close the list, end a tab cycle, clear the search, drop the reply (vi: leave typing)"),
    ],
  },
  Group {
    name: "editing the box",
    rows: &[
      v("h l w b e", "move by char or word"),
      v("0 ^ $", "line start, first word, end"),
      v("f t F T", "jump to a letter"),
      v("x X D", "delete"),
      v("d c y", "delete, change, copy, then a move: dw, c$"),
      v("p  P", "paste after, before"),
      v("u  ctrl-r", "undo, redo"),
      v("i a I A", "start typing here, after, ends"),
      v("j k", "older, newer sent lines"),
      v("enter", "send (empty: start typing)"),
      v("esc", "drop the reply"),
    ],
  },
  Group {
    name: "picker",
    rows: &[
      k("h j k l  arrows", "move"),
      k("enter", "pick (shift-enter keeps it open)"),
      k("/ i  tab", "search, switch search and grid"),
      k(
        "] [  ctrl-tab",
        "next, previous tab: emotes, gifs, emoji (gifs: type to search, enter inserts the link; emoji: fuzzy search, enter inserts it)",
      ),
      k("alt-1 alt-2 alt-3 alt-4", "search 7tv, ffz, heatsync, exact names only (the chips; kept on this machine)"),
      k("g G", "first, last"),
      k("ctrl-d ctrl-u  pagedown pageup", "page down, up"),
      k("backspace  ctrl-w", "erase a letter in the search, clear it"),
      k("esc q ctrl-;", "close"),
    ],
  },
  Group {
    name: "your emotes (ctrl-p)",
    rows: &[
      k("j k  up down", "move"),
      k("] [  tab", "mine, removed, sets"),
      k("n  enter", "rename · removed: restore · sets: apply (y yes, n no)"),
      k("m", "move it to a slot you type (a taken slot trades places)"),
      k("J K", "carry it down, up one place"),
      k("x", "remove it (it asks first)"),
      k("y", "yes to a question it asks (n or esc: no)"),
      k("u  ctrl-r", "undo, redo your last change"),
      k("s S", "share your set (copies the link), stop sharing"),
      k("i", "import a channel's 7tv, bttv and ffz emotes"),
      k("/  R", "filter, read again"),
      k("esc q", "close"),
    ],
  },
  Group {
    name: "tabs",
    rows: &[
      k("alt-1 .. alt-0", "jump to a tab (alt-0 is the tenth)"),
      k("ctrl-tab", "next tab (shift for previous)"),
      k("click", "open a tab; the ⚙ is settings, the ? is this screen"),
      k(
        "right-click",
        "clear a red, cyan or yellow tab; else its menu: edit, remove (a temporary tab: keep; built-ins: hide), clear lines, share layout, use a shared layout, and announce, poll, prediction for a twitch room you run",
      ),
      k("edit form", "tab to the buttons: watch, status, predictions, summary; j l next, k h back, enter runs"),
      v("gt  gT  space ] [", "next, previous tab"),
      v("space 1 .. space 9", "jump to a tab"),
    ],
  },
  Group {
    name: "mod",
    rows: &[
      k("/ban /timeout /unban", "by name"),
      k("/delete /purge", "this line, a person's lines"),
      k("/confirmban", "ask before a ban"),
      k("/modbuttons", "hover buttons on a line: /modbuttons x 10m ban, off clears"),
      k("allow  deny", "an automod hold: click, or its line menu"),
      k("/blocked", "blocked terms: j k move, x delete, a add, / filter"),
      k("/warn /shoutout /shield", "warn with a reason, shout out, shield mode"),
      k("/modtools", "shield, automod (h l), unban requests (y approve, x deny), chatters"),
      k("/status", "stream and chat modes: enter flips a mode, h l its wait"),
      k("/slow /followers", "a wait, or off: /slow 30, /followers 10m"),
      k("/emoteonly /subscribers /unique", "on, or off: /emoteonly off"),
      v("m", "line menu: delete, timeout, ban"),
    ],
  },
  Group {
    name: "tab mode (space t)",
    rows: &[
      v("j k  up down", "move the cursor (3j: three)"),
      v("g G  home end", "first, last"),
      v("enter l right", "open the tab"),
      v("J K", "carry a channel down, up"),
      v("d x", "close a channel"),
      v("a o", "add a channel"),
      v("esc q t", "leave"),
    ],
  },
  Group {
    name: "picture",
    rows: &[
      k("click", "a picture in chat fills the pane"),
      k("click ▶", "a video plays (mpv, else the browser)"),
      k("+ - 0", "zoom in, out, fit"),
      k("h j k l  arrows", "move around it · or the wheel"),
      k("pageup pagedown space", "scroll a page"),
      k("esc q", "close it · or click, or the ← back tab"),
    ],
  },
  Group {
    name: "settings (ctrl-,)",
    rows: &[
      k("j k  up down", "move"),
      k("ctrl-d ctrl-u  pageup pagedown", "half a page, a page"),
      k("gg G  home end", "first, last"),
      k("/", "filter the list: type, enter stops typing, esc clears"),
      k("enter space", "flip it, apply a preset, or finish a list's command in the box"),
      k("h l  left right", "step a choice down, up (zoom, tab side, counter, pickers)"),
      k("x", "delete the picked preset of yours"),
      k("?", "the key list"),
      k("esc q", "close (esc clears the filter first)"),
    ],
  },
  Group {
    name: "followed (ctrl-b)",
    rows: &[
      k("j k  up down", "move"),
      k("enter", "open their tab, else an underlined temporary one (one slot; /keep or chatting in it saves it)"),
      k("w", "watch the stream (mpv, else the browser)"),
      k("r", "read the list again"),
      k("/", "filter by name"),
      k("esc q", "close"),
    ],
  },
  Group {
    name: "import (/import)",
    rows: &[
      k("j k  up down", "move"),
      k("space", "pick or unpick a channel"),
      k("s", "pick everyone who is live"),
      k("enter", "add the picked ones as tabs"),
      k("r", "read your follows again"),
      k("/", "filter by name"),
      k("esc q", "close"),
    ],
  },
  Group {
    name: "mod tools (/modtools)",
    rows: &[
      k("j k  up down", "move"),
      k("enter space", "flip shield, run a row, ask about a request"),
      k("h l  left right", "automod level down, up"),
      k("y x", "on an unban request: approve, deny (then y or enter sure, n or esc cancel)"),
      k("r", "read it all again"),
      k("/", "filter"),
      k("esc q", "close"),
    ],
  },
  Group {
    name: "menus and cards",
    rows: &[
      k("j k  up down", "move"),
      k("enter", "choose"),
      k("esc", "close (right-click too)"),
      k("1 .. 9", "a ban's reasons: the number picks it"),
      k("y", "yes, when it asks to unpin for everyone"),
      k("q", "close a user card"),
      k("c", "in a card of a live twitch channel: clip it, copies the link (again: opens the edit page)"),
    ],
  },
  Group {
    name: "your lists (settings → muted & blocked people, highlights, rules…)",
    rows: &[
      k("j k  up down", "move"),
      k("enter", "a person: their card · the add row: type a new entry"),
      k("x", "remove the picked one (unmute, unblock, unhide, delete)"),
      k("a", "add an entry: a word, pattern, rule or ban reason"),
      k("esc q", "close"),
    ],
  },
];

#[cfg(test)]
mod tests {
  use super::*;
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
    match g.name {
      n if n.starts_with("chat logs") => "logs",
      "a picked line" => "picked",
      "picker" => "picker",
      "tab mode (space t)" => "tab mode",
      "picture" => "picture",
      "your emotes (ctrl-p)" => "emotes",
      "menus and cards" => "menus",
      n if n.starts_with("your lists") => "lists",
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
      assert!(!g.name.is_empty() && !g.rows.is_empty());
      assert_eq!(g.name, g.name.to_lowercase());
      assert!(names.insert(g.name), "{} twice", g.name);
      for r in g.rows {
        assert!(!r.keys.is_empty() && !r.does.is_empty(), "{}", r.keys);
        assert_eq!(r.does, r.does.to_lowercase(), "{}", r.does);
      }
    }
  }

  #[test]
  fn no_key_is_listed_twice_in_one_place() {
    let mut seen = HashSet::new();
    for g in &KEYS {
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
      let code = handlers(layer(g));
      for r in g.rows {
        for t in tokens(r.keys) {
          let ok = if t.len() > 1 && t.starts_with('/') { slash.contains(t.as_str()) } else { handled(&code, &t) };
          assert!(ok, "{t} ({}) has no handler in the {} layer", g.name, layer(g));
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
  const INTERNAL: [(&str, &str); 6] = [
    // the shifted twins of keys that have rows (ctrl-v / ctrl-shift-v, ctrl-+ / ctrl-=, ctrl-_ / ctrl--, + / =)
    ("app/clip.rs", "V"),
    ("app/zoom.rs", "+"),
    ("app/zoom.rs", "_"),
    ("app/img_view.rs", "="),
    // a space typed into the picker's search
    ("app/picker.rs", "Space"),
    // a space typed into the emotes screen's rename / move box
    ("app/inventory.rs", "Space"),
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

  #[test]
  fn vi_only_rows_are_marked_and_the_plain_ones_come_first() {
    // keys that only exist in vi normal mode carry the mark
    for (group, keys) in
      [("chat", "ctrl-d ctrl-u"), ("chat", "ctrl-w"), ("anywhere", "?"), ("tabs", "gt  gT  space ] [")]
    {
      let row = KEYS.iter().find(|g| g.name == group).and_then(|g| g.rows.iter().find(|r| r.keys == keys));
      assert!(row.expect(keys).vi, "{group}: {keys}");
    }
    // a mixed group lists the plain rows first, then the vi ones
    for g in &KEYS {
      let first_vi = g.rows.iter().position(|r| r.vi).unwrap_or(g.rows.len());
      assert!(g.rows[first_vi..].iter().all(|r| r.vi), "{}: a plain row after the vi ones", g.name);
    }
    assert!(KEYS.iter().any(Group::all_vi) && KEYS.iter().any(|g| !g.all_vi() && g.rows.iter().any(|r| r.vi)));
  }
}
