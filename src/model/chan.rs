// add/edit/remove-channel rules, ported from the ext's channel-mgmt (channelAddError, the form's
// charset gates, tab-registry reserved ids). pure: no ui, no net
use super::ident;
use crate::i18n::t;
use crate::model::line::Line;

// ids a channel may never take: the ext's page tabs, plus this app's synthetic flood tab
pub const RESERVED: [&str; 10] =
  ["feed", "discover", "whispers", "mentions", "pinned", "modlog", "live", "add", "settings", "flood"];

// site pages that sit where a channel's name does (the ext's MC_SITE_PATHS): a pasted
// twitch.tv/directory or kick.com/browse is refused, not saved as a channel called "directory"
const SITE_PATHS: &[&str] = &[
  "directory",
  "settings",
  "p",
  "downloads",
  "jobs",
  "turbo",
  "subscriptions",
  "inventory",
  "wallet",
  "drops",
  "friends",
  "messages",
  "search",
  "videos",
  "video",
  "clip",
  "clips",
  "categories",
  "category",
  "browse",
  "dashboard",
  "terms",
  "privacy",
  "community-guidelines",
  "popout",
  "embed",
  "moderator",
  "u",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Err {
  Empty,
  Invalid,
  YtHandle,
  Reserved,
  TwitchExists,
  KickExists,
  YoutubeExists,
}

impl Err {
  // plain words, the ext's locale strings where it has one
  pub fn msg(self) -> &'static str {
    match self {
      Err::Empty => t("chat1.chan.err.empty"),
      Err::Invalid => t("chat1.chan.err.invalid"),
      Err::YtHandle => t("chat1.chan.err.yt_handle"),
      Err::Reserved => t("chat1.chan.err.reserved"),
      Err::TwitchExists => t("chat1.chan.err.twitch_exists"),
      Err::KickExists => t("chat1.chan.err.kick_exists"),
      Err::YoutubeExists => t("chat1.chan.err.youtube_exists"),
    }
  }
}

// one tab's feeds, cleaned: twitch login, kick slug, youtube `@handle` (each empty when absent)
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Spec {
  pub twitch: String,
  pub kick: String,
  pub youtube: String,
}

fn after<'a>(s: &'a str, host: &str) -> Option<&'a str> {
  let i = s.to_ascii_lowercase().find(host)?;
  Some(&s[i + host.len()..])
}

fn first_segment(s: &str) -> &str {
  s.split(['/', '?', '#']).next().unwrap_or("")
}

// the word before the name in a link that is not the channel page (popout/name/chat, embed/name/chat...)
fn step_over<'a>(rest: &'a str, words: &[&str]) -> &'a str {
  match rest.split_once('/') {
    Some((head, tail)) if words.iter().any(|w| head.eq_ignore_ascii_case(w)) => tail,
    _ => rest,
  }
}

// the channel= of a player.twitch.tv/?channel=name link
fn player_channel(v: &str) -> Option<&str> {
  let rest = after(v, "player.twitch.tv/")?;
  let query = rest.strip_prefix('?')?.split('#').next()?;
  query.split('&').find_map(|kv| kv.split_once('=').filter(|(k, _)| k.eq_ignore_ascii_case("channel")).map(|(_, n)| n))
}

// a login or a twitch.tv link (channel page, popout, embed, moderator view, dashboard, the bare
// player), lowercased
pub fn twitch_login(raw: &str) -> String {
  let v = raw.trim().trim_start_matches(['@', '#']);
  let v = match (player_channel(v), after(v, "twitch.tv/")) {
    (Some(name), _) => first_segment(name),
    (None, Some(rest)) => first_segment(step_over(rest, &["popout", "embed", "moderator", "u"])),
    (None, None) => v,
  };
  v.to_ascii_lowercase()
}

// a slug or a kick.com link (channel page, popout, embed, the player), lowercased
pub fn kick_slug(raw: &str) -> String {
  let v = raw.trim().trim_start_matches('@');
  let v = after(v, "kick.com/").map_or(v, |rest| first_segment(step_over(rest, &["popout", "embed"])));
  v.to_ascii_lowercase()
}

// a bare handle, @handle, or a youtube.com/@handle link -> `@handle` (lowercase). a channel id
// (`UC` + 22), bare or from a youtube.com/channel/UC... link, is kept as typed: it is case-sensitive and
// goes upstream as it is (the server's shared/yt-channel-id.ts). anything else that looks like one is refused
pub fn yt_key(raw: &str) -> Result<String, Err> {
  let v = raw.trim();
  if v.is_empty() {
    return Ok(String::new());
  }
  let v = match after(v, "youtube.com/") {
    Some(rest) => {
      let mut segs = rest.split(['/', '?', '#']);
      match (segs.next().unwrap_or(""), segs.next()) {
        (seg, _) if seg.starts_with('@') => seg,
        (kind, Some(id)) if kind.eq_ignore_ascii_case("channel") && ident::yt_channel_id(id) => return Ok(id.into()),
        _ => return Err(Err::YtHandle),
      }
    }
    None => v,
  };
  let name = v.trim_start_matches('@');
  if ident::yt_channel_id(name) {
    return Ok(name.into());
  }
  if !ident::yt_handle(name) {
    return Err(Err::Invalid);
  }
  if name.len() >= 22 && name.starts_with("UC") {
    return Err(Err::YtHandle);
  }
  Ok(format!("@{}", name.to_ascii_lowercase()))
}

fn bare(s: &str) -> &str {
  s.trim_start_matches('@')
}

impl Spec {
  // clean what was typed and apply the charset gates (twitch [a-z0-9_], kick adds `-`, 1-25)
  pub fn parse(twitch: &str, kick: &str, youtube: &str) -> Result<Spec, Err> {
    let (twitch, kick) = (twitch_login(twitch), kick_slug(kick));
    let youtube = yt_key(youtube)?;
    if twitch.is_empty() && kick.is_empty() && youtube.is_empty() {
      return Err(Err::Empty);
    }
    let site = |n: &str| SITE_PATHS.contains(&n);
    if !(twitch.is_empty() || ident::twitch_room(&twitch) && !site(&twitch))
      || !(kick.is_empty() || ident::kick_slug(&kick) && !site(&kick))
    {
      return Err(Err::Invalid);
    }
    Ok(Spec { twitch, kick, youtube })
  }

  // the tab's name: its first feed (`forsen`, `kick:slug`, `yt:@handle`)
  pub fn name(&self) -> String {
    self.tokens().into_iter().next().unwrap_or_default()
  }

  // the spec of feeds already running, dropping any that would not survive a restart
  // (a heatsync link can hand over keys the form would refuse)
  pub fn lenient(twitch: &str, kick: &str, youtube: &str) -> Spec {
    let t = Spec::parse(twitch, "", "").map(|s| s.twitch).unwrap_or_default();
    let k = Spec::parse("", kick, "").map(|s| s.kick).unwrap_or_default();
    let y = Spec::parse("", "", youtube).map(|s| s.youtube).unwrap_or_default();
    Spec { twitch: t, kick: k, youtube: y }
  }

  fn tokens(&self) -> Vec<String> {
    let mut t = Vec::with_capacity(3);
    if !self.twitch.is_empty() {
      t.push(self.twitch.clone());
    }
    if !self.kick.is_empty() {
      t.push(format!("kick:{}", self.kick));
    }
    if !self.youtube.is_empty() {
      t.push(format!("yt:{}", self.youtube));
    }
    t
  }

  // the saved form of a tab: its feeds, space separated. a lone name is the old format
  pub fn line(&self) -> String {
    self.tokens().join(" ")
  }

  // parse a saved line, an argv word, or the words after /join. a bare word is twitch
  pub fn from_line(line: &str) -> Result<Spec, Err> {
    let (mut tw, mut ki, mut yt) = (None, None, None);
    for w in line.split_whitespace() {
      let (slot, v) = match w.split_once(':') {
        Some(("kick", v)) => (&mut ki, v),
        Some(("yt", v)) => (&mut yt, v),
        Some(_) if !w.contains("://") => return Err(Err::Invalid),
        _ => (&mut tw, w),
      };
      if slot.replace(v).is_some() {
        return Err(Err::Invalid);
      }
    }
    Spec::parse(tw.unwrap_or(""), ki.unwrap_or(""), yt.unwrap_or(""))
  }

  // why this can't join `have` (the other tabs), or Ok. same rules the ext's channelAddError
  // keeps in one place: reserved ids first, then one tab per feed
  pub fn check(&self, have: &[Spec]) -> Result<(), Err> {
    let id = if self.twitch.is_empty() { &self.kick } else { &self.twitch };
    if !id.is_empty() && RESERVED.contains(&id.as_str()) {
      return Err(Err::Reserved);
    }
    if !self.twitch.is_empty() && have.iter().any(|h| h.twitch == self.twitch) {
      return Err(Err::TwitchExists);
    }
    if !self.kick.is_empty() && have.iter().any(|h| h.kick == self.kick) {
      return Err(Err::KickExists);
    }
    if !self.youtube.is_empty() && have.iter().any(|h| bare(&h.youtube) == bare(&self.youtube)) {
      return Err(Err::YoutubeExists);
    }
    Ok(())
  }
}

// `/join [feeds…]`, `/part`, `/watch`
#[derive(Debug, PartialEq, Eq)]
pub enum Cmd {
  // None = open the form
  Join(Option<String>),
  Part,
  // save the temporary tab
  Keep,
  // this tab's stream, in mpv or the browser
  Watch,
  // copy the whole lineup as a heatsync.org/m/ link
  Share,
  // open the tabs of a pasted link (None = say how)
  Open(Option<String>),
}

pub fn command(text: &str) -> Option<Cmd> {
  let t = text.trim();
  let (head, rest) = t.split_once(char::is_whitespace).unwrap_or((t, ""));
  match head.to_ascii_lowercase().as_str() {
    "/join" => Some(Cmd::Join((!rest.trim().is_empty()).then(|| rest.trim().to_owned()))),
    "/part" if rest.trim().is_empty() => Some(Cmd::Part),
    "/keep" if rest.trim().is_empty() => Some(Cmd::Keep),
    "/watch" if rest.trim().is_empty() => Some(Cmd::Watch),
    // the ext's names; /share and /open stay as aliases
    "/sharelayout" | "/sl" | "/share" if rest.trim().is_empty() => Some(Cmd::Share),
    "/layout" | "/open" => Some(Cmd::Open((!rest.trim().is_empty()).then(|| rest.trim().to_owned()))),
    _ => None,
  }
}

// a button in the edit form: what the tab menu used to hold besides edit and remove
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Act {
  // play the stream of the tab's nth channel
  Watch(usize),
  // its twitch channel's stream and chat modes (/status)
  Status,
  Pred,
  // its stream stats (/summary)
  Summary,
}

// the add/edit form: three fields (twitch, kick, youtube), which one has focus, the last error
pub struct Form {
  pub fields: [Line; 3],
  pub focus: usize,
  pub err: Option<&'static str>,
  // the first window of a run with no channels: a title and a line that say what to do
  pub welcome: bool,
  // Some(tab index) while editing
  pub edit: Option<usize>,
  // edit only: the buttons under the fields. focus 3.. lands on them (vim keys move there)
  pub acts: Vec<(Act, String)>,
}

impl Form {
  pub fn new(edit: Option<usize>, spec: &Spec) -> Self {
    let mut fields: [Line; 3] = Default::default();
    for (f, v) in fields.iter_mut().zip([&spec.twitch, &spec.kick, &spec.youtube]) {
      f.set_text(v.clone());
      f.end();
    }
    Self { fields, focus: 0, err: None, welcome: false, edit, acts: Vec::new() }
  }

  pub fn cycle(&mut self, back: bool) {
    let n = 3 + self.acts.len();
    self.focus = (self.focus + if back { n - 1 } else { 1 }) % n;
  }

  // the focused button, if focus is on one
  pub fn act(&self) -> Option<Act> {
    self.focus.checked_sub(3).and_then(|i| self.acts.get(i)).map(|a| a.0)
  }

  pub fn spec(&self) -> Result<Spec, Err> {
    Spec::parse(&self.fields[0].text, &self.fields[1].text, &self.fields[2].text)
  }

  // validate against the other tabs
  pub fn submit(&self, others: &[Spec]) -> Result<Spec, Err> {
    let s = self.spec()?;
    s.check(others)?;
    Ok(s)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn sp(t: &str, k: &str, y: &str) -> Spec {
    Spec::parse(t, k, y).unwrap()
  }

  #[test]
  fn cleans_names_and_links() {
    assert_eq!(twitch_login("  @XQC "), "xqc");
    assert_eq!(twitch_login("#xqc"), "xqc");
    assert_eq!(twitch_login("https://www.twitch.tv/popout/Xqc/chat?x=1"), "xqc");
    assert_eq!(kick_slug("https://kick.com/Some-One/clips"), "some-one");
    assert_eq!(yt_key("https://www.youtube.com/@Handle/live"), Ok("@handle".into()));
    assert_eq!(yt_key("handle"), Ok("@handle".into()));
    assert_eq!(yt_key(""), Ok(String::new()));
  }

  #[test]
  fn links_that_are_not_the_channel_page_give_their_channel() {
    for (link, want) in [
      ("https://www.twitch.tv/popout/xQc/chat?popout=", "xqc"),
      ("https://www.twitch.tv/embed/xqc/chat?parent=x", "xqc"),
      ("https://www.twitch.tv/moderator/xqc", "xqc"),
      ("https://dashboard.twitch.tv/u/xqc/stream-manager", "xqc"),
      ("https://player.twitch.tv/?channel=xQc&parent=example.com", "xqc"),
      ("player.twitch.tv/?parent=example.com&channel=xqc", "xqc"),
      ("https://www.twitch.tv/xqc", "xqc"),
    ] {
      assert_eq!(Spec::parse(link, "", "").map(|s| s.twitch), Ok(want.into()), "{link}");
    }
    for (link, want) in [
      ("https://kick.com/popout/Sxb/chat", "sxb"),
      ("https://kick.com/embed/sxb", "sxb"),
      ("kick.com/sxb/chatroom", "sxb"),
      ("https://player.kick.com/sxb", "sxb"),
    ] {
      assert_eq!(Spec::parse("", link, "").map(|s| s.kick), Ok(want.into()), "{link}");
    }
  }

  #[test]
  fn a_link_to_a_site_page_is_refused() {
    for link in [
      "https://www.twitch.tv/directory",
      "twitch.tv/videos/2301234567",
      "https://www.twitch.tv/popout",
      "https://www.twitch.tv/settings/profile",
      "https://player.twitch.tv/?channel=directory",
      "directory",
    ] {
      assert_eq!(Spec::parse(link, "", ""), Err(Err::Invalid), "{link}");
    }
    for link in ["https://kick.com/browse", "kick.com/category/just-chatting", "https://kick.com/popout", "categories"]
    {
      assert_eq!(Spec::parse("", link, ""), Err(Err::Invalid), "{link}");
    }
    // a real name beside one of those words is its own name
    assert!(Spec::parse("videosgames", "clipsy", "").is_ok());
  }

  #[test]
  fn charset_gates() {
    assert_eq!(Spec::parse("", "", ""), Err(Err::Empty));
    assert_eq!(Spec::parse("a b", "", ""), Err(Err::Invalid));
    assert_eq!(Spec::parse("a-b", "", ""), Err(Err::Invalid));
    assert!(Spec::parse("", "a-b", "").is_ok());
    assert_eq!(Spec::parse(&"a".repeat(26), "", ""), Err(Err::Invalid));
    assert_eq!(Spec::parse("", "", "ab"), Err(Err::Invalid));
    // a channel id is kept exactly, bare or from its link; the near misses are refused
    const UC: &str = "UC8yh-UI81uUteMDSOwQyx1g";
    for typed in [
      UC,
      &format!("@{UC}"),
      &format!("https://www.youtube.com/channel/{UC}"),
      &format!("youtube.com/Channel/{UC}/live?x=1"),
    ] {
      assert_eq!(yt_key(typed), Ok(UC.into()), "{typed}");
    }
    assert_eq!(Spec::parse("", "", UC).unwrap().name(), format!("yt:{UC}"));
    assert_eq!(Spec::from_line(&format!("yt:{UC}")), Spec::parse("", "", UC));
    assert_eq!(Spec::parse("", "", "UCabcdefghijklmnopqrstuvw"), Err(Err::YtHandle), "25 chars");
    assert_eq!(Spec::parse("", "", "UCabcdefghijklmnopqrstu"), Err(Err::YtHandle), "23 chars");
    assert_eq!(Spec::parse("", "", "https://youtube.com/channel/UCx"), Err(Err::YtHandle));
    assert_eq!(Spec::parse("", "", "https://youtube.com/channel/"), Err(Err::YtHandle));
    assert_eq!(Spec::parse("", "", "https://youtube.com/c/name"), Err(Err::YtHandle));
    let id = Spec::parse("", "", UC).unwrap();
    assert_eq!(id.check(std::slice::from_ref(&id)), Err(Err::YoutubeExists), "one tab per channel");
  }

  #[test]
  fn reserved_and_duplicates() {
    let have = vec![sp("forsen", "forsen", ""), sp("", "", "@somebody")];
    assert_eq!(sp("live", "", "").check(&have), Err(Err::Reserved));
    assert_eq!(sp("", "modlog", "").check(&have), Err(Err::Reserved));
    assert_eq!(sp("forsen", "", "").check(&have), Err(Err::TwitchExists));
    assert_eq!(sp("", "forsen", "").check(&have), Err(Err::KickExists));
    assert_eq!(sp("", "", "somebody").check(&have), Err(Err::YoutubeExists));
    assert_eq!(sp("xqc", "xqc", "newone").check(&have), Ok(()));
    // reserved only bites the id, not a youtube handle
    assert_eq!(sp("", "", "livelive").check(&have), Ok(()));
  }

  #[test]
  fn line_roundtrip() {
    for s in [sp("forsen", "", ""), sp("", "xqc", ""), sp("a", "b", "@c33"), sp("", "k", "@yy_y"), sp("", "", "@abc")] {
      assert_eq!(Spec::from_line(&s.line()), Ok(s.clone()));
    }
    assert_eq!(sp("a", "b", "@cde").name(), "a");
    assert_eq!(sp("", "b", "@cde").name(), "kick:b");
    assert_eq!(sp("", "", "@cde").name(), "yt:@cde");
    // old saved tabs are single words
    assert_eq!(Spec::from_line("forsen"), Ok(sp("forsen", "", "")));
    assert_eq!(Spec::from_line("kick:xqc"), Ok(sp("", "xqc", "")));
    assert_eq!(Spec::from_line("yt:handle"), Ok(sp("", "", "@handle")));
    assert_eq!(Spec::from_line("a b"), Err(Err::Invalid));
    assert_eq!(Spec::from_line("nope:x"), Err(Err::Invalid));
  }

  #[test]
  fn saved_tabs_survive_a_restart() {
    let tabs = [sp("forsen", "forsen", "@fors"), sp("", "xqc", "")];
    let lines: Vec<Box<str>> = tabs.iter().map(|t| t.line().into()).collect();
    crate::store::save_tabs(&lines);
    let back: Vec<Spec> = crate::store::load_tabs().iter().filter_map(|l| Spec::from_line(l).ok()).collect();
    assert_eq!(back, tabs);
  }

  #[test]
  fn typed_commands() {
    assert_eq!(command("/join"), Some(Cmd::Join(None)));
    assert_eq!(command(" /JOIN  forsen kick:x "), Some(Cmd::Join(Some("forsen kick:x".into()))));
    assert_eq!(command("/part"), Some(Cmd::Part));
    assert_eq!(command("/keep"), Some(Cmd::Keep));
    assert_eq!(command("/watch"), Some(Cmd::Watch));
    assert_eq!(command("/share"), Some(Cmd::Share));
    assert_eq!(command("/sharelayout"), Some(Cmd::Share));
    assert_eq!(command("/sl"), Some(Cmd::Share));
    assert_eq!(command("/layout"), Some(Cmd::Open(None)));
    assert_eq!(command("/share now"), None);
    assert_eq!(command("/open"), Some(Cmd::Open(None)));
    assert_eq!(command("/OPEN t:xqc"), Some(Cmd::Open(Some("t:xqc".into()))));
    assert_eq!(command("/part now"), None);
    assert_eq!(command("/joined"), None);
    assert_eq!(command("hello /join"), None);
  }

  #[test]
  fn form_validates() {
    let mut f = Form::new(None, &Spec::default());
    assert_eq!(f.submit(&[]), Err(Err::Empty));
    f.fields[0].insert("Forsen");
    assert_eq!(f.submit(&[sp("forsen", "", "")]), Err(Err::TwitchExists));
    assert_eq!(f.submit(&[]), Ok(sp("forsen", "", "")));
    f.cycle(true);
    assert_eq!(f.focus, 2);
    f.cycle(false);
    assert_eq!(f.focus, 0);
  }
}
