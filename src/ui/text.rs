// parley layout (bidi, fallback, inline boxes) + swash raster into a bounded glyph cache
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::ops::Range;

use parley::{
  Alignment, AlignmentOptions, FontContext, FontFamily, FontWeight, GenericFamily, InlineBox, Layout, LayoutContext,
  LineHeight, PositionedInlineBox, PositionedLayoutItem, StyleProperty,
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
    }
  }

  pub fn set_scale(&mut self, scale: f32) {
    self.font_px = (theme::FONT_PX * scale).round();
    self.line_px = (theme::LINE_PX * scale).round();
    self.glyphs.clear();
  }

  pub fn layout(&mut self, text: &str, spans: &[Span], boxes: &[InlineBox], width: f32) -> TextLayout {
    let mut b = self.layouts.ranged_builder(&mut self.fonts, text, 1.0, true);
    b.push_default(StyleProperty::FontFamily(FontFamily::from(GenericFamily::Monospace)));
    b.push_default(StyleProperty::FontSize(self.font_px));
    b.push_default(StyleProperty::LineHeight(LineHeight::Absolute(self.line_px)));
    b.push_default(StyleProperty::Brush(Ink::default()));
    for s in spans {
      b.push(StyleProperty::Brush(s.ink), s.range.clone());
      if s.bold {
        b.push(StyleProperty::FontWeight(FontWeight::BOLD), s.range.clone());
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
    let Self { glyphs, scale, .. } = self;
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
        if ink & Ink::PAINTED != 0 {
          continue;
        }
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
