// the one table of every setting: the settings screen, the slash commands and the settings file
// all read it. labels and tips are the ext's (settings-schema.js + locale strings), lowercase.
// adding a switch = one Settings field and one row here
use crate::model::settings::{Choice, Settings};
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
  // the last-read line switch flipped: marks drop when it goes off
  ReadLine,
  Paints,
  // 7tv personal emotes flipped: what is held goes (or starts coming), lines are laid out again
  Personal,
  Media,
  // streamer mode flipped: previews, whisper rows and the marker follow
  Streamer,
  // the chat look changed (clock, stripes, name colors, tags, emote size, spacing, animation, dimming):
  // the look is rebuilt and every line laid out again
  Look,
  // the partial-link rule flipped: lay every line out again
  Links,
  // the filter flags changed: judge every line again
  Filters,
  // a content switch flipped: what flagged pictures and emotes show
  Content,
  // the mention sound was picked: it is heard once
  Ping,
  // the mention sound's file was set or cleared: set means custom, cleared means the ping; then heard
  PingFile,
  // the font family changed: the new one is looked up and, once found, all text is laid out again
  Font,
  // chat logs flipped: the writer starts or lets go
  ChatLogs,
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
  // the tab counter's mode, stepped
  Counter,
  // a few named values, stepped with h / l and typed as `/cmd value`: read the place, write it
  Pick(&'static Choice, fn(&Settings) -> usize, fn(&mut Settings, usize)),
  // the hide / highlight rules (hidden words, highlights and patterns are rules), kept with /rule
  Rules,
  // the ban reasons a ban offers, kept with /banreason
  Reasons,
  // the people you muted and blocked: opens their list screen
  People,
  // the server's mention rules: opens their list screen
  Alerts,
  // your own slash commands: opens their list screen
  Commands,
  // the mod toolbar's picked buttons (a mask over model::modbar::CATALOG), typed as `/cmd x 1m ban`
  Buttons(fn(&Settings) -> u16, fn(&mut Settings, u16)),
  // a few words typed as `/cmd words` (a font family, a file path): kept as typed, `off` clears them
  Text(&'static Words),
}

// how a words row reads and writes its value
pub struct Words {
  // `/cmd <usage>`
  pub usage: &'static str,
  // what an empty value means, for the settings screen
  pub none: &'static str,
  pub get: fn(&Settings) -> &str,
  // false = refused: too long, or holds a control character
  pub put: fn(&mut Settings, &str) -> bool,
  // the value as the screen and status lines show it (a path shows only its file name)
  pub show: fn(&str) -> &str,
  // whether a value can stick, in plain words if not. asked before it is kept; empty always passes
  pub check: fn(&str) -> Result<(), &'static str>,
}

static FONT: Words = Words {
  usage: "<family|off>",
  none: "system monospace",
  get: |s| s.font_family.as_str(),
  put: |s, v| s.font_family.set(v),
  show: |v| v,
  // the lookup is the check, and it takes a moment: it is asked off the ui thread (net/font.rs)
  check: |_| Ok(()),
};

static PING_FILE: Words = Words {
  usage: "<path|off>",
  none: "none",
  get: |s| s.ping_file.as_str(),
  put: |s, v| s.ping_file.set(v),
  show: |v| v.rsplit('/').next().unwrap_or(v),
  check: |p| crate::util::wav::read(p).map(drop).map_err(|b| b.said()),
};

// how a row travels to and from the account's ui-state (model/sync.rs holds the codecs). the ext's key
// names are its storage names, never renamed; rows with none stay on this device
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Wire {
  None,
  // an on/off switch under its own key
  Bool(&'static str),
  // a named choice: the ext stores the choice's name
  Name(&'static str),
  // a volume the ext stores as 0..1
  Level(&'static str),
  // one flag of a {name: bool} map shared with other flags
  Flag(&'static str, &'static str),
  // one built-in tab: shown unless its id is in a list of hidden ids shared with other tabs
  Hidden(&'static str, &'static str),
}

impl Wire {
  pub fn key(self) -> Option<&'static str> {
    match self {
      Wire::None => None,
      Wire::Bool(k) | Wire::Name(k) | Wire::Level(k) | Wire::Flag(k, _) | Wire::Hidden(k, _) => Some(k),
    }
  }
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
  // where the heatsync account keeps it (the ext's key), so it follows you between clients
  pub wire: Wire,
}

macro_rules! pill {
  ($f:ident) => {
    Ctl::Pill(|s| s.$f, |s, v| s.$f = v)
  };
}

macro_rules! pick {
  ($f:ident, $t:ident) => {
    Ctl::Pick(&crate::model::settings::$t::CHOICE, |s| s.$f as usize, |s, i| s.$f = crate::model::settings::$t::ALL[i])
  };
}

pub static DEFS: [Def; 70] = [
  Def {
    id: "font_family",
    cmd: Some("/font"),
    cat: Cat::Display,
    section: "font",
    label: "font family",
    tip: "type a font name, like fira code. off = the default. stays on this computer",
    alias: "fontFamily customFontName typeface monospace",
    ctl: Ctl::Text(&FONT),
    fx: Fx::Font,
    wire: Wire::None,
  },
  Def {
    id: "zoom",
    cmd: Some("/zoom"),
    cat: Cat::Display,
    section: "font",
    label: "zoom",
    tip: "everything's size, 90 to 300 percent. ctrl-= / ctrl-- / ctrl-0 or ctrl-wheel anywhere. /zoom 1.5",
    alias: "size scale font",
    ctl: Ctl::Zoom,
    fx: Fx::None,
    wire: Wire::None,
  },
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
    wire: Wire::Bool("firstChatterGlow"),
  },
  Def {
    id: "read_line",
    cmd: Some("/readline"),
    cat: Cat::Display,
    section: "chat messages",
    label: "last-read line",
    tip: "a red line where you stopped reading when you left the tab or the window",
    alias: "unread marker divider new messages",
    ctl: pill!(read_line),
    fx: Fx::ReadLine,
    wire: Wire::None,
  },
  Def {
    id: "timestamps",
    cmd: Some("/timestamps"),
    cat: Cat::Display,
    section: "chat messages",
    label: "timestamps",
    tip: "a time on each message",
    alias: "timestamps clock time",
    ctl: pill!(timestamps),
    fx: Fx::Look,
    wire: Wire::Bool("timestamps"),
  },
  Def {
    id: "time_fmt",
    cmd: Some("/timeformat"),
    cat: Cat::Display,
    section: "chat messages",
    label: "timestamp format",
    tip: "clock format for chat row timestamps: 24h (16:05) or 12h (4:05pm). /timeformat 12h",
    alias: "timestampFormat 12h 24h clock am pm",
    ctl: pick!(time_fmt, TimeFmt),
    fx: Fx::Look,
    wire: Wire::Name("timestampFormat"),
  },
  Def {
    id: "timestamp_seconds",
    cmd: Some("/timeseconds"),
    cat: Cat::Display,
    section: "chat messages",
    label: "timestamp seconds",
    tip: "seconds on the clock: 16:05:09, or 4:05:09pm in 12h. stays on this computer. /timeseconds",
    alias: "seconds hh:mm:ss clock time precise",
    ctl: pill!(timestamp_seconds),
    fx: Fx::Look,
    wire: Wire::None,
  },
  Def {
    id: "zebra",
    cmd: Some("/zebra"),
    cat: Cat::Display,
    section: "chat messages",
    label: "zebra striping",
    tip: "alternating row shading",
    alias: "zebra stripes rows",
    ctl: pill!(zebra),
    fx: Fx::Look,
    wire: Wire::Bool("zebra"),
  },
  Def {
    id: "readable",
    cmd: Some("/readable"),
    cat: Cat::Display,
    section: "chat messages",
    label: "fix dim usernames",
    tip: "brighten dim username colors so they're readable on the black bg",
    alias: "hs_readable_names readablenames contrast",
    ctl: pill!(readable),
    fx: Fx::Look,
    wire: Wire::None,
  },
  Def {
    id: "platform_tags",
    cmd: Some("/platformtags"),
    cat: Cat::Display,
    section: "chat messages",
    label: "platform tags",
    tip: "[T] [K] [Y] labels on messages in tabs that mix platforms",
    alias: "showPlatformBadges platform badges",
    ctl: pill!(platform_tags),
    fx: Fx::Look,
    wire: Wire::Bool("showPlatformBadges"),
  },
  Def {
    id: "avatars",
    cmd: Some("/avatars"),
    cat: Cat::Display,
    section: "chat messages",
    label: "profile pictures",
    tip: "show each person's picture next to their name",
    alias: "avatars profile pictures",
    ctl: pill!(avatars),
    fx: Fx::Look,
    wire: Wire::Bool("avatars"),
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
    wire: Wire::Bool("sevenTvPaints"),
  },
  Def {
    id: "hs_paints",
    cmd: Some("/hspaints"),
    cat: Cat::Display,
    section: "cosmetics",
    label: "heatsync name paints",
    tip: "animated name colors from heatsync. beats a 7tv paint when a chatter has both",
    alias: "showNamePaints heatsync paints",
    ctl: pill!(hs_paints),
    fx: Fx::Paints,
    wire: Wire::Bool("showNamePaints"),
  },
  Def {
    id: "render_inventory",
    cmd: Some("/renderinventory"),
    cat: Cat::Display,
    section: "cosmetics",
    label: "show heatsync emotes",
    tip: "draw the emotes people own on heatsync. off = the plain word. channel and global emotes stay on",
    alias: "renderInventoryEmotes heatsync emotes",
    ctl: pill!(render_inventory),
    fx: Fx::Look,
    wire: Wire::Bool("renderInventoryEmotes"),
  },
  Def {
    id: "personal_emotes",
    cmd: Some("/personalemotes"),
    cat: Cat::Display,
    section: "cosmetics",
    label: "7tv personal emotes",
    tip: "draw the personal emotes 7tv subscribers picked, in their own lines. asks 7tv about chatters in your open channels; nothing about you goes with it",
    alias: "personal emote set seventv 7tv subscriber",
    ctl: pill!(personal_emotes),
    fx: Fx::Personal,
    wire: Wire::None,
  },
  Def {
    id: "third_party_badges",
    cmd: Some("/thirdpartybadges"),
    cat: Cat::Display,
    section: "cosmetics",
    label: "third-party badges",
    tip: "betterttv, frankerfacez and chatterino badges on chatters. a channel's own mod and vip badges stay either way",
    alias: "thirdPartyBadges bttvBadges ffzBadges chatterinoBadges betterttv frankerfacez bttv ffz chatterino",
    ctl: pill!(third_party_badges),
    fx: Fx::Look,
    wire: Wire::Bool("thirdPartyBadges"),
  },
  Def {
    id: "sub_flair",
    cmd: Some("/subflair"),
    cat: Cat::Display,
    section: "cosmetics",
    label: "sub flair",
    tip: "a small sub badge before a chatter's name, the one they picked on heatsync. twitch lines only. asks heatsync about the names on screen",
    alias: "flair sub badge",
    ctl: pill!(sub_flair),
    fx: Fx::Look,
    wire: Wire::Bool("subFlair"),
  },
  Def {
    id: "flame",
    cmd: Some("/flame"),
    cat: Cat::Display,
    section: "cosmetics",
    label: "heatsync user flame",
    tip: "a flame before the name of a chatter you have seen use heatsync emotes. twitch lines only, never your own. a heat bar replaces it",
    alias: "fire marker hsflame",
    ctl: pill!(flame),
    fx: Fx::Look,
    wire: Wire::Bool("flame"),
  },
  Def {
    id: "heat_bars",
    cmd: Some("/heatbars"),
    cat: Cat::Display,
    section: "cosmetics",
    label: "heat bars",
    tip: "a thin bar before a chatter's name, thicker as their heat climbs: white from 5, yellow from 50, red from 1000. twitch lines only. asks heatsync about the names on screen",
    alias: "heat score border",
    ctl: pill!(heat_bars),
    fx: Fx::Look,
    wire: Wire::Bool("heatBars"),
  },
  Def {
    id: "animate_emotes",
    cmd: Some("/animateemotes"),
    cat: Cat::Display,
    section: "cosmetics",
    label: "animate emotes",
    tip: "emotes and name paints move. never = still pictures, saves cpu. /animateemotes never",
    alias: "animateEmotes gif motion static reduced paints animatePaints",
    ctl: pick!(animate_emotes, Animate),
    fx: Fx::Look,
    wire: Wire::Name("animateEmotes"),
  },
  Def {
    id: "pronouns",
    cmd: Some("/pronouns"),
    cat: Cat::Display,
    section: "cosmetics",
    label: "pronouns",
    tip: "pronoundb.org pronouns on the user card + name popup (twitch only)",
    alias: "hs_show_pronouns pronoundb",
    ctl: pill!(pronouns),
    fx: Fx::None,
    wire: Wire::Bool("hs_show_pronouns"),
  },
  Def {
    id: "line_height",
    cmd: Some("/lineheight"),
    cat: Cat::Display,
    section: "density",
    label: "line height",
    tip: "chat row line height: 20 is tight, 22 and 26 add room between lines. /lineheight 22",
    alias: "lineHeight line spacing",
    ctl: pick!(line_height, LineHeight),
    fx: Fx::Look,
    wire: Wire::Name("lineHeight"),
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
    wire: Wire::Name("tabPosition"),
  },
  Def {
    id: "tab_counter",
    cmd: Some("/tabcounter"),
    cat: Cat::Display,
    section: "layout",
    label: "tab counter",
    tip: "the number after a channel's name: unread since you looked, heat (messages in the last minute), or off. /tabcounter heat",
    alias: "tabCounter unread heat count number",
    ctl: Ctl::Counter,
    fx: Fx::None,
    wire: Wire::Name("tabCounter"),
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
    wire: Wire::Bool("viMode"),
  },
  Def {
    id: "auto_hide",
    cmd: Some("/autohide"),
    cat: Cat::Chat,
    section: "input",
    label: "auto-hide input",
    tip: "hides the input bar when idle",
    alias: "autohide",
    ctl: pill!(auto_hide),
    fx: Fx::None,
    wire: Wire::Bool("autoHideEmpty"),
  },
  Def {
    id: "suggest_inventory",
    cmd: Some("/suggestinventory"),
    cat: Cat::Chat,
    section: "input",
    label: "suggest your heatsync emotes",
    tip: "offer your heatsync emotes while you type and on tab. off leaves only channel and global emotes",
    alias: "suggestInventoryEmotes complete",
    ctl: pill!(suggest_inventory),
    fx: Fx::None,
    wire: Wire::Bool("suggestInventoryEmotes"),
  },
  Def {
    id: "my_commands",
    cmd: None,
    cat: Cat::Chat,
    section: "input",
    label: "your commands",
    tip: "your own slash commands: /gg good game {1}. {1} {2} are words you type, {1+} the rest, {input} everything, {channel}, {user}. enter opens the list: a adds, x removes. a built-in command with the same name always wins",
    alias: "custom commands aliases macros shortcuts snippets expand",
    ctl: Ctl::Commands,
    fx: Fx::None,
    wire: Wire::None,
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
    wire: Wire::Bool("mediaEmbedsEnabled"),
  },
  Def {
    id: "links",
    cmd: Some("/links"),
    cat: Cat::Chat,
    section: "messages",
    label: "clickable links",
    tip: "urls in chat become clickable",
    alias: "linksEnabled urls",
    ctl: pill!(links),
    fx: Fx::Links,
    wire: Wire::Bool("linksEnabled"),
  },
  Def {
    id: "dim_timeouts",
    cmd: Some("/dimtimeouts"),
    cat: Cat::Chat,
    section: "messages",
    label: "dim timed-out messages",
    tip: "a timed-out or banned person's lines go dim and lose their pictures. off leaves them as they were",
    alias: "hs_dim_timeouts dimtimeouts timeout ban",
    ctl: pill!(dim_timeouts),
    fx: Fx::Look,
    wire: Wire::None,
  },
  Def {
    id: "collapse_timeouts",
    cmd: Some("/collapsetimeouts"),
    cat: Cat::Chat,
    section: "messages",
    label: "fold timeout runs",
    tip: "timeouts and bans in a row fold into one line, 5 people timed out. click or enter opens it. a chat line between them ends the run. off shows each one",
    alias: "combine stack timeouts bans",
    ctl: pill!(collapse_timeouts),
    fx: Fx::None,
    wire: Wire::None,
  },
  Def {
    id: "hover_reply",
    cmd: Some("/hoverreply"),
    cat: Cat::Chat,
    section: "messages",
    label: "reply button on hover",
    tip: "a \u{21aa} button on the line under the pointer: one click starts a reply, same as reply in the line menu",
    alias: "hoverReply reply hover button",
    ctl: pill!(hover_reply),
    fx: Fx::None,
    wire: Wire::Bool("hoverReply"),
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
    wire: Wire::None,
  },
  Def {
    id: "chat_logs",
    cmd: Some("/chatlogs"),
    cat: Cat::System,
    section: "privacy",
    label: "save chat logs",
    tip: "saved on this computer, one file a day per channel. never uploaded, whispers never saved",
    alias: "log logging history save disk record",
    ctl: pill!(chat_logs),
    fx: Fx::ChatLogs,
    wire: Wire::None,
  },
  Def {
    id: "notify",
    cmd: Some("/notify"),
    cat: Cat::Notifs,
    section: "when you get @mentioned",
    label: "desktop notifications",
    tip: "show a desktop notification when someone @s you, or a channel goes live or switches game, and the window is not focused",
    alias: "hs_notifications browser notification",
    ctl: pill!(notify),
    fx: Fx::None,
    wire: Wire::None,
  },
  Def {
    id: "mention_volume",
    cmd: Some("/mentionsound"),
    cat: Cat::Notifs,
    section: "when you get @mentioned",
    label: "mention sound volume",
    tip: "a ping when someone @s you and the window is not focused. 0 = silent. /mentionsound 50",
    alias: "mentionSoundVolume ping audio sound volume",
    ctl: pick!(mention_volume, Volume),
    fx: Fx::None,
    wire: Wire::Level("mentionSoundVolume"),
  },
  Def {
    id: "ping_sound",
    cmd: Some("/pingsound"),
    cat: Cat::Notifs,
    section: "when you get @mentioned",
    label: "mention sound",
    tip: "which sound plays when someone @s you: ping, blip, knock, chime, or custom (your own wav file, set in custom sound file). you hear it as you pick. stays on this computer. /pingsound chime",
    alias: "ping audio tone sound chime blip knock wav custom",
    ctl: pick!(ping_sound, PingSound),
    fx: Fx::Ping,
    wire: Wire::None,
  },
  Def {
    id: "ping_file",
    cmd: Some("/pingfile"),
    cat: Cat::Notifs,
    section: "when you get @mentioned",
    label: "custom sound file",
    tip: "a .wav file, up to 2mb and 3 seconds. only plays when mention sound is custom, and picking a file turns custom on. off = the ping again",
    alias: "ping audio sound wav file path custom",
    ctl: Ctl::Text(&PING_FILE),
    fx: Fx::PingFile,
    wire: Wire::None,
  },
  Def {
    id: "mention_rules",
    cmd: None,
    cat: Cat::Notifs,
    section: "when you get @mentioned",
    label: "mention rules",
    tip: "words the server watches for in every channel and pings you with here, like an @. enter opens the list: a adds, enter turns one on or off, x removes. needs a heatsync login",
    alias: "alerts mention-rules keyword watch ping server",
    ctl: Ctl::Alerts,
    fx: Fx::None,
    wire: Wire::None,
  },
  Def {
    id: "title_flash",
    cmd: Some("/titleflash"),
    cat: Cat::Notifs,
    section: "when the window is away",
    label: "title flash",
    tip: "the window title flashes who @'d you until you come back to the window",
    alias: "mentionTitleFlash tab title flash",
    ctl: pill!(title_flash),
    fx: Fx::None,
    wire: Wire::Bool("mentionTitleFlash"),
  },
  Def {
    id: "taskbar_flash",
    cmd: Some("/taskbarflash"),
    cat: Cat::Notifs,
    section: "when the window is away",
    label: "taskbar flash",
    tip: "your taskbar entry lights up when someone @s you and the window is not in front. muted channels and streamer mode hold it back, like the ping. stays on this computer",
    alias: "urgent attention taskbar tag highlight blink dock",
    ctl: pill!(taskbar_flash),
    fx: Fx::None,
    wire: Wire::None,
  },
  Def {
    id: "streamer",
    cmd: Some("/streamer"),
    cat: Cat::Notifs,
    section: "streamer mode",
    label: "streamer mode",
    tip: "hides whisper text, link previews and popup text, and mutes sounds, so nothing private shows on stream. auto = on while obs is running. space s shows the whispers tab for this session",
    alias: "obs stream privacy hide whisper dm live",
    ctl: pick!(streamer, Streamer),
    fx: Fx::Streamer,
    wire: Wire::None,
  },
  Def {
    id: "streamer_sounds",
    cmd: Some("/streamersounds"),
    cat: Cat::Notifs,
    section: "streamer mode",
    label: "keep sounds",
    tip: "the mention ping and rule sounds still play while streamer mode is on",
    alias: "streamer mode sound mute",
    ctl: pill!(streamer_sounds),
    fx: Fx::None,
    wire: Wire::None,
  },
  Def {
    id: "whisper_toast",
    cmd: Some("/whispertoast"),
    cat: Cat::Notifs,
    section: "whispers",
    label: "whisper toast",
    tip: "popup toast when a whisper or dm arrives while you are on another tab — click it to jump to whispers",
    alias: "whisperToast dm popup",
    ctl: pill!(whisper_toast),
    fx: Fx::None,
    wire: Wire::Bool("whisperToast"),
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
    wire: Wire::Flag("hermesEvents", "online"),
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
    wire: Wire::Flag("hermesEvents", "offline"),
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
    wire: Wire::Flag("hermesEvents", "gameSwitch"),
  },
  Def {
    id: "ev_raid",
    cmd: Some("/raids"),
    cat: Cat::Notifs,
    section: "stream events",
    label: "raids",
    tip: "line when a channel starts a raid",
    alias: "raid",
    ctl: pill!(ev_raid),
    fx: Fx::None,
    wire: Wire::Flag("hermesEvents", "raid"),
  },
  Def {
    id: "ev_sub",
    cmd: Some("/giftbombs"),
    cat: Cat::Notifs,
    section: "stream events",
    label: "gift bombs",
    tip: "a line when someone gifts a batch of subs",
    alias: "sub gift subs",
    ctl: pill!(ev_sub),
    fx: Fx::None,
    wire: Wire::Flag("hermesEvents", "sub"),
  },
  Def {
    id: "ev_redeem",
    cmd: Some("/redeems"),
    cat: Cat::Notifs,
    section: "stream events",
    label: "redeems",
    tip: "line when someone redeems a channel-point reward",
    alias: "redeem channel points rewards",
    ctl: pill!(ev_redeem),
    fx: Fx::None,
    wire: Wire::Flag("hermesEvents", "redeem"),
  },
  Def {
    id: "ev_pred",
    cmd: Some("/predbanner"),
    cat: Cat::Notifs,
    section: "stream events",
    label: "prediction banner",
    tip: "a line under the tabs while a twitch prediction is open; click it to see it all",
    alias: "pred predictions poll",
    ctl: pill!(ev_pred),
    fx: Fx::None,
    wire: Wire::Flag("hermesEvents", "pred"),
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
    wire: Wire::Flag("inlineNotifs", "moment"),
  },
  Def {
    id: "mod_buttons",
    cmd: Some("/modbuttons"),
    cat: Cat::Mod,
    section: "mod toolbar",
    label: "mod toolbar buttons",
    tip: "pick which buttons show on a line: delete, timeouts, ban, unban",
    alias: "hs_mod_toolbar_buttons hover delete timeout purge ban unban",
    ctl: Ctl::Buttons(|s| s.mod_buttons, |s, m| s.mod_buttons = m),
    fx: Fx::None,
    wire: Wire::None,
  },
  Def {
    id: "confirm_ban",
    cmd: Some("/confirmban"),
    cat: Cat::Mod,
    section: "mod toolbar",
    label: "confirm before ban",
    tip: "ask before a permanent ban goes out: guards against a misclick (ban reasons ask too)",
    alias: "modConfirmBan",
    ctl: pill!(confirm_ban),
    fx: Fx::None,
    wire: Wire::Bool("modConfirmBan"),
  },
  Def {
    id: "ban_reasons",
    cmd: Some("/banreason"),
    cat: Cat::Mod,
    section: "mod toolbar",
    label: "ban reasons",
    tip: "quick reasons offered when you ban (a ban then always asks first). enter opens the list: a adds, x removes",
    alias: "modBanReasons reason presets",
    ctl: Ctl::Reasons,
    fx: Fx::None,
    wire: Wire::None,
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
    wire: Wire::Bool("hideBots"),
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
    wire: Wire::Bool("hideCommands"),
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
    wire: Wire::Bool("hideDuplicates"),
  },
  Def {
    id: "filter_rules",
    cmd: Some("/rule"),
    cat: Cat::Filters,
    section: "rules",
    label: "filter rules",
    tip: "rules that hide or highlight lines, by word or by user, badge and more. the first hide wins. enter opens the list: a adds (hide spoilers), x removes",
    alias: "chatFilterRules rule keywordhighlights highlight words hidden words mute keywords automodRegex pattern regex hideword",
    ctl: Ctl::Rules,
    fx: Fx::None,
    wire: Wire::None,
  },
  Def {
    id: "people",
    cmd: None,
    cat: Cat::Filters,
    section: "people",
    label: "muted & blocked people",
    tip: "everyone you muted, blocked or hid. enter opens the list: x lets one back in, enter on a name opens their card",
    alias: "mutes blocks hidden users mute block unmute unblock",
    ctl: Ctl::People,
    fx: Fx::None,
    wire: Wire::None,
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
    wire: Wire::None,
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
    wire: Wire::None,
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
    wire: Wire::None,
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
    wire: Wire::None,
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
    wire: Wire::None,
  },
  Def {
    id: "tab_feed",
    cmd: None,
    cat: Cat::System,
    section: "tabs",
    label: "feed tab",
    tip: "the heatsync social feed, shown in the tab bar",
    alias: "visible tabs hiddenTabs",
    ctl: pill!(tab_feed),
    fx: Fx::Tab(Kind::Feed),
    wire: Wire::Hidden("hiddenTabs", "feed"),
  },
  Def {
    id: "tab_whispers",
    cmd: None,
    cat: Cat::System,
    section: "tabs",
    label: "whispers tab",
    tip: "your twitch whispers, shown in the tab bar",
    alias: "visible tabs hiddenTabs dm",
    ctl: pill!(tab_whispers),
    fx: Fx::Tab(Kind::Whispers),
    wire: Wire::Hidden("hiddenTabs", "whispers"),
  },
  Def {
    id: "tab_mentions",
    cmd: None,
    cat: Cat::System,
    section: "tabs",
    label: "mentions tab",
    tip: "every @mention of you, shown in the tab bar",
    alias: "visible tabs hiddenTabs",
    ctl: pill!(tab_mentions),
    fx: Fx::Tab(Kind::Mentions),
    wire: Wire::Hidden("hiddenTabs", "mentions"),
  },
  Def {
    id: "tab_modlog",
    cmd: None,
    cat: Cat::System,
    section: "tabs",
    label: "mod log tab",
    tip: "every ban, timeout, unban and delete seen in your channels, shown in the tab bar",
    alias: "visible tabs hiddenTabs moderation",
    ctl: pill!(tab_modlog),
    fx: Fx::Tab(Kind::ModLog),
    wire: Wire::Hidden("hiddenTabs", "modlog"),
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

  // a button row's picks as the settings file keeps them (the ext's ids); None when it isn't one
  pub fn listed(&self, s: &Settings) -> Option<String> {
    match self.ctl {
      Ctl::Buttons(get, _) => Some(crate::model::modbar::ids(get(s))),
      _ => None,
    }
  }

  // set a button row from the file's ids; unknown ones are dropped. false when it isn't one
  pub fn list_put(&self, s: &mut Settings, text: &str) -> bool {
    match self.ctl {
      Ctl::Buttons(_, set) => {
        set(s, crate::model::modbar::parse(text).0);
        true
      }
      _ => false,
    }
  }

  // a words row's value now; None when it isn't one
  pub fn texted<'a>(&self, s: &'a Settings) -> Option<&'a str> {
    match self.ctl {
      Ctl::Text(w) => Some((w.get)(s)),
      _ => None,
    }
  }

  // set a words row from the file or a command; false when it isn't one or the words don't fit
  pub fn text_put(&self, s: &mut Settings, words: &str) -> bool {
    match self.ctl {
      Ctl::Text(w) => (w.put)(s, words),
      _ => false,
    }
  }

  // a row that does nothing right now, so the screen draws it dim (the sound file while the sound isn't custom)
  pub fn idle(&self, s: &Settings) -> bool {
    self.id == "ping_file" && s.ping_sound != crate::model::settings::PingSound::Custom
  }

  // a Pick row's value name now
  pub fn picked(&self, s: &Settings) -> Option<&'static str> {
    match self.ctl {
      Ctl::Pick(c, get, _) => Some(c.names[get(s)]),
      _ => None,
    }
  }

  // a Pick row's value place now
  pub fn at(&self, s: &Settings) -> Option<usize> {
    match self.ctl {
      Ctl::Pick(_, get, _) => Some(get(s)),
      _ => None,
    }
  }

  // set a Pick row to value `i`; false when it isn't one or `i` is past its names
  pub fn choose(&self, s: &mut Settings, i: usize) -> bool {
    match self.ctl {
      Ctl::Pick(c, _, set) if i < c.names.len() => {
        set(s, i);
        true
      }
      _ => false,
    }
  }

  // set a Pick row by the value's name (the settings file); false when it names nothing
  pub fn pick(&self, s: &mut Settings, name: &str) -> bool {
    match self.ctl {
      Ctl::Pick(c, ..) => c.names.iter().position(|n| *n == name.trim()).is_some_and(|i| self.choose(s, i)),
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

  // a section is one run of rows under one heading: it never comes back further down the screen
  #[test]
  fn every_section_is_one_run_of_rows() {
    let mut seen: Vec<(Cat, &str)> = Vec::new();
    for d in DEFS.iter() {
      let at = (d.cat, d.section);
      if seen.last() != Some(&at) {
        assert!(!seen.contains(&at), "{} comes back: {:?}", d.id, at.1);
        seen.push(at);
      }
    }
  }

  // one home for each look switch, and the pairs that belong together sit together
  #[test]
  fn each_look_switch_has_one_home_and_pairs_sit_together() {
    let home = |id: &str| def(id).map(|d| (d.cat, d.section)).unwrap();
    for id in ["zebra", "timestamps", "read_line", "time_fmt", "timestamp_seconds"] {
      assert_eq!(home(id), (Cat::Display, "chat messages"), "{id}");
    }
    for id in ["hover_reply", "dim_timeouts"] {
      assert_eq!(home(id), (Cat::Chat, "messages"), "{id}");
    }
    let place = |id: &str| DEFS.iter().position(|d| d.id == id).unwrap();
    assert_eq!(home("zoom"), home("font_family"));
    assert_eq!(place("zoom"), place("font_family") + 1, "zoom sits next to the font");
    assert_eq!(home("auto_hide"), (Cat::Chat, "input"));
    assert_eq!(place("auto_hide"), place("vi") + 1, "auto-hide sits beside vi mode");
    assert_eq!(home("title_flash"), home("taskbar_flash"));
    assert_eq!(home("title_flash").1, "when the window is away");
    assert_ne!(home("title_flash"), home("mention_volume"));
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
  fn pick_rows_read_write_and_name_their_values() {
    let d = def("line_height").unwrap();
    let mut s = Settings::default();
    assert_eq!((d.picked(&s), d.at(&s)), (Some("20"), Some(0)));
    assert!(d.pick(&mut s, " 26 ") && d.at(&s) == Some(2));
    assert!(!d.pick(&mut s, "21") && !d.choose(&mut s, 3));
    assert!(d.on(&s).is_none() && !d.put(&mut s, true));
    assert!(def("zebra").unwrap().picked(&s).is_none());
    // every pick row's names are lowercase
    for d in DEFS.iter().filter(|d| matches!(d.ctl, Ctl::Pick(..))) {
      assert!(d.picked(&s).is_some_and(|n| n == n.to_lowercase()), "{}", d.id);
    }
    assert_eq!(def("mention_volume").unwrap().picked(&Settings::default()), Some("30"), "the ext's 0.3");
  }

  #[test]
  fn words_rows_read_write_and_refuse_what_does_not_fit() {
    let d = def("ping_file").unwrap();
    let mut s = Settings::default();
    assert_eq!(d.texted(&s), Some(""));
    assert!(d.text_put(&mut s, "  ~/s/Ding.wav "));
    assert_eq!(d.texted(&s), Some("~/s/Ding.wav"), "trimmed, case kept");
    assert!(!d.text_put(&mut s, &"x".repeat(300)), "too long");
    assert!(!d.text_put(&mut s, "a\nb"), "a line break would split the file");
    assert_eq!(d.texted(&s), Some("~/s/Ding.wav"), "a refusal changes nothing");
    assert!(d.text_put(&mut s, ""));
    assert_eq!(d.texted(&s), Some(""));
    // only a words row answers
    let z = def("zebra").unwrap();
    assert!(z.texted(&s).is_none() && !z.text_put(&mut s, "x"));
    assert!(d.on(&s).is_none() && d.picked(&s).is_none() && !d.put(&mut s, true) && !d.pick(&mut s, "x"));
    // the screen shows a path as its file name only
    let Ctl::Text(w) = d.ctl else { panic!("a words row") };
    assert_eq!((w.show)("/home/me/Sounds/Ding.wav"), "Ding.wav");
    assert_eq!((w.show)("Ding.wav"), "Ding.wav");
  }

  // a file that isn't a wav is refused before it is kept, in plain words
  #[test]
  fn the_sound_file_row_checks_the_file_first() {
    let Ctl::Text(w) = def("ping_file").unwrap().ctl else { panic!("a words row") };
    assert_eq!((w.check)("/hsc/no/such.wav"), Err("can't open that file"));
    let dir = std::env::temp_dir().join(format!("hsc-schema-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let junk = dir.join("junk.wav");
    std::fs::write(&junk, b"not a wav").unwrap();
    assert_eq!((w.check)(junk.to_str().unwrap()), Err("that isn't a wav file"));
    let good = dir.join("good.wav");
    std::fs::write(&good, crate::util::wav::encode(8000, 1, &[1, 2, 3])).unwrap();
    assert_eq!((w.check)(good.to_str().unwrap()), Ok(()));
    let _ = std::fs::remove_dir_all(&dir);
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
