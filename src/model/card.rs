// what the user card shows from POST /api/card (ext card-model.js, trimmed to what a text card can say):
// account age, type, heat, posts, followers, who follows whom, how long they've followed this channel,
// and how much the archive has seen of them. every part may be missing; a missing part shows nothing
// (never a guess: "not following" only when the server said so)
use super::ident;
use crate::i18n::{f, t};
use serde_json::Value;

use crate::util::time::parse_iso;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Info {
  // false = no heatsync account behind this name (a plain chatter)
  pub registered: bool,
  // (label, value), in the ext's order
  pub rows: Vec<(&'static str, String)>,
  // their name color: twitch's own when the net layer fetched it (ext hsResolveUserColor step 1,
  // see `chat_color`), else the one they picked on heatsync (step 2)
  pub color: Option<u32>,
  // the twitch login linked to their heatsync account: who step 1 asks twitch about
  pub twitch_name: Option<Box<str>>,
  // their twitch user id: what their paints hang off
  pub twitch_id: Option<u64>,
  // their kick user id / youtube channel id, from the account linked to a kick / youtube login: what a
  // stream line's paint hangs off on those platforms
  pub kick_id: Option<u64>,
  pub youtube_id: Option<Box<str>>,
  // their picture (ext card-model avatarUrl): heatsync's copy, else twitch's own, asked by the net layer
  pub avatar: Option<Box<str>>,
  // their heatsync user id (only a real account's: what a server note hangs off) and the note the
  // server holds for you on them (signed in; may be empty)
  pub profile_id: Option<Box<str>>,
  pub note: Option<String>,
  // whether you follow them on heatsync (signed in, a real account): what /follow and the card's
  // follow row flip
  pub follow: Option<bool>,
  // their twitch channel is live right now (the ext's gate for the card's clip action)
  pub live: bool,
}

// "4y 3mo", "2mo 5d", "3d", "5h", "just now"
pub fn ago(now: i64, then: i64) -> String {
  let days = (now - then).max(0) / 86_400_000;
  let (y, rest) = (days / 365, days % 365);
  let (mo, d) = (rest / 30, rest % 30);
  match (y, mo, d) {
    (0, 0, 0) => {
      let h = (now - then).max(0) / 3_600_000;
      if h > 0 { f("chat1.ago.h", &[("n", &h.to_string())]) } else { t("chat1.ago.now").into() }
    }
    (0, 0, d) => f("chat1.ago.d", &[("d", &d.to_string())]),
    (0, mo, d) if mo < 6 && d > 0 => f("chat1.ago.mo_d", &[("mo", &mo.to_string()), ("d", &d.to_string())]),
    (0, mo, _) => f("chat1.ago.mo", &[("mo", &mo.to_string())]),
    (y, 0, _) => f("chat1.ago.y", &[("y", &y.to_string())]),
    (y, mo, _) => f("chat1.ago.y_mo", &[("y", &y.to_string()), ("mo", &mo.to_string())]),
  }
}

fn count(n: u64) -> String {
  match n {
    n if n >= 1_000_000 => format!("{:.1}m", n as f64 / 1e6),
    n if n >= 10_000 => format!("{}k", n / 1000),
    n if n >= 1000 => format!("{:.1}k", n as f64 / 1e3),
    n => n.to_string(),
  }
}

fn num(v: &Value, path: &str) -> u64 {
  v.pointer(path).and_then(|x| x.as_u64().or_else(|| x.as_f64().map(|f| f.max(0.0) as u64))).unwrap_or(0)
}

fn text<'a>(v: &'a Value, path: &str) -> Option<&'a str> {
  v.pointer(path).and_then(Value::as_str).filter(|s| !s.is_empty())
}

// the earliest of several iso dates (a platform's own date beats the day heatsync recorded it)
fn earliest(v: &Value, paths: &[&str]) -> Option<i64> {
  paths.iter().filter_map(|p| text(v, p)).filter_map(parse_iso).min()
}

pub fn parse(v: &Value, now: i64) -> Info {
  let mut rows: Vec<(&'static str, String)> = Vec::new();
  let profile = v.get("profile").filter(|p| p.is_object());
  let anonymous = profile.is_some_and(|p| p.get("is_anonymous").and_then(Value::as_bool) == Some(true));
  if let Some(p) = profile.filter(|_| !anonymous) {
    // the oldest account across platforms
    let born = [p.get("twitch_created_at"), p.get("kick_created_at")]
      .into_iter()
      .flatten()
      .filter_map(Value::as_str)
      .filter_map(parse_iso)
      .min();
    if let Some(b) = born {
      rows.push(("age", ago(now, b)));
    }
    match text(p, "/twitch_broadcaster_type") {
      Some("partner") => rows.push(("type", t("chat1.card.type.partner").into())),
      Some("affiliate") => rows.push(("type", t("chat1.card.type.affiliate").into())),
      _ => {
        if let Some(r @ ("admin" | "staff")) = text(p, "/role") {
          let key = if r == "admin" { "chat1.card.type.admin" } else { "chat1.card.type.staff" };
          rows.push(("type", t(key).into()));
        }
      }
    }
    let heat = num(p, "/stats/user_heat");
    if heat > 0 {
      rows.push(("heat", count(heat)));
    }
    let posts = num(p, "/stats/op_count") + num(p, "/stats/mop_count") + num(p, "/stats/re_count");
    if posts > 0 {
      rows.push(("posts", count(posts)));
    }
    let followers = [num(p, "/stats/followers"), num(p, "/twitch_followers"), num(p, "/kick_followers")]
      .into_iter()
      .max()
      .unwrap_or(0);
    if followers > 0 {
      rows.push(("followers", count(followers)));
    }
    let following = [num(p, "/stats/following"), num(p, "/twitch_following_count"), num(p, "/kick_following_count")]
      .into_iter()
      .max()
      .unwrap_or(0);
    if following > 0 {
      rows.push(("following", count(following)));
    }
    if let Some(r) = p.get("relationship").filter(|r| r.is_object()) {
      let flag = |k: &str| r.get(k).and_then(Value::as_bool) == Some(true);
      let follows_you = flag("followsYou") || flag("profileFollowsViewerOnTwitch");
      let you_follow = flag("isFollowing") || flag("followsOnTwitch") || flag("followsOnKick");
      let since = |a: &[&str]| earliest(r, a).map(|t| format!(" {}", ago(now, t))).unwrap_or_default();
      if follows_you && you_follow {
        rows.push(("rel", f("chat1.card.mutual", &[("since", &since(&["/followsYouSince", "/followedAt"]))])));
      } else if you_follow {
        rows.push((
          "you",
          f(
            "chat1.card.follow",
            &[("since", &since(&["/followedAt", "/followsOnTwitchSince", "/followsOnKickSince"]))],
          ),
        ));
      } else if follows_you {
        rows.push((
          "they",
          f("chat1.card.follow_you", &[("since", &since(&["/followsYouSince", "/profileFollowsViewerOnTwitchSince"]))]),
        ));
      }
      if flag("subscribesToYou") || flag("profileSubbedToViewerOnTwitch") {
        rows.push((
          "they",
          f("chat1.card.sub_to_you", &[("since", &since(&["/subscribesToYouSince", "/profileTwitchSubSince"]))]),
        ));
      }
    }
  }
  // this channel's followage: only a real answer counts
  if let Some(f) =
    v.get("followage").filter(|f| f.is_object() && f.get("degraded").and_then(Value::as_bool) != Some(true))
  {
    match text(f, "/followedAt").and_then(parse_iso) {
      Some(t) => rows.push(("ch follow", ago(now, t))),
      None => rows.push(("ch follow", t("chat1.card.no").into())),
    }
  }
  let seen = num(v, "/corpus/messages");
  if seen > 0 {
    let since = text(v, "/corpus/firstDay").and_then(|d| parse_iso(&format!("{d}T00:00:00Z")));
    rows.push((
      "seen",
      match since {
        Some(t) => f("chat1.card.seen_over", &[("n", &count(seen)), ("ago", &ago(now, t))]),
        None => f("chat1.card.seen", &[("n", &count(seen))]),
      },
    ));
  }
  let real = profile.filter(|_| !anonymous);
  let color = real
    .and_then(|p| ["/color", "/user_color", "/userColor"].iter().find_map(|k| text(p, k)))
    .and_then(crate::model::color::parse_hex);
  let twitch_id = real
    .and_then(|p| ["/twitch_user_id", "/twitch_id"].iter().find_map(|k| p.pointer(k)))
    .and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
    .filter(|n| *n > 0);
  let kick_id = real
    .and_then(|p| ["/kick_user_id", "/kick_id"].iter().find_map(|k| p.pointer(k)))
    .and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
    .filter(|n| *n > 0);
  let youtube_id = real
    .and_then(|p| text(p, "/youtube_channel_id"))
    .and_then(|c| crate::cosmetics::id::PaintId::of("youtube", c))
    .and_then(|p| match p {
      crate::cosmetics::id::PaintId::Youtube(c) => Some(c),
      _ => None,
    });
  let twitch_name = real.and_then(|p| text(p, "/twitch_username")).filter(|n| ident::twitch_login(n)).map(Into::into);
  let avatar = real
    .and_then(|p| ["/twitch_profile_pic", "/kick_profile_pic", "/profile_image_url"].iter().find_map(|k| text(p, k)))
    .or_else(|| text(v, "/chatter/avatarUrl"))
    .and_then(avatar_url);
  let profile_id = real.and_then(|p| text(p, "/id")).filter(|id| crate::model::notes::server_id(id)).map(Into::into);
  let note = text(v, "/note").map(|n| n.chars().take(crate::model::notes::MAX_CHARS * 2).collect());
  let follow = real
    .and_then(|p| p.get("relationship"))
    .filter(|r| r.is_object())
    .map(|r| ["youFollow", "isFollowing"].iter().any(|k| r.get(*k).and_then(Value::as_bool) == Some(true)));
  let follow = follow.filter(|_| profile_id.is_some());
  let live = real.is_some_and(|p| {
    p.get("twitch_is_live").and_then(Value::as_bool) == Some(true)
      || p.pointer("/live_status/twitch").and_then(Value::as_bool) == Some(true)
  });
  Info {
    registered: profile.is_some() && !anonymous,
    rows,
    color,
    twitch_name,
    twitch_id,
    kick_id,
    youtube_id,
    avatar,
    profile_id,
    note,
    follow,
    live,
  }
}

// a picture url we will fetch: an allowlisted image host, bounded
pub fn avatar_url(u: &str) -> Option<Box<str>> {
  crate::images::url::allowed(u).then(|| u.into())
}

// the picture out of twitch's `user(login){profileImageURL}` answer, hostile like the color
pub fn twitch_avatar(v: &Value) -> Option<Box<str>> {
  text(v, "/data/user/profileImageURL").and_then(avatar_url)
}

// the color out of twitch's `user(login){chatColor}` answer. the body is hostile: anything but
// `#rgb` / `#rrggbb` in the right place is no color (a user who never picked one answers null)
pub fn chat_color(v: &Value) -> Option<u32> {
  v.pointer("/data/user/chatColor").and_then(Value::as_str).and_then(crate::model::color::parse_hex)
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  const NOW: i64 = 1793399211099; // 2026-10-30

  #[test]
  fn spans_read_short() {
    let day = 86_400_000;
    assert_eq!(ago(NOW, NOW - 3 * day), "3d");
    assert_eq!(ago(NOW, NOW - 100 * day), "3mo 10d");
    assert_eq!(ago(NOW, NOW - 400 * day), "1y 1mo");
    assert_eq!(ago(NOW, NOW - 730 * day), "2y");
    assert_eq!(ago(NOW, NOW - 5 * 3_600_000), "5h");
    assert_eq!(ago(NOW, NOW + 5), "just now");
  }

  #[test]
  fn a_full_card() {
    let v = json!({
      "profile": {
        "twitch_created_at": "2016-10-30T00:00:00.000Z", "kick_created_at": "2023-01-01T00:00:00.000Z",
        "twitch_broadcaster_type": "affiliate",
        "stats": { "user_heat": 1234, "op_count": 3, "re_count": 9, "followers": 12 },
        "twitch_followers": 15000,
        "relationship": { "followsYou": true, "isFollowing": true, "followsYouSince": "2025-10-30T00:00:00.000Z" }
      },
      "followage": { "followedAt": "2024-10-30T00:00:00.000Z" },
      "corpus": { "messages": 420, "firstDay": "2025-01-01" }
    });
    let i = parse(&v, NOW);
    assert!(i.registered);
    let get = |k: &str| i.rows.iter().find(|r| r.0 == k).map(|r| r.1.clone());
    assert_eq!(get("age").as_deref(), Some("10y"));
    assert_eq!(get("type").as_deref(), Some("affiliate"));
    assert_eq!(get("heat").as_deref(), Some("1.2k"));
    assert_eq!(get("posts").as_deref(), Some("12"));
    assert_eq!(get("followers").as_deref(), Some("15k"));
    assert_eq!(get("rel").as_deref(), Some("mutual 1y"));
    assert_eq!(get("ch follow").as_deref(), Some("2y"));
    assert_eq!(get("seen").as_deref(), Some("420 msgs over 1y 10mo"));
    assert_eq!((i.color, i.twitch_id), (None, None));
  }

  #[test]
  fn live_is_the_twitch_channel_being_live_for_a_real_account_only() {
    let live = |p: Value| parse(&json!({ "profile": p }), NOW).live;
    assert!(live(json!({ "twitch_is_live": true })));
    assert!(live(json!({ "live_status": { "twitch": true } })));
    assert!(!live(json!({ "twitch_is_live": false, "live_status": { "twitch": false, "kick": true } })));
    assert!(!live(json!({ "twitch_is_live": "yes" })));
    assert!(!live(json!({ "is_anonymous": true, "twitch_is_live": true })));
    assert!(!parse(&json!({}), NOW).live);
  }

  #[test]
  fn the_profile_id_and_server_note_come_through_but_shadow_ids_do_not() {
    let i = parse(&json!({ "profile": { "id": "u42" }, "note": "likes cats" }), NOW);
    assert_eq!((i.profile_id.as_deref(), i.note.as_deref()), (Some("u42"), Some("likes cats")));
    for id in ["kick_9", "yt_UCx", ""] {
      assert_eq!(parse(&json!({ "profile": { "id": id } }), NOW).profile_id, None);
    }
    assert_eq!(parse(&json!({ "profile": {}, "note": "" }), NOW).note, None);
  }

  #[test]
  fn the_heatsync_follow_is_known_only_for_a_real_account_with_a_relationship() {
    let f = |v: serde_json::Value| parse(&v, NOW).follow;
    assert_eq!(f(json!({ "profile": { "id": "u42", "relationship": { "isFollowing": true } } })), Some(true));
    assert_eq!(f(json!({ "profile": { "id": "u42", "relationship": { "followsYou": true } } })), Some(false));
    assert_eq!(f(json!({ "profile": { "id": "u42" } })), None, "signed out: unknown");
    assert_eq!(f(json!({ "profile": { "id": "kick_9", "relationship": {} } })), None);
  }

  #[test]
  fn the_picture_comes_from_an_allowed_host_only() {
    let pic = |p: Value| parse(&json!({ "profile": p }), NOW).avatar;
    let ok = "https://static-cdn.jtvnw.net/jtv_user_pictures/a-70x70.png";
    assert_eq!(pic(json!({"twitch_profile_pic": ok})).as_deref(), Some(ok));
    assert_eq!(
      pic(json!({"twitch_profile_pic": null, "kick_profile_pic": "https://files.kick.com/a.webp"})).as_deref(),
      Some("https://files.kick.com/a.webp")
    );
    assert_eq!(pic(json!({"profile_image_url": "https://evil.example/a.png"})), None);
    assert_eq!(pic(json!({"profile_image_url": "javascript:x"})), None);
    let chatter = parse(&json!({ "chatter": { "avatarUrl": ok } }), NOW);
    assert_eq!(chatter.avatar.as_deref(), Some(ok));
    assert_eq!(twitch_avatar(&json!({"data": {"user": {"profileImageURL": ok}}})).as_deref(), Some(ok));
    assert_eq!(twitch_avatar(&json!({"data": {"user": {"profileImageURL": 5}}})), None);
  }

  #[test]
  fn twitch_chat_color_survives_hostile_answers() {
    let ok = |c: Value| chat_color(&json!({ "data": { "user": { "chatColor": c } } }));
    assert_eq!(ok(json!("#1E90FF")), Some(0x1e90ff));
    assert_eq!(ok(json!("#f80")), Some(0xff8800));
    for bad in [
      json!(null),
      json!(""),
      json!("#"),
      json!("1E90FF"),
      json!("#+12345"),
      json!("#12345g"),
      json!("#1E90FF00"),
      json!(5),
      json!(["#fff"]),
      json!("#\u{202e}abcd"),
    ] {
      assert_eq!(ok(bad.clone()), None, "{bad}");
    }
    for bad in [
      json!(null),
      json!([]),
      json!("x"),
      json!({ "data": null }),
      json!({ "data": { "user": null } }),
      json!({ "data": [1] }),
    ] {
      assert_eq!(chat_color(&bad), None);
    }
  }

  #[test]
  fn only_a_sane_linked_twitch_login_is_asked_about() {
    let name = |n: Value| parse(&json!({ "profile": { "twitch_username": n } }), NOW).twitch_name;
    assert_eq!(name(json!("nl_kripp")).as_deref(), Some("nl_kripp"));
    for bad in [json!(""), json!("a b"), json!("x".repeat(26)), json!("é"), json!(7), json!("a\"}")] {
      assert_eq!(name(bad), None);
    }
  }

  #[test]
  fn the_picked_color_and_twitch_id_are_read_when_valid() {
    let i = parse(&json!({ "profile": { "color": "#ff8800", "twitch_user_id": "1234" } }), NOW);
    assert_eq!((i.color, i.twitch_id), (Some(0xff8800), Some(1234)));
    let i = parse(&json!({ "profile": { "user_color": "red", "twitch_id": 0 } }), NOW);
    assert_eq!((i.color, i.twitch_id), (None, None));
    // an anonymous profile is nobody
    let i = parse(&json!({ "profile": { "is_anonymous": true, "color": "#ff8800" } }), NOW);
    assert_eq!(i.color, None);
  }

  #[test]
  fn a_chatter_with_no_account_and_degraded_data() {
    let i = parse(&json!({ "profile": null, "followage": { "degraded": true }, "corpus": { "messages": 0 } }), NOW);
    assert!(!i.registered);
    assert!(i.rows.is_empty());
    // "not following" only when the server really answered
    let n = parse(&json!({ "followage": { "followedAt": null } }), NOW);
    assert_eq!(n.rows, vec![("ch follow", "no".to_owned())]);
    assert!(!parse(&json!({ "profile": { "is_anonymous": true } }), NOW).registered);
    assert!(parse(&json!("junk"), NOW).rows.is_empty());
  }

  #[test]
  fn every_row_label_has_its_words() {
    for l in ["age", "type", "heat", "posts", "followers", "following", "rel", "you", "they", "ch follow", "seen"] {
      assert!(!t(&format!("chat1.card.row.{}", l.replace(' ', "_"))).is_empty(), "{l}");
    }
  }
}
