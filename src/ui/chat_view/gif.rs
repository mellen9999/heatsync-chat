// the picture of a twitch gif-keyboard line, put on the row already here: it draws where the caption was
use super::*;
use crate::model::chatgif::Picture;

// the rows an event for a line looks back over: the event follows the line by a moment, so the line is near the tail
const LOOK_BACK: usize = 500;

impl ChatView {
  // heatsync named this twitch line's picture (by its message id, never by words). what the row holds already is
  // not news. true = a row changed
  pub fn set_gif(&mut self, id: &str, pic: &Picture) -> bool {
    let Some(r) =
      self.rows.iter_mut().rev().take(LOOK_BACK).find(|r| r.msg.platform == Platform::Twitch && &*r.msg.id == id)
    else {
      return false;
    };
    if r.msg.x.as_ref().is_some_and(|x| x.gif.as_deref() == Some(pic)) {
      return false;
    }
    r.msg.extra().gif = Some(pic.clone().into());
    Self::touch(r);
    self.stale = true;
    true
  }
}
