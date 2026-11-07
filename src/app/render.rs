// the frame: what a redraw paints, how it reaches the window, and the canvas following the window's size

use super::{ANIM_MIN_MS, App, LOGIN_HINT, Tab, screen};
use crate::model::message::Platform;
use crate::model::settings::Counter;
use crate::model::tabs::Kind;
use crate::model::vi::Mode;
use crate::net::Cmd;
use crate::net::marks::Ask;
use crate::ui::canvas::Rect;
use crate::ui::chat_view::{ChatView, Paint};
use crate::ui::tab_bar::TabLabel;
use crate::ui::{input_bar, mode_label, tab_bar, union};
use crate::util::time::now_ms;
use std::num::NonZeroU32;
use std::time::{Duration, Instant};

// what the next present has to send: the window was resized or shown again (after a fullscreen app, a
// hidden tag), so the buffer or the compositor's copy of it may hold nothing and the whole frame goes;
// and the damage of recent presents, for buffers handed back with age > 1
#[derive(Default)]
pub(super) struct Damage {
  pub(super) expose: bool,
  pub(super) hist: [Option<Rect>; 2],
}

impl App {
  // the vi mode line for what the box is doing right now, and the half-typed command
  pub(super) fn mode_line(&self) -> (Vec<mode_label::Seg>, String) {
    if self.prompt.is_some() && !self.bar_find() {
      return (Vec::new(), String::new());
    }
    if let Some(note) = self.rover_note() {
      return (Vec::new(), note);
    }
    let short = !self.line.text.is_empty() && !self.hidden();
    let segs = mode_label::segs(self.set.vi, self.mode, self.selecting(), short);
    let cmd = match self.chord {
      _ if segs.is_empty() || self.mode != Mode::Normal => String::new(),
      Some(' ') => "space".into(),
      _ => Some(self.vi.showcmd()).filter(|c| !c.is_empty()).unwrap_or_default(),
    };
    (segs, cmd)
  }

  pub(super) fn redraw(&mut self) {
    let mut dirty: Option<Rect> = None;
    let (label, showcmd) = self.mode_line();
    let hidden_now = self.hidden();
    if (&label, &showcmd, hidden_now) != (&self.shown_mode.0, &self.shown_mode.1, self.shown_mode.2) {
      self.shown_mode = (label.clone(), showcmd.clone(), hidden_now);
      self.input_dirty = true;
      self.bar_dirty = true;
    }
    if self.input_next.is_some_and(|t| t <= Instant::now()) {
      self.input_next = None;
      self.input_dirty = true;
    }
    if std::mem::take(&mut self.input_dirty) {
      self.sync_suggest();
      // the filter bar has its own box at the top: the input bar carries on with the message
      let bf = self.bar_find();
      let open = self.prompt.is_some() && !bf;
      let chips = if open { Vec::new() } else { self.input_chips() };
      // a lock shows in the empty box even while inserting (vi), and its countdown re-arms a redraw each second
      let locked = self.lock_here().filter(|_| !open && self.line.text.is_empty());
      self.lock_sec = locked
        .and_then(|l| l.flips_in(super::now_ms()))
        .map(|ms| Instant::now() + Duration::from_millis(ms.max(1) as u64));
      let prompt = self.prompt.as_ref().filter(|_| open);
      let show_hint = self.line.text.is_empty() && (!self.set.vi || self.mode == Mode::Normal);
      let hint = match prompt {
        Some(_) if self.find.on && self.logs_shown().is_some() => {
          "type a search · enter finds it · esc cancels".to_owned()
        }
        Some(_) if self.find.on => "type to filter · @name for one person · enter keeps it · esc clears".to_owned(),
        Some(_) => "type or drop a file path · tab completes · enter uploads · esc cancels".to_owned(),
        None if show_hint || locked.is_some() => self.hint(),
        None => String::new(),
      };
      // the picture view is no place to type (ext): the box steps aside
      let hidden = self.hidden() || self.img_view.is_some();
      let tag = match prompt {
        Some(_) if self.find.on => vec![("find: ", crate::ui::theme::YELLOW)],
        Some(_) => vec![("path: ", crate::ui::theme::YELLOW)],
        None => self.send_tag(),
      };
      let reply = self.reply.as_ref().filter(|_| prompt.is_none()).map(|r| &*r.nick);
      // past what the platform takes, the words go red (ext updateCharCount)
      let typed = &self.line.text;
      let legs: Vec<Platform> = self.legs().into_iter().map(|l| l.0).collect();
      let over = match (prompt, legs.is_empty()) {
        (None, false) => typed.char_indices().nth(crate::model::sendto::limit(&legs)).map_or(typed.len(), |(i, _)| i),
        _ => usize::MAX,
      };
      // near the cap, a n/limit counter (ext #hs-mc-charcount): yellow close to it, red past it
      let count = match (prompt, legs.is_empty()) {
        (None, false) => crate::model::sendto::count(typed, crate::model::sendto::limit(&legs)),
        _ => None,
      };
      let count_ink =
        |f| if f == crate::model::sendto::Fill::Over { crate::ui::theme::RED } else { crate::ui::theme::YELLOW };
      let bar = input_bar::Bar {
        line: prompt.unwrap_or(&self.line),
        mode: if prompt.is_some() { Mode::Insert } else { self.mode },
        target: &tag,
        hint: &hint,
        hint_ink: locked.map_or(crate::ui::theme::DIM, Self::lock_ink),
        over,
        count: count.as_ref().map(|(t, f)| (t.as_str(), count_ink(*f))),
        link: if prompt.is_none() && self.me.is_none() && hint.starts_with(LOGIN_HINT) { LOGIN_HINT.len() } else { 0 },
        link_on: self.hover.link,
        reply,
        edit: self.edit.is_some() && prompt.is_none(),
        blur: bf,
        preedit: self.ime.preedit(),
        preedit_caret: self.ime.caret(),
        picker_on: self.picker.is_some() || self.hover.btn,
        upload_on: self.uploading || (prompt.is_some() && !self.find.on) || self.hover.up,
        logo: &self.res.logo,
        label: if hidden { &[] } else { &label },
        showcmd: &showcmd,
        chips: &chips,
      };
      let w = self.body().w;
      let h = if hidden { 0 } else { input_bar::height(&mut self.res.text, &bar, w) };
      if h != self.input_h {
        // the chat area changed size: repaint it whole
        self.input_h = h;
        if let Some(t) = self.tabs.get_mut(self.active) {
          t.view.invalidate();
        }
      }
      if h > 0 {
        let r = self.input_area();
        let (caret, next, hit) = input_bar::draw(&mut self.canvas, &mut self.res.text, &mut self.res.images, r, &bar);
        self.ime_win.area = Some(caret);
        self.login_link = hit.link;
        self.input_hit = (prompt.is_none() && !self.line.text.is_empty()).then_some(hit);
        self.input_next = next.map(|ms| Instant::now() + Duration::from_millis(ms.max(ANIM_MIN_MS) as u64));
        dirty = Some(r);
      } else {
        self.login_link = None;
      }
    }
    if let Some(r) = self.sync_pin() {
      dirty = Some(dirty.map_or(r, |d| union(d, r)));
    }
    if let Some(r) = self.sync_pred() {
      dirty = Some(dirty.map_or(r, |d| union(d, r)));
    }
    if let Some(r) = self.sync_find() {
      dirty = Some(dirty.map_or(r, |d| union(d, r)));
    }
    let area = self.chat_area();
    if (self.overlay.is_some() || self.tip.is_some() || self.media_peek.is_some() || self.over_input)
      && let Some(t) = self.tabs.get_mut(self.active)
    {
      // the menu floats over the chat: repaint under it so blits and scrolls leave no ghosts
      t.view.invalidate();
    }
    if std::mem::take(&mut self.bar_dirty) {
      let shown: Vec<usize> = (0..self.tabs.len()).filter(|&i| self.visible(i)).collect();
      let (mode, now) = (self.set.tab_counter, now_ms() / 1000);
      let counts: Vec<String> = shown
        .iter()
        .map(|&i| {
          let active = i == self.active;
          let t = &mut self.tabs[i];
          if t.kind == Kind::Chat { t.count.text(mode, active, now) } else { String::new() }
        })
        .collect();
      self.heat_due = (mode == Counter::Heat && counts.iter().any(|c| !c.is_empty()))
        .then(|| Instant::now() + Duration::from_secs(1));
      let labels: Vec<TabLabel> = shown
        .iter()
        .zip(&counts)
        .map(|(&i, count)| {
          let t = &self.tabs[i];
          let hot = match t.kind {
            Kind::Mentions => crate::ui::theme::RED,
            Kind::Whispers => crate::ui::theme::CYAN,
            _ if t.event => crate::ui::theme::YELLOW,
            _ => crate::ui::theme::WHITE,
          };
          // a tab something took over reads `← back` while you look at it (ext takeover.js)
          let back = (t.under.is_some() || self.img_view.is_some()) && i == self.active;
          let name = if back { "← back" } else { &t.name };
          let count = if back { "" } else { count.as_str() };
          let cursor = self.rover.as_ref().is_some_and(|r| r.at == i);
          let live = t.sources.iter().any(|(p, k)| self.live.has(*p, k));
          TabLabel {
            name,
            count,
            active: i == self.active,
            unread: !back && (t.unread || t.event),
            hot,
            cursor,
            live,
            temp: t.temp,
            muted: t.sources.first().is_some_and(|(p, k)| self.pings.muted(*p, k)),
          }
        })
        .collect();
      let bar = self.bar();
      let up = self.update_label();
      let mark = self.streamer_mark();
      let (gear, keys) = (
        self.hover.gear || self.screen_is(screen::Page::Settings),
        self.hover.keys || self.screen_is(screen::Page::Keys),
      );
      self.bar_hits = tab_bar::draw(
        &mut self.canvas,
        &mut self.res.text,
        bar,
        &labels,
        if hidden_now { (&label, &showcmd) } else { (&[], "") },
        tab_bar::Hot {
          tab: self.hover.tab,
          plus: self.hover.plus,
          gear,
          keys,
          update: up.as_deref().map(|l| (l, self.hover.update)),
          mark,
        },
      );
      self.bar_map = shown;
      dirty = Some(dirty.map_or(bar, |d| union(d, bar)));
    }
    if self.res.images.next_tick() {
      self.trim_soon();
    }
    // pictures that failed a while ago get another go: rows that left one out lay out again and ask
    if self.res.images.take_revived() {
      for t in &mut self.tabs {
        t.views().for_each(ChatView::relayout);
      }
      self.wake();
    }
    if self.form.is_some() || self.screen.is_some() || self.img_view.is_some() || self.picker.is_some() {
      self.jump_forget();
      self.modbar_forget();
    }
    if self.form.is_some() {
      self.next_anim = None;
      if let Some(r) = self.draw_form() {
        dirty = Some(dirty.map_or(r, |d| union(d, r)));
      }
    } else if self.screen.is_some() {
      self.next_anim = None;
      if let Some(r) = self.draw_screen() {
        dirty = Some(dirty.map_or(r, |d| union(d, r)));
      }
    } else if self.img_view.is_some() {
      if let Some(r) = self.draw_img_view() {
        dirty = Some(dirty.map_or(r, |d| union(d, r)));
      }
    } else if self.picker.is_some() {
      // the picker covers the chat: paint only it
      self.next_anim = None;
      if let Some(r) = self.draw_picker() {
        dirty = Some(dirty.map_or(r, |d| union(d, r)));
      }
    } else {
      let (canvas, res) = (&mut self.canvas, &mut self.res);
      let add = |d: Option<Rect>, r: Rect| Some(d.map_or(r, |d| union(d, r)));
      let (mut full, mut animated, mut ghost) = (false, None, 0);
      if let Some(t) = self.tabs.get_mut(self.active) {
        match t.view.take_paint(res, area.h) {
          Paint::Nothing => {}
          Paint::Full => {
            t.view.draw(canvas, res, area, area);
            dirty = add(dirty, area);
            full = true;
          }
          Paint::Append { shift } => {
            canvas.scroll_up(area, shift);
            t.view.scrolled(area, shift);
            t.view.draw(canvas, res, area, Rect::new(area.x, area.bottom() - shift, area.w, shift));
            dirty = add(dirty, area);
            ghost = shift;
          }
        }
        let due = self.next_anim.is_some_and(|t| t <= Instant::now());
        if due && let Some(r) = t.view.animate(canvas, res, area) {
          dirty = add(dirty, r);
          animated = Some(r);
        }
        self.next_anim = t.view.next_anim.map(|ms| Instant::now() + Duration::from_millis(ms.max(ANIM_MIN_MS) as u64));
      } else if dirty.is_some() {
        canvas.fill(area, crate::ui::theme::BLACK);
        dirty = add(dirty, area);
        full = true;
      }
      if ghost > 0
        && let Some(r) = self.modbar_shifted(ghost)
        && let Some(t) = self.tabs.get_mut(self.active)
      {
        t.view.draw(&mut self.canvas, &mut self.res, area, r);
      }
      if let Some(r) = self.sync_modbar(full, animated) {
        dirty = add(dirty, r);
      }
      if let Some(r) = self.sync_jump(full, animated) {
        dirty = add(dirty, r);
      }
    }
    if let Some(r) = self.draw_over_input() {
      dirty = Some(dirty.map_or(r, |d| union(d, r)));
    }
    if let Some(r) = self.draw_card() {
      dirty = Some(dirty.map_or(r, |d| union(d, r)));
    }
    if let Some(r) = self.draw_menu() {
      dirty = Some(dirty.map_or(r, |d| union(d, r)));
    }
    if let Some(r) = self.draw_tab_menu() {
      dirty = Some(dirty.map_or(r, |d| union(d, r)));
    }
    if let Some(r) = self.draw_pop() {
      dirty = Some(dirty.map_or(r, |d| union(d, r)));
    }
    if let Some(r) = self.draw_peek() {
      dirty = Some(dirty.map_or(r, |d| union(d, r)));
    }
    if let Some(r) = self.draw_tag_peek() {
      dirty = Some(dirty.map_or(r, |d| union(d, r)));
    }
    if let Some(r) = self.draw_media_peek() {
      dirty = Some(dirty.map_or(r, |d| union(d, r)));
    }
    if let Some(r) = self.draw_tip() {
      dirty = Some(dirty.map_or(r, |d| union(d, r)));
    }
    for t in [self.tip_next, self.input_next].into_iter().flatten() {
      self.next_anim = Some(self.next_anim.map_or(t, |n| n.min(t)));
    }
    for (id, url, h) in self.res.images.take_wanted() {
      self.net.send(Cmd::Image(id, url, h));
    }
    for url in self.res.previews.take_wanted() {
      self.net.send(Cmd::Preview(url));
    }
    for id in self.res.senders.take_wanted() {
      self.net.send(Cmd::Sender(id, None));
    }
    self.ask_people();
    for id in self.res.cosmetics.take_wanted() {
      self.net.send(Cmd::Cosmetics(id));
    }
    for login in self.res.heats.take_wanted() {
      self.net.send(Cmd::Mark(Ask::Heat(login)));
    }
    for id in self.res.flairs.take_wanted() {
      self.net.send(Cmd::Mark(Ask::Flair(id)));
    }
    for (p, login) in self.res.avatars.take_wanted() {
      self.net.send(Cmd::Avatar(p, login));
    }
    self.present(dirty);
  }

  pub(super) fn present(&mut self, dirty: Option<Rect>) {
    let whole = self.canvas.bounds();
    let dirty = if std::mem::take(&mut self.damage.expose) { Some(whole) } else { dirty };
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
      2 => self.damage.hist[0].map(|h| union(h, d)),
      _ => None,
    };
    self.damage.hist = [Some(d), self.damage.hist[0]];
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
    // a fresh buffer carries the whole frame: say so, or the compositor keeps only the damage of it
    let d = if copy.is_none() { whole } else { d };
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

  pub(super) fn resize(&mut self, w: u32, h: u32) {
    let (Some(g), Some(nw), Some(nh)) = (&mut self.gfx, NonZeroU32::new(w), NonZeroU32::new(h)) else {
      return;
    };
    if let Err(e) = g.surface.resize(nw, nh) {
      eprintln!("resize: {e}");
      return;
    }
    self.resized(w as i32, h as i32);
  }

  // the canvas at its new size, and everything on it due to paint again
  pub(super) fn resized(&mut self, w: i32, h: i32) {
    self.canvas.resize(w, h);
    self.damage.hist = [None; 2];
    // softbuffer keeps a resized buffer's age though its pixels are new (zeroed): send it all
    self.damage.expose = true;
    self.input_dirty = true;
    // the canvas came back zeroed: everything paints again, not only what changed width (a height-only
    // resize, e.g. a new window tiling beside this one, left the chat black but for its animations)
    let bw = self.body().w;
    for v in self.tabs.iter_mut().flat_map(Tab::views) {
      v.resize(bw);
      v.invalidate();
    }
    self.pin_bar.sig = Default::default();
    self.pred_bar.sig = Default::default();
    self.find.sig = None;
    self.bar_dirty = true;
    self.wake();
  }
}
