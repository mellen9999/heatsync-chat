// every key the app answers, in plain words, grouped for the `?` screen. the handlers live in
// app.rs (global_key, key_insert, key_normal, pick_key, leader_key), app/picker.rs, app/img_view.rs and the menus;
// when a binding changes there, change its row here. `k` rows work with plain input, `v` rows only with vi mode on.
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

pub static KEYS: [Group; 12] = [
  Group {
    name: "anywhere",
    rows: &[
      k("f1", "this screen · the ? button · /keys"),
      k("ctrl-,", "settings · the ⚙ button"),
      k("ctrl-t", "add a channel · the + button"),
      k("ctrl-l", "log in · account menu (link, log out)"),
      k("ctrl-shift-l", "log out"),
      k("/login /logout", "like ctrl-l and ctrl-shift-l"),
      k("/account", "who you're logged in as · log out"),
      k("ctrl-b", "who you follow is live: enter opens a tab, w watches (also /following, /live)"),
      k("ctrl-o", "upload a picture or video"),
      k("ctrl-;", "emote picker"),
      k("ctrl-v", "paste text or a picture"),
      k("ctrl-= ctrl-- ctrl-0", "zoom in, out, reset"),
      k("ctrl-wheel", "zoom"),
      k("pageup pagedown", "scroll a page"),
      v("?", "this screen"),
    ],
  },
  Group {
    name: "chat",
    rows: &[
      k("click", "pick a line"),
      k("right-click", "line menu: reply, copy, note, mute, block, mod"),
      k("ctrl-f", "search this tab"),
      k("ctrl-g", "next match (shift: previous)"),
      v("ctrl-d ctrl-u", "scroll half a page"),
      v("space g  space G", "oldest, newest"),
      v("v", "pick a line"),
      v("/  space /", "search this tab"),
      v("n  N", "next, previous match"),
      v("space x", "hide the pinned line"),
      v("space p", "this twitch channel's prediction (also /predictions, or click its banner)"),
      v("space t", "tab mode: move, reorder, close tabs from the keys"),
      v("space w", "watch this channel's stream (mpv, else the browser)"),
      v("ctrl-w", "close the tab"),
    ],
  },
  Group {
    name: "a picked line",
    rows: &[
      v("j k", "move the pick"),
      v("r", "reply to it"),
      v("m", "line menu: copy, note, mute, block, hide; a post: upvote, bookmark, edit, delete; op to feed"),
      v("c", "user card"),
      v("o", "open its first link (a video plays)"),
      v("ctrl-c", "copy the line"),
      v("enter", "open it (mentions, notifs, feed)"),
      v("u", "upvote (feed)"),
      v("h", "back out of a thread, like the ← back tab"),
      v("v esc", "let go"),
    ],
  },
  Group {
    name: "chat logs (line menu or card → chat logs)",
    rows: &[
      k("j k  click", "pick a line, older / newer"),
      k("g G  wheel pageup", "oldest, newest (the ▼ chip jumps down too); the top loads older lines"),
      k("/", "search their lines (enter finds, esc clears)"),
      k("a", "swap this channel and every channel"),
      k("y", "copy the picked line's public link"),
      k("r", "reload"),
      k("esc q h", "back (esc clears a search first)"),
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
      k("ctrl-z  ctrl-y", "undo, redo"),
      k("click", "put the cursor there"),
      k("ctrl-n", "merged tab sends to: all feeds, then each alone"),
      k("alt-t alt-k alt-y", "merged tab: send to twitch, kick, youtube on or off"),
      k("ctrl-a ctrl-e", "start, end of line"),
      k("ctrl-left ctrl-right", "jump a word"),
      k("ctrl-w", "delete the word before"),
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
      k("] [  ctrl-tab", "emotes and gifs tab (gifs: type to search, enter inserts the link)"),
      k("g G", "first, last"),
      k("ctrl-d ctrl-u", "page down, up"),
      k("esc q", "close"),
    ],
  },
  Group {
    name: "tabs",
    rows: &[
      k("alt-1 .. alt-9", "jump to a tab"),
      k("ctrl-tab", "next tab (shift for previous)"),
      k("click", "open a tab; the ⚙ is settings, the ? is this screen"),
      k("right-click", "clear a red, cyan or yellow tab; else edit or remove it (built-ins: hide)"),
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
      v("g G", "first, last"),
      v("enter l", "open the tab"),
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
      k("esc q", "close it · or click, or the ← back tab"),
    ],
  },
  Group {
    name: "menus and cards",
    rows: &[
      k("j k  up down", "move"),
      k("enter", "choose"),
      k("esc", "close (right-click too)"),
      k("q", "close a user card"),
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
      "menus and cards" => "menus",
      _ => "chat",
    }
  }

  fn src(file: &str) -> String {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(file);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
  }

  // the handler sources a layer's keys live in
  fn handlers(layer: &str) -> String {
    let files: Vec<String> = match layer {
      "logs" => vec!["app/logs.rs".into(), "app/keys.rs".into()],
      "picked" => vec!["app/keys.rs".into()],
      "picker" => vec!["app/picker.rs".into()],
      "tab mode" => vec!["app/rover.rs".into()],
      "picture" => vec!["app/img_view.rs".into()],
      "menus" => ["pop", "card", "tabs", "emote"].map(|f| format!("app/{f}.rs")).to_vec(),
      _ => {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/app");
        let mut v: Vec<String> = std::fs::read_dir(dir)
          .expect("src/app")
          .filter_map(|e| Some(format!("app/{}", e.ok()?.file_name().to_str()?)))
          .filter(|f| f.ends_with(".rs"))
          .collect();
        v.extend(["app.rs".into(), "model/vi.rs".into()]);
        v
      }
    };
    files.iter().map(|f| src(f)).collect()
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
