// what is under the pointer: lines, emotes, links, names, pictures and buttons

use super::layout::layout_row;
use super::{ChatView, EmoteHit, Laid, Layer, MARK_W, Mentioned, WhoHit, room};
use crate::emotes::resolve::find_word;
use crate::images::ImgId;
use crate::model::message::{Kind, Message, Platform};
use crate::ui::canvas::Rect;
use crate::ui::res::Res;
use crate::ui::text::TextLayout;
use crate::ui::theme;
use parley::{Cluster, PositionedLayoutItem};

// which of `ranges` (text offsets) the point (lx, ly) in layout coords is over: on a line, inside a
// glyph run that overlaps one, and the character there is inside it
pub(super) fn over(layout: &TextLayout, lx: f32, ly: f32, ranges: &[std::ops::Range<usize>]) -> Option<usize> {
  let hit = layout.lines().any(|line| {
    let lm = line.metrics();
    (lm.block_min_coord..lm.block_max_coord).contains(&ly)
      && line.items().any(|it| match it {
        PositionedLayoutItem::GlyphRun(gr) => {
          let at = gr.run().text_range();
          ranges.iter().any(|r| !r.is_empty() && r.start < at.end && at.start < r.end)
            && (gr.offset()..gr.offset() + gr.advance()).contains(&lx)
        }
        _ => false,
      })
  });
  if !hit {
    return None;
  }
  let at = Cluster::from_point(layout, lx, ly)?.0.text_range().start;
  ranges.iter().position(|r| r.contains(&at))
}

impl ChatView {
  // the line under y (in `area`'s coordinates), as drawn last
  pub fn select_at(&mut self, area: Rect, y: i32, res: &mut Res) -> bool {
    let mut bottom = self.edge(area, res);
    for i in (0..self.rows.len()).rev() {
      if bottom <= area.y {
        break;
      }
      let top = bottom - self.height_at(i, res);
      if y >= top && y < bottom {
        if !self.rows[i].msg.kind.said() {
          return false;
        }
        self.selected = Some(self.rows[i].seq);
        self.stale = true;
        return true;
      }
      bottom = top;
    }
    false
  }

  // the line under y as drawn last: its top, its bottom and its seq. hidden lines have no height, so none match
  pub fn line_at(&mut self, area: Rect, y: i32, res: &mut Res) -> Option<(i32, i32, i64)> {
    let mut bottom = self.edge(area, res);
    for i in (0..self.rows.len()).rev() {
      if bottom <= area.y {
        return None;
      }
      let top = bottom - self.height_at(i, res);
      if y >= top && y < bottom {
        return Some((top, bottom, self.rows[i].seq));
      }
      bottom = top;
    }
    None
  }

  // the went-live line under y (a click there opens its channel): its platform and channel
  pub fn opens_at(&mut self, area: Rect, y: i32, res: &mut Res) -> Option<(Platform, Box<str>)> {
    let mut bottom = self.edge(area, res);
    for i in (0..self.rows.len()).rev() {
      if bottom <= area.y {
        break;
      }
      let top = bottom - self.height_at(i, res);
      if y >= top && y < bottom {
        let m = &self.rows[i].msg;
        return m.x.as_ref().filter(|x| x.opens).map(|_| (m.platform, m.login.clone()));
      }
      bottom = top;
    }
    None
  }

  // the emote under (x, y), as drawn last. badges and plain text are not emotes
  pub fn emote_at(&mut self, area: Rect, x: i32, y: i32, res: &mut Res) -> Option<EmoteHit> {
    let mut bottom = self.edge(area, res);
    for i in (0..self.rows.len()).rev() {
      if bottom <= area.y {
        return None;
      }
      let top = bottom - self.height_at(i, res);
      if y >= top && y < bottom {
        let (m, muted) = (&self.rows[i].msg, self.rows[i].muted());
        let laid = layout_row(res, &self.channel, m, muted, self.width, opts!(self));
        let (lx, ly) = ((x - area.x - MARK_W) as f32 - theme::PAD_X, (y - top) as f32);
        let eb = laid.layout.lines().flat_map(|l| l.items().collect::<Vec<_>>()).find_map(|it| match it {
          PositionedLayoutItem::InlineBox(b)
            if (b.x..b.x + b.width).contains(&lx) && (b.y..b.y + b.height).contains(&ly) =>
          {
            laid.boxes.get(b.id as usize)
          }
          _ => None,
        })?;
        let (a, b) = (eb.word.0 as usize, eb.word.1 as usize);
        let word = m.body.get(a..b).filter(|w| !w.is_empty())?;
        let (url, _, zero_width, scope) = find_word(&mut res.sources(), room(m, &self.channel), m, a, b, word)?;
        let over = eb
          .zw_words
          .iter()
          .filter_map(|&(a, b)| {
            let word = m.body.get(a as usize..b as usize)?;
            let (url, _, _, scope) =
              find_word(&mut res.sources(), room(m, &self.channel), m, a as usize, b as usize, word)?;
            Some(Layer { name: word.into(), url, scope })
          })
          .collect();
        return Some(EmoteHit { name: word.into(), url, zero_width, scope, over });
      }
      bottom = top;
    }
    None
  }

  // the link under (x, y), as drawn last: its target
  pub fn link_at(&mut self, area: Rect, x: i32, y: i32, res: &mut Res) -> Option<String> {
    let mut bottom = self.edge(area, res);
    for i in (0..self.rows.len()).rev() {
      if bottom <= area.y {
        return None;
      }
      let top = bottom - self.height_at(i, res);
      if y >= top && y < bottom {
        let (m, muted) = (&self.rows[i].msg, self.rows[i].muted());
        if !m.kind.said() || muted {
          return None;
        }
        let mut laid = layout_row(res, &self.channel, m, muted, self.width, opts!(self));
        if laid.links.is_empty() {
          return None;
        }
        let (lx, ly) = ((x - area.x - MARK_W) as f32 - theme::PAD_X, (y - top) as f32);
        let over = laid.layout.lines().any(|line| {
          let lm = line.metrics();
          (lm.block_min_coord..lm.block_max_coord).contains(&ly)
            && line.items().any(|it| match it {
              PositionedLayoutItem::GlyphRun(gr) => {
                let at = gr.run().text_range();
                let hit = laid.link_runs.iter().any(|(r, _)| r.start < at.end && at.start < r.end);
                hit && (gr.offset()..gr.offset() + gr.advance()).contains(&lx)
              }
              _ => false,
            })
        });
        if !over {
          return None;
        }
        // the cluster under the pointer says which link, when one run holds several
        let at = Cluster::from_point(&laid.layout, lx, ly)?.0.text_range().start;
        let k = laid.link_runs.iter().find(|(r, _)| r.contains(&at)).map(|(_, k)| *k)?;
        return Some(laid.links.swap_remove(k).url);
      }
      bottom = top;
    }
    None
  }

  // the id of the line whose ▲ is under (x, y), as drawn last
  pub fn vote_at(&mut self, area: Rect, x: i32, y: i32, res: &mut Res) -> Option<Box<str>> {
    self.spot_at(area, (x, y), res, |l| &l.vote)
  }

  // the feed post whose held box's button (show / hide / retry) is under (x, y)
  pub fn reveal_at(&mut self, area: Rect, x: i32, y: i32, res: &mut Res) -> Option<Box<str>> {
    self.spot_at(area, (x, y), res, |l| &l.reveal)
  }

  // the picture under (x, y): a line's inline picture or a feed post's, never an emote or badge; and
  // the video it plays, when it is a still
  pub fn pic_at(&mut self, area: Rect, x: i32, y: i32, res: &mut Res) -> Option<(ImgId, Option<Box<str>>)> {
    let (i, top) = self.row_at(area, y, res)?;
    let (m, muted) = (&self.rows[i].msg, self.rows[i].muted());
    let laid = layout_row(res, &self.channel, m, muted, self.width, opts!(self));
    let (lx, ly) = ((x - area.x - MARK_W) as f32 - theme::PAD_X, (y - top) as f32);
    let img = laid.layout.lines().flat_map(|l| l.items().collect::<Vec<_>>()).find_map(|it| match it {
      PositionedLayoutItem::InlineBox(b)
        if (b.x..b.x + b.width).contains(&lx) && (b.y..b.y + b.height).contains(&ly) =>
      {
        laid.boxes.get(b.id as usize).map(|eb| eb.img)
      }
      _ => None,
    })?;
    let play = laid.plays.into_iter().find(|p| p.0 == img).map(|p| p.1);
    (laid.embed == Some(img) || laid.pics.contains(&img)).then_some((img, play))
  }

  // the feed post whose `[img]` tag is under (x, y)
  pub fn media_at(&mut self, area: Rect, x: i32, y: i32, res: &mut Res) -> Option<Box<str>> {
    self.spot_at(area, (x, y), res, |l| &l.media)
  }

  // the feed post whose >>id is under (x, y): a post's own, or one written in a line's words
  pub fn thread_at(&mut self, area: Rect, x: i32, y: i32, res: &mut Res) -> Option<Box<str>> {
    if let Some(id) = self.spot_at(area, (x, y), res, |l| &l.thread) {
      return Some(id);
    }
    let (i, top) = self.row_at(area, y, res)?;
    let (m, muted) = (&self.rows[i].msg, self.rows[i].muted());
    let laid = layout_row(res, &self.channel, m, muted, self.width, opts!(self));
    let ranges: Vec<_> = laid.refs.iter().map(|r| r.0.clone()).collect();
    let k = over(&laid.layout, (x - area.x - MARK_W) as f32 - theme::PAD_X, (y - top) as f32, &ranges)?;
    laid.refs.into_iter().nth(k).map(|r| r.1)
  }

  // the reply whose `↳ @name words` line is under (x, y): its id (the chain it opens). a click on the
  // @name itself is the person's, so the caller asks for names first
  pub fn reply_at(&mut self, area: Rect, x: i32, y: i32, res: &mut Res) -> Option<Box<str>> {
    self.text_at(area, (x, y), res, |m, laid, i, _| (laid.hdr.contains(&i) && !m.id.is_empty()).then(|| m.id.clone()))
  }

  // the row under height `y` and its top, as drawn last
  pub(super) fn row_at(&mut self, area: Rect, y: i32, res: &mut Res) -> Option<(usize, i32)> {
    let mut bottom = self.edge(area, res);
    for i in (0..self.rows.len()).rev() {
      if bottom <= area.y {
        return None;
      }
      let top = bottom - self.height_at(i, res);
      if y >= top && y < bottom {
        return Some((i, top));
      }
      bottom = top;
    }
    None
  }

  // the line whose `spot` (a range of its laid-out text) is under (x, y)
  pub(super) fn spot_at(
    &mut self,
    area: Rect,
    at: (i32, i32),
    res: &mut Res,
    spot: fn(&Laid) -> &std::ops::Range<usize>,
  ) -> Option<Box<str>> {
    self.text_at(area, at, res, |m, laid, i, _| spot(laid).contains(&i).then(|| m.id.clone()))
  }

  // the automod hold whose allow (true) or deny (false) is under (x, y)
  pub fn held_at(&mut self, area: Rect, x: i32, y: i32, res: &mut Res) -> Option<(Box<str>, bool)> {
    self.text_at(area, (x, y), res, |m, laid, i, _| {
      let id = m.x.as_ref()?.held.as_ref()?.id.clone();
      [(true, &laid.allow), (false, &laid.deny)].into_iter().find(|(_, r)| r.contains(&i)).map(|(a, _)| (id, a))
    })
  }

  // the `#tag` under (x, y): the tag, and which line (seq) and where in it, so a second copy is a new spot
  pub fn tag_at(&mut self, area: Rect, x: i32, y: i32, res: &mut Res) -> Option<(Box<str>, (i64, usize))> {
    self.text_at(area, (x, y), res, |_, laid, i, seq| {
      laid.tags.iter().find(|(r, _)| r.contains(&i)).map(|(r, t)| (t.clone(), (seq, r.start)))
    })
  }

  // ask `f` about the line under (x, y), where in its laid-out text the pointer is, and its seq
  pub(super) fn text_at<T>(
    &mut self,
    area: Rect,
    (x, y): (i32, i32),
    res: &mut Res,
    f: impl FnOnce(&Message, &Laid, usize, i64) -> Option<T>,
  ) -> Option<T> {
    let mut bottom = self.edge(area, res);
    for i in (0..self.rows.len()).rev() {
      if bottom <= area.y {
        return None;
      }
      let top = bottom - self.height_at(i, res);
      if y >= top && y < bottom {
        let (m, muted) = (&self.rows[i].msg, self.rows[i].muted());
        let laid = layout_row(res, &self.channel, m, muted, self.width, opts!(self));
        let (lx, ly) = ((x - area.x - MARK_W) as f32 - theme::PAD_X, (y - top) as f32);
        let at = Cluster::from_point(&laid.layout, lx, ly)?.0.text_range();
        return f(m, &laid, at.start, self.rows[i].seq);
      }
      bottom = top;
    }
    None
  }

  // the person under (x, y): a line's sender, or someone named in its words. only an identity (no
  // clone of the line), so a pointer moving over chat can ask on every move
  pub fn who_at(&mut self, area: Rect, x: i32, y: i32, res: &mut Res) -> Option<WhoHit> {
    let mut bottom = self.edge(area, res);
    for i in (0..self.rows.len()).rev() {
      if bottom <= area.y {
        return None;
      }
      let top = bottom - self.height_at(i, res);
      if y >= top && y < bottom {
        let (m, muted) = (&self.rows[i].msg, self.rows[i].muted());
        // a stream line (went live, switched to) heads with the streamer's name: that name is them too
        let stream = m.kind == Kind::Event && !m.login.is_empty();
        if !m.kind.said() && !stream {
          return None;
        }
        let laid = layout_row(res, &self.channel, m, muted, self.width, opts!(self));
        let (lx, ly) = ((x - area.x - MARK_W) as f32 - theme::PAD_X, (y - top) as f32);
        let mut ranges = Vec::with_capacity(1 + laid.mentions.len());
        ranges.push(laid.nick.clone());
        if !stream {
          ranges.extend(laid.mentions.iter().map(|n| n.range.clone()));
        }
        let k = over(&laid.layout, lx, ly, &ranges)?;
        let seq = self.rows[i].seq;
        return Some(match k.checked_sub(1).and_then(|k| laid.mentions.get(k)) {
          Some(n) => WhoHit {
            seq,
            start: n.range.start,
            mention: Some(Mentioned { login: n.login.clone(), color: n.color, paint: n.paint.clone() }),
          },
          None => WhoHit { seq, start: laid.nick.start, mention: None },
        });
      }
      bottom = top;
    }
    None
  }
}
