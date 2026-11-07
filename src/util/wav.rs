// wav files by hand: the sounds we make are written out as one, and a sound you pick yourself for the
// mention ping is read from one. plain pcm only (8 or 16 bit, mono or stereo), at most 2MB and 3 seconds.
// a file you point us at is hostile input: every size it claims is checked against the bytes really there
// before it is used, nothing is allocated from a number the file states, and anything else is a plain error
use std::io::Read;

// the biggest file read, and the longest sound played
pub const MAX_BYTES: usize = 2 << 20;
pub const MAX_MS: u64 = 3000;
// rates a player takes without fuss
const RATES: std::ops::RangeInclusive<u32> = 8_000..=192_000;
const HEADER: usize = 44;

#[derive(Debug, PartialEq, Eq)]
pub enum Bad {
  // no such file, a folder, or a file we cannot open
  Unreadable,
  TooBig,
  NotWav,
  // a wav, but not plain 8 / 16 bit pcm in one or two channels
  Format,
  TooLong,
  Silent,
}

impl Bad {
  // one plain line for the status bar
  pub fn said(&self) -> &'static str {
    match self {
      Bad::Unreadable => "can't open that file",
      Bad::TooBig => "that file is too big: 2mb at most",
      Bad::NotWav => "that isn't a wav file",
      Bad::Format => "that wav isn't plain pcm: 8 or 16 bit, mono or stereo",
      Bad::TooLong => "that sound is too long: 3 seconds at most",
      Bad::Silent => "that wav has no sound in it",
    }
  }
}

// interleaved 16-bit samples, whatever the file held
#[derive(Debug, PartialEq, Eq)]
pub struct Pcm {
  pub rate: u32,
  pub channels: u16,
  pub samples: Vec<i16>,
}

fn le16(b: &[u8], at: usize) -> Option<u16> {
  Some(u16::from_le_bytes(b.get(at..at.checked_add(2)?)?.try_into().ok()?))
}

fn le32(b: &[u8], at: usize) -> Option<u32> {
  Some(u32::from_le_bytes(b.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

// (rate, channels, bits) of a plain pcm `fmt ` chunk, or why it is not one
fn format(f: &[u8]) -> Result<(u32, u16, u16), Bad> {
  let (tag, channels, rate) = (le16(f, 0).ok_or(Bad::NotWav)?, le16(f, 2).ok_or(Bad::NotWav)?, le32(f, 4));
  let (align, bits) = (le16(f, 12).ok_or(Bad::NotWav)?, le16(f, 14).ok_or(Bad::NotWav)?);
  let rate = rate.ok_or(Bad::NotWav)?;
  let plain = tag == 1 && matches!(channels, 1 | 2) && matches!(bits, 8 | 16) && RATES.contains(&rate);
  // the frame size the file states must be the one its own numbers make
  if !plain || u32::from(align) != u32::from(channels) * u32::from(bits) / 8 {
    return Err(Bad::Format);
  }
  Ok((rate, channels, bits))
}

pub fn parse(b: &[u8]) -> Result<Pcm, Bad> {
  if b.len() > MAX_BYTES {
    return Err(Bad::TooBig);
  }
  if b.get(..4) != Some(b"RIFF") || b.get(8..12) != Some(b"WAVE") {
    return Err(Bad::NotWav);
  }
  let (mut at, mut fmt, mut data) = (12usize, None, None);
  // chunks: id, size, body, padded to an even length. the walk moves at least 8 bytes a turn
  while let (Some(id), Some(size)) = (b.get(at..at + 4), le32(b, at + 4)) {
    let body = at + 8;
    // a size past the end of the file is cut to what is there: streamed files state a huge one
    let end = body.saturating_add(size as usize).min(b.len());
    match id {
      b"fmt " if fmt.is_none() => fmt = Some(format(&b[body..end])?),
      b"data" if data.is_none() => data = Some(&b[body..end]),
      _ => {}
    }
    if fmt.is_some() && data.is_some() {
      break;
    }
    at = end.saturating_add((end - body) & 1);
  }
  let ((rate, channels, bits), data) = fmt.zip(data).ok_or(Bad::NotWav)?;
  let width = usize::from(bits / 8);
  // whole frames only: a cut-off last one is dropped
  let frame = width * usize::from(channels);
  let data = &data[..data.len() / frame * frame];
  if data.is_empty() {
    return Err(Bad::Silent);
  }
  if (data.len() / frame) as u64 * 1000 > MAX_MS * u64::from(rate) {
    return Err(Bad::TooLong);
  }
  let samples = if width == 1 {
    data.iter().map(|&u| (i16::from(u) - 128) << 8).collect()
  } else {
    data.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect()
  };
  Ok(Pcm { rate, channels, samples })
}

// a file by path: `~/` is your home, only a regular file of at most 2MB is opened (a pipe would block)
pub fn read(path: &str) -> Result<Pcm, Bad> {
  let path = home(path);
  let meta = std::fs::metadata(&path).map_err(|_| Bad::Unreadable)?;
  if !meta.is_file() {
    return Err(Bad::Unreadable);
  }
  if meta.len() > MAX_BYTES as u64 {
    return Err(Bad::TooBig);
  }
  let mut bytes = Vec::new();
  let file = std::fs::File::open(&path).map_err(|_| Bad::Unreadable)?;
  // a file that grew since the look is cut at the cap and refused
  file.take(MAX_BYTES as u64 + 1).read_to_end(&mut bytes).map_err(|_| Bad::Unreadable)?;
  parse(&bytes)
}

fn home(path: &str) -> std::path::PathBuf {
  match (path.strip_prefix("~/"), super::dirs::home()) {
    (Some(rest), Some(h)) => h.join(rest),
    _ => path.into(),
  }
}

// 16-bit pcm as a wav file: the 44-byte header, then the samples
pub fn encode(rate: u32, channels: u16, samples: &[i16]) -> Vec<u8> {
  let data = (samples.len() * 2) as u32;
  let frame = channels * 2;
  let mut out = Vec::with_capacity(HEADER + samples.len() * 2);
  out.extend_from_slice(b"RIFF");
  out.extend_from_slice(&(36 + data).to_le_bytes());
  out.extend_from_slice(b"WAVEfmt ");
  out.extend_from_slice(&16u32.to_le_bytes());
  out.extend_from_slice(&1u16.to_le_bytes());
  out.extend_from_slice(&channels.to_le_bytes());
  out.extend_from_slice(&rate.to_le_bytes());
  out.extend_from_slice(&(rate * u32::from(frame)).to_le_bytes());
  out.extend_from_slice(&frame.to_le_bytes());
  out.extend_from_slice(&16u16.to_le_bytes());
  out.extend_from_slice(b"data");
  out.extend_from_slice(&data.to_le_bytes());
  for s in samples {
    out.extend_from_slice(&s.to_le_bytes());
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  // a hand-made file: fmt (tag, channels, rate, bits) then a data chunk of `data`
  fn file(tag: u16, channels: u16, rate: u32, bits: u16, data: &[u8]) -> Vec<u8> {
    let align = channels * bits / 8;
    let mut b = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
    b.extend_from_slice(&16u32.to_le_bytes());
    for v in [tag, channels] {
      b.extend_from_slice(&v.to_le_bytes());
    }
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * u32::from(align)).to_le_bytes());
    for v in [align, bits] {
      b.extend_from_slice(&v.to_le_bytes());
    }
    b.extend_from_slice(b"data");
    b.extend_from_slice(&(data.len() as u32).to_le_bytes());
    b.extend_from_slice(data);
    let n = (b.len() - 8) as u32;
    b[4..8].copy_from_slice(&n.to_le_bytes());
    b
  }

  fn ok16(frames: usize) -> Vec<u8> {
    file(1, 1, 8000, 16, &vec![1; frames * 2])
  }

  #[test]
  fn plain_16_bit_mono_reads_back_exactly() {
    let pcm = parse(&file(1, 1, 44100, 16, &[0, 0, 0xff, 0x7f, 0x00, 0x80, 0x34, 0x12])).unwrap();
    assert_eq!(pcm, Pcm { rate: 44100, channels: 1, samples: vec![0, 32767, -32768, 0x1234] });
  }

  #[test]
  fn eight_bit_stereo_becomes_signed_16() {
    // 8-bit is unsigned with silence at 128
    let pcm = parse(&file(1, 2, 22050, 8, &[128, 255, 0, 129])).unwrap();
    assert_eq!((pcm.rate, pcm.channels), (22050, 2));
    assert_eq!(pcm.samples, vec![0, 127 << 8, -128 << 8, 256]);
  }

  #[test]
  fn what_we_write_is_what_we_read() {
    let samples: Vec<i16> = (-500..500).map(|i| i * 60).collect();
    for ch in [1u16, 2] {
      let back = parse(&encode(48000, ch, &samples)).unwrap();
      assert_eq!((back.rate, back.channels, back.samples), (48000, ch, samples.clone()));
    }
    let w = encode(48000, 1, &[1, 2, 3]);
    assert_eq!((&w[..4], &w[36..40], w.len()), (&b"RIFF"[..], &b"data"[..], 44 + 6));
  }

  #[test]
  fn other_chunks_are_skipped_and_odd_ones_are_padded() {
    let mut b = b"RIFF\0\0\0\0WAVE".to_vec();
    // an odd-sized junk chunk (3 bytes + 1 pad), then the real fmt and data
    b.extend_from_slice(b"junk\x03\0\0\0abc\0");
    b.extend_from_slice(&ok16(10)[12..]);
    assert_eq!(parse(&b).unwrap().samples.len(), 10);
    // a bigger fmt chunk (18 bytes: the extension size) is fine too
    let mut b = b"RIFF\0\0\0\0WAVEfmt \x12\0\0\0".to_vec();
    b.extend_from_slice(&[1, 0, 1, 0, 0x40, 0x1f, 0, 0, 0x80, 0x3e, 0, 0, 2, 0, 16, 0, 0, 0]);
    b.extend_from_slice(b"data\x04\0\0\0\x01\0\x02\0");
    assert_eq!(parse(&b).unwrap().samples, vec![1, 2]);
  }

  #[test]
  fn a_size_that_lies_is_cut_to_what_is_there() {
    // a streamed file states a huge data size; a cut one states more than it has
    let mut b = ok16(100);
    b[40..44].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(parse(&b).unwrap().samples.len(), 100);
    let cut = &ok16(100)[..44 + 51];
    assert_eq!(parse(cut).unwrap().samples.len(), 25, "a cut-off frame is dropped");
  }

  #[test]
  fn not_a_wav_is_said_so() {
    for b in
      [&b""[..], b"RIFF", b"RIFX\0\0\0\0WAVE", b"RIFF\0\0\0\0AVI LIST", b"\x89PNG\r\n\x1a\n", b"RIFF\0\0\0\0WAVE"]
    {
      assert_eq!(parse(b), Err(Bad::NotWav), "{b:?}");
    }
    // a header with no data chunk, and data with no format
    assert_eq!(parse(&ok16(4)[..36]), Err(Bad::NotWav));
    let mut b = b"RIFF\0\0\0\0WAVE".to_vec();
    b.extend_from_slice(&ok16(4)[36..]);
    assert_eq!(parse(&b), Err(Bad::NotWav));
  }

  #[test]
  fn only_plain_pcm_in_one_or_two_channels_plays() {
    for (tag, ch, rate, bits) in [
      (3, 1, 44100, 32),
      (3, 1, 44100, 16),
      (0xfffe, 2, 44100, 16),
      (1, 1, 44100, 24),
      (1, 1, 44100, 32),
      (1, 1, 44100, 4),
      (1, 0, 44100, 16),
      (1, 3, 44100, 16),
      (1, 6, 44100, 16),
      (1, 1, 0, 16),
      (1, 1, 100, 16),
      (1, 1, u32::MAX, 16),
    ] {
      assert_eq!(parse(&file(tag, ch, rate, bits, &[0; 64])), Err(Bad::Format), "{tag} {ch} {rate} {bits}");
    }
    // a frame size that doesn't fit the format it claims
    let mut b = ok16(8);
    b[32] = 7;
    assert_eq!(parse(&b), Err(Bad::Format));
    // a fmt chunk too short to hold the format
    let mut b = b"RIFF\0\0\0\0WAVEfmt \x04\0\0\0\x01\0\x01\0".to_vec();
    b.extend_from_slice(b"data\x02\0\0\0\0\0");
    assert_eq!(parse(&b), Err(Bad::NotWav));
  }

  #[test]
  fn three_seconds_is_the_longest_and_empty_is_silent() {
    assert_eq!(parse(&file(1, 1, 8000, 16, &vec![0; 8000 * 3 * 2])).map(|p| p.samples.len()), Ok(24000));
    assert_eq!(parse(&file(1, 1, 8000, 16, &vec![0; 8001 * 3 * 2])), Err(Bad::TooLong));
    // stereo counts frames, not samples: 4 seconds is 4 × 44100 frames of 4 bytes, 3 seconds still fits
    assert_eq!(parse(&file(1, 2, 44100, 16, &vec![0; 44100 * 4 * 4])), Err(Bad::TooLong));
    assert_eq!(parse(&file(1, 2, 44100, 16, &vec![0; 44100 * 3 * 4])).map(|p| p.samples.len()), Ok(44100 * 3 * 2));
    assert_eq!(parse(&file(1, 1, 8000, 16, &[])), Err(Bad::Silent));
    assert_eq!(parse(&file(1, 2, 8000, 16, &[1, 2, 3])), Err(Bad::Silent), "less than one frame");
    assert_eq!(parse(&vec![b'x'; MAX_BYTES + 1]), Err(Bad::TooBig));
  }

  // chunk sizes made to wrap, overlap or run past the end must end the walk, never panic or allocate
  #[test]
  fn hostile_sizes_end_the_walk() {
    for size in [u32::MAX, u32::MAX - 1, u32::MAX - 7, 0x8000_0000, 1 << 24, 0] {
      let mut b = b"RIFF\0\0\0\0WAVEjunk".to_vec();
      b.extend_from_slice(&size.to_le_bytes());
      b.extend_from_slice(&ok16(4)[12..]);
      let _ = parse(&b);
      let mut b = ok16(4);
      b[16..20].copy_from_slice(&size.to_le_bytes());
      let _ = parse(&b);
      let mut b = ok16(4);
      b[40..44].copy_from_slice(&size.to_le_bytes());
      let _ = parse(&b);
    }
    // a zero-size chunk repeated to the cap makes progress each turn
    let mut b = b"RIFF\0\0\0\0WAVE".to_vec();
    for _ in 0..10_000 {
      b.extend_from_slice(b"pad \0\0\0\0");
    }
    assert_eq!(parse(&b), Err(Bad::NotWav));
  }

  // every cut of a good file, and a lot of noise with a real header on it: an answer, never a panic
  #[test]
  fn truncated_and_noisy_files_never_panic() {
    let good = ok16(64);
    for n in 0..good.len() {
      let _ = parse(&good[..n]);
    }
    let mut x = 0x1234_5678u32;
    let mut next = || {
      x ^= x << 13;
      x ^= x >> 17;
      x ^= x << 5;
      x
    };
    for _ in 0..3000 {
      let mut b = good.clone();
      for _ in 0..(next() % 8 + 1) {
        let i = next() as usize % b.len();
        b[i] = next() as u8;
      }
      let _ = parse(&b);
      let junk: Vec<u8> = (0..next() % 200).map(|_| next() as u8).collect();
      let _ = parse(&junk);
    }
  }

  #[test]
  fn files_come_by_path_and_only_regular_ones() {
    let dir = std::env::temp_dir().join(format!("hsc-wav-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let ok = dir.join("ok.wav");
    std::fs::write(&ok, ok16(80)).unwrap();
    assert_eq!(read(ok.to_str().unwrap()).map(|p| p.samples.len()), Ok(80));
    let bad = dir.join("bad.wav");
    std::fs::write(&bad, b"not a wav at all").unwrap();
    assert_eq!(read(bad.to_str().unwrap()), Err(Bad::NotWav));
    let big = dir.join("big.wav");
    std::fs::write(&big, vec![0u8; MAX_BYTES + 1]).unwrap();
    assert_eq!(read(big.to_str().unwrap()), Err(Bad::TooBig));
    assert_eq!(read(dir.to_str().unwrap()), Err(Bad::Unreadable), "a folder");
    assert_eq!(read(dir.join("none.wav").to_str().unwrap()), Err(Bad::Unreadable));
    #[cfg(unix)]
    assert_eq!(read("/dev/zero"), Err(Bad::Unreadable), "a device is not a file");
    assert_eq!(read(""), Err(Bad::Unreadable));
    let _ = std::fs::remove_dir_all(&dir);
  }

  #[test]
  fn tilde_is_home_and_nothing_else_is() {
    assert_eq!(home("/a/b.wav"), std::path::PathBuf::from("/a/b.wav"));
    assert_eq!(home("~x/b.wav"), std::path::PathBuf::from("~x/b.wav"));
    if let Some(h) = crate::util::dirs::home() {
      assert_eq!(home("~/s/b.wav"), h.join("s/b.wav"));
    }
  }

  #[test]
  fn every_error_says_something_plain() {
    for b in [Bad::Unreadable, Bad::TooBig, Bad::NotWav, Bad::Format, Bad::TooLong, Bad::Silent] {
      assert!(!b.said().is_empty() && b.said() == b.said().to_lowercase());
    }
  }
}
