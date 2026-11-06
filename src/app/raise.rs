// a second launch wakes this window. x11 lets an app take focus, so it asks. wayland doesn't: its
// focus request does nothing and an urgency mark is invisible behind a fullscreen game, so a window
// you aren't looking at is closed and made again, and the compositor puts the new one where you are
// (dwl: the tag you're on, focused). a channel it was given opens as the temp tab, like a click on one
use super::App;
use crate::model::chan::Spec;
use winit::event_loop::ActiveEventLoop;
use winit::platform::wayland::ActiveEventLoopExtWayland;
use winit::window::UserAttentionType;

impl App {
  pub(super) fn raise(&mut self, el: &ActiveEventLoop, open: &[Box<str>]) {
    if el.is_wayland() && !self.focused {
      let size = self.gfx.take().map(|g| g.window.inner_size());
      if let Some(size) = size {
        self.open_window(el, size.into());
      }
    } else if let Some(g) = &self.gfx {
      g.window.set_minimized(false);
      g.window.focus_window();
      g.window.request_user_attention(Some(UserAttentionType::Informational));
    }
    // one temp slot: the first channel takes it
    if let Some(spec) = open.iter().find_map(|l| Spec::from_line(l).ok()) {
      match spec.check(&self.other_specs(None)) {
        Ok(_) => self.open_look(&spec),
        Err(e) => self.say(e.msg()),
      }
    }
    self.wake();
  }
}
