// the mention sound, headless: one of the four, or a wav file of your own. every pick is heard once as it is
// made, so these run at volume 0 and never make a sound
use super::*;
use crate::model::settings::{Local, PingSound, Settings, Volume};
use crate::util::sound::{Preset, Voice};

fn app() -> App {
  let (net, _rx) = Net::tap();
  let mut a = App::new(net, vec!["forsen".into()], false);
  a.set = Settings::default();
  a.set.mention_volume = Volume::V0;
  a
}

fn said(a: &App) -> String {
  a.tabs[a.active].view.last_msg().map(|m| m.body.to_string()).unwrap_or_default()
}

fn typed(a: &mut App, s: &str) {
  a.line.set_text(s.into());
  a.send();
}

// a folder of this test's own, and a wav in it
fn wav_in(dir: &str, name: &str) -> String {
  let d = std::env::temp_dir().join(format!("hsc-ping-{}", std::process::id())).join(dir);
  std::fs::create_dir_all(&d).unwrap();
  let p = d.join(name);
  std::fs::write(&p, crate::util::wav::encode(8000, 1, &[100, -100, 200])).unwrap();
  p.to_str().unwrap().to_owned()
}

#[test]
fn the_mention_sound_is_one_of_the_four_and_the_pick_is_kept() {
  let mut a = app();
  assert_eq!((a.set.ping_sound, a.ping_voice()), (PingSound::Ping, Voice::Preset(Preset::Ping)), "today's ping");
  typed(&mut a, "/pingsound chime");
  assert_eq!((a.set.ping_sound, a.ping_voice()), (PingSound::Chime, Voice::Preset(Preset::Chime)));
  for (name, p) in [("blip", Preset::Blip), ("knock", Preset::Knock), ("ping", Preset::Ping)] {
    typed(&mut a, &format!("/pingsound {name}"));
    assert_eq!(a.ping_voice(), Voice::Preset(p), "{name}");
  }
  typed(&mut a, "/pingsound knock");
  assert!(Settings::parse(&a.set.render()).ping_sound == PingSound::Knock, "kept between runs");
  // a name that isn't one says the four
  typed(&mut a, "/pingsound bell");
  assert!(said(&a).starts_with("usage: /pingsound <ping|blip|knock|chime|custom>"), "{}", said(&a));
  assert_eq!(a.set.ping_sound, PingSound::Knock);
}

#[test]
fn custom_without_a_file_says_what_is_missing_and_plays_nothing_odd() {
  let mut a = app();
  a.local(Local::Pick("ping_sound", Some(4)));
  assert_eq!(a.set.ping_sound, PingSound::Custom);
  assert!(said(&a).starts_with("custom plays a wav file of yours: pick it in settings"), "{}", said(&a));
  assert_eq!(a.ping_voice(), Voice::File(String::new()), "the player falls back to the ping when it finds no file");
}

#[test]
fn a_wav_of_your_own_becomes_the_sound_and_off_gives_the_ping_back() {
  let mut a = app();
  let path = wav_in("a", "Ding.wav");
  typed(&mut a, &format!("/pingfile {path}"));
  assert_eq!(a.set.ping_file.as_str(), path);
  assert_eq!(a.set.ping_sound, PingSound::Custom, "a file is the thing to play");
  assert_eq!(a.ping_voice(), Voice::File(path.clone()));
  let kept = Settings::parse(&a.set.render());
  assert_eq!((kept.ping_file.as_str(), kept.ping_sound), (path.as_str(), PingSound::Custom));
  // asking shows the file's name, never the folders it sits in
  typed(&mut a, "/pingfile");
  assert!(said(&a).starts_with("custom sound file: Ding.wav \u{b7} /pingfile"), "{}", said(&a));
  assert!(!said(&a).contains("hsc-ping"), "no path on screen");
  typed(&mut a, "/pingfile off");
  assert!(a.set.ping_file.is_empty());
  assert_eq!(a.set.ping_sound, PingSound::Ping, "custom with nothing behind it is the ping");
  // a sound picked from the four is left alone when the file goes
  typed(&mut a, "/pingsound blip");
  typed(&mut a, &format!("/pingfile {path}"));
  typed(&mut a, "/pingsound blip");
  typed(&mut a, "/pingfile none");
  assert_eq!(a.set.ping_sound, PingSound::Blip);
}

#[test]
fn a_file_that_cannot_play_is_refused_in_plain_words_and_changes_nothing() {
  let mut a = app();
  typed(&mut a, "/pingsound chime");
  let dir = std::env::temp_dir().join(format!("hsc-ping-{}", std::process::id())).join("bad");
  std::fs::create_dir_all(&dir).unwrap();
  let junk = dir.join("junk.wav");
  std::fs::write(&junk, b"RIFF this is not a wav file at all").unwrap();
  for (path, why) in [
    (junk.to_str().unwrap().to_owned(), "that isn't a wav file"),
    (dir.join("missing.wav").to_str().unwrap().to_owned(), "can't open that file"),
    (dir.to_str().unwrap().to_owned(), "can't open that file"),
  ] {
    typed(&mut a, &format!("/pingfile {path}"));
    assert_eq!(said(&a), why);
    assert!(a.set.ping_file.is_empty() && a.set.ping_sound == PingSound::Chime, "nothing changed for {why}");
  }
  typed(&mut a, &format!("/pingfile /{}", "x".repeat(300)));
  assert_eq!(said(&a), "custom sound file is too long or has odd characters");
  assert!(a.set.ping_file.is_empty());
}

#[test]
fn a_path_with_spaces_and_capitals_is_kept_whole() {
  let mut a = app();
  let path = wav_in("My Sounds", "Big DING.WAV");
  typed(&mut a, &format!("/pingfile {path}"));
  assert_eq!(a.set.ping_file.as_str(), path);
  assert_eq!(Settings::parse(&a.set.render()).ping_file.as_str(), path);
}

#[test]
fn a_reset_puts_the_ping_back() {
  let mut a = app();
  let path = wav_in("r", "r.wav");
  typed(&mut a, &format!("/pingfile {path}"));
  a.reset_settings();
  assert!(a.set.ping_file.is_empty() && a.set.ping_sound == PingSound::Ping);
}
