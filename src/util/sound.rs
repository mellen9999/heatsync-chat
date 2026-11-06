// the sounds (ext mentions.js): the mention ping, and the four a highlight rule can pick (playFilterRuleSound).
// each is a few sine tones, (hz, start, length), 10ms up to full then falling away to silence at its end. made
// here, no sound file shipped, and played by the first of pw-play, paplay, aplay on PATH, fed a wav on stdin.
// best effort: no player is said once on stderr. a burst sounds once per 1.2s, so a raid of @s can't
// machine-gun
use std::f32::consts::TAU;
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};

use super::open::installed;

const RATE: u32 = 48_000;
const GAP_MS: i64 = 1200;
const PLAYERS: [(&str, &[&str]); 3] = [("pw-play", &["-"]), ("paplay", &[]), ("aplay", &["-q", "-"])];

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

// the sound as a 16-bit mono wav, `volume` 0..1 (the ext's gain is 0.35 of it, per tone)
fn wav(p: Preset, volume: f32) -> Vec<u8> {
  let tones = p.tones();
  let secs = tones.iter().map(|t| t.1 + t.2).fold(0.0, f32::max);
  let n = (RATE as f32 * secs) as usize;
  let peak = volume.clamp(0.0, 1.0) * 0.35 * f32::from(i16::MAX);
  let data = (n * 2) as u32;
  let mut out = Vec::with_capacity(44 + n * 2);
  out.extend_from_slice(b"RIFF");
  out.extend_from_slice(&(36 + data).to_le_bytes());
  out.extend_from_slice(b"WAVEfmt ");
  out.extend_from_slice(&16u32.to_le_bytes());
  out.extend_from_slice(&1u16.to_le_bytes());
  out.extend_from_slice(&1u16.to_le_bytes());
  out.extend_from_slice(&RATE.to_le_bytes());
  out.extend_from_slice(&(RATE * 2).to_le_bytes());
  out.extend_from_slice(&2u16.to_le_bytes());
  out.extend_from_slice(&16u16.to_le_bytes());
  out.extend_from_slice(b"data");
  out.extend_from_slice(&data.to_le_bytes());
  for i in 0..n {
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
    out.extend_from_slice(&((s * peak) as i16).to_le_bytes());
  }
  out
}

// the mention ping at `percent` volume (0 = silent)
pub fn ping(percent: u8, now_ms: i64) {
  play(Preset::Ping, percent, now_ms)
}

// a sound at `percent` volume (0 = silent)
pub fn play(p: Preset, percent: u8, now_ms: i64) {
  static LAST: AtomicI64 = AtomicI64::new(i64::MIN / 2);
  static MISSING: AtomicBool = AtomicBool::new(false);
  if percent == 0 || now_ms - LAST.load(Ordering::Relaxed) < GAP_MS {
    return;
  }
  LAST.store(now_ms, Ordering::Relaxed);
  let Some((bin, args)) = PLAYERS.iter().find(|(b, _)| installed(b)) else {
    if !MISSING.swap(true, Ordering::Relaxed) {
      eprintln!("the mention sound needs pw-play, paplay or aplay");
    }
    return;
  };
  let spawned = Command::new(bin).args(*args).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
  let Ok(mut child) = spawned else { return };
  let bytes = wav(p, f32::from(percent.min(100)) / 100.0);
  // fed and reaped off the ui thread: a stuck player never stalls a frame or leaves a zombie
  std::thread::spawn(move || {
    if let Some(mut stdin) = child.stdin.take() {
      let _ = stdin.write_all(&bytes);
    }
    let _ = child.wait();
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
    let w = wav(Preset::Ping, 0.3);
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
      wav(Preset::Ping, 1.0)[44..].chunks(2).all(|c| i16::from_le_bytes([c[0], c[1]]) != i16::MIN),
      "never clips"
    );
  }

  #[test]
  fn each_preset_is_its_own_length_and_stays_quiet() {
    // 120ms, 230ms, 440ms: the ext's presets
    for (p, ms) in [(Preset::Blip, 120), (Preset::Knock, 230), (Preset::Chime, 440)] {
      let w = wav(p, 1.0);
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
}
