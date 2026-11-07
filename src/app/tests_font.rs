// the font family, headless: a name is asked of fontconfig, the answer lays every line out again in that font (the
// leftmost green pixel of a name moves), and anything that goes wrong says so and keeps the font that works.
// the real fonts of this computer are used where there are any; a test without the font it needs ends early
use super::*;
use crate::model::message::Message;
use crate::model::settings::Local;
use crate::net::Cmd;
use crate::util::font::Found;
use tokio::sync::mpsc::UnboundedReceiver;

// a window with one chat line from a green-named chatter and the clock on (the clock is where a font shows)
fn app() -> (App, UnboundedReceiver<Cmd>) {
  let (net, rx) = Net::tap();
  let mut app = App::new(net, vec!["aaa".into()], false);
  app.set = Settings::default();
  app.res.apply_look(&app.set);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.canvas.resize(640, 200);
  app.relayout_body();
  let pos = app.tabs.iter().position(|t| &*t.name == "aaa").unwrap();
  app.select(pos);
  let mut m = Message::system(1_700_000_000_000, Platform::Twitch, "hi");
  m.kind = crate::model::message::Kind::Chat;
  (m.nick, m.login, m.color) = ("bob".into(), "bob".into(), 0x00ff00);
  app.on_chat("aaa", Chat::Msg(m));
  app.local(Local::Toggle("timestamps"));
  app.local(Local::Toggle("platform_tags"));
  app.redraw();
  (app, rx)
}

// where the name starts: the leftmost green-leaning pixel
fn name_x(app: &App) -> Option<usize> {
  let w = app.canvas.w as usize;
  let green = |p: u32| (p >> 8 & 0xff) >= 100 && (p >> 16 & 0xff) < 50 && (p & 0xff) < 50;
  app.canvas.px.iter().enumerate().filter(|(_, p)| green(**p)).map(|(i, _)| i % w).min()
}

// what has been asked of fontconfig so far
fn asked(rx: &mut UnboundedReceiver<Cmd>) -> Vec<Box<str>> {
  std::iter::from_fn(|| rx.try_recv().ok()).filter_map(|c| if let Cmd::Font(n) = c { Some(n) } else { None }).collect()
}

fn said(a: &App) -> String {
  a.tabs[a.active].view.last_msg().map(|m| m.body.to_string()).unwrap_or_default()
}

fn typed(a: &mut App, s: &str) {
  a.line.set_text(s.into());
  a.send();
}

// the font fontconfig gives a generic name, found the way the app finds it (None without fc-match)
fn installed(generic: &str) -> Option<Found> {
  let out = std::process::Command::new("fc-match").args(["-f", "%{family[0]}", generic]).output().ok()?;
  let family = String::from_utf8(out.stdout).ok().filter(|f| !f.is_empty())?;
  crate::util::font::find(&family).ok()
}

// a font that sets "iiii" and "WWWW" to different widths is not a monospace one
fn proportional(a: &mut App) -> bool {
  let w = |a: &mut App, s: &str| a.res.text.layout(s, &[], &[], f32::MAX).full_width();
  (w(a, "iiiiiiiiii") - w(a, "WWWWWWWWWW")).abs() > 1.0
}

#[test]
fn typing_a_family_asks_for_it_and_changes_nothing_until_it_is_answered() {
  let (mut a, mut rx) = app();
  let before = name_x(&a);
  typed(&mut a, "/font Fira Code");
  assert_eq!(asked(&mut rx), ["Fira Code".into()]);
  assert_eq!(a.res.text.family(), None, "still the system monospace");
  assert_eq!(a.set.font_family.as_str(), "Fira Code", "the name is the setting while it is asked");
  a.redraw();
  assert_eq!(name_x(&a), before, "nothing moved");
  // nothing typed says what it is now
  typed(&mut a, "/font");
  assert!(said(&a).starts_with("font family: Fira Code"), "{}", said(&a));
}

#[test]
fn a_missing_family_is_said_in_plain_words_and_the_old_font_stays() {
  let (mut a, mut rx) = app();
  typed(&mut a, "/font nosuchfont");
  asked(&mut rx);
  a.font_found(
    "nosuchfont".into(),
    Err("no font family called nosuchfont is installed (the closest is Noto Sans)".into()),
  );
  assert_eq!(
    said(&a),
    "no font family called nosuchfont is installed (the closest is Noto Sans): still using the system monospace"
  );
  assert_eq!(a.set.font_family.as_str(), "", "the failed name is taken back");
  assert_eq!(a.res.text.family(), None);
  assert!(!a.font_stale());
  assert!(Settings::parse(&a.set.render()).font_family.is_empty(), "and never saved");
}

#[test]
fn a_found_font_lays_every_line_out_again_and_is_kept_under_its_own_name() {
  let Some(f) = installed("sans-serif") else { return };
  let (mut a, mut rx) = app();
  let before = name_x(&a).expect("the name is drawn");
  assert!(!proportional(&mut a), "the default is a monospace");
  typed(&mut a, &format!("/font {}", f.family.to_uppercase()));
  let name = asked(&mut rx).remove(0);
  a.font_found(name, Ok(f.clone()));
  a.redraw();
  assert_eq!(a.res.text.family(), Some(f.family.as_str()));
  assert_eq!(a.set.font_family.as_str(), f.family, "saved as the font spells itself");
  assert_eq!(said(&a), format!("font: {}", f.family));
  assert!(!a.font_stale());
  if proportional(&mut a) {
    let after = name_x(&a).expect("still drawn");
    assert_ne!(after, before, "the clock is set in the new font, so the name starts somewhere else");
  }
  assert_eq!(Settings::parse(&a.set.render()).font_family.as_str(), f.family, "kept between runs");
}

#[test]
fn off_goes_back_to_the_system_monospace_at_once() {
  let Some(f) = installed("sans-serif") else { return };
  let (mut a, mut rx) = app();
  let before = name_x(&a);
  typed(&mut a, &format!("/font {}", f.family));
  a.font_found(asked(&mut rx).remove(0), Ok(f));
  assert!(a.res.text.family().is_some());
  typed(&mut a, "/font off");
  a.redraw();
  assert!(asked(&mut rx).is_empty(), "nothing to look up");
  assert_eq!(a.res.text.family(), None);
  assert_eq!(a.set.font_family.as_str(), "");
  assert_eq!(name_x(&a), before, "laid out as it was");
}

#[test]
fn a_family_that_will_not_load_keeps_the_font_that_was_working() {
  let Some(f) = installed("sans-serif") else { return };
  let (mut a, mut rx) = app();
  typed(&mut a, &format!("/font {}", f.family));
  a.font_found(asked(&mut rx).remove(0), Ok(f.clone()));
  a.redraw();
  let working = name_x(&a);
  // fontconfig names a file that is not a font at all
  let dir = std::env::temp_dir().join(format!("hsc-font-{}", std::process::id()));
  std::fs::create_dir_all(&dir).unwrap();
  let junk = dir.join("junk.ttf");
  std::fs::write(&junk, b"this is not a font").unwrap();
  typed(&mut a, "/font Junk Sans");
  a.font_found(asked(&mut rx).remove(0), Ok(Found { family: "Junk Sans".into(), file: junk }));
  std::fs::remove_dir_all(&dir).ok();
  a.redraw();
  assert_eq!(said(&a), format!("couldn't load the font Junk Sans: still using {}", f.family));
  assert_eq!(a.res.text.family(), Some(f.family.as_str()), "the working font stays");
  assert_eq!(a.set.font_family.as_str(), f.family, "and so does the setting");
  assert_eq!(name_x(&a), working);
}

#[test]
fn an_answer_for_an_older_name_is_dropped() {
  let Some(f) = installed("sans-serif") else { return };
  let (mut a, mut rx) = app();
  typed(&mut a, "/font aaa");
  typed(&mut a, "/font bbb");
  assert_eq!(asked(&mut rx), ["aaa".into(), "bbb".into()]);
  a.font_found("aaa".into(), Ok(Found { family: f.family.clone(), file: f.file }));
  assert_eq!(a.res.text.family(), None, "aaa is not what is wanted now");
  assert_eq!(a.set.font_family.as_str(), "bbb");
  a.font_found("bbb".into(), Err("gone".into()));
  assert_eq!(said(&a), "gone: still using the system monospace");
}

#[test]
fn the_saved_font_is_asked_at_start_and_a_failure_then_leaves_it_saved() {
  let (mut a, mut rx) = app();
  a.set.font_family.set("Gone Mono");
  a.font_follow(false);
  assert_eq!(asked(&mut rx), ["Gone Mono".into()]);
  a.font_found("Gone Mono".into(), Err("the font lookup didn't answer: try again".into()));
  assert_eq!(said(&a), "the font lookup didn't answer: try again: still using the system monospace");
  assert_eq!(a.set.font_family.as_str(), "Gone Mono", "kept, the font may be back tomorrow");
  assert!(a.font_stale());
  // typing the same name again tries again instead of saying it is already that
  typed(&mut a, "/font Gone Mono");
  assert_eq!(asked(&mut rx), ["Gone Mono".into()]);
}

// a font the system's font list doesn't hold is added from its file: the same font with its family renamed, put
// where fontconfig doesn't look, stands in for one the person keeps in their own folder
#[test]
fn a_font_file_the_system_does_not_list_is_loaded_from_its_file() {
  let Some(f) = installed("sans-serif") else { return };
  let Ok(mut bytes) = std::fs::read(&f.file) else { return };
  if bytes.len() > 8 << 20 {
    return;
  }
  // the family as the name table spells it (utf-16, big endian), with its last letter changed
  let utf16 = |s: &str| s.encode_utf16().flat_map(u16::to_be_bytes).collect::<Vec<u8>>();
  let stem: String = f.family.chars().take(f.family.chars().count() - 1).collect();
  let (old, new_name) = (utf16(&f.family), format!("{stem}Q"));
  let new = utf16(&new_name);
  let at = bytes.windows(old.len()).position(|w| w == old.as_slice());
  let Some(at) = at else { return };
  let mut n = 0;
  let mut i = at;
  while let Some(k) = bytes[i..].windows(old.len()).position(|w| w == old.as_slice()) {
    bytes[i + k..i + k + old.len()].copy_from_slice(&new);
    i += k + old.len();
    n += 1;
  }
  assert!(n > 0);
  let dir = std::env::temp_dir().join(format!("hsc-font-own-{}", std::process::id()));
  std::fs::create_dir_all(&dir).unwrap();
  let mine = dir.join("mine.ttf");
  std::fs::write(&mine, &bytes).unwrap();
  let (mut a, _rx) = app();
  let loaded = a.res.text.use_family(&new_name, &mine);
  std::fs::remove_dir_all(&dir).ok();
  assert!(loaded, "{new_name} is added from its file");
  assert_eq!(a.res.text.family(), Some(new_name.as_str()));
  a.relayout_all();
  a.redraw();
}
