// a second launch wakes this window: un-minimize, ask to be focused, and flag urgent in case focus is
// refused (wayland's focus_window does nothing; the urgency request goes through xdg-activation, so a
// compositor like dwl marks the tag). a channel it was given opens as the temp tab, like a click on one
use super::App;
use crate::model::chan::Spec;
use winit::window::UserAttentionType;

impl App {
  pub(super) fn raise(&mut self, open: &[Box<str>]) {
    if let Some(g) = &self.gfx {
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
      self.wake();
    }
  }
}
