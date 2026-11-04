// add/edit/remove-channel rules, ported from the ext's channel-mgmt (channelAddError, the form's
// charset gates, tab-registry reserved ids). pure: no ui, no net
use super::ident;
use crate::model::line::Line;

// ids a channel may never take: the ext's page tabs, plus this app's synthetic flood tab
pub const RESERVED: [&str; 10] =
  ["feed", "discover", "whispers", "mentions", "pinned", "modlog", "live", "add", "settings", "flood"];

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
      Err::Empty => "enter at least one platform",
      Err::Invalid => "invalid channel name — check for typos or spaces",
      Err::YtHandle => "youtube needs the @handle or its url",
      Err::Reserved => "reserved name",
      Err::TwitchExists => "twitch channel already added",
      Err::KickExists => "kick channel already added",
      Err::YoutubeExists => "youtube channel already added",
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

// a login or a twitch.tv link (popout/moderator links too), lowercased
pub fn twitch_login(raw: &str) -> String {
  let v = raw.trim().trim_start_matches(['@', '#']);
  let v = match after(v, "twitch.tv/") {
    Some(rest) => {
      let rest = rest.strip_prefix("popout/").or_else(|| rest.strip_prefix("moderator/")).unwrap_or(rest);
      first_segment(rest)
    }
    None => v,
  };
  v.to_ascii_lowercase()
}

pub fn kick_slug(raw: &str) -> String {
  let v = raw.trim().trim_start_matches('@');
  let v = after(v, "kick.com/").map_or(v, first_segment);
  v.to_ascii_lowercase()
}

// a bare handle, @handle, or a youtube.com/@handle link -> `@handle`. channel ids (UC...) are
// not handles and the relay keys on lowercase, so they are refused with a plain hint
pub fn yt_key(raw: &str) -> Result<String, Err> {
  let v = raw.trim();
  if v.is_empty() {
    return Ok(String::new());
  }
  let v = match after(v, "youtube.com/") {
    Some(rest) => {
      let seg = first_segment(rest);
      if !seg.starts_with('@') {
        return Err(Err::YtHandle);
      }
      seg
    }
    None => v,
  };
  let name = v.trim_start_matches('@');
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
    if !(twitch.is_empty() || ident::twitch_room(&twitch)) || !(kick.is_empty() || ident::kick_slug(&kick)) {
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
    "/watch" if rest.trim().is_empty() => Some(Cmd::Watch),
    "/share" if rest.trim().is_empty() => Some(Cmd::Share),
    "/open" => Some(Cmd::Open((!rest.trim().is_empty()).then(|| rest.trim().to_owned()))),
    _ => None,
  }
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
}

impl Form {
  pub fn new(edit: Option<usize>, spec: &Spec) -> Self {
    let mut fields: [Line; 3] = Default::default();
    for (f, v) in fields.iter_mut().zip([&spec.twitch, &spec.kick, &spec.youtube]) {
      f.set_text(v.clone());
      f.end();
    }
    Self { fields, focus: 0, err: None, welcome: false, edit }
  }

  pub fn cycle(&mut self, back: bool) {
    self.focus = (self.focus + if back { 2 } else { 1 }) % 3;
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
  fn charset_gates() {
    assert_eq!(Spec::parse("", "", ""), Err(Err::Empty));
    assert_eq!(Spec::parse("a b", "", ""), Err(Err::Invalid));
    assert_eq!(Spec::parse("a-b", "", ""), Err(Err::Invalid));
    assert!(Spec::parse("", "a-b", "").is_ok());
    assert_eq!(Spec::parse(&"a".repeat(26), "", ""), Err(Err::Invalid));
    assert_eq!(Spec::parse("", "", "ab"), Err(Err::Invalid));
    assert_eq!(Spec::parse("", "", "UCabcdefghijklmnopqrstuv"), Err(Err::YtHandle));
    assert_eq!(Spec::parse("", "", "https://youtube.com/channel/UCx"), Err(Err::YtHandle));
  }

  #[test]
  fn reserved_and_duplicates() {
    let have = vec![sp("forsen", "forsen", ""), sp("", "", "@somebody")];
    assert_eq!(sp("live", "", "").check(&have), Err(Err::Reserved));
    assert_eq!(sp("", "settings", "").check(&have), Err(Err::Reserved));
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
    assert_eq!(command("/watch"), Some(Cmd::Watch));
    assert_eq!(command("/share"), Some(Cmd::Share));
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
