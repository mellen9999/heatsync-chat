// parley layout (bidi, fallback, inline boxes) + swash raster into a bounded glyph cache
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::ops::Range;

use parley::{
  Alignment, AlignmentOptions, FontContext, FontFamily, FontWeight, GenericFamily, InlineBox, Layout, LayoutContext,
  LineHeight, OverflowWrap, PositionedInlineBox, PositionedLayoutItem, StyleProperty,
};
use swash::FontRef;
use swash::scale::image::Content;
use swash::scale::{Render, ScaleContext, Source, StrikeWith};
use swash::zeno::{Format, Vector};

use super::canvas::{Canvas, Rect};
use super::theme;

pub type TextLayout = Layout<Ink>;

// brush = plain rgb; parley keys runs on it
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Ink(pub u32);

impl Ink {
  // set on a painted username's brush: its glyphs are skipped here, the row draws the shaded bitmap
  pub const PAINTED: u32 = 1 << 24;
  // set on a search hit: drawn in reverse video, white behind black glyphs
  pub const REVERSE: u32 = 1 << 25;
  // dim that lifts to the lighter gray when drawn on a zebra stripe (`Text::stripe`)
  pub const LIFT: u32 = 1 << 26;
}

impl Default for Ink {
  fn default() -> Self {
    Ink(theme::WHITE)
  }
}

pub struct Span {
  pub range: Range<usize>,
  pub ink: Ink,
  pub bold: bool,
}

#[derive(Hash, PartialEq, Eq, Clone, Copy)]
struct GlyphKey {
  font: u64,
  index: u32,
  id: u32,
  size: u32,
  subx: u8,
  bold: bool,
}

struct Glyph {
  left: i32,
  top: i32,
  w: i32,
  h: i32,
  color: bool,
  data: Box<[u8]>,
}

// caps the glyph cache; cleared wholesale on overflow (cheap to refill, no lru bookkeeping)
const GLYPH_CACHE_MAX: usize = 4096;
const SUBPIXEL_STEPS: f32 = 4.0;

pub struct Text {
  fonts: FontContext,
  layouts: LayoutContext<Ink>,
  scale: ScaleContext,
  glyphs: HashMap<GlyphKey, Option<Glyph>>,
  pub font_px: f32,
  pub line_px: f32,
  // the row being drawn sits on a zebra stripe
  pub stripe: bool,
  // the row being drawn is the picked one: every glyph black (the row is filled white), painted names too
  pub reverse: bool,
  // a span of the text being drawn that is reversed (an inline button under the pointer)
  pub lit: Option<Range<usize>>,
}

impl Text {
  pub fn new(scale: f32) -> Self {
    Self {
      fonts: FontContext::new(),
      layouts: LayoutContext::new(),
      scale: ScaleContext::new(),
      glyphs: HashMap::new(),
      font_px: (theme::FONT_PX * scale).round(),
      line_px: (theme::LINE_PX * scale).round(),
      stripe: false,
      reverse: false,
      lit: None,
    }
  }

  pub fn set_scale(&mut self, scale: f32) {
    self.font_px = (theme::FONT_PX * scale).round();
    self.line_px = (theme::LINE_PX * scale).round();
    self.glyphs.clear();
  }

  pub fn layout(&mut self, text: &str, spans: &[Span], boxes: &[InlineBox], width: f32) -> TextLayout {
    self.layout_at(text, spans, boxes, width, self.line_px, (0..0, 1))
  }

  // the same with each line `line` px tall (chat rows: the line-height setting)
  // `emoji` = (range, k): emoji inside the range draw k× the font size and their line grows to fit
  pub fn layout_at(
    &mut self,
    text: &str,
    spans: &[Span],
    boxes: &[InlineBox],
    width: f32,
    line: f32,
    emoji: (Range<usize>, u16),
  ) -> TextLayout {
    let mut b = self.layouts.ranged_builder(&mut self.fonts, text, 1.0, true);
    b.push_default(StyleProperty::FontFamily(FontFamily::from(GenericFamily::Monospace)));
    b.push_default(StyleProperty::FontSize(self.font_px));
    b.push_default(StyleProperty::LineHeight(LineHeight::Absolute(line)));
    b.push_default(StyleProperty::Brush(Ink::default()));
    // a word wider than the line breaks where it must instead of running off the edge (the ext's
    // overflow-wrap: anywhere on chat lines and the message box)
    b.push_default(StyleProperty::OverflowWrap(OverflowWrap::Anywhere));
    for s in spans {
      b.push(StyleProperty::Brush(s.ink), s.range.clone());
      if s.bold {
        b.push(StyleProperty::FontWeight(FontWeight::BOLD), s.range.clone());
      }
    }
    for r in emoji_forced(text) {
      b.push(StyleProperty::FontFamily(FontFamily::from(GenericFamily::Emoji)), r);
    }
    if emoji.1 > 1 {
      let big = self.font_px * emoji.1 as f32;
      let (lo, hi) = (emoji.0.start.min(text.len()), emoji.0.end.min(text.len()));
      for r in emoji_runs(&text[lo..hi]) {
        let r = lo + r.start..lo + r.end;
        b.push(StyleProperty::FontSize(big), r.clone());
        b.push(StyleProperty::LineHeight(LineHeight::Absolute(line.max((big * 1.2).ceil()))), r);
      }
    }
    for ib in boxes {
      b.push_inline_box(ib.clone());
    }
    let mut l = b.build(text);
    l.break_all_lines(Some(width.max(1.0)));
    l.align(Alignment::Start, AlignmentOptions::default());
    l
  }

  // inline boxes are handed to `on_box` with absolute coordinates
  pub fn draw(
    &mut self,
    l: &TextLayout,
    c: &mut Canvas,
    ox: f32,
    oy: f32,
    clip: Rect,
    mut on_box: impl FnMut(&mut Canvas, &PositionedInlineBox),
  ) {
    let Self { glyphs, scale, stripe: self_stripe, reverse: self_reverse, lit, .. } = self;
    let (self_stripe, self_reverse) = (*self_stripe, *self_reverse);
    let lit = lit.clone();
    if glyphs.len() > GLYPH_CACHE_MAX {
      glyphs.clear();
    }
    for line in l.lines() {
      let m = line.metrics();
      if oy + m.block_max_coord < clip.y as f32 || oy + m.block_min_coord > clip.bottom() as f32 {
        continue;
      }
      for item in line.items() {
        let gr = match item {
          PositionedLayoutItem::GlyphRun(gr) => gr,
          PositionedLayoutItem::InlineBox(mut b) => {
            b.x += ox;
            b.y += oy;
            on_box(c, &b);
            continue;
          }
        };
        let run = gr.run();
        let font = run.font();
        let Some(fref) = FontRef::from_index(font.data.data(), font.index as usize) else {
          continue;
        };
        let size = run.font_size();
        let bold = run.synthesis().embolden();
        let ink = gr.style().brush.0;
        if ink & Ink::PAINTED != 0 && !self_reverse {
          continue;
        }
        if ink & Ink::REVERSE != 0 && !self_reverse {
          let (x, y0, y1) = (ox + gr.offset(), oy + m.block_min_coord, oy + m.block_max_coord);
          let bar = Rect::new(x.floor() as i32, y0.floor() as i32, gr.advance().ceil() as i32, (y1 - y0).ceil() as i32);
          c.fill(bar.intersect(clip), theme::WHITE);
        }
        let on = lit.as_ref().is_some_and(|r| {
          let at = run.text_range();
          r.start <= at.start && at.end <= r.end
        });
        let ink = if self_reverse || on {
          theme::BLACK
        } else if ink & Ink::LIFT != 0 && self_stripe {
          theme::DIM_STRIPE
        } else {
          ink & 0xff_ffff
        };
        let coords = run.normalized_coords();
        let key = |g: &parley::Glyph| {
          let gx = ox + g.x;
          let subx = ((gx - gx.floor()) * SUBPIXEL_STEPS) as u8;
          GlyphKey { font: font.data.id(), index: font.index, id: g.id, size: size.to_bits(), subx, bold }
        };
        // rasterize misses first (one scaler per run), then blit from the cache
        if gr.positioned_glyphs().any(|g| !glyphs.contains_key(&key(&g))) {
          let mut s =
            scale.builder(fref).size(size).hint(true).normalized_coords(coords.iter().map(|c| c.to_bits())).build();
          for g in gr.positioned_glyphs() {
            let k = key(&g);
            if let Entry::Vacant(v) = glyphs.entry(k) {
              let mut r =
                Render::new(&[Source::ColorOutline(0), Source::ColorBitmap(StrikeWith::BestFit), Source::Outline]);
              r.format(Format::Alpha).offset(Vector::new(k.subx as f32 / SUBPIXEL_STEPS, 0.0));
              if bold {
                r.embolden(size / 24.0);
              }
              v.insert(r.render(&mut s, g.id as u16).map(|img| Glyph {
                left: img.placement.left,
                top: img.placement.top,
                w: img.placement.width as i32,
                h: img.placement.height as i32,
                color: img.content == Content::Color,
                data: img.data.into_boxed_slice(),
              }));
            }
          }
        }
        for g in gr.positioned_glyphs() {
          let fx = (ox + g.x).floor();
          let glyph = glyphs.get(&key(&g)).and_then(Option::as_ref);
          let Some(gl) = glyph else { continue };
          if gl.w == 0 || gl.h == 0 {
            continue;
          }
          let x = fx as i32 + gl.left;
          let y = (oy + g.y).round() as i32 - gl.top;
          if gl.color {
            c.rgba(x, y, gl.w, gl.h, &gl.data, clip);
          } else {
            c.mask(x, y, gl.w, gl.h, &gl.data, ink, clip);
          }
        }
      }
    }
  }
}

// a sequence asked to draw as emoji (❤️, ☺️, 1️⃣, 🏳️‍🌈: holds U+FE0F or a keycap) goes to the emoji
// font. parley tries the monospace font first even for emoji clusters, and it has text-style ❤ ☺ 1,
// so these came out mono. a sequence = a base char plus what joins onto it: selectors, keycaps, zwj
// and the char after it, skin tones, flag tags
fn emoji_forced(text: &str) -> impl Iterator<Item = Range<usize>> + '_ {
  let joins =
    |c: char| matches!(c, '\u{fe0f}' | '\u{20e3}' | '\u{200d}' | '\u{1f3fb}'..='\u{1f3ff}' | '\u{e0020}'..='\u{e007f}');
  let mut it = text.char_indices().peekable();
  std::iter::from_fn(move || {
    while let Some((start, c)) = it.next() {
      let (mut end, mut asked, mut zwj) = (start + c.len_utf8(), false, false);
      while let Some(&(i, n)) = it.peek() {
        if !joins(n) && !zwj {
          break;
        }
        asked |= matches!(n, '\u{fe0f}' | '\u{20e3}');
        zwj = n == '\u{200d}';
        end = i + n.len_utf8();
        it.next();
      }
      if asked {
        return Some(start..end);
      }
    }
    None
  })
}

// every emoji cluster in `text`, for scaling (a superset of `emoji_forced`): pictographs, flags (a
// regional indicator pair), keycaps, and what joins on (selectors, skin tones, zwj + next, tags). a
// bmp symbol (❤ ✔ ☺) counts only with U+FE0F or when it is emoji by default; digits # * only as keycaps
// the word is one emoji and nothing else (what an overlay emote can sit on)
pub fn lone_emoji(word: &str) -> bool {
  matches!(emoji_runs(word).as_slice(), [r] if !word.is_empty() && *r == (0..word.len()))
}

fn emoji_runs(text: &str) -> Vec<Range<usize>> {
  let ri = |c: char| matches!(c, '\u{1f1e6}'..='\u{1f1ff}');
  let pict = |c: char| matches!(c, '\u{1f300}'..='\u{1faff}' | '\u{1f004}' | '\u{1f0cf}' | '\u{1f18e}' | '\u{1f191}'..='\u{1f19a}' | '\u{1f201}' | '\u{1f21a}' | '\u{1f22f}' | '\u{1f232}'..='\u{1f23a}' | '\u{1f250}' | '\u{1f251}');
  let by_default = |c: char| {
    matches!(c, '\u{231a}' | '\u{231b}' | '\u{23e9}'..='\u{23ec}' | '\u{23f0}' | '\u{23f3}' | '\u{25fd}' | '\u{25fe}'
      | '\u{2614}' | '\u{2615}' | '\u{2648}'..='\u{2653}' | '\u{267f}' | '\u{2693}' | '\u{26a1}' | '\u{26aa}' | '\u{26ab}'
      | '\u{26bd}' | '\u{26be}' | '\u{26c4}' | '\u{26c5}' | '\u{26ce}' | '\u{26d4}' | '\u{26ea}' | '\u{26f2}' | '\u{26f3}'
      | '\u{26f5}' | '\u{26fa}' | '\u{26fd}' | '\u{2705}' | '\u{270a}' | '\u{270b}' | '\u{2728}' | '\u{274c}' | '\u{274e}'
      | '\u{2753}'..='\u{2755}' | '\u{2757}' | '\u{2795}'..='\u{2797}' | '\u{27b0}' | '\u{27bf}' | '\u{2b1b}' | '\u{2b1c}'
      | '\u{2b50}' | '\u{2b55}')
  };
  // a symbol that wants U+FE0F to be emoji: the bmp misc/dingbat/arrow blocks and © ® ™ ‼ ⁉ ℹ Ⓜ ▪ ▫ ▶ ◀ ◻-◾ ⤴ ⤵ ⬅-⬇ 〰 〽 ㊗ ㊙
  let symbol = |c: char| {
    matches!(c, '\u{a9}' | '\u{ae}' | '\u{203c}' | '\u{2049}' | '\u{2122}' | '\u{2139}' | '\u{2194}'..='\u{21aa}' | '\u{231a}'..='\u{23ff}'
      | '\u{24c2}' | '\u{25aa}'..='\u{25fe}' | '\u{2600}'..='\u{27bf}' | '\u{2934}' | '\u{2935}' | '\u{2b05}'..='\u{2b55}'
      | '\u{3030}' | '\u{303d}' | '\u{3297}' | '\u{3299}')
  };
  let joins = |c: char| matches!(c, '\u{fe0f}' | '\u{20e3}' | '\u{1f3fb}'..='\u{1f3ff}' | '\u{e0020}'..='\u{e007f}');
  let mut out = Vec::new();
  if text.is_ascii() {
    return out;
  }
  let mut it = text.char_indices().peekable();
  while let Some((start, c)) = it.next() {
    let mut end = start + c.len_utf8();
    let vs = |it: &mut std::iter::Peekable<std::str::CharIndices>| it.peek().is_some_and(|&(_, n)| n == '\u{fe0f}');
    let ok = if ri(c) {
      if let Some(&(i, n)) = it.peek().filter(|&&(_, n)| ri(n)) {
        end = i + n.len_utf8();
        it.next();
      }
      true
    } else if matches!(c, '0'..='9' | '#' | '*') {
      // keycap: base, optional U+FE0F, U+20E3
      let mut cl = it.clone();
      let mut e = start + 1;
      let mut cap = false;
      for (i, n) in cl.by_ref() {
        if n == '\u{fe0f}' && !cap {
          e = i + 3;
        } else {
          cap = n == '\u{20e3}';
          if cap {
            e = i + 3;
          }
          break;
        }
      }
      if cap {
        end = e;
        while it.peek().is_some_and(|&(i, _)| i < e) {
          it.next();
        }
      }
      cap
    } else {
      pict(c) || by_default(c) || (symbol(c) && vs(&mut it))
    };
    if !ok {
      continue;
    }
    loop {
      match it.peek() {
        Some(&(i, n)) if joins(n) => {
          end = i + n.len_utf8();
          it.next();
        }
        Some(&(i, '\u{200d}')) => {
          // zwj takes the next char too
          let mut cl = it.clone();
          cl.next();
          match cl.next() {
            Some((j, n)) if pict(n) || symbol(n) || by_default(n) || ri(n) => {
              end = j + n.len_utf8();
              it = cl;
            }
            _ => {
              end = i + 3;
              it.next();
              break;
            }
          }
        }
        _ => break,
      }
    }
    out.push(start..end);
  }
  out
}

#[cfg(test)]
mod tests {
  use super::{Text, emoji_forced, emoji_runs};

  #[test]
  fn a_word_wider_than_the_line_breaks_inside_it() {
    let mut t = Text::new(1.0);
    let word = "a".repeat(600);
    let l = t.layout(&word, &[], &[], 200.0);
    assert!(l.lines().count() > 5, "{} lines", l.lines().count());
    assert!(l.full_width() <= 200.0, "nothing past the edge ({})", l.full_width());
    // words that fit still break between words, never inside one
    let w = t.layout("hello ther", &[], &[], f32::MAX).full_width();
    let l = t.layout("hello there", &[], &[], w);
    assert_eq!(l.lines().count(), 2);
    assert_eq!(l.lines().next().map(|x| x.text_range()), Some(0..6));
  }

  fn scaled(s: &str) -> Vec<&str> {
    emoji_runs(s).into_iter().map(|r| &s[r]).collect()
  }

  #[test]
  fn emoji_clusters_scale_and_text_does_not() {
    for e in [
      "🌇",
      "👍🏽",
      "🇯🇵",
      "👨\u{200d}👩\u{200d}👧",
      "❤\u{fe0f}",
      "1\u{fe0f}\u{20e3}",
      "#\u{20e3}",
      "✅",
      "🏳\u{fe0f}\u{200d}🌈",
    ] {
      assert_eq!(scaled(&format!("a {e} b")), [e], "{e}");
    }
    assert_eq!(scaled("😂😂"), ["😂", "😂"]);
    assert_eq!(scaled("🇯🇵🇺🇸"), ["🇯🇵", "🇺🇸"]);
    for t in ["1", "#", "*", "12 #1", "❤", "✔ ☺", "漢字かな", "مرحبا", "plain text"] {
      assert!(scaled(t).is_empty(), "{t}");
    }
  }

  fn runs(s: &str) -> Vec<&str> {
    emoji_forced(s).map(|r| &s[r]).collect()
  }

  #[test]
  fn selector_and_keycap_force_emoji() {
    assert_eq!(runs("i ❤\u{fe0f} u"), ["❤\u{fe0f}"]);
    assert_eq!(runs("1\u{fe0f}\u{20e3}#\u{20e3}"), ["1\u{fe0f}\u{20e3}", "#\u{20e3}"]);
    assert_eq!(runs("🏳\u{fe0f}\u{200d}🌈!"), ["🏳\u{fe0f}\u{200d}🌈"]);
    assert_eq!(runs("plain ❤ 🌇 text"), Vec::<&str>::new());
  }
}
