// ui-side image registry: url → id, decoded frames under a byte cap, lru eviction back to disk
pub mod cache;
pub mod decode;
pub mod url;

use std::collections::HashMap;
use std::time::Instant;

use decode::Decoded;

pub type ImgId = u32;

enum State {
  Idle,
  Loading,
  Ready(Decoded),
  Failed,
}

struct Slot {
  url: Box<str>,
  // logical display height; physical = × scale
  lh: u16,
  state: State,
  last_used: u64,
  // ms since epoch when last drawn; idle frames go back to the disk cache
  seen_ms: u64,
  // width the layout assumed before the image arrived; a mismatch forces a relayout
  guessed_w: u32,
  // chat pictures only: fit inside this many px of width and never enlarge (0 = emote sizing)
  max_w: u32,
  // size once decoded, kept after the frames are dropped so rows hold their height
  dims: (u32, u32),
}

pub struct Images {
  slots: Vec<Slot>,
  by_url: HashMap<(Box<str>, u16), ImgId>,
  bytes: usize,
  cap: usize,
  tick: u64,
  epoch: Instant,
  scale: f32,
  wanted: Vec<(ImgId, Box<str>, u32)>,
  swept_ms: u64,
}

pub const DEFAULT_CAP: usize = 24 << 20;
// off-screen emotes re-decode from disk in ~1ms, so holding them in ram buys nothing
const IDLE_MS: u64 = 60_000;
const SWEEP_MS: u64 = 10_000;

impl Images {
  pub fn new(scale: f32) -> Self {
    Self {
      slots: Vec::new(),
      by_url: HashMap::new(),
      bytes: 0,
      cap: DEFAULT_CAP,
      tick: 0,
      epoch: Instant::now(),
      scale,
      wanted: Vec::new(),
      swept_ms: 0,
    }
  }

  // drop every decoded frame (e.g. hidpi change); they reload at the new size
  pub fn set_scale(&mut self, scale: f32) {
    if scale == self.scale {
      return;
    }
    self.scale = scale;
    for s in &mut self.slots {
      if !matches!(s.state, State::Failed) {
        s.state = State::Idle;
      }
    }
    self.bytes = 0;
  }

  pub fn px(&self, logical: u16) -> u32 {
    (logical as f32 * self.scale).round().max(1.0) as u32
  }

  pub fn id(&mut self, url: &str, lh: u16) -> ImgId {
    let key = (Box::<str>::from(url), lh);
    if let Some(&id) = self.by_url.get(&key) {
      return id;
    }
    let id = self.slots.len() as ImgId;
    self.slots.push(Slot {
      url: key.0.clone(),
      lh,
      state: State::Idle,
      last_used: 0,
      seen_ms: 0,
      guessed_w: 0,
      max_w: 0,
      dims: (0, 0),
    });
    self.by_url.insert(key, id);
    id
  }

  // a chat picture: at most `lh` high and `max_w` px wide, kept at its own size when smaller
  pub fn id_fit(&mut self, url: &str, lh: u16, max_w: u32) -> ImgId {
    let id = self.id(url, lh);
    let s = &mut self.slots[id as usize];
    let max_w = max_w.clamp(1, 0xffff);
    if s.max_w != max_w {
      s.max_w = max_w;
      // the window got narrower: the picture decoded for the old width no longer fits
      if let State::Ready(d) = &s.state
        && d.w > max_w
      {
        self.bytes -= d.bytes();
        s.state = State::Idle;
      }
    }
    id
  }

  // drop the decoded frames of one picture now (a popup that closed); it reloads from disk if asked again
  pub fn release(&mut self, id: ImgId) {
    if let Some(s) = self.slots.get_mut(id as usize)
      && let State::Ready(d) = std::mem::replace(&mut s.state, State::Idle)
    {
      self.bytes -= d.bytes();
    }
  }

  // ask for the picture now (draw time), without drawing it
  pub fn want(&mut self, id: ImgId) {
    let Some(want) = self.slots.get(id as usize).map(|s| self.target(s)) else { return };
    let s = &mut self.slots[id as usize];
    if matches!(s.state, State::Idle) {
      s.state = State::Loading;
      self.wanted.push((id, s.url.clone(), want));
    }
  }

  // decoded size, if it ever decoded; layout reserves exactly this
  pub fn dims(&mut self, id: ImgId) -> Option<(u32, u32)> {
    let s = self.slots.get_mut(id as usize)?;
    (s.dims.0 > 0).then(|| {
      s.guessed_w = s.dims.0;
      s.dims
    })
  }

  pub fn failed(&self, id: ImgId) -> bool {
    self.slots.get(id as usize).is_some_and(|s| matches!(s.state, State::Failed))
  }

  // what the decoder is asked for: physical height, and (chat pictures) max width in the high half
  fn target(&self, s: &Slot) -> u32 {
    self.px(s.lh) | (s.max_w << 16)
  }

  // width at target height: real if decoded, else from provider dims, else square
  pub fn width(&mut self, id: ImgId, native: (u16, u16)) -> u32 {
    let Some(lh) = self.slots.get(id as usize).map(|s| s.lh) else { return 1 };
    let th = self.px(lh);
    let s = &mut self.slots[id as usize];
    if let State::Ready(d) = &s.state {
      return d.w;
    }
    let w = if native.0 > 0 && native.1 > 0 { (native.0 as u32 * th).div_ceil(native.1 as u32) } else { th };
    s.guessed_w = w;
    w
  }

  // frame to draw now, plus ms until it changes (None = static). queues a load if needed
  pub fn frame(&mut self, id: ImgId) -> Option<(&decode::Frame, u32, u32, Option<u32>)> {
    let now = self.epoch.elapsed().as_millis() as u64;
    let h = self.target(self.slots.get(id as usize)?);
    let s = &mut self.slots[id as usize];
    s.last_used = self.tick;
    s.seen_ms = now;
    if matches!(s.state, State::Idle) {
      s.state = State::Loading;
      self.wanted.push((id, s.url.clone(), h));
    }
    let State::Ready(d) = &s.state else { return None };
    if d.frames.len() == 1 {
      return Some((&d.frames[0], d.w, d.h, None));
    }
    let total: u64 = d.frames.iter().map(|f| f.delay_ms as u64).sum::<u64>().max(1);
    let mut t = now % total;
    for f in &d.frames {
      if t < f.delay_ms as u64 {
        return Some((f, d.w, d.h, Some((f.delay_ms as u64 - t) as u32)));
      }
      t -= f.delay_ms as u64;
    }
    d.frames.first().map(|f| (f, d.w, d.h, Some(f.delay_ms)))
  }

  pub fn take_wanted(&mut self) -> Vec<(ImgId, Box<str>, u32)> {
    std::mem::take(&mut self.wanted)
  }

  // returns true when the real width differs from what layout guessed (rows must relayout)
  pub fn loaded(&mut self, id: ImgId, h: u32, res: Result<Decoded, String>) -> bool {
    let Some(want) = self.slots.get(id as usize).map(|s| self.target(s)) else { return false };
    let s = &mut self.slots[id as usize];
    if h != want {
      // decoded for a stale scale; ask again
      s.state = State::Idle;
      return false;
    }
    match res {
      Ok(d) => {
        let relayout = d.w != s.guessed_w;
        s.dims = (d.w, d.h);
        self.bytes += d.bytes();
        s.state = State::Ready(d);
        self.evict();
        relayout
      }
      Err(_) => {
        s.state = State::Failed;
        false
      }
    }
  }

  // called once per frame: anything drawn this frame survives eviction.
  // true when idle frames were dropped (worth a malloc_trim)
  pub fn next_tick(&mut self) -> bool {
    self.tick += 1;
    let now = self.epoch.elapsed().as_millis() as u64;
    if now < self.swept_ms + SWEEP_MS {
      return false;
    }
    self.swept_ms = now;
    let before = self.bytes;
    for s in &mut self.slots {
      if s.seen_ms + IDLE_MS < now
        && let State::Ready(d) = &s.state
      {
        self.bytes -= d.bytes();
        s.state = State::Idle;
      }
    }
    self.bytes < before
  }

  fn evict(&mut self) {
    if self.bytes <= self.cap {
      return;
    }
    let mut ready: Vec<(u64, usize)> = self
      .slots
      .iter()
      .enumerate()
      .filter(|(_, s)| matches!(s.state, State::Ready(_)) && s.last_used + 1 < self.tick)
      .map(|(i, s)| (s.last_used, i))
      .collect();
    ready.sort_unstable();
    for (_, i) in ready {
      if self.bytes <= self.cap {
        break;
      }
      let s = &mut self.slots[i];
      if let State::Ready(d) = std::mem::replace(&mut s.state, State::Idle) {
        self.bytes -= d.bytes();
      }
    }
  }

  pub fn bytes(&self) -> usize {
    self.bytes
  }
}
