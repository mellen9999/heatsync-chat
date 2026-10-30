mod bench;
mod fake;
mod model;
mod ui;

use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::{Duration, Instant};

use softbuffer::{Context, Surface};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Window, WindowId};

use fake::Fake;
use ui::canvas::{Canvas, Rect};
use ui::chat_view::{ChatView, Paint};
use ui::text::Text;

const FLOOD_EVERY: Duration = Duration::from_millis(50);

struct Gfx {
  window: Rc<Window>,
  surface: Surface<Rc<Window>, Rc<Window>>,
}

struct App {
  gfx: Option<Gfx>,
  canvas: Canvas,
  text: Text,
  chat: ChatView,
  damage: Option<Rect>,
  mods: ModifiersState,
  fake: Fake,
  flood: Option<Instant>,
  ts: i64,
  // HSC_PROF=1 prints per-frame render time
  prof: bool,
}

impl App {
  fn new(flood: bool) -> Self {
    Self {
      gfx: None,
      canvas: Canvas::new(0, 0),
      text: Text::new(1.0),
      chat: ChatView::new(1000),
      damage: None,
      mods: ModifiersState::empty(),
      fake: Fake::new(0x5eed),
      flood: flood.then(Instant::now),
      ts: 0,
      prof: std::env::var_os("HSC_PROF").is_some(),
    }
  }

  fn area(&self) -> Rect {
    self.canvas.bounds()
  }

  fn damage(&mut self, r: Rect) {
    self.damage = Some(match self.damage {
      None => r,
      Some(d) => {
        let x0 = d.x.min(r.x);
        let y0 = d.y.min(r.y);
        Rect::new(x0, y0, (d.x + d.w).max(r.x + r.w) - x0, d.bottom().max(r.bottom()) - y0)
      }
    });
    if let Some(g) = &self.gfx {
      g.window.request_redraw();
    }
  }

  fn full(&mut self) {
    let a = self.area();
    self.damage(a);
  }

  fn scroll(&mut self, px: i32) {
    self.chat.scroll_by(px, &mut self.text);
    self.full();
  }

  fn redraw(&mut self) {
    let area = self.area();
    let mut dirty = self.damage.take();
    match self.chat.take_paint(&mut self.text, area.h) {
      Paint::Nothing => {}
      Paint::Full => dirty = Some(area),
      Paint::Append { shift } if dirty != Some(area) => {
        self.canvas.scroll_up(area, shift);
        let strip = Rect::new(area.x, area.bottom() - shift, area.w, shift);
        self.chat.draw(&mut self.canvas, &mut self.text, area, strip);
        // whole pane moved, so the compositor needs all of it
        dirty = Some(area);
        if let Some(d) = self.damage.take() {
          self.chat.draw(&mut self.canvas, &mut self.text, area, d);
        }
        return self.present(dirty);
      }
      Paint::Append { .. } => dirty = Some(area),
    }
    if let Some(d) = dirty {
      self.chat.draw(&mut self.canvas, &mut self.text, area, d);
    }
    self.present(dirty)
  }

  fn present(&mut self, dirty: Option<Rect>) {
    let (Some(g), Some(d)) = (&mut self.gfx, dirty) else {
      return;
    };
    let Ok(mut buf) = g.surface.buffer_mut() else { return };
    if buf.len() != self.canvas.px.len() {
      return;
    }
    buf.copy_from_slice(&self.canvas.px);
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
    self.chat.resize(w as i32);
    self.full();
  }
}

impl ApplicationHandler for App {
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
    self.text.set_scale(window.scale_factor() as f32);
    let size = window.inner_size();
    self.gfx = Some(Gfx { window, surface });
    self.resize(size.width, size.height);
  }

  fn window_event(&mut self, el: &ActiveEventLoop, _: WindowId, ev: WindowEvent) {
    match ev {
      WindowEvent::CloseRequested => el.exit(),
      WindowEvent::Resized(s) => self.resize(s.width, s.height),
      WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
        self.text.set_scale(scale_factor as f32);
        self.chat.resize(0);
        self.chat.resize(self.canvas.w);
        self.full();
      }
      WindowEvent::RedrawRequested => {
        let t = Instant::now();
        self.redraw();
        if self.prof {
          eprintln!("frame {:.3}ms", t.elapsed().as_secs_f64() * 1e3);
        }
      }
      WindowEvent::ModifiersChanged(m) => self.mods = m.state(),
      WindowEvent::MouseWheel { delta, .. } => {
        let px = match delta {
          MouseScrollDelta::LineDelta(_, y) => (y * self.text.line_px * 3.0) as i32,
          MouseScrollDelta::PixelDelta(p) => p.y as i32,
        };
        self.scroll(px);
      }
      WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
        let line = self.text.line_px as i32;
        let half = self.canvas.h / 2;
        let ctrl = self.mods.control_key();
        match event.logical_key.as_ref() {
          Key::Character("j") => self.scroll(-line),
          Key::Character("k") => self.scroll(line),
          Key::Character("d") if ctrl => self.scroll(-half),
          Key::Character("u") if ctrl => self.scroll(half),
          Key::Character("G") | Key::Named(NamedKey::End) => {
            self.chat.scroll_to_bottom();
            self.full();
          }
          _ => {}
        }
      }
      _ => {}
    }
  }

  fn about_to_wait(&mut self, el: &ActiveEventLoop) {
    let Some(next) = self.flood else {
      return el.set_control_flow(ControlFlow::Wait);
    };
    let now = Instant::now();
    if now >= next {
      self.ts += 1;
      let m = self.fake.message(self.ts);
      self.chat.push(m);
      if let Some(g) = &self.gfx {
        g.window.request_redraw();
      }
      self.flood = Some(now + FLOOD_EVERY);
    }
    el.set_control_flow(ControlFlow::WaitUntil(self.flood.unwrap_or(now)));
  }
}

fn main() {
  let args: Vec<String> = std::env::args().skip(1).collect();
  if args.first().map(String::as_str) == Some("--bench") {
    let n = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(10_000);
    return bench::run(n);
  }
  let flood = args.iter().any(|a| a == "--flood");
  let el = match EventLoop::new() {
    Ok(el) => el,
    Err(e) => {
      eprintln!("event loop: {e}");
      std::process::exit(1);
    }
  };
  let mut app = App::new(flood);
  if let Err(e) = el.run_app(&mut app) {
    eprintln!("{e}");
    std::process::exit(1);
  }
}
