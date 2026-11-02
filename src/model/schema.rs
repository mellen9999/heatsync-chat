// the one table of every setting: the settings screen, the slash commands, the settings file and the
// presets all read it. labels and tips are the ext's (settings-schema.js + locale strings), lowercase.
// adding a switch = one Settings field and one row here
use crate::model::settings::Settings;
use crate::model::tabs::Kind;

// the ext's settings categories, in its order
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cat {
  Display,
  Chat,
  Notifs,
  Mod,
  Filters,
  System,
}

impl Cat {
  pub fn name(self) -> &'static str {
    match self {
      Cat::Display => "display",
      Cat::Chat => "chat",
      Cat::Notifs => "notifications",
      Cat::Mod => "mod",
      Cat::Filters => "filters",
      Cat::System => "system",
    }
  }
}

// what the app has to redo besides saving when a switch flips
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fx {
  None,
  // the box changes kind: back to typing
  Vi,
  Glow,
  Paints,
  // paint motion flipped: every animated name is built again
  PaintMotion,
  Media,
  // the partial-link rule flipped: lay every line out again
  Links,
  // the filter flags changed: judge every line again
  Filters,
  // a content switch flipped: what flagged pictures and emotes show
  Content,
  // a built-in tab came or went
  Tab(Kind),
}

#[derive(Clone, Copy)]
pub enum Ctl {
  // an on/off switch: read it, write it
  Pill(fn(&Settings) -> bool, fn(&mut Settings, bool)),
  // the zoom percent, stepped
  Zoom,
  // the tab bar's side, stepped
  Side,
  // a list kept with slash commands (hidden words, highlight words); true = highlight
  Words(bool),
}

pub struct Def {
  // the key in the settings file
  pub id: &'static str,
  // its slash command, if it has one
  pub cmd: Option<&'static str>,
  pub cat: Cat,
  pub section: &'static str,
  pub label: &'static str,
  pub tip: &'static str,
  // extra search words
  pub alias: &'static str,
  pub ctl: Ctl,
  pub fx: Fx,
}

macro_rules! pill {
  ($f:ident) => {
    Ctl::Pill(|s| s.$f, |s, v| s.$f = v)
  };
}

pub static DEFS: [Def; 32] = [
  Def {
    id: "first_glow",
    cmd: Some("/glow"),
    cat: Cat::Display,
    section: "chat messages",
    label: "first-message glow",
    tip: "yellow edge on someone's first message this session",
    alias: "firstchatter first chatter",
    ctl: pill!(first_glow),
    fx: Fx::Glow,
  },
  Def {
    id: "auto_hide",
    cmd: Some("/autohide"),
    cat: Cat::Display,
    section: "chat messages",
    label: "auto-hide input",
    tip: "hides the input bar when idle",
    alias: "autohide",
    ctl: pill!(auto_hide),
    fx: Fx::None,
  },
  Def {
    id: "paints",
    cmd: Some("/paints"),
    cat: Cat::Display,
    section: "cosmetics",
    label: "7tv paints + badges",
    tip: "name paints and 7tv badges on chatters",
    alias: "sevenTvPaints seventv cosmetics",
    ctl: pill!(paints),
    fx: Fx::Paints,
  },
  Def {
    id: "paint_motion",
    cmd: Some("/paintmotion"),
    cat: Cat::Display,
    section: "cosmetics",
    label: "animate paints",
    tip: "name paints move. off freezes each to its first frame",
    alias: "animatePaints motion scenes effects reduced",
    ctl: pill!(paint_motion),
    fx: Fx::PaintMotion,
  },
  Def {
    id: "zoom",
    cmd: Some("/zoom"),
    cat: Cat::Display,
    section: "font",
    label: "zoom",
    tip: "everything's size, 50 to 300 percent. ctrl-= / ctrl-- / ctrl-0 or ctrl-wheel anywhere. /zoom 1.5",
    alias: "size scale font",
    ctl: Ctl::Zoom,
    fx: Fx::None,
  },
  Def {
    id: "tab_side",
    cmd: Some("/tabs"),
    cat: Cat::Display,
    section: "layout",
    label: "tab position",
    tip: "where the tabs sit: top, bottom, left or right. /tabs left",
    alias: "tabPosition tab bar side",
    ctl: Ctl::Side,
    fx: Fx::None,
  },
  Def {
    id: "vi",
    cmd: Some("/vi"),
    cat: Cat::Chat,
    section: "input",
    label: "vi mode",
    tip: "vim keys on the message box, space then a key for the chat",
    alias: "viMode vim keys",
    ctl: pill!(vi),
    fx: Fx::Vi,
  },
  Def {
    id: "media",
    cmd: Some("/media"),
    cat: Cat::Chat,
    section: "messages",
    label: "inline media",
    tip: "show images, gifs and link cards inline in chat",
    alias: "mediaembeds embeds pictures",
    ctl: pill!(media),
    fx: Fx::Media,
  },
  Def {
    id: "partial_links",
    cmd: Some("/partiallinks"),
    cat: Cat::Chat,
    section: "messages",
    label: "partial links",
    tip: "also linkify partial and defanged links — bare watch?v= youtube refs and \"site (dot) com\" style domains chats use to dodge link filters.",
    alias: "partiallinks defang",
    ctl: pill!(partial_links),
    fx: Fx::Links,
  },
  Def {
    id: "highlights",
    cmd: Some("/highlight"),
    cat: Cat::Chat,
    section: "messages",
    label: "keyword highlights",
    tip: "tints matched messages orange. whole-word, case-insensitive. enter adds one, /unhighlight removes",
    alias: "keywordhighlights words",
    ctl: Ctl::Words(true),
    fx: Fx::None,
  },
  Def {
    id: "updates",
    cmd: Some("/updates"),
    cat: Cat::System,
    section: "updates",
    label: "check for updates",
    tip: "look for a new version on github every few hours; a ↑ button on the tab bar installs it in one click",
    alias: "update upgrade new version",
    ctl: pill!(updates),
    fx: Fx::None,
  },
  Def {
    id: "notify",
    cmd: Some("/notify"),
    cat: Cat::Notifs,
    section: "when you get @mentioned",
    label: "desktop notification",
    tip: "show a desktop notification when someone @s you and the window is not focused",
    alias: "hs_notifications browser notification",
    ctl: pill!(notify),
    fx: Fx::None,
  },
  Def {
    id: "ev_online",
    cmd: Some("/online"),
    cat: Cat::Notifs,
    section: "stream events",
    label: "went live",
    tip: "line when a channel goes live",
    alias: "online",
    ctl: pill!(ev_online),
    fx: Fx::None,
  },
  Def {
    id: "ev_offline",
    cmd: Some("/offline"),
    cat: Cat::Notifs,
    section: "stream events",
    label: "went offline",
    tip: "line when a channel goes offline (off by default, noisy)",
    alias: "offline",
    ctl: pill!(ev_offline),
    fx: Fx::None,
  },
  Def {
    id: "ev_game",
    cmd: Some("/game"),
    cat: Cat::Notifs,
    section: "stream events",
    label: "game switches",
    tip: "line when a streamer changes the game",
    alias: "game category",
    ctl: pill!(ev_game),
    fx: Fx::None,
  },
  Def {
    id: "inline_op",
    cmd: Some("/inlineop"),
    cat: Cat::Notifs,
    section: "feed in chat",
    label: "new threads",
    tip: "a new heatsync thread ([OP]) shows as a line in every chat tab",
    alias: "inline op feed posts",
    ctl: pill!(inline_op),
    fx: Fx::None,
  },
  Def {
    id: "inline_mop",
    cmd: Some("/inlinemop"),
    cat: Cat::Notifs,
    section: "feed in chat",
    label: "op replies",
    tip: "a thread's author answering in their own thread shows in every chat tab",
    alias: "inline op reply feed",
    ctl: pill!(inline_mop),
    fx: Fx::None,
  },
  Def {
    id: "inline_re",
    cmd: Some("/inlinere"),
    cat: Cat::Notifs,
    section: "feed in chat",
    label: "replies",
    tip: "a reply ([RE]) to a heatsync thread shows in every chat tab",
    alias: "inline re feed replies",
    ctl: pill!(inline_re),
    fx: Fx::None,
  },
  Def {
    id: "moments",
    cmd: Some("/moments"),
    cat: Cat::Notifs,
    section: "inline notifications",
    label: "moment alerts",
    tip: "a channel's chat suddenly explodes: a heat spike line in that tab",
    alias: "spike heat",
    ctl: pill!(moments),
    fx: Fx::None,
  },
  Def {
    id: "confirm_ban",
    cmd: Some("/confirmban"),
    cat: Cat::Mod,
    section: "mod toolbar",
    label: "confirm before ban",
    tip: "ask before a permanent ban goes out: guards against a misclick",
    alias: "modConfirmBan",
    ctl: pill!(confirm_ban),
    fx: Fx::None,
  },
  Def {
    id: "hide_bots",
    cmd: Some("/hidebots"),
    cat: Cat::Filters,
    section: "messages",
    label: "hide bots",
    tip: "hide messages from known chat bots (nightbot, streamelements, fossabot...)",
    alias: "hideBots",
    ctl: pill!(hide_bots),
    fx: Fx::Filters,
  },
  Def {
    id: "hide_cmds",
    cmd: Some("/hidecmds"),
    cat: Cat::Filters,
    section: "messages",
    label: "hide !commands",
    tip: "hide messages that start with !",
    alias: "hideCommands",
    ctl: pill!(hide_cmds),
    fx: Fx::Filters,
  },
  Def {
    id: "hide_dupes",
    cmd: Some("/hidedupes"),
    cat: Cat::Filters,
    section: "messages",
    label: "hide duplicates",
    tip: "collapse identical consecutive messages (spam waves) to the first one",
    alias: "hideDuplicates spam",
    ctl: pill!(hide_dupes),
    fx: Fx::Filters,
  },
  Def {
    id: "hidden_words",
    cmd: Some("/hideword"),
    cat: Cat::Filters,
    section: "messages",
    label: "mute keywords",
    tip: "messages containing any of these get hidden. enter adds one, /unhideword removes",
    alias: "hideword hidden words",
    ctl: Ctl::Words(false),
    fx: Fx::None,
  },
  Def {
    id: "show_nsfw",
    cmd: Some("/nsfw"),
    cat: Cat::Filters,
    section: "content",
    label: "show nsfw",
    tip: "sexual pictures and emotes (flagged by moderation). hidden behind a show button when off",
    alias: "viewer_show_sexual sexual cw content warning sensitive",
    ctl: pill!(show_nsfw),
    fx: Fx::Content,
  },
  Def {
    id: "show_violence",
    cmd: Some("/violence"),
    cat: Cat::Filters,
    section: "content",
    label: "show violence",
    tip: "gore and violence. hidden behind a show button when off",
    alias: "viewer_show_gore gore cw content warning sensitive",
    ctl: pill!(show_violence),
    fx: Fx::Content,
  },
  Def {
    id: "show_weapons",
    cmd: Some("/weapons"),
    cat: Cat::Filters,
    section: "content",
    label: "show weapons",
    tip: "weapons. hidden behind a show button when off",
    alias: "viewer_show_weapon weapon cw content warning sensitive",
    ctl: pill!(show_weapons),
    fx: Fx::Content,
  },
  Def {
    id: "show_drugs",
    cmd: Some("/drugs"),
    cat: Cat::Filters,
    section: "content",
    label: "show drugs",
    tip: "drugs. hidden behind a show button when off",
    alias: "viewer_show_drug drug cw content warning sensitive",
    ctl: pill!(show_drugs),
    fx: Fx::Content,
  },
  Def {
    id: "show_hate",
    cmd: Some("/hate"),
    cat: Cat::Filters,
    section: "content",
    label: "show hate speech",
    tip: "hate symbols and speech. hidden behind a show button when off",
    alias: "viewer_show_hate cw content warning sensitive",
    ctl: pill!(show_hate),
    fx: Fx::Content,
  },
  Def {
    id: "tab_feed",
    cmd: None,
    cat: Cat::System,
    section: "tabs",
    label: "feed tab",
    tip: "the heatsync social feed in the tab bar. /show feed, /hide feed",
    alias: "visible tabs hiddenTabs",
    ctl: pill!(tab_feed),
    fx: Fx::Tab(Kind::Feed),
  },
  Def {
    id: "tab_whispers",
    cmd: None,
    cat: Cat::System,
    section: "tabs",
    label: "whispers tab",
    tip: "your twitch whispers in the tab bar. /show whispers, /hide whispers",
    alias: "visible tabs hiddenTabs dm",
    ctl: pill!(tab_whispers),
    fx: Fx::Tab(Kind::Whispers),
  },
  Def {
    id: "tab_mentions",
    cmd: None,
    cat: Cat::System,
    section: "tabs",
    label: "mentions tab",
    tip: "every @mention of you in the tab bar. /show mentions, /hide mentions",
    alias: "visible tabs hiddenTabs",
    ctl: pill!(tab_mentions),
    fx: Fx::Tab(Kind::Mentions),
  },
];

pub fn def(id: &str) -> Option<&'static Def> {
  DEFS.iter().find(|d| d.id == id)
}

// `/vi` -> its row. exact word only
pub fn by_cmd(word: &str) -> Option<&'static Def> {
  DEFS.iter().find(|d| d.cmd == Some(word))
}

// every row that is a plain switch with a command of its own (zoom and the word lists parse their own arguments)
pub fn switch_for(word: &str) -> Option<&'static Def> {
  by_cmd(word).filter(|d| matches!(d.ctl, Ctl::Pill(..)))
}

impl Def {
  pub fn on(&self, s: &Settings) -> Option<bool> {
    match self.ctl {
      Ctl::Pill(get, _) => Some(get(s)),
      _ => None,
    }
  }

  // false when it isn't a switch
  pub fn put(&self, s: &mut Settings, v: bool) -> bool {
    match self.ctl {
      Ctl::Pill(_, set) => {
        set(s, v);
        true
      }
      _ => false,
    }
  }

  // every word of the query must appear in the label, tip, section, category, key or alias (the ext's search)
  pub fn matches(&self, query: &str) -> bool {
    let hay = format!("{} {} {} {} {} {}", self.label, self.tip, self.section, self.cat.name(), self.id, self.alias)
      .to_lowercase();
    query.split_whitespace().all(|w| hay.contains(&w.to_lowercase()))
  }
}

// a whole bundle of switches at once, from the ext's presets, keeping only settings this app has
pub struct Preset {
  pub name: &'static str,
  pub tip: &'static str,
  pub diff: &'static [(&'static str, bool)],
}

pub static PRESETS: [Preset; 3] = [
  Preset {
    name: "normal",
    tip: "the defaults: every tab, paints and pictures on, plain input",
    diff: &[
      ("vi", false),
      ("first_glow", true),
      ("paints", true),
      ("media", true),
      ("tab_feed", true),
      ("tab_whispers", true),
      ("tab_mentions", true),
    ],
  },
  Preset {
    name: "minimal",
    tip: "just chat: no glow, paints, pictures or extra tabs",
    diff: &[
      ("first_glow", false),
      ("paints", false),
      ("media", false),
      ("tab_feed", false),
      ("tab_whispers", false),
      ("tab_mentions", false),
    ],
  },
  Preset {
    name: "power user",
    tip: "every tab on, vi keys",
    diff: &[("vi", true), ("tab_feed", true), ("tab_whispers", true), ("tab_mentions", true)],
  },
];

impl Preset {
  pub fn active(&self, s: &Settings) -> bool {
    self.diff.iter().all(|(id, v)| def(id).and_then(|d| d.on(s)) == Some(*v))
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn ids_and_commands_are_unique_and_lowercase() {
    for (i, d) in DEFS.iter().enumerate() {
      assert_eq!(d.label, d.label.to_lowercase());
      assert_eq!(d.section, d.section.to_lowercase());
      assert!(DEFS[i + 1..].iter().all(|o| o.id != d.id && (d.cmd.is_none() || o.cmd != d.cmd)));
      assert!(d.cmd.is_none_or(|c| c.starts_with('/') && c == c.to_lowercase()));
    }
  }

  #[test]
  fn presets_only_name_real_switches() {
    for p in &PRESETS {
      for (id, _) in p.diff {
        assert!(def(id).and_then(|d| d.on(&Settings::default())).is_some(), "{id}");
      }
    }
    assert!(PRESETS[2].diff.iter().any(|d| d.0 == "vi"));
    // a fresh install reads as normal
    assert!(PRESETS[0].active(&Settings::default()));
  }

  #[test]
  fn a_preset_reads_active_only_when_all_of_it_matches() {
    let mut s = Settings::default();
    let power = &PRESETS[2];
    assert!(!power.active(&s));
    for (id, v) in power.diff {
      assert!(def(id).unwrap().put(&mut s, *v));
    }
    assert!(power.active(&s));
    s.vi = false;
    assert!(!power.active(&s));
  }

  #[test]
  fn search_needs_every_word_and_reads_aliases() {
    let vi = def("vi").unwrap();
    assert!(vi.matches("vim"));
    assert!(vi.matches("VI mode"));
    assert!(vi.matches("box chat"));
    assert!(!vi.matches("vi banana"));
    assert!(vi.matches(""));
    assert!(def("paints").unwrap().matches("seventv"));
    assert!(def("hide_bots").unwrap().matches("filters bots"));
  }

  #[test]
  fn commands_find_their_rows() {
    assert_eq!(switch_for("/vi").map(|d| d.id), Some("vi"));
    assert_eq!(by_cmd("/zoom").map(|d| d.id), Some("zoom"));
    assert!(switch_for("/zoom").is_none());
    assert!(switch_for("/hideword").is_none());
    assert!(by_cmd("/nope").is_none());
  }
}
