// window + event loop glue: tabs of chat views fed by the net thread
use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::{Duration, Instant};

use softbuffer::{Context, Surface};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Window, WindowId};

use crate::fake::Fake;
use crate::model::message::{Message, Platform};
use crate::net::irc::Chat;
use crate::net::{Cmd, Net, NetEvent};
use crate::store;
use crate::ui::canvas::{Canvas, Rect};
use crate::ui::chat_view::{ChatView, Paint};
use crate::ui::res::Res;
use crate::ui::tab_bar::{self, TabLabel};
use crate::ui::union;
use crate::util::time::now_ms;

const FLOOD_EVERY: Duration = Duration::from_millis(50);
const FLOOD_TAB: &str = "flood";
const SCROLLBACK: usize = 1000;
// animation repaint ceiling: 30fps — smooth for emotes, ~35% fewer wakeups than native gif timing
const ANIM_MIN_MS: u32 = 33;
const TRIM_AFTER: Duration = Duration::from_secs(2);

struct Gfx {
  window: Rc<Window>,
  surface: Surface<Rc<Window>, Rc<Window>>,
}

struct Tab {
  name: Box<str>,
  view: ChatView,
  unread: bool,
}

pub struct App {
  gfx: Option<Gfx>,
  canvas: Canvas,
  res: Res,
  tabs: Vec<Tab>,
  active: usize,
  net: Net,
  bar_dirty: bool,
  status: String,
  mods: ModifiersState,
  // first key of a two-key vim chord (g)
  chord: Option<char>,
  fake: Fake,
  flood: Option<Instant>,
  prof: bool,
  // twitch room id per channel, once known (roomstate repeats on every mode change)
  rooms: Vec<Box<str>>,
  next_anim: Option<Instant>,
  occluded: bool,
  // damage of recent presents, for buffers handed back with age > 1
  damage_hist: [Option<Rect>; 2],
  // pending heap trim after bursts of parsing (emote sets, history)
  trim_at: Option<Instant>,
}

impl App {
  pub fn new(net: Net, channels: Vec<Box<str>>, flood: bool) -> Self {
    let mut tabs: Vec<Tab> = Vec::new();
    for ch in channels {
      if !tabs.iter().any(|t| t.name == ch) {
        net.send(Cmd::Join(ch.clone()));
        tabs.push(Tab {
          name: ch.clone(),
          view: ChatView::new(SCROLLBACK, ch),
          unread: false,
        });
      }
    }
    if flood {
      tabs.push(Tab {
        name: FLOOD_TAB.into(),
        view: ChatView::new(SCROLLBACK, FLOOD_TAB.into()),
        unread: false,
      });
    }
    let status = if tabs.is_empty() {
      "no channels — run: heatsync-chat <channel> …".into()
    } else {
      String::new()
    };
    Self {
      gfx: None,
      canvas: Canvas::new(0, 0),
      res: Res::new(1.0),
      tabs,
      active: 0,
      net,
      bar_dirty: true,
      status,
      mods: ModifiersState::empty(),
      chord: None,
      fake: Fake::new(0x5eed),
      flood: flood.then(Instant::now),
      prof: std::env::var_os("HSC_PROF").is_some(),
      rooms: Vec::new(),
      next_anim: None,
      occluded: false,
      damage_hist: [None; 2],
      trim_at: None,
    }
  }

  fn bar(&self) -> Rect {
    Rect::new(0, 0, self.canvas.w, self.res.text.line_px as i32)
  }

  fn chat_area(&self) -> Rect {
    let b = self.bar().h;
    Rect::new(0, b, self.canvas.w, (self.canvas.h - b).max(0))
  }

  fn wake(&self) {
    if let Some(g) = &self.gfx {
      g.window.request_redraw();
    }
  }

  fn view(&mut self) -> Option<&mut ChatView> {
    self.tabs.get_mut(self.active).map(|t| &mut t.view)
  }

  fn scroll(&mut self, px: i32) {
    let res = &mut self.res;
    if let Some(t) = self.tabs.get_mut(self.active) {
      t.view.scroll_by(px, res);
    }
    self.wake();
  }

  fn select(&mut self, i: usize) {
    if i < self.tabs.len() && i != self.active {
      self.active = i;
      self.tabs[i].unread = false;
      self.tabs[i].view.invalidate();
      self.bar_dirty = true;
      self.wake();
    }
  }

  fn close_tab(&mut self) {
    if self.active >= self.tabs.len() {
      return;
    }
    let t = self.tabs.remove(self.active);
    if &*t.name != FLOOD_TAB {
      self.res.emotes.drop_channel(&t.name);
      self.rooms.retain(|r| *r != t.name);
      self.net.send(Cmd::Part(t.name));
    }
    persist(
      &self
        .tabs
        .iter()
        .filter(|t| &*t.name != FLOOD_TAB)
        .map(|t| t.name.clone())
        .collect::<Vec<_>>(),
    );
    self.active = self.active.min(self.tabs.len().saturating_sub(1));
    if let Some(t) = self.tabs.get_mut(self.active) {
      t.unread = false;
      t.view.invalidate();
    }
    self.bar_dirty = true;
    self.wake();
  }

  fn trim_soon(&mut self) {
    self.trim_at = Some(Instant::now() + TRIM_AFTER);
  }

  fn on_chat(&mut self, ch: &str, ev: Chat) {
    let Some(i) = self.tabs.iter().position(|t| &*t.name == ch) else {
      return;
    };
    let tab = &mut self.tabs[i];
    match ev {
      Chat::Msg(m) => tab.view.push(m),
      Chat::ClearMsg(id) => tab.view.clear_msg(&id),
      Chat::ClearUser { login, secs } => {
        tab.view.clear_user(&login);
        let what = secs.map_or_else(|| "banned".to_owned(), |s| format!("timed out for {s}s"));
        tab.view.push(Message::system(
          now_ms(),
          Platform::Twitch,
          format!("{login} was {what}"),
        ));
      }
      Chat::ClearAll => tab.view.push(Message::system(
        now_ms(),
        Platform::Twitch,
        "chat was cleared by a moderator",
      )),
      Chat::Room { id } => {
        if !self.rooms.contains(&tab.name) {
          self.rooms.push(tab.name.clone());
          self.net.send(Cmd::Room(tab.name.clone(), id));
        }
        return;
      }
    }
    if i == self.active {
      self.wake();
    } else if !tab.unread {
      tab.unread = true;
      self.bar_dirty = true;
      self.wake();
    }
  }

  fn redraw(&mut self) {
    let area = self.chat_area();
    let mut dirty: Option<Rect> = None;
    if std::mem::take(&mut self.bar_dirty) {
      let labels: Vec<TabLabel> = self
        .tabs
        .iter()
        .enumerate()
        .map(|(i, t)| TabLabel {
          name: &t.name,
          active: i == self.active,
          unread: t.unread,
        })
        .collect();
      let bar = self.bar();
      tab_bar::draw(&mut self.canvas, &mut self.res.text, bar, &labels, &self.status);
      dirty = Some(bar);
    }
    let (canvas, res) = (&mut self.canvas, &mut self.res);
    res.images.next_tick();
    let add = |d: Option<Rect>, r: Rect| Some(d.map_or(r, |d| union(d, r)));
    if let Some(t) = self.tabs.get_mut(self.active) {
      match t.view.take_paint(res, area.h) {
        Paint::Nothing => {}
        Paint::Full => {
          t.view.draw(canvas, res, area, area);
          dirty = add(dirty, area);
        }
        Paint::Append { shift } => {
          canvas.scroll_up(area, shift);
          t.view.scrolled(area, shift);
          t.view.draw(
            canvas,
            res,
            area,
            Rect::new(area.x, area.bottom() - shift, area.w, shift),
          );
          dirty = add(dirty, area);
        }
      }
      let due = self.next_anim.is_some_and(|t| t <= Instant::now());
      if due && let Some(r) = t.view.animate(canvas, res, area) {
        dirty = add(dirty, r);
      }
      self.next_anim = t
        .view
        .next_anim
        .map(|ms| Instant::now() + Duration::from_millis(ms.max(ANIM_MIN_MS) as u64));
    } else if dirty.is_some() {
      canvas.fill(area, crate::ui::theme::BLACK);
      dirty = add(dirty, area);
    }
    for (id, url) in self.res.images.take_wanted() {
      self.net.send(Cmd::Image(id, url, self.res.images.target_h));
    }
    self.present(dirty);
  }

  fn present(&mut self, dirty: Option<Rect>) {
    let (Some(g), Some(d)) = (&mut self.gfx, dirty) else {
      return;
    };
    let Ok(mut buf) = g.surface.buffer_mut() else { return };
    if buf.len() != self.canvas.px.len() {
      return;
    }
    // a reused buffer is `age` presents old: copy this frame's damage plus what it missed since
    let copy = match buf.age() {
      1 => Some(d),
      2 => self.damage_hist[0].map(|h| union(h, d)),
      _ => None,
    };
    self.damage_hist = [Some(d), self.damage_hist[0]];
    match copy {
      Some(r) => {
        let r = r.intersect(self.canvas.bounds());
        let w = self.canvas.w as usize;
        for y in r.y..r.bottom() {
          let a = y as usize * w + r.x as usize;
          buf[a..a + r.w as usize].copy_from_slice(&self.canvas.px[a..a + r.w as usize]);
        }
      }
      None => buf.copy_from_slice(&self.canvas.px),
    }
    let r = softbuffer::Rect {
      x: d.x as u32,
      y: d.y as u32,
      width: NonZeroU32::new(d.w as u32).unwrap_or(NonZeroU32::MIN),
      height: NonZeroU32::new(d.h as u32).unwrap_or(NonZeroU32::MIN),
    };
    if let Err(e) = buf.present_with_damage(&[r]) {
      eprintln!("present: {e}");
    }
  }

  fn resize(&mut self, w: u32, h: u32) {
    let (Some(g), Some(nw), Some(nh)) = (&mut self.gfx, NonZeroU32::new(w), NonZeroU32::new(h)) else {
      return;
    };
    if let Err(e) = g.surface.resize(nw, nh) {
      eprintln!("resize: {e}");
      return;
    }
    self.canvas.resize(w as i32, h as i32);
    self.damage_hist = [None; 2];
    for t in &mut self.tabs {
      t.view.resize(w as i32);
    }
    self.bar_dirty = true;
    self.wake();
  }

  fn key(&mut self, key: Key<&str>) {
    let line = self.res.text.line_px as i32;
    let half = self.chat_area().h / 2;
    let ctrl = self.mods.control_key();
    let alt = self.mods.alt_key();
    let chord = self.chord.take();
    match key {
      Key::Character(d) if alt && d.len() == 1 && d.as_bytes()[0].is_ascii_digit() => {
        let n = (d.as_bytes()[0] - b'0') as usize;
        self.select(if n == 0 { 9 } else { n - 1 });
      }
      Key::Character("t") if chord == Some('g') => self.select((self.active + 1) % self.tabs.len().max(1)),
      Key::Character("T") if chord == Some('g') => {
        let n = self.tabs.len().max(1);
        self.select((self.active + n - 1) % n);
      }
      Key::Character("g") if chord == Some('g') => {
        let res = &mut self.res;
        if let Some(t) = self.tabs.get_mut(self.active) {
          t.view.scroll_to_top(res);
        }
        self.wake();
      }
      Key::Character("g") => self.chord = Some('g'),
      Key::Character("j") | Key::Named(NamedKey::ArrowDown) => self.scroll(-line),
      Key::Character("k") | Key::Named(NamedKey::ArrowUp) => self.scroll(line),
      Key::Character("w") if ctrl => self.close_tab(),
      Key::Character("d") if ctrl => self.scroll(-half),
      Key::Character("u") if ctrl => self.scroll(half),
      Key::Named(NamedKey::PageDown) => self.scroll(-half * 2),
      Key::Named(NamedKey::PageUp) => self.scroll(half * 2),
      Key::Character("G") | Key::Named(NamedKey::End) => {
        if let Some(v) = self.view() {
          v.scroll_to_bottom();
        }
        self.wake();
      }
      _ => {}
    }
  }
}

impl ApplicationHandler<NetEvent> for App {
  fn resumed(&mut self, el: &ActiveEventLoop) {
    if self.gfx.is_some() {
      return;
    }
    let attrs = Window::default_attributes()
      .with_title("heatsync")
      .with_inner_size(winit::dpi::LogicalSize::new(420, 800));
    let window = match el.create_window(attrs) {
      Ok(w) => Rc::new(w),
      Err(e) => {
        eprintln!("window: {e}");
        return el.exit();
      }
    };
    let surface = Context::new(window.clone()).and_then(|ctx| Surface::new(&ctx, window.clone()));
    let surface = match surface {
      Ok(s) => s,
      Err(e) => {
        eprintln!("surface: {e}");
        return el.exit();
      }
    };
    self.res.set_scale(window.scale_factor() as f32);
    let size = window.inner_size();
    self.gfx = Some(Gfx { window, surface });
    self.resize(size.width, size.height);
  }

  fn user_event(&mut self, _: &ActiveEventLoop, ev: NetEvent) {
    match ev {
      NetEvent::Chat(ch, c) => self.on_chat(&ch, c),
      NetEvent::Backfill(ch, events) => {
        let Some(i) = self.tabs.iter().position(|t| t.name == ch) else {
          return;
        };
        let history = events
          .into_iter()
          .filter_map(|e| match e {
            Chat::Msg(m) => Some(m),
            _ => None,
          })
          .collect();
        self.tabs[i].view.prepend(history);
        self.trim_soon();
        if i == self.active {
          self.wake();
        }
      }
      NetEvent::Emotes(ch, p, set) => {
        self.res.emotes.set(ch.as_deref(), p, set);
        self.trim_soon();
        for t in &mut self.tabs {
          if ch.as_ref().is_none_or(|c| *c == t.name) {
            t.view.relayout();
          }
        }
        self.wake();
      }
      NetEvent::Image(id, h, res) => {
        if self.res.images.loaded(id, h, res) {
          if let Some(t) = self.tabs.get_mut(self.active) {
            t.view.relayout();
          }
        } else if let Some(t) = self.tabs.get_mut(self.active) {
          t.view.invalidate();
        }
        self.wake();
      }
      NetEvent::Status(s) => {
        self.status = s;
        self.bar_dirty = true;
        self.wake();
      }
    }
  }

  fn window_event(&mut self, el: &ActiveEventLoop, _: WindowId, ev: WindowEvent) {
    match ev {
      WindowEvent::CloseRequested => el.exit(),
      WindowEvent::Resized(s) => self.resize(s.width, s.height),
      WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
        self.res.set_scale(scale_factor as f32);
        for t in &mut self.tabs {
          t.view.relayout();
        }
        self.bar_dirty = true;
        self.wake();
      }
      WindowEvent::RedrawRequested => {
        let t = Instant::now();
        self.redraw();
        if self.prof {
          eprintln!(
            "frame {:.3}ms img {}KB",
            t.elapsed().as_secs_f64() * 1e3,
            self.res.images.bytes() >> 10
          );
        }
      }
      WindowEvent::ModifiersChanged(m) => self.mods = m.state(),
      WindowEvent::Occluded(o) => {
        self.occluded = o;
        if !o {
          self.wake();
        }
      }
      WindowEvent::MouseWheel { delta, .. } => {
        let px = match delta {
          MouseScrollDelta::LineDelta(_, y) => (y * self.res.text.line_px * 3.0) as i32,
          MouseScrollDelta::PixelDelta(p) => p.y as i32,
        };
        self.scroll(px);
      }
      WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
        self.key(event.logical_key.as_ref());
      }
      _ => {}
    }
  }

  fn about_to_wait(&mut self, el: &ActiveEventLoop) {
    let now = Instant::now();
    if let Some(next) = self.flood
      && now >= next
    {
      let m = self.fake.message(now_ms());
      self.on_chat(FLOOD_TAB, Chat::Msg(m));
      self.flood = Some(now + FLOOD_EVERY);
    }
    if self.trim_at.is_some_and(|t| t <= now) {
      self.trim_at = None;
      crate::util::mem::trim();
    }
    // animations sleep while the window is hidden
    let anim = self.next_anim.filter(|_| !self.occluded);
    if anim.is_some_and(|t| t <= now) {
      self.wake();
    }
    let wake = [self.flood, anim, self.trim_at].into_iter().flatten().min();
    el.set_control_flow(wake.map_or(ControlFlow::Wait, ControlFlow::WaitUntil));
  }
}

pub fn persist(channels: &[Box<str>]) {
  store::save_tabs(channels);
}
