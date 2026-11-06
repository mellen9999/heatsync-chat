// painting rows onto the canvas, and the repaint of what animates alone

use super::layout::{layout_row, notice_ink};
use super::{Anim, ChatView, Frame, MARK_W, NameAnim, TICK_SCENE_MS, TICK_SMOOTH_MS};
use crate::cosmetics::paint::Fill;
use crate::images::{Images, ImgId};
use crate::model::filter;
use crate::model::message::flag;
use crate::ui::canvas::{Canvas, Rect};
use crate::ui::name_art::anim::AnimArt;
use crate::ui::name_art::{ArtKey, PAINT_IMG_PX};
use crate::ui::res::Res;
use crate::ui::text::{Ink, Span, Text, TextLayout};
use crate::ui::{name_art, theme, union};
use parley::PositionedLayoutItem;
use std::sync::Arc;

// the letter of a name's picture that is not here: colored text in a 1px box of that color, on black
pub(super) fn stand_in(c: &mut Canvas, text: &mut Text, r: Rect, ch: char, ink: u32, clip: Rect) {
  c.fill(r.intersect(clip), theme::BLACK);
  c.outline(r, clip, ink);
  let mut buf = [0u8; 4];
  let t: &str = ch.encode_utf8(&mut buf);
  let l = text.layout(t, &[Span { range: 0..t.len(), ink: Ink(ink), bold: true }], &[], r.w as f32);
  let (x, y) = (r.x as f32 + (r.w as f32 - l.full_width()) / 2.0, r.y as f32 + (r.h as f32 - l.height()) / 2.0);
  text.draw(&l, c, x, y, r.intersect(clip), |_, _| {});
}

// an emoji as big as the emote slot it sits in, centered (an overlay emote goes on top)
pub fn emoji_glyph(c: &mut Canvas, text: &mut Text, r: Rect, g: &str, clip: Rect) {
  let k = ((r.h as f32 / text.font_px).round() as u16).max(1);
  let spans = [Span { range: 0..g.len(), ink: Ink(theme::WHITE), bold: false }];
  let l = text.layout_at(g, &spans, &[], f32::MAX, text.line_px, (0..g.len(), k));
  let (x, y) = (r.x as f32 + (r.w as f32 - l.full_width()) / 2.0, r.y as f32 + (r.h as f32 - l.height()) / 2.0);
  text.draw(&l, c, x, y, r.intersect(clip), |_, _| {});
}

// a ▶ in the middle of a video's still: white on a black square with a white edge
pub(super) fn play_mark(c: &mut Canvas, r: Rect, clip: Rect) {
  let side = (r.w.min(r.h) / 3).clamp(12, 48);
  let sq = Rect::new(r.x + (r.w - side) / 2, r.y + (r.h - side) / 2, side, side);
  c.fill(sq.intersect(clip), theme::BLACK);
  c.outline(sq, clip, theme::WHITE);
  // a right-pointing triangle, half the square tall, drawn one row at a time
  let th = side / 2;
  let tw = th * 7 / 8;
  let (x0, y0) = (sq.x + (side - tw) / 2 + 1, sq.y + (side - th) / 2);
  for i in 0..th {
    let len = tw * (th - (2 * i - th + 1).abs()) / th;
    c.fill(Rect::new(x0, y0 + i, len.max(1), 1).intersect(clip), theme::WHITE);
  }
}

// the box around `range` on its first line: (x, top, width, bottom) in layout coordinates
pub(super) fn range_rect(layout: &TextLayout, range: &std::ops::Range<usize>) -> Option<(f32, f32, f32, f32)> {
  for line in layout.lines() {
    let m = line.metrics();
    let mut found: Option<(f32, f32)> = None;
    for item in line.items() {
      let PositionedLayoutItem::GlyphRun(gr) = item else { continue };
      let at = gr.run().text_range();
      if at.start >= range.end || range.start >= at.end {
        continue;
      }
      // only the clusters inside the range: a run can carry more than it
      let mut x = gr.offset();
      for c in gr.run().visual_clusters() {
        let w = c.advance();
        if range.contains(&c.text_range().start) {
          found = Some(found.map_or((x, x + w), |(a, b)| (a.min(x), b.max(x + w))));
        }
        x += w;
      }
    }
    if let Some((a, b)) = found {
      return Some((a, m.block_min_coord, b - a, m.block_max_coord));
    }
  }
  None
}

// one layer of an emote, drawn `dst` big (its decoded size in chat), centered in `rect`: zero-width
// overlays center on the base. the hover popup draws its enlarged stack through this same placement
pub fn blit_layer(c: &mut Canvas, rgba: &[u8], src: (u32, u32), dst: (i32, i32), rect: Rect, clip: Rect) {
  let x = rect.x + (rect.w - dst.0) / 2;
  let y = rect.y + (rect.h - dst.1) / 2;
  if dst == (src.0 as i32, src.1 as i32) {
    c.rgba(x, y, dst.0, dst.1, rgba, clip);
  } else {
    c.rgba_to(x, y, (src.0 as i32, src.1 as i32), dst, rgba, clip);
  }
}

pub(super) fn blit_emote(
  c: &mut Canvas,
  images: &mut Images,
  play: bool,
  imgs: &[ImgId],
  rect: Rect,
  clip: Rect,
  next: &mut Option<u32>,
) {
  for &id in imgs {
    let Some((f, w, h, flip)) = images.frame_with(id, play) else {
      continue;
    };
    blit_layer(c, &f.rgba, (w, h), (w as i32, h as i32), rect, clip);
    if let Some(ms) = flip {
      *next = Some(next.map_or(ms, |n| n.min(ms)));
    }
  }
}

impl ChatView {
  // draw rows bottom-up into `area`, touching only pixels inside `clip`
  pub fn draw(&mut self, c: &mut Canvas, res: &mut Res, area: Rect, clip: Rect) {
    let clip = clip.intersect(area);
    c.fill(clip, theme::BLACK);
    self.anims.retain(|a| a.rect.intersect(clip).is_empty());
    self.name_anims.retain(|a| a.rect.intersect(clip).is_empty());
    if self.rows.is_empty() && !self.empty.is_empty() {
      let spans = [Span { range: 0..self.empty.len(), ink: Ink(theme::DIM), bold: false }];
      let l = res.text.layout(&self.empty, &spans, &[], (self.width - MARK_W) as f32 - 2.0 * theme::PAD_X);
      res.text.draw(&l, c, (area.x + MARK_W) as f32 + theme::PAD_X, area.y as f32, clip, |_, _| {});
      return;
    }
    let (w, zebra) = (self.width, self.zebra && res.look.zebra);
    let mut bottom = self.edge(area, res);
    let o = opts!(self);
    let mut drawn_lo = self.rows.len();
    let hot = self.hot;
    for (i, r) in self.rows.iter_mut().enumerate().rev() {
      if bottom <= area.y {
        break;
      }
      if r.hidden() {
        continue;
      }
      if r.height == 0 || (r.laid.is_none() && bottom - r.height < clip.bottom()) {
        let laid = layout_row(res, &self.channel, &r.msg, r.muted(), w, o);
        r.height = laid.layout.height().ceil() as i32;
        r.laid = Some(Box::new(laid));
      }
      let top = bottom - r.height;
      if top < clip.bottom() && bottom > clip.y {
        let row = Rect::new(area.x, top, area.w, r.height);
        let stripe = zebra && r.seq % 2 == 0;
        let (seq, picked) = (r.seq, self.selected == Some(r.seq));
        let bg = match (r.msg.has(flag::ALERT), stripe) {
          _ if picked => theme::WHITE,
          (true, true) => theme::MENTION_STRIPE,
          (true, false) => theme::MENTION_BG,
          (false, true) => theme::ZEBRA,
          (false, false) => theme::BLACK,
        };
        if bg != theme::BLACK {
          c.fill(row.intersect(clip), bg);
        }
        // a mention's signal is its red fill (ext), so it takes no edge of its own
        let mark = if r.msg.has(flag::BRAND) {
          Some(theme::HEATSYNC)
        } else if r.msg.has(flag::FIRST) {
          Some(theme::MAGENTA)
        } else if r.look & filter::MARKED != 0 {
          // a highlight rule's own colour (clamped to the palette), else the plain highlight
          Some(filter::mark_ink(r.look))
        } else if r.msg.has(flag::HIGHLIGHTED) {
          Some(theme::CYAN)
        } else if let Some(ink) = notice_ink(r.msg.notice()) {
          // an event's ring outranks the session-first glow (ext: declared after it)
          Some(ink)
        } else if r.msg.has(flag::NEW) && self.glow {
          Some(theme::YELLOW)
        } else {
          r.msg.x.as_ref().map(|x| x.edge).filter(|&e| e != 0)
        };
        if let Some(m) = mark {
          c.fill(Rect::new(area.x, top, MARK_W, r.height).intersect(clip), m);
        }
        if let Some(laid) = &r.laid {
          let play = res.look.animate_emotes;
          let Res { text, images, arts, .. } = res;
          if let Some(id) = laid.embed {
            images.want(id);
          }
          for &id in &laid.pics {
            images.want(id);
          }
          let x0 = (area.x + MARK_W) as f32 + theme::PAD_X;
          let mut lit_range = None;
          for (range, f) in &laid.frames {
            let Some((fx, ft, fw, fb)) = range_rect(&laid.layout, range) else { continue };
            let rect =
              Rect::new((x0 + fx).round() as i32, top + ft.round() as i32, fw.round() as i32, (fb - ft).round() as i32);
            let lit = hot == Some((seq, range.start)) && !picked;
            let f = if picked { &Frame::Chip(theme::BLACK) } else { f };
            if lit && matches!(f, Frame::Button | Frame::Hold) {
              // an inline button under the pointer: reversed, white behind black words
              c.fill(rect.intersect(clip), theme::WHITE);
              lit_range = Some(range.clone());
            }
            match f {
              Frame::Held => c.dashed(rect, clip, theme::CYAN),
              // a plain underlined word: the ext's 1px box is gone, hover reverses it
              Frame::Button => {
                let ink = if lit || picked { theme::BLACK } else { theme::WHITE };
                c.fill(Rect::new(rect.x, rect.bottom() - 1, rect.w, 1).intersect(clip), ink);
              }
              Frame::Hold | Frame::Announce => c.outline(rect, clip, theme::YELLOW),
              Frame::Bits => c.outline(rect, clip, theme::TWITCH),
              Frame::Chip(ink) => c.outline(rect, clip, *ink),
            }
          }
          let anims = &mut self.anims;
          let name_anims = &mut self.name_anims;
          let x = (area.x + MARK_W) as f32 + theme::PAD_X;
          text.stripe = stripe;
          text.reverse = picked;
          text.lit = lit_range;
          let mut stills = Vec::new();
          let mut stand_ins = Vec::new();
          let mut on_emoji = Vec::new();
          text.draw(&laid.layout, c, x, top as f32, clip, |c, b| {
            let Some(eb) = laid.boxes.get(b.id as usize) else {
              return;
            };
            let rect =
              Rect::new(b.x.round() as i32, b.y.round() as i32, b.width.round() as i32, b.height.round() as i32);
            if laid.plays.iter().any(|p| p.0 == eb.img) {
              stills.push(rect);
            }
            if eb.blocked {
              c.dashed(rect, clip, theme::WHITE);
              return;
            }
            // a name's picture not yet drawable (loading, failed, none): its stand-in
            if let Some((ch, ink)) = eb.stand_in
              && images.frame_with(eb.img, false).is_none()
            {
              stand_ins.push((rect, ch, ink));
              return;
            }
            let mut imgs = Vec::with_capacity(1 + eb.zw.len());
            imgs.push(eb.img);
            imgs.extend_from_slice(&eb.zw);
            // the emoji under it needs the text drawer, busy here: drawn right after
            if let Some(g) = &eb.glyph {
              on_emoji.push((rect, g.clone(), imgs));
              return;
            }
            if let Some(bg) = eb.bg {
              c.fill(rect.intersect(clip), bg);
            }
            let mut flip = None;
            blit_emote(c, images, play, &imgs, rect, clip, &mut flip);
            if flip.is_some() {
              anims.push(Anim { rect: rect.intersect(area), bg, imgs, glyph: None });
            }
          });
          for (r, g, imgs) in on_emoji {
            emoji_glyph(c, text, r, &g, clip);
            let mut flip = None;
            blit_emote(c, images, play, &imgs, r, clip, &mut flip);
            if flip.is_some() {
              self.anims.push(Anim { rect: r.intersect(area), bg, imgs, glyph: Some(g) });
            }
          }
          text.reverse = false;
          text.lit = None;
          for r in stills {
            play_mark(c, r, clip);
          }
          for (r, ch, ink) in stand_ins {
            stand_in(c, text, r, ch, ink, clip);
          }
          // a picked row is reversed: its names are drawn black by the text pass, not as colored art
          for n in laid.names.iter().filter(|_| !picked) {
            let key = ArtKey {
              uid: n.uid.clone(),
              text: name_art::text_key(&n.nick),
              color: n.color,
              font_px: text.font_px.to_bits(),
              paint: Arc::as_ptr(&n.paint) as usize,
            };
            let w = n.w.ceil() as i32;
            if let Some(fx) = &n.paint.fx {
              // scene + effects: built once, then a premultiplied frame per tick
              let (aw, ah, base, animated, smooth) = {
                let a = arts.anim(&key, &n.nick, |motion| AnimArt::build(text, &n.nick, w, fx, motion, n.color));
                (a.w, a.h, a.base, a.animated(), a.smooth())
              };
              let (ax, ay) = ((x + n.x).round() as i32, (top as f32 + n.baseline - base).round() as i32);
              if animated {
                if let Some((_, px)) = arts.frame(&key, name_art::clock()) {
                  c.rgba_pm(ax, ay, aw, ah, px, clip);
                }
                name_anims.push(NameAnim { rect: Rect::new(ax, ay, aw, ah), bg, key, smooth });
              } else if let Some(art) = arts.still(&key) {
                c.rgba(ax, ay, art.w, art.h, &art.rgba, clip);
              }
            } else {
              // image paints wait for their (still) picture; the first bitmap is redone when it lands
              let pic_id = match &n.paint.fill {
                Fill::Image { still } => Some(images.id(still, PAINT_IMG_PX)),
                _ => None,
              };
              let pic = pic_id.and_then(|id| images.frame(id)).map(|(f, w, h, _)| (&f.rgba[..], w, h));
              let art = arts.get(&key, &n.nick, pic.is_some() || pic_id.is_none(), || {
                name_art::build(text, &n.nick, w, n.color, &n.paint, pic)
              });
              let (ax, ay) = ((x + n.x).round() as i32, (top as f32 + n.baseline - art.base).round() as i32);
              c.rgba(ax, ay, art.w, art.h, &art.rgba, clip);
            }
          }
        }
      }
      drawn_lo = i;
      bottom = top;
    }
    // layouts that scrolled out of view are dropped; heights stay
    for r in self.rows.iter_mut().take(drawn_lo) {
      r.laid = None;
    }
    self.next_anim = self.soonest(res);
  }

  pub(super) fn soonest(&self, res: &mut Res) -> Option<u32> {
    let mut next = self.name_tick();
    for a in &self.anims {
      for &id in &a.imgs {
        if let Some((_, _, _, Some(ms))) = res.images.frame(id) {
          next = Some(next.map_or(ms, |n: u32| n.min(ms)));
        }
      }
    }
    next
  }

  // animated painted names currently on screen
  pub fn animated_names(&self) -> usize {
    self.name_anims.len()
  }

  // when the animated names on screen tick next (none on screen: nothing wakes)
  pub(super) fn name_tick(&self) -> Option<u32> {
    self.name_anims.iter().map(|a| if a.smooth { TICK_SMOOTH_MS } else { TICK_SCENE_MS }).min()
  }

  // repaint only on-screen animated emotes and painted names; returns the damaged region
  pub fn animate(&mut self, c: &mut Canvas, res: &mut Res, area: Rect) -> Option<Rect> {
    let mut dirty: Option<Rect> = None;
    let mut next = None;
    for a in &self.anims {
      let r = a.rect.intersect(area);
      if r.is_empty() {
        continue;
      }
      c.fill(r, a.bg);
      if let Some(g) = &a.glyph {
        emoji_glyph(c, &mut res.text, a.rect, g, r);
      }
      blit_emote(c, &mut res.images, res.look.animate_emotes, &a.imgs, a.rect, r, &mut next);
      dirty = Some(dirty.map_or(r, |d| union(d, r)));
    }
    let t = name_art::clock();
    for a in &self.name_anims {
      let r = a.rect.intersect(area);
      if r.is_empty() {
        continue;
      }
      let Some((art, px)) = res.arts.frame(&a.key, t) else { continue };
      c.fill(r, a.bg);
      c.rgba_pm(a.rect.x, a.rect.y, art.w, art.h, px, r);
      let ms = if a.smooth { TICK_SMOOTH_MS } else { TICK_SCENE_MS };
      next = Some(next.map_or(ms, |n: u32| n.min(ms)));
      dirty = Some(dirty.map_or(r, |d| union(d, r)));
    }
    self.next_anim = next;
    dirty
  }
}
