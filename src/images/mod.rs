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
  state: State,
  last_used: u64,
  // width the layout assumed before the image arrived; a mismatch forces a relayout
  guessed_w: u32,
}

pub struct Images {
  slots: Vec<Slot>,
  by_url: HashMap<Box<str>, ImgId>,
  bytes: usize,
  cap: usize,
  tick: u64,
  epoch: Instant,
  pub target_h: u32,
  wanted: Vec<(ImgId, Box<str>)>,
}

pub const DEFAULT_CAP: usize = 24 << 20;

impl Images {
  pub fn new(target_h: u32) -> Self {
    Self {
      slots: Vec::new(),
      by_url: HashMap::new(),
      bytes: 0,
      cap: DEFAULT_CAP,
      tick: 0,
      epoch: Instant::now(),
      target_h,
      wanted: Vec::new(),
    }
  }

  // drop every decoded frame (e.g. hidpi change); they reload at the new size
  pub fn set_target_h(&mut self, h: u32) {
    if h == self.target_h {
      return;
    }
    self.target_h = h;
    for s in &mut self.slots {
      if !matches!(s.state, State::Failed) {
        s.state = State::Idle;
      }
    }
    self.bytes = 0;
  }

  pub fn id(&mut self, url: &str) -> ImgId {
    if let Some(&id) = self.by_url.get(url) {
      return id;
    }
    let id = self.slots.len() as ImgId;
    self.slots.push(Slot {
      url: url.into(),
      state: State::Idle,
      last_used: 0,
      guessed_w: 0,
    });
    self.by_url.insert(url.into(), id);
    id
  }

  // width at target height: real if decoded, else from provider dims, else square
  pub fn width(&mut self, id: ImgId, native: (u16, u16)) -> u32 {
    let th = self.target_h;
    let Some(s) = self.slots.get_mut(id as usize) else {
      return th;
    };
    if let State::Ready(d) = &s.state {
      return d.w;
    }
    let w = if native.0 > 0 && native.1 > 0 {
      (native.0 as u32 * th).div_ceil(native.1 as u32)
    } else {
      th
    };
    s.guessed_w = w;
    w
  }

  // frame to draw now, plus ms until it changes (None = static). queues a load if needed
  pub fn frame(&mut self, id: ImgId) -> Option<(&decode::Frame, u32, u32, Option<u32>)> {
    let now = self.epoch.elapsed().as_millis() as u64;
    let s = self.slots.get_mut(id as usize)?;
    s.last_used = self.tick;
    if matches!(s.state, State::Idle) {
      s.state = State::Loading;
      self.wanted.push((id, s.url.clone()));
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

  pub fn take_wanted(&mut self) -> Vec<(ImgId, Box<str>)> {
    std::mem::take(&mut self.wanted)
  }

  // returns true when the real width differs from what layout guessed (rows must relayout)
  pub fn loaded(&mut self, id: ImgId, h: u32, res: Result<Decoded, String>) -> bool {
    let Some(s) = self.slots.get_mut(id as usize) else {
      return false;
    };
    if h != self.target_h {
      // decoded for a stale scale; ask again
      s.state = State::Idle;
      return false;
    }
    match res {
      Ok(d) => {
        let relayout = d.w != s.guessed_w;
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

  // called once per frame: anything drawn this frame survives eviction
  pub fn next_tick(&mut self) {
    self.tick += 1;
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
