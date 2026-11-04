// share links `https://heatsync.org/m/<layout>`: a whole lineup in one url. the grammar is the
// site's (server/lib/multichat-layout.ts), ported exactly, and parsing is total like there: junk
// is dropped, never an error. pure: no ui, no net. a pasted link is hostile, every size is bounded
//   layout := tab ('+' tab)*      '+' = one tab each
//   tab    := leg (',' leg)*      ',' = platforms merged into one tab
//   leg    := ('t'|'k'|'y') ':' name
use super::chan::{Err, Spec, yt_key};

pub const MAX_TABS: usize = 8;
pub const MAX_LEN: usize = 400;
const MAX_LEGS: usize = 3;
// the most of a pasted line we look at at all
const MAX_INPUT: usize = 2048;
const BASE: &str = "https://heatsync.org/m/";
const HOSTS: [&str; 2] = ["heatsync.org", "www.heatsync.org"];

// one tab of a layout. youtube is a lowercase handle (no @) or a case-kept UC channel id
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tab {
  pub twitch: String,
  pub kick: String,
  pub youtube: String,
}

fn name_ok(s: &str) -> bool {
  (1..=30).contains(&s.len())
    && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

fn handle_ok(s: &str) -> bool {
  (1..=40).contains(&s.len())
    && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'.' || b == b'-')
}

// 'UC' + 22 of [A-Za-z0-9_-], case matters
pub fn is_channel_id(s: &str) -> bool {
  s.len() == 24 && s.starts_with("UC") && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

// a layout segment into validated tabs: bad segments dropped, identical tabs dropped, at most 8
pub fn parse(layout: &str) -> Vec<Tab> {
  // the site measures utf-16 units
  if layout.is_empty() || layout.encode_utf16().count() > MAX_LEN {
    return Vec::new();
  }
  let mut out: Vec<Tab> = Vec::new();
  for tab_str in layout.split('+').take(MAX_TABS) {
    let mut tab = Tab::default();
    for seg in tab_str.split(',').take(MAX_LEGS) {
      let Some(i) = seg.find(':').filter(|&i| i >= 1) else { continue };
      let code = seg[..i].trim().to_lowercase();
      let exact = seg[i + 1..].trim();
      let exact = exact.strip_prefix('@').unwrap_or(exact);
      let low = exact.to_lowercase();
      match code.as_str() {
        "t" if name_ok(&low) => tab.twitch = low,
        "k" if name_ok(&low) => tab.kick = low,
        "y" if is_channel_id(exact) => tab.youtube = exact.to_owned(),
        "y" if handle_ok(&low) => tab.youtube = low,
        _ => {}
      }
    }
    if tab == Tab::default() || out.contains(&tab) {
      continue;
    }
    out.push(tab);
  }
  out
}

// one tab as its legs (twitch, kick, youtube order)
fn legs(t: &Tab) -> String {
  [("t", &t.twitch), ("k", &t.kick), ("y", &t.youtube)]
    .into_iter()
    .filter(|(_, v)| !v.is_empty())
    .map(|(c, v)| format!("{c}:{v}"))
    .collect::<Vec<_>>()
    .join(",")
}

// our tabs as a layout string (the site's buildLayout): every youtube leg here was typed in, so
// all ride; legs that would not parse back are left out. over 400 chars the whole link would parse
// to nothing, so tabs drop from the tail until it fits ("" if even one cannot)
pub fn build(specs: &[Spec]) -> String {
  let mut parts: Vec<String> = specs
    .iter()
    .map(|s| {
      let yt = s.youtube.trim_start_matches('@').to_lowercase();
      legs(&Tab {
        twitch: if name_ok(&s.twitch) { s.twitch.clone() } else { String::new() },
        kick: if name_ok(&s.kick) { s.kick.clone() } else { String::new() },
        youtube: if handle_ok(&yt) { yt } else { String::new() },
      })
    })
    .filter(|p| !p.is_empty())
    .take(MAX_TABS)
    .collect();
  loop {
    let layout = parts.join("+");
    if layout.len() <= MAX_LEN {
      return layout;
    }
    if parts.len() <= 1 {
      return String::new();
    }
    parts.pop();
  }
}

// the link to copy, or None when no tab can be shared
pub fn link(specs: &[Spec]) -> Option<String> {
  let l = build(specs);
  (!l.is_empty()).then(|| format!("{BASE}{l}"))
}

fn hex(b: u8) -> Option<u8> {
  (b as char).to_digit(16).map(|d| d as u8)
}

// %2B %2C %3A (a link that got url-encoded on the way) back to + , :. nothing else is decoded
fn unescape(s: &str) -> String {
  let b = s.as_bytes();
  let mut out = Vec::with_capacity(b.len());
  let mut i = 0;
  while i < b.len() {
    if b[i] == b'%'
      && i + 2 < b.len()
      && let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2]))
      && matches!(h * 16 + l, b'+' | b',' | b':')
    {
      out.push(h * 16 + l);
      i += 3;
    } else {
      out.push(b[i]);
      i += 1;
    }
  }
  // only ascii bytes were swapped in for ascii bytes, so this stays valid
  String::from_utf8(out).unwrap_or_default()
}

// what was pasted or typed -> the layout inside it. a heatsync.org/m/ link (query, fragment and a
// trailing slash ignored) or a bare layout like `t:xqc+k:foo`. None when it is neither
pub fn extract(input: &str) -> Option<String> {
  let t = input.trim();
  if t.is_empty() || t.len() > MAX_INPUT || t.contains(char::is_whitespace) {
    return None;
  }
  let rest = match t.split_once("://") {
    Some((scheme, rest)) if scheme.eq_ignore_ascii_case("https") || scheme.eq_ignore_ascii_case("http") => rest,
    Some(_) => return None,
    None => t,
  };
  let seg = match rest.split_once('/') {
    // a host: ours or nothing. `/m/…` alone has an empty host and is a path-only link
    Some((host, path)) if !host.is_empty() => {
      if !HOSTS.contains(&host.to_ascii_lowercase().as_str()) {
        return None;
      }
      path.strip_prefix("m/")?
    }
    Some((_, path)) => path.strip_prefix("m/")?,
    None if HOSTS.contains(&rest.to_ascii_lowercase().as_str()) => return None,
    None => rest,
  };
  let seg = unescape(seg.split(['?', '#']).next().unwrap_or("").trim_end_matches('/'));
  (seg.contains(':') && !parse(&seg).is_empty()).then_some(seg)
}

// a layout's tabs as ours: legs our form would refuse (a youtube channel id, an over-long name)
// drop. returns the tabs and how many legs were left out
pub fn to_specs(tabs: &[Tab]) -> (Vec<Spec>, usize) {
  let mut skipped = 0;
  let mut out = Vec::new();
  for t in tabs {
    let tw = Spec::parse(&t.twitch, "", "").map(|s| s.twitch).unwrap_or_default();
    let ki = Spec::parse("", &t.kick, "").map(|s| s.kick).unwrap_or_default();
    // our youtube feeds are @handles; a UC id has no handle to open
    let yt = if is_channel_id(&t.youtube) { Err(Err::YtHandle) } else { yt_key(&t.youtube) };
    let yt = yt.unwrap_or_default();
    skipped += [(&t.twitch, &tw), (&t.kick, &ki), (&t.youtube, &yt)]
      .iter()
      .filter(|(want, got)| !want.is_empty() && got.is_empty())
      .count();
    let spec = Spec { twitch: tw, kick: ki, youtube: yt };
    if spec != Spec::default() {
      out.push(spec);
    }
  }
  (out, skipped)
}

// what of `spec` is not already open in `have`: legs already running are dropped so a lineup only
// adds what's new. None when nothing is left (or it is a reserved name)
pub fn fresh(spec: &Spec, have: &[Spec]) -> Option<Spec> {
  let mut s = spec.clone();
  if have.iter().any(|h| !s.twitch.is_empty() && h.twitch == s.twitch) {
    s.twitch.clear();
  }
  if have.iter().any(|h| !s.kick.is_empty() && h.kick == s.kick) {
    s.kick.clear();
  }
  if have.iter().any(|h| !s.youtube.is_empty() && h.youtube == s.youtube) {
    s.youtube.clear();
  }
  (s != Spec::default() && s.check(have).is_ok()).then_some(s)
}

#[cfg(test)]
mod tests {
  use super::*;

  const UC: &str = "UC8yh-UI81uUteMDSOwQyx1g";

  fn tab(t: &str, k: &str, y: &str) -> Tab {
    Tab { twitch: t.into(), kick: k.into(), youtube: y.into() }
  }

  fn sp(t: &str, k: &str, y: &str) -> Spec {
    Spec { twitch: t.into(), kick: k.into(), youtube: y.into() }
  }

  #[test]
  fn parses_simulcast_and_solo_tabs() {
    assert_eq!(
      parse("t:xqc,k:xqc+t:asmongold+k:trainwreckstv"),
      vec![tab("xqc", "xqc", ""), tab("asmongold", "", ""), tab("", "trainwreckstv", "")]
    );
    assert_eq!(parse("t:caedrel,k:absi,y:lofigirl"), vec![tab("caedrel", "absi", "lofigirl")]);
  }

  #[test]
  fn youtube_keeps_an_id_exactly_and_lowers_a_handle() {
    assert_eq!(parse(&format!("y:{UC}")), vec![tab("", "", UC)]);
    assert_eq!(parse("y:LofiGirl"), vec![tab("", "", "lofigirl")]);
    assert_eq!(parse("y:@LofiGirl"), vec![tab("", "", "lofigirl")]);
    assert_eq!(parse("y:UCtooshort"), vec![tab("", "", "uctooshort")]);
    assert_eq!(parse(&format!("y:{}", "x".repeat(60))), vec![]);
    assert_eq!(parse("y:has spaces"), vec![]);
  }

  #[test]
  fn twitch_and_kick_lowercase() {
    assert_eq!(parse("t:XQC,k:ABSI"), vec![tab("xqc", "absi", "")]);
    assert_eq!(parse("k:some-streamer"), vec![tab("", "some-streamer", "")]);
  }

  #[test]
  fn lowercases_and_dedupes_identical_tabs() {
    assert_eq!(parse("t:XQC+t:xqc+t:Asmon"), vec![tab("xqc", "", ""), tab("asmon", "", "")]);
  }

  #[test]
  fn junk_is_dropped_never_an_error() {
    let long = format!("garbage::stuff+x:bad+t:valid_1+t:{}", "z".repeat(50));
    assert_eq!(parse(&long), vec![tab("valid_1", "", "")]);
    assert_eq!(parse(""), vec![]);
    assert_eq!(parse(&format!("t:{}", "a".repeat(500))), vec![]);
    assert_eq!(parse("+,+:,t:,:x,t"), vec![]);
    assert_eq!(parse("t:ünï"), vec![]);
  }

  #[test]
  fn caps_the_tab_count_at_8() {
    let many = (0..20).map(|i| format!("t:c{i}")).collect::<Vec<_>>().join("+");
    assert_eq!(parse(&many).len(), 8);
  }

  #[test]
  fn a_tab_reads_three_legs_at_most() {
    assert_eq!(parse("t:a,k:b,y:c,t:d"), vec![tab("a", "b", "c")]);
    assert_eq!(parse("t:a,t:b"), vec![tab("b", "", "")]);
  }

  #[test]
  fn builds_the_canonical_order() {
    let l = build(&[sp("xqc", "xqc", ""), sp("", "foo", "@lofigirl")]);
    assert_eq!(l, "t:xqc,k:xqc+k:foo,y:lofigirl");
    assert_eq!(parse(&l).len(), 2);
  }

  #[test]
  fn build_only_carries_youtube_that_was_typed() {
    // a twitch-only tab shares as twitch-only: our tabs have no guessed youtube legs
    assert_eq!(build(&[sp("caedrel", "", "")]), "t:caedrel");
    assert_eq!(build(&[sp("", "", "@Kripp")]), "y:kripp");
  }

  #[test]
  fn build_drops_tail_tabs_until_it_fits() {
    let specs: Vec<Spec> =
      (0..8).map(|i| sp(&format!("{}{i}", "a".repeat(29)), &format!("{}{i}", "b".repeat(29)), "")).collect();
    let l = build(&specs);
    assert!(l.len() <= MAX_LEN && !l.is_empty());
    assert_eq!(parse(&l).len(), l.matches('+').count() + 1);
    assert!(parse(&l).len() < 8);
    // the front of the lineup survives
    assert!(l.starts_with(&format!("t:{}0", "a".repeat(29))));
  }

  #[test]
  fn build_caps_at_8_and_skips_empty_tabs() {
    let specs: Vec<Spec> = (0..12).map(|i| sp(&format!("c{i}"), "", "")).collect();
    assert_eq!(build(&specs).matches('+').count(), 7);
    assert_eq!(build(&[Spec::default()]), "");
    assert_eq!(link(&[]), None);
    assert_eq!(link(&[sp("xqc", "", "")]).as_deref(), Some("https://heatsync.org/m/t:xqc"));
  }

  #[test]
  fn build_then_parse_round_trips() {
    let specs = [sp("forsen", "forsen", "@fors"), sp("", "xqc", ""), sp("a_b", "", "")];
    let back = to_specs(&parse(&build(&specs)));
    assert_eq!(back, (specs.to_vec(), 0));
  }

  #[test]
  fn extracts_from_links_and_bare_layouts() {
    for s in [
      "https://heatsync.org/m/t:xqc+k:foo",
      "http://www.heatsync.org/m/t:xqc+k:foo/",
      "HTTPS://HeatSync.org/m/t:xqc+k:foo?utm=1#x",
      "heatsync.org/m/t:xqc+k:foo",
      "/m/t:xqc+k:foo",
      "t:xqc+k:foo",
      "https://heatsync.org/m/t%3Axqc%2Bk%3Afoo",
    ] {
      assert_eq!(extract(s).as_deref().map(parse), Some(parse("t:xqc+k:foo")), "{s}");
    }
  }

  #[test]
  fn refuses_other_hosts_schemes_and_junk() {
    for s in [
      "",
      "https://evil.example/m/t:xqc",
      "https://heatsync.org.evil.example/m/t:xqc",
      "https://heatsync.org/x/t:xqc",
      "https://heatsync.org/m/",
      "https://heatsync.org",
      "javascript://heatsync.org/m/t:xqc",
      "t:xqc k:foo",
      "xqc",
      "x:bad",
      "%",
      "t%",
      "t:é%é",
    ] {
      assert_eq!(extract(s), None, "{s}");
    }
    assert_eq!(extract(&format!("t:{}", "a".repeat(3000))), None);
    assert_eq!(extract(&format!("t:{}", "a".repeat(500))), None);
  }

  #[test]
  fn channel_ids_cannot_open_here() {
    let (specs, skipped) = to_specs(&parse(&format!("t:xqc,y:{UC}+y:{UC}+k:foo")));
    assert_eq!(specs, vec![sp("xqc", "", ""), sp("", "foo", "")]);
    assert_eq!(skipped, 2);
  }

  #[test]
  fn legs_our_form_refuses_are_counted_not_fatal() {
    let (specs, skipped) = to_specs(&parse(&format!("t:{},k:ok+y:ab", "a".repeat(30))));
    assert_eq!(specs, vec![sp("", "ok", "")]);
    assert_eq!(skipped, 2);
  }

  #[test]
  fn fresh_drops_what_is_open_and_keeps_the_rest() {
    let have = [sp("xqc", "", "")];
    assert_eq!(fresh(&sp("xqc", "xqc", ""), &have), Some(sp("", "xqc", "")));
    assert_eq!(fresh(&sp("xqc", "", ""), &have), None);
    assert_eq!(fresh(&sp("asmon", "", ""), &have), Some(sp("asmon", "", "")));
    assert_eq!(fresh(&sp("live", "", ""), &have), None);
    assert_eq!(fresh(&sp("", "", "@kripp"), &[sp("", "", "@kripp")]), None);
  }
}
