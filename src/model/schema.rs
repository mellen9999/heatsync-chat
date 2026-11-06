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
  // paint motion flipped: every animated name is built again
  PaintMotion,
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
}

// how a row travels to and from the account's ui-state (model/sync.rs holds the codecs). the ext's key
// names are its storage names, never renamed; rows with none stay on this device
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Wire {
  None,
  // an on/off switch under its own key
  Bool(&'static str),
  // a named choice: the ext stores the choice's name
  Name(&'static str),
  // a switch the ext stores as "always" / "never"
  Anim(&'static str),
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
      Wire::Bool(k) | Wire::Name(k) | Wire::Anim(k) | Wire::Level(k) | Wire::Flag(k, _) | Wire::Hidden(k, _) => Some(k),
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

pub static DEFS: [Def; 71] = [
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
    id: "auto_hide",
    cmd: Some("/autohide"),
    cat: Cat::Display,
    section: "chat messages",
    label: "auto-hide input",
    tip: "hides the input bar when idle",
    alias: "autohide",
    ctl: pill!(auto_hide),
    fx: Fx::None,
    wire: Wire::Bool("autoHideEmpty"),
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
    id: "emote_size",
    cmd: Some("/emotesize"),
    cat: Cat::Display,
    section: "chat messages",
    label: "emote size",
    tip: "1x default, 2x retina, 4x huge. /emotesize 2x",
    alias: "hs_emote_size scale",
    ctl: pick!(emote_size, EmoteSize),
    fx: Fx::Look,
    wire: Wire::None,
  },
  Def {
    id: "emoji_size",
    cmd: Some("/emojisize"),
    cat: Cat::Display,
    section: "chat messages",
    label: "emoji size",
    tip: "1x native, 2x default, 4x scale unicode emoji. /emojisize 2x",
    alias: "hs_emoji_size",
    ctl: pick!(emoji_size, EmoteSize),
    fx: Fx::Look,
    wire: Wire::None,
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
    tip: "animated name paints from heatsync — takes precedence over a 7tv paint when a chatter has both",
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
    tip: "draw the emotes people own on heatsync, yours too. off shows the plain word instead. channel and global emotes stay on",
    alias: "renderInventoryEmotes heatsync emotes",
    ctl: pill!(render_inventory),
    fx: Fx::Look,
    wire: Wire::Bool("renderInventoryEmotes"),
  },
  Def {
    id: "third_party_badges",
    cmd: Some("/thirdpartybadges"),
    cat: Cat::Display,
    section: "cosmetics",
    label: "third-party badges",
    tip: "betterttv, frankerfacez and chatterino badges on chatters. a channel's own ffz mod and vip art stays either way",
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
    tip: "the twitch sub badge a chatter picked on heatsync, before their name (twitch lines only). asks heatsync about the names on screen",
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
    tip: "a flame before the name of a chatter you have seen use heatsync emotes (twitch lines only, never on your own lines). it gives way to the heat bar when they have one",
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
    tip: "a thin bar before the name of a chatter with heat on heatsync, 5 and up: white while cold, yellow warming, red from 1000, thicker as it climbs (twitch lines only). asks heatsync about the names on screen",
    alias: "heat score border",
    ctl: pill!(heat_bars),
    fx: Fx::Look,
    wire: Wire::Bool("heatBars"),
  },
  Def {
    id: "text_badges",
    cmd: Some("/textbadges"),
    cat: Cat::Display,
    section: "chat messages",
    label: "text badges",
    tip: "mod, vip, sub and the rest as short text chips (mod, vip, sub) instead of pictures",
    alias: "textBadges chips labels",
    ctl: pill!(text_badges),
    fx: Fx::Look,
    wire: Wire::Bool("textBadges"),
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
    wire: Wire::Anim("animatePaints"),
  },
  Def {
    id: "animate_emotes",
    cmd: Some("/animateemotes"),
    cat: Cat::Display,
    section: "cosmetics",
    label: "animate emotes",
    tip: "always plays animated emotes. never holds each one on its first frame, a big cpu saver on busy channels. /animateemotes never",
    alias: "animateEmotes gif motion static reduced",
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
    tip: "writes every chat line of your open channels to text files on this computer, one per channel per day, in ~/.local/share/heatsync-chat/logs. off by default, never uploaded, whispers never saved. /chatlogs",
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
    id: "title_flash",
    cmd: Some("/titleflash"),
    cat: Cat::Notifs,
    section: "when you get @mentioned",
    label: "title flash",
    tip: "the window title flashes who @'d you until you come back to the window",
    alias: "mentionTitleFlash tab title flash",
    ctl: pill!(title_flash),
    fx: Fx::None,
    wire: Wire::Bool("mentionTitleFlash"),
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
    tip: "line when the channel raids another",
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
    label: "gift subs",
    tip: "line when someone gifts a batch of subs to the channel",
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
    tip: "a line under the tabs while a twitch prediction is open or locked; click it for the whole view (/predictions)",
    alias: "pred predictions poll",
    ctl: pill!(ev_pred),
    fx: Fx::None,
    wire: Wire::Flag("hermesEvents", "pred"),
  },
  Def {
    id: "stream_stats",
    cmd: Some("/streamstats"),
    cat: Cat::Notifs,
    section: "stream events",
    label: "stream stats",
    tip: "count each channel tab's lines, chatters and emotes for /summary",
    alias: "summary subsystems",
    ctl: pill!(stream_stats),
    fx: Fx::None,
    wire: Wire::Flag("subsystems", "stream-stats"),
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
    wire: Wire::Flag("inlineNotifs", "op"),
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
    wire: Wire::Flag("inlineNotifs", "mop"),
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
    wire: Wire::Flag("inlineNotifs", "re"),
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
    id: "inline_dm",
    cmd: Some("/inlinedm"),
    cat: Cat::Notifs,
    section: "inline notifications",
    label: "[dm] whispers & dms",
    tip: "a [dm] line in the chat you are on when a whisper or dm arrives while you are on another tab",
    alias: "inline dm whisper",
    ctl: pill!(inline_dm),
    fx: Fx::None,
    wire: Wire::Flag("inlineNotifs", "dm"),
  },
  Def {
    id: "mod_buttons",
    cmd: Some("/modbuttons"),
    cat: Cat::Mod,
    section: "mod toolbar",
    label: "mod toolbar buttons",
    tip: "buttons on the line under the pointer in rooms you moderate: x 1m 10m 1h 24h 7d purge \u{26d4} \u{2713}. /modbuttons x 10m ban, /modbuttons off",
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
    tip: "rules that hide or highlight lines by word, pattern, user, badge or kind of message; the first hide wins. mute keywords, keyword highlights and hide-by-pattern are rules now. enter opens the list: a adds (hide keyword spoilers), x removes",
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
    tip: "the heatsync social feed in the tab bar. /show feed, /hide feed",
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
    tip: "your twitch whispers in the tab bar. /show whispers, /hide whispers",
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
    tip: "every @mention of you in the tab bar. /show mentions, /hide mentions",
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
    tip: "every ban, timeout, unban and delete seen in your channels. /show modlog, /hide modlog",
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
    let d = def("emote_size").unwrap();
    let mut s = Settings::default();
    assert_eq!((d.picked(&s), d.at(&s)), (Some("1x"), Some(0)));
    assert!(d.pick(&mut s, " 4x ") && d.at(&s) == Some(2));
    assert!(!d.pick(&mut s, "3x") && !d.choose(&mut s, 3));
    assert!(d.on(&s).is_none() && !d.put(&mut s, true));
    assert!(def("zebra").unwrap().picked(&s).is_none());
    // every pick row's names are lowercase
    for d in DEFS.iter().filter(|d| matches!(d.ctl, Ctl::Pick(..))) {
      assert!(d.picked(&s).is_some_and(|n| n == n.to_lowercase()), "{}", d.id);
    }
    assert_eq!(def("mention_volume").unwrap().picked(&Settings::default()), Some("30"), "the ext's 0.3");
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
