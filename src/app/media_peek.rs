// a feed post's `[img]` tag under the pointer (ext img-view.js peek): its first picture, at most
// 320×240, beside the pointer; a held post shows its box's words instead, a post still in moderation
// nothing. square, black, 1px white border, like the emote popup
use super::App;
use crate::images::ImgId;
use crate::model::post;
use crate::model::tabs::Kind;
use crate::ui::canvas::Rect;
use crate::ui::chat_view::blit_layer;
use crate::ui::text::{Ink, Span};
use crate::ui::{theme, tip};

const MAX_W: u16 = 320;
const MAX_H: u16 = 240;
const PAD: i32 = 4;

pub(super) struct MediaPeek {
  id: Box<str>,
  at: (i32, i32),
  what: What,
}

enum What {
  Pic(ImgId),
  Words(Box<str>),
}

impl App {
  // the pointer moved. true = it is on a tag (the peek is up, or there is nothing to peek)
  pub(super) fn media_hover(&mut self) -> bool {
    let (x, y) = self.pointer;
    let chat = self.chat_area();
    let on = (self.tip_free() && self.picker.is_none() && self.kind() == Kind::Feed && chat.contains(x, y))
      .then(|| {
        let res = &mut self.res;
        self.tabs.get_mut(self.active)?.view.media_at(chat, x, y, res)
      })
      .flatten();
    let Some(id) = on else {
      self.hide_media_peek();
      return false;
    };
    if self.media_peek.as_ref().is_some_and(|p| p.id == id) {
      return true;
    }
    self.hide_media_peek();
    let Some(pic) = self.feed.find(&id).and_then(|p| post::pic(p, false, self.feed.reveal(&id))) else { return true };
    let what = match (pic.hidden, pic.peek) {
      (Some(h), _) => What::Words(h),
      (None, Some(url)) => {
        let w = self.res.images.px(MAX_W);
        What::Pic(self.res.images.id_fit(&url, MAX_H, w))
      }
      (None, None) => return true,
    };
    self.media_peek = Some(MediaPeek { id, at: self.pointer, what });
    self.tip_repaint();
    true
  }

  pub(super) fn hide_media_peek(&mut self) {
    if let Some(p) = self.media_peek.take() {
      if let What::Pic(id) = p.what {
        self.res.images.release(id);
      }
      self.tip_repaint();
    }
  }

  // painted over everything; a picture not decoded yet asks and waits
  pub(super) fn draw_media_peek(&mut self) -> Option<Rect> {
    let bounds = self.chat_area();
    let p = self.media_peek.as_ref()?;
    let c = &mut self.canvas;
    match &p.what {
      What::Pic(id) => {
        let (f, w, h, _) = self.res.images.frame(*id)?;
        let (w, h) = (w as i32, h as i32);
        let (x, y) = tip::place(p.at, w + 2 * PAD, h + 2 * PAD, bounds);
        let r = Rect::new(x, y, w + 2 * PAD, h + 2 * PAD);
        c.fill(r, theme::BLACK);
        blit_layer(c, &f.rgba, (w as u32, h as u32), (w, h), Rect::new(x + PAD, y + PAD, w, h), r);
        c.outline(r, r, theme::WHITE);
        Some(r)
      }
      What::Words(t) => {
        let spans = [Span { range: 0..t.len(), ink: Ink(theme::WHITE), bold: false }];
        let l = self.res.text.layout(t, &spans, &[], 480.0);
        let (w, h) = (l.width().ceil() as i32 + 4 * PAD, l.height().ceil() as i32 + 2 * PAD);
        let (x, y) = tip::place(p.at, w, h, bounds);
        let r = Rect::new(x, y, w, h);
        c.fill(r, theme::BLACK);
        self.res.text.draw(&l, c, (x + 2 * PAD) as f32, (y + PAD) as f32, r, |_, _| {});
        c.dashed(r, r, theme::CYAN);
        Some(r)
      }
    }
  }
}
