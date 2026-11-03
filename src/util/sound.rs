// the mention ping (ext mentions.js playMentionPing): a sine at 880 Hz and another at 1175 Hz from
// 80ms, 10ms up to full, then falling away to silence by 300ms. made here, no sound file shipped, and
// played by the first of pw-play, paplay, aplay on PATH, fed a wav on stdin. best effort: no player
// is said once on stderr. a burst of pings sounds once per 1.2s, so a raid of @s can't machine-gun
use std::f32::consts::TAU;
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};

use super::open::installed;

const RATE: u32 = 48_000;
const GAP_MS: i64 = 1200;
const PLAYERS: [(&str, &[&str]); 3] = [("pw-play", &["-"]), ("paplay", &[]), ("aplay", &["-q", "-"])];

// the ping as a 16-bit mono wav, `volume` 0..1 (the ext's gain is 0.35 of it, per tone)
fn wav(volume: f32) -> Vec<u8> {
  let n = (RATE as f32 * 0.32) as usize;
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
    let env = match t {
      t if t < 0.01 => t / 0.01,
      t if t < 0.3 => 0.0001f32.powf((t - 0.01) / 0.29),
      _ => 0.0,
    };
    let mut s = (TAU * 880.0 * t).sin();
    if t >= 0.08 {
      s += (TAU * 1175.0 * (t - 0.08)).sin();
    }
    out.extend_from_slice(&((s * env * peak) as i16).to_le_bytes());
  }
  out
}

// ping at `percent` volume (0 = silent)
pub fn ping(percent: u8, now_ms: i64) {
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
  let bytes = wav(f32::from(percent.min(100)) / 100.0);
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

  #[test]
  fn the_ping_is_a_short_quiet_wav() {
    let w = wav(0.3);
    assert_eq!(&w[..4], b"RIFF");
    assert_eq!(&w[36..40], b"data");
    assert_eq!(w.len(), 44 + 15_360 * 2, "320ms at 48k");
    let samples: Vec<i16> = w[44..].chunks(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect();
    let loudest = samples.iter().map(|s| s.unsigned_abs()).max().unwrap();
    // two tones at 0.35 × 0.3 each: never past 0.21 of full scale
    assert!(loudest > 2000 && loudest <= (0.21 * 32767.0) as u16 + 1, "{loudest}");
    assert_eq!(samples[0], 0, "starts silent, no click");
    assert!(samples[samples.len() - 1] == 0, "ends silent");
    assert!(wav(1.0)[44..].chunks(2).all(|c| i16::from_le_bytes([c[0], c[1]]) != i16::MIN), "never clips");
  }
}
