// ui-side image registry: url → id, decoded frames under a byte cap, lru eviction back to disk
pub mod cache;
pub mod decode;
pub mod svg;
pub mod url;

use std::collections::HashMap;
use std::time::Instant;

use decode::Decoded;

// an id is the slot's index (low 20 bits) and its era (high 12): a slot that was recycled for
// another picture has a new era, so an id a chat line kept from before can never show the new one
pub type ImgId = u32;

const IX_BITS: u32 = 20;
const IX_MASK: u32 = (1 << IX_BITS) - 1;
const ERA_MASK: u32 = 0xfff;
// the id handed out if the registry were ever full: no slot answers to it
const NO_IMG: ImgId = u32::MAX;

enum State {
  Idle,
  Loading,
  Ready(Decoded),
  Failed,
}

struct Slot {
  // false once recycled: waiting on the free list for the next url
  live: bool,
  era: u32,
  // ms since epoch when last registered, drawn or asked for; a picture quiet this long is recycled
  used_ms: u64,
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
  // how many times in a row it failed, and when (ms since epoch): a failure is tried again later
  fails: u8,
  failed_ms: u64,
}

pub struct Images {
  slots: Vec<Slot>,
  free: Vec<u32>,
  by_url: HashMap<(Box<str>, u16), ImgId>,
  bytes: usize,
  cap: usize,
  tick: u64,
  epoch: Instant,
  scale: f32,
  wanted: Vec<(ImgId, Box<str>, u32)>,
  swept_ms: u64,
  // a failed picture was given another chance since the app last asked
  revived: bool,
}

pub const DEFAULT_CAP: usize = 24 << 20;
// off-screen emotes re-decode from disk in ~1ms, so holding them in ram buys nothing
const IDLE_MS: u64 = 60_000;
const SWEEP_MS: u64 = 10_000;
// a picture that failed (offline, a 5xx, a bad file) is tried again after this, doubling each time it
// fails again, up to 8×: 2, 4, 8, 16 minutes
const RETRY_FAILED_MS: u64 = 120_000;
// a picture nobody has drawn or asked for in this long (and holding no frames) gives its slot back
const RECYCLE_MS: u64 = 30 * 60_000;

impl Images {
  pub fn new(scale: f32) -> Self {
    Self {
      slots: Vec::new(),
      free: Vec::new(),
      by_url: HashMap::new(),
      bytes: 0,
      cap: DEFAULT_CAP,
      tick: 0,
      epoch: Instant::now(),
      scale,
      wanted: Vec::new(),
      swept_ms: 0,
      revived: false,
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

  fn now_ms(&self) -> u64 {
    self.epoch.elapsed().as_millis() as u64
  }

  // the slot an id still points at; None for an id from before its slot was recycled
  fn ix(&self, id: ImgId) -> Option<usize> {
    let i = (id & IX_MASK) as usize;
    self.slots.get(i).filter(|s| s.live && s.era == id >> IX_BITS).map(|_| i)
  }

  // an id whose slot was recycled: whoever holds it must lay out again and ask by url
  fn note_stale(&mut self, id: ImgId) {
    if id != NO_IMG && self.ix(id).is_none() && ((id & IX_MASK) as usize) < self.slots.len() {
      self.revived = true;
    }
  }

  pub fn id(&mut self, url: &str, lh: u16) -> ImgId {
    let key = (Box::<str>::from(url), lh);
    let now = self.now_ms();
    if let Some(&id) = self.by_url.get(&key) {
      if let Some(i) = self.ix(id) {
        self.slots[i].used_ms = now;
      }
      return id;
    }
    let fresh = Slot {
      live: true,
      era: 0,
      used_ms: now,
      url: key.0.clone(),
      lh,
      state: State::Idle,
      last_used: 0,
      seen_ms: 0,
      guessed_w: 0,
      max_w: 0,
      dims: (0, 0),
      fails: 0,
      failed_ms: 0,
    };
    let id = if let Some(i) = self.free.pop() {
      let era = (self.slots[i as usize].era + 1) & ERA_MASK;
      self.slots[i as usize] = Slot { era, ..fresh };
      i | era << IX_BITS
    } else if self.slots.len() < IX_MASK as usize {
      self.slots.push(fresh);
      (self.slots.len() - 1) as ImgId
    } else {
      return NO_IMG;
    };
    self.by_url.insert(key, id);
    id
  }

  // a chat picture: at most `lh` high and `max_w` px wide, kept at its own size when smaller
  pub fn id_fit(&mut self, url: &str, lh: u16, max_w: u32) -> ImgId {
    let id = self.id(url, lh);
    let Some(i) = self.ix(id) else { return id };
    let s = &mut self.slots[i];
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

  // where a picture comes from
  pub fn url(&self, id: ImgId) -> Option<&str> {
    self.ix(id).map(|i| &*self.slots[i].url)
  }

  // drop the decoded frames of one picture now (a popup that closed); it reloads from disk if asked again
  pub fn release(&mut self, id: ImgId) {
    if let Some(i) = self.ix(id)
      && let State::Ready(d) = std::mem::replace(&mut self.slots[i].state, State::Idle)
    {
      self.bytes -= d.bytes();
    }
  }

  // ask for the picture now (draw time), without drawing it
  pub fn want(&mut self, id: ImgId) {
    let Some(i) = self.ix(id) else { return self.note_stale(id) };
    let (want, now) = (self.target(&self.slots[i]), self.now_ms());
    let s = &mut self.slots[i];
    s.used_ms = now;
    if matches!(s.state, State::Idle) {
      s.state = State::Loading;
      self.wanted.push((id, s.url.clone(), want));
    }
  }

  // decoded size, if it ever decoded; layout reserves exactly this
  pub fn dims(&mut self, id: ImgId) -> Option<(u32, u32)> {
    let Some(i) = self.ix(id) else {
      self.note_stale(id);
      return None;
    };
    let s = &mut self.slots[i];
    (s.dims.0 > 0).then(|| {
      s.guessed_w = s.dims.0;
      s.dims
    })
  }

  pub fn failed(&self, id: ImgId) -> bool {
    self.ix(id).is_some_and(|i| matches!(self.slots[i].state, State::Failed))
  }

  // what the decoder is asked for: physical height, and (chat pictures) max width in the high half
  fn target(&self, s: &Slot) -> u32 {
    self.px(s.lh) | (s.max_w << 16)
  }

  // width at target height: real if decoded, else from provider dims, else square
  pub fn width(&mut self, id: ImgId, native: (u16, u16)) -> u32 {
    let Some(i) = self.ix(id) else {
      self.note_stale(id);
      return 1;
    };
    let th = self.px(self.slots[i].lh);
    let s = &mut self.slots[i];
    if let State::Ready(d) = &s.state {
      return d.w;
    }
    let w = if native.0 > 0 && native.1 > 0 { (native.0 as u32 * th).div_ceil(native.1 as u32) } else { th };
    s.guessed_w = w;
    w
  }

  // frame to draw now, plus ms until it changes (None = static). queues a load if needed
  pub fn frame(&mut self, id: ImgId) -> Option<(&decode::Frame, u32, u32, Option<u32>)> {
    self.frame_with(id, true)
  }

  // `play` false holds an animated image on its first frame (and asks for no ticks)
  pub fn frame_with(&mut self, id: ImgId, play: bool) -> Option<(&decode::Frame, u32, u32, Option<u32>)> {
    let now = self.now_ms();
    let Some(i) = self.ix(id) else {
      self.note_stale(id);
      return None;
    };
    let h = self.target(&self.slots[i]);
    let s = &mut self.slots[i];
    s.used_ms = now;
    s.last_used = self.tick;
    s.seen_ms = now;
    if matches!(s.state, State::Idle) {
      s.state = State::Loading;
      self.wanted.push((id, s.url.clone(), h));
    }
    let State::Ready(d) = &s.state else { return None };
    if d.frames.len() == 1 || !play {
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
    let Some(i) = self.ix(id) else { return false };
    let want = self.target(&self.slots[i]);
    let s = &mut self.slots[i];
    if h != want {
      // decoded for a stale scale; ask again
      s.state = State::Idle;
      return false;
    }
    match res {
      Ok(d) => {
        s.fails = 0;
        let relayout = d.w != s.guessed_w;
        s.dims = (d.w, d.h);
        self.bytes += d.bytes();
        s.state = State::Ready(d);
        self.evict();
        relayout
      }
      Err(_) => {
        s.state = State::Failed;
        s.fails = s.fails.saturating_add(1);
        s.failed_ms = self.epoch.elapsed().as_millis() as u64;
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
    self.sweep(now)
  }

  // true once if a failed picture was given another chance (rows that left it out must lay out again)
  pub fn take_revived(&mut self) -> bool {
    std::mem::take(&mut self.revived)
  }

  fn sweep(&mut self, now: u64) -> bool {
    let before = self.bytes;
    for (i, s) in self.slots.iter_mut().enumerate() {
      if !s.live {
        continue;
      }
      if matches!(s.state, State::Failed) && now >= s.failed_ms + (RETRY_FAILED_MS << (s.fails.max(1) - 1).min(3)) {
        s.state = State::Idle;
        self.revived = true;
      }
      if s.seen_ms + IDLE_MS < now
        && let State::Ready(d) = &s.state
      {
        self.bytes -= d.bytes();
        s.state = State::Idle;
      }
      // nothing in it, nobody asking for a long while: the slot (and its url) go back for the next picture
      if matches!(s.state, State::Idle | State::Failed) && s.used_ms + RECYCLE_MS < now {
        s.live = false;
        self.by_url.remove(&(std::mem::take(&mut s.url), s.lh));
        self.free.push(i as u32);
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

#[cfg(test)]
mod tests {
  use super::*;

  // one picture asked for and failed at `at` ms
  fn failed_at(im: &mut Images, id: ImgId, at: u64) {
    let h = im.target(&im.slots[id as usize]);
    im.slots[id as usize].state = State::Loading;
    assert!(!im.loaded(id, h, Err("http 503".into())));
    im.slots[id as usize].failed_ms = at;
  }

  #[test]
  fn a_failed_picture_is_tried_again_later_not_never() {
    let mut im = Images::new(1.0);
    let id = im.id("https://cdn.7tv.app/emote/x/1x.webp", 28);
    failed_at(&mut im, id, 1_000);
    assert!(im.failed(id));
    // soon after: still failed, no retry (a down server isn't hammered)
    im.sweep(1_000 + 60_000);
    assert!(im.failed(id) && !im.take_revived());
    // a couple of minutes on: it goes back to idle, and the app is told to lay rows out again
    im.sweep(1_000 + RETRY_FAILED_MS);
    assert!(!im.failed(id));
    assert!(im.take_revived());
    assert!(!im.take_revived(), "told once");
    // drawing it asks for it again
    assert!(im.frame(id).is_none());
    assert_eq!(im.take_wanted().len(), 1);
  }

  #[test]
  fn a_picture_that_keeps_failing_waits_longer_each_time() {
    let mut im = Images::new(1.0);
    let id = im.id("https://cdn.7tv.app/emote/y/1x.webp", 28);
    for round in 0..4u32 {
      failed_at(&mut im, id, 0);
      let wait = RETRY_FAILED_MS << round;
      im.sweep(wait - 1);
      assert!(im.failed(id), "round {round}: not before {wait}ms");
      im.sweep(wait);
      assert!(!im.failed(id), "round {round}: at {wait}ms");
    }
    // the wait stops growing at 8 times the first
    for _ in 0..4 {
      failed_at(&mut im, id, 0);
    }
    im.sweep(RETRY_FAILED_MS * 8);
    assert!(!im.failed(id));
  }

  fn loaded_px(im: &mut Images, id: ImgId, px: u32) {
    let h = im.target(&im.slots[(id & 0xfffff) as usize]);
    im.slots[(id & 0xfffff) as usize].state = State::Loading;
    let frame = decode::Frame { rgba: vec![7; (px * px * 4) as usize].into(), delay_ms: 0 };
    im.loaded(id, h, Ok(Decoded { w: px, h: px, frames: vec![frame] }));
  }

  #[test]
  fn pictures_nobody_looks_at_any_more_give_their_slots_back() {
    let mut im = Images::new(1.0);
    for round in 0..4u64 {
      for i in 0..500 {
        im.id(&format!("https://cdn.7tv.app/emote/r{round}x{i}/1x.webp"), 28);
      }
      // the next batch arrives well after the last went quiet
      im.sweep((round + 1) * (RECYCLE_MS + 1));
    }
    assert!(im.slots.len() <= 1_000, "the registry kept {} slots for 2000 urls", im.slots.len());
    assert!(im.by_url.len() <= 1_000);
  }

  #[test]
  fn an_old_id_never_shows_the_picture_that_took_its_slot() {
    let mut im = Images::new(1.0);
    let old = im.id("https://cdn.7tv.app/emote/old/1x.webp", 28);
    im.sweep(RECYCLE_MS + 1);
    let new = im.id("https://cdn.7tv.app/emote/new/1x.webp", 28);
    loaded_px(&mut im, new, 8);
    assert_ne!(old, new, "a recycled slot gets a fresh id");
    assert!(im.url(old).is_none());
    assert!(im.frame(old).is_none());
    assert!(im.dims(old).is_none());
    assert!(!im.failed(old));
    // rows holding the old id are told to lay out again, and ask by url
    assert!(im.take_revived());
    let again = im.id("https://cdn.7tv.app/emote/old/1x.webp", 28);
    assert_eq!(im.url(again), Some("https://cdn.7tv.app/emote/old/1x.webp"));
    assert_eq!(im.url(new), Some("https://cdn.7tv.app/emote/new/1x.webp"));
    assert!(im.dims(new).is_some());
  }

  #[test]
  fn a_picture_that_is_on_screen_or_loading_keeps_its_slot() {
    let mut im = Images::new(1.0);
    let busy = im.id("https://cdn.7tv.app/emote/busy/1x.webp", 28);
    let shown = im.id("https://cdn.7tv.app/emote/shown/1x.webp", 28);
    assert!(im.frame(busy).is_none());
    loaded_px(&mut im, shown, 8);
    im.sweep(RECYCLE_MS / 2);
    // drawn again at that moment (the clock here is real time, so say when by hand)
    im.frame(shown);
    im.slots[(shown & 0xfffff) as usize].used_ms = RECYCLE_MS / 2;
    im.sweep(RECYCLE_MS + 1);
    assert_eq!(im.id("https://cdn.7tv.app/emote/busy/1x.webp", 28), busy, "still loading");
    assert_eq!(im.id("https://cdn.7tv.app/emote/shown/1x.webp", 28), shown, "drawn recently");
  }
}
