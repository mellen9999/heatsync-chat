// every key the app answers, in plain words, grouped for the `?` screen. the handlers live in
// app.rs (global_key, key, key_insert, key_normal), app/picker.rs and the menus; when a binding
// changes there, change its row here. a test keeps this list well-formed
pub struct Group {
  pub name: &'static str,
  pub rows: &'static [(&'static str, &'static str)],
}

pub static KEYS: [Group; 9] = [
  Group {
    name: "anywhere",
    rows: &[
      ("?  f1", "this screen"),
      ("ctrl-,", "settings"),
      ("ctrl-t", "add a channel"),
      ("ctrl-l", "log in (twitch, kick, youtube)"),
      ("ctrl-shift-l", "log out"),
      ("ctrl-o", "upload a picture or video"),
      ("ctrl-;", "emote picker"),
      ("ctrl-v", "paste text or a picture"),
      ("ctrl-= ctrl-- ctrl-0", "zoom in, out, reset"),
      ("ctrl-wheel", "zoom"),
      ("pageup pagedown", "scroll a page"),
    ],
  },
  Group {
    name: "chat",
    rows: &[
      ("j k  up down", "scroll a line"),
      ("ctrl-d ctrl-u", "scroll half a page"),
      ("gg  G", "oldest, newest"),
      ("i a enter", "start typing"),
      ("v", "pick a line"),
      ("/", "search this tab"),
      ("ctrl-f", "search this tab (any mode)"),
      ("n  N", "next, previous match"),
      ("ctrl-g", "next match (shift: previous)"),
      ("esc", "clear the search"),
      ("x", "hide the pinned line"),
      ("ctrl-w", "close the tab"),
    ],
  },
  Group {
    name: "a picked line",
    rows: &[
      ("j k", "move the pick"),
      ("r", "reply to it"),
      ("m", "line menu: copy, mute, block"),
      ("c", "user card"),
      ("ctrl-c", "copy the line"),
      ("enter", "open it (mentions, feed)"),
      ("u", "upvote (feed)"),
      ("h", "back out of a thread (feed)"),
      ("v esc", "let go"),
    ],
  },
  Group {
    name: "typing",
    rows: &[
      ("enter", "send"),
      ("tab  shift-tab", "complete name, emote, command"),
      ("up down", "older, newer sent lines"),
      ("ctrl-n", "next platform (merged tab)"),
      ("ctrl-a ctrl-e", "start, end of line"),
      ("ctrl-left ctrl-right", "jump a word"),
      ("ctrl-w", "delete the word before"),
      ("ctrl-u ctrl-k", "delete to start, to end"),
      ("esc", "drop the reply (vi: leave typing)"),
    ],
  },
  Group {
    name: "vi box",
    rows: &[
      ("h l w b e", "move by char or word"),
      ("0 ^ $", "line start, first word, end"),
      ("f t F T", "jump to a letter"),
      ("x X D", "delete"),
      ("d c y + move", "delete, change, copy"),
      ("p  P", "paste after, before"),
      ("u  ctrl-r", "undo, redo"),
      ("i a I A", "start typing here, after, ends"),
      ("j k", "older, newer sent lines"),
      ("enter", "send"),
      ("esc", "back to the chat"),
    ],
  },
  Group {
    name: "picker",
    rows: &[
      ("h j k l  arrows", "move"),
      ("enter", "pick (shift-enter keeps it open)"),
      ("/ i  tab", "search, switch search and grid"),
      ("g G", "first, last"),
      ("ctrl-d ctrl-u", "page down, up"),
      ("esc q", "close"),
    ],
  },
  Group {
    name: "tabs",
    rows: &[
      ("gt  gT", "next, previous tab"),
      ("alt-1 .. alt-9", "jump to a tab"),
      ("ctrl-tab", "next tab (shift for previous)"),
      ("click", "open a tab; the gear is settings"),
      ("right-click", "edit, remove or hide a tab"),
    ],
  },
  Group {
    name: "mod",
    rows: &[
      ("m", "line menu: delete, timeout, ban"),
      ("/ban /timeout /unban", "by name"),
      ("/delete /purge", "this line, a person's lines"),
      ("/confirmban", "ask before a ban"),
    ],
  },
  Group {
    name: "menus and cards",
    rows: &[("j k  up down", "move"), ("enter", "choose"), ("esc q", "close"), ("right-click", "close")],
  },
];

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn every_group_is_named_and_lowercase_with_no_empty_cells() {
    for g in &KEYS {
      assert!(!g.name.is_empty() && !g.rows.is_empty());
      assert_eq!(g.name, g.name.to_lowercase());
      for (k, d) in g.rows {
        assert!(!k.is_empty() && !d.is_empty(), "{k}");
        assert_eq!(*d, d.to_lowercase(), "{d}");
      }
    }
  }
}
