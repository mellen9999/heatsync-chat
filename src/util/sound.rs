// the sounds (ext mentions.js): the mention ping, and the four a highlight rule can pick (playFilterRuleSound).
// each is a few sine tones, (hz, start, length), 10ms up to full then falling away to silence at its end. made
// here, no sound file shipped, and played by the first of pw-play, paplay, aplay on PATH, fed a wav on stdin
// (windows: PlaySound, from the same bytes in memory; sound_unix.rs and sound_windows.rs).
// the mention ping can be any of the four, or a wav file of your own (util::wav reads it: plain pcm, 2MB, 3
// seconds); a file that has gone bad since it was picked is said once on stderr and the ping plays instead.
// best effort: no player is said once on stderr. a burst sounds once per 1.2s, so a raid of @s can't
// machine-gun
use std::f32::consts::TAU;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};

use super::wav;

// handing a wav to the platform's player
#[cfg_attr(unix, path = "sound_unix.rs")]
#[cfg_attr(windows, path = "sound_windows.rs")]
mod os;

const RATE: u32 = 48_000;
const GAP_MS: i64 = 1200;

// the ext's FILTER_SOUND_PRESETS names; anything else is silent
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Preset {
  Ping,
  Blip,
  Knock,
  Chime,
}

impl Preset {
  pub fn parse(name: &str) -> Option<Preset> {
    match name {
      "ping" => Some(Preset::Ping),
      "blip" => Some(Preset::Blip),
      "knock" => Some(Preset::Knock),
      "chime" => Some(Preset::Chime),
      _ => None,
    }
  }

  // (hz, start s, length s)
  fn tones(self) -> &'static [(f32, f32, f32)] {
    match self {
      Preset::Ping => &[(880.0, 0.0, 0.32), (1175.0, 0.08, 0.24)],
      Preset::Blip => &[(1320.0, 0.0, 0.12)],
      Preset::Knock => &[(200.0, 0.0, 0.1), (200.0, 0.13, 0.1)],
      Preset::Chime => &[(660.0, 0.0, 0.18), (880.0, 0.09, 0.18), (1318.0, 0.18, 0.26)],
    }
  }
}

// what a sound is made of: one of ours, or a wav file of your own (a path as typed, `~/` and all)
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Voice {
  Preset(Preset),
  File(String),
}

// the sound as a 16-bit mono wav, `volume` 0..1 (the ext's gain is 0.35 of it, per tone)
fn synth(p: Preset, volume: f32) -> Vec<u8> {
  let tones = p.tones();
  let secs = tones.iter().map(|t| t.1 + t.2).fold(0.0, f32::max);
  let n = (RATE as f32 * secs) as usize;
  let peak = volume.clamp(0.0, 1.0) * 0.35 * f32::from(i16::MAX);
  let samples: Vec<i16> = (0..n)
    .map(|i| {
      let t = i as f32 / RATE as f32;
      let mut s = 0.0;
      for &(hz, start, len) in tones {
        let u = t - start;
        if !(0.0..len).contains(&u) {
          continue;
        }
        let env = if u < 0.01 { u / 0.01 } else { 0.0001f32.powf((u - 0.01) / (len - 0.01)) };
        s += (TAU * hz * u).sin() * env;
      }
      (s * peak) as i16
    })
    .collect();
  wav::encode(RATE, 1, &samples)
}

// a wav file at `volume` (0..1 of the file's own loudness), ready for the player. one that won't read or
// parse is the ping instead, and why comes back with it
fn from_file(path: &str, volume: f32) -> (Vec<u8>, Option<wav::Bad>) {
  match wav::read(path) {
    Ok(pcm) => {
      let gain = volume.clamp(0.0, 1.0);
      let samples: Vec<i16> = pcm.samples.iter().map(|&s| (f32::from(s) * gain) as i16).collect();
      (wav::encode(pcm.rate, pcm.channels, &samples), None)
    }
    Err(bad) => (synth(Preset::Ping, volume), Some(bad)),
  }
}

// the mention ping at `percent` volume (0 = silent), paced: a burst sounds once
pub fn mention(voice: Voice, percent: u8, now_ms: i64) {
  start(voice, percent, Some(now_ms))
}

// a rule's sound at `percent` volume (0 = silent), paced like the ping
pub fn play(p: Preset, percent: u8, now_ms: i64) {
  start(Voice::Preset(p), percent, Some(now_ms))
}

// hear a sound now (a pick just made), whatever else sounded a moment ago
pub fn preview(voice: Voice, percent: u8) {
  start(voice, percent, None)
}

// `pace` = when it is, for the one-per-1.2s limit; None = not limited
fn start(voice: Voice, percent: u8, pace: Option<i64>) {
  static LAST: AtomicI64 = AtomicI64::new(i64::MIN / 2);
  static FILE_SAID: AtomicBool = AtomicBool::new(false);
  if percent == 0 {
    return;
  }
  if let Some(now) = pace {
    if now - LAST.load(Ordering::Relaxed) < GAP_MS {
      return;
    }
    LAST.store(now, Ordering::Relaxed);
  }
  let volume = f32::from(percent.min(100)) / 100.0;
  // made off the ui thread by the player: a slow disk or a stuck player never stalls a frame
  os::play(move || match voice {
    Voice::Preset(p) => synth(p, volume),
    Voice::File(path) => {
      let (bytes, bad) = from_file(&path, volume);
      if let Some(bad) = bad.filter(|_| !FILE_SAID.swap(true, Ordering::Relaxed)) {
        eprintln!("your mention sound file didn't play ({}): the ping plays instead", bad.said());
      }
      bytes
    }
  });
}

#[cfg(test)]
mod tests {
  use super::*;

  fn samples(w: &[u8]) -> Vec<i16> {
    w[44..].chunks(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect()
  }

  #[test]
  fn the_ping_is_a_short_quiet_wav() {
    let w = synth(Preset::Ping, 0.3);
    assert_eq!(&w[..4], b"RIFF");
    assert_eq!(&w[36..40], b"data");
    assert_eq!(w.len(), 44 + 15_360 * 2, "320ms at 48k");
    let samples = samples(&w);
    let loudest = samples.iter().map(|s| s.unsigned_abs()).max().unwrap();
    // two tones at 0.35 × 0.3 each: never past 0.21 of full scale
    assert!(loudest > 2000 && loudest <= (0.21 * 32767.0) as u16 + 1, "{loudest}");
    assert_eq!(samples[0], 0, "starts silent, no click");
    assert!(samples[samples.len() - 1] == 0, "ends silent");
    assert!(
      synth(Preset::Ping, 1.0)[44..].chunks(2).all(|c| i16::from_le_bytes([c[0], c[1]]) != i16::MIN),
      "never clips"
    );
  }

  #[test]
  fn each_preset_is_its_own_length_and_stays_quiet() {
    // 120ms, 230ms, 440ms: the ext's presets
    for (p, ms) in [(Preset::Blip, 120), (Preset::Knock, 230), (Preset::Chime, 440)] {
      let w = synth(p, 1.0);
      assert!(w.len().abs_diff(44 + (48 * ms) * 2) <= 4, "{p:?} {}", w.len());
      let s = samples(&w);
      assert_eq!(s[0], 0, "{p:?} starts silent");
      assert!(s.iter().any(|x| x.abs() > 2000), "{p:?} is heard");
      // three tones at 0.35 can stack to 1.05 of full scale only if all peak at once: they don't
      assert!(s.iter().all(|x| x.unsigned_abs() < 32000), "{p:?} never clips");
    }
  }

  #[test]
  fn only_the_four_names_play() {
    assert_eq!(Preset::parse("chime"), Some(Preset::Chime));
    for bad in ["", "none", "Ping", "bell", "../ping"] {
      assert_eq!(Preset::parse(bad), None, "{bad}");
    }
  }

  fn temp(name: &str, bytes: &[u8]) -> String {
    let dir = std::env::temp_dir().join(format!("hsc-sound-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path.to_str().unwrap().to_owned()
  }

  #[test]
  fn a_wav_of_your_own_plays_at_the_volume_asked() {
    let path = temp("own.wav", &wav::encode(22050, 2, &[10_000, -10_000, 400, -400]));
    let (bytes, bad) = from_file(&path, 0.5);
    assert_eq!(bad, None);
    let back = wav::parse(&bytes).unwrap();
    assert_eq!((back.rate, back.channels), (22050, 2), "its own rate and channels, the player resamples");
    assert_eq!(back.samples, vec![5000, -5000, 200, -200]);
    let (loud, _) = from_file(&path, 1.0);
    assert_eq!(wav::parse(&loud).unwrap().samples, vec![10_000, -10_000, 400, -400]);
    // a volume past the top is the top, never a boost past the file's own level
    assert_eq!(from_file(&path, 7.0).0, loud);
    let _ = std::fs::remove_file(path);
  }

  #[test]
  fn a_file_that_will_not_play_becomes_the_ping_and_says_why() {
    let ping = synth(Preset::Ping, 0.3);
    let junk = temp("junk.wav", b"RIFF....not really a wav file");
    assert_eq!(from_file(&junk, 0.3), (ping.clone(), Some(wav::Bad::NotWav)));
    assert_eq!(from_file("/hsc/no/such/file.wav", 0.3), (ping.clone(), Some(wav::Bad::Unreadable)));
    assert_eq!(from_file("", 0.3), (ping, Some(wav::Bad::Unreadable)), "nothing picked yet");
    let _ = std::fs::remove_file(junk);
  }
}
