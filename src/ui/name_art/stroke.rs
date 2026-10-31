// the outline: a stroke of `width` css px centred on the glyph edge (-webkit-text-stroke), and the
// drop-shadow chain (`filter: drop-shadow(..) drop-shadow(..)`, each shadow casting everything so far)
use super::raster::{Px, over};
use super::shadow_layer;
use crate::cosmetics::fill::{Drop, Rgba};
use crate::cosmetics::paint::Shadow;

// coverage grown by r device px: a neighbour at distance d counts for r - d + 1 (so a fraction of a
// pixel grows the edge by that fraction)
fn grow(alpha: &[f32], w: usize, h: usize, r: f32) -> Vec<f32> {
  let reach = r.ceil() as i32;
  let offs: Vec<(i32, i32, f32)> = (-reach..=reach)
    .flat_map(|dy| (-reach..=reach).map(move |dx| (dx, dy)))
    .filter_map(|(dx, dy)| {
      let k = (r - (dx as f32).hypot(dy as f32) + 1.0).clamp(0.0, 1.0);
      (k > 0.0).then_some((dx, dy, k))
    })
    .collect();
  let mut out = vec![0f32; w * h];
  for y in 0..h as i32 {
    for x in 0..w as i32 {
      let mut m = 0f32;
      for &(dx, dy, k) in &offs {
        let (sx, sy) = (x + dx, y + dy);
        if sx >= 0 && sy >= 0 && (sx as usize) < w && (sy as usize) < h {
          m = m.max(alpha[sy as usize * w + sx as usize] * k);
        }
      }
      out[y as usize * w + x as usize] = m;
    }
  }
  out
}

pub struct Stroke {
  // the stroke's own pixels, premultiplied
  pub px: Vec<Px>,
  // coverage of glyph + stroke: what a shadow under it is cast from
  pub outer: Vec<f32>,
  // over the fill (a see-through fill shows the inner half) or under it (an opaque fill hides it)
  pub top: bool,
}

pub fn premul(c: Rgba, a: f32) -> Px {
  let q = (a.clamp(0.0, 1.0) * c[3] as f32 + 0.5) as u32;
  [(c[0] as u32 * q / 255) as u8, (c[1] as u32 * q / 255) as u8, (c[2] as u32 * q / 255) as u8, q as u8]
}

// `r` = half the width in device px; `inner` = the fill is see-through, so the inner half shows
pub fn stroke(alpha: &[f32], w: usize, h: usize, r: f32, color: Rgba, inner: bool) -> Stroke {
  let outer = grow(alpha, w, h, r);
  let ero: Vec<f32> = if inner {
    let inv: Vec<f32> = alpha.iter().map(|a| 1.0 - a).collect();
    grow(&inv, w, h, r).iter().map(|v| 1.0 - v).collect()
  } else {
    alpha.to_vec()
  };
  let px = outer.iter().zip(&ero).map(|(o, e)| premul(color, o - e)).collect();
  Stroke { px, outer, top: inner }
}

// the shadows a name wears, as one premultiplied layer to draw the glyph over. `src` is the alpha
// the first shadow is cast from; `s` css px -> device px
pub fn chain(src: &[f32], w: usize, h: usize, s: f32, drops: &[Drop]) -> Vec<Px> {
  let mut seen = src.to_vec();
  let mut out = vec![[0u8; 4]; w * h];
  for d in drops {
    // drop-shadow's length is the gaussian's deviation; shadow_layer takes the css blur radius (2 sigma)
    let sh = Shadow { dx: d.x * s, dy: d.y * s, blur: d.blur * s * 2.0, color: 0xff << 24 };
    let a = shadow_layer(&seen, w, h, &sh);
    for i in 0..w * h {
      let mut px = premul(d.color, a[i]);
      over(&mut px, out[i]);
      out[i] = px;
      seen[i] += a[i] * d.color[3] as f32 / 255.0 * (1.0 - seen[i]);
    }
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  // a 3px-wide vertical bar in a 12x8 box
  fn bar() -> Vec<f32> {
    (0..96).map(|i| if (4..7).contains(&(i % 12)) { 1.0 } else { 0.0 }).collect()
  }

  #[test]
  fn stroke_straddles_the_edge_in_whole_and_partial_pixels() {
    let s = stroke(&bar(), 12, 8, 1.0, [255, 0, 0, 255], true);
    let row = |x: usize| s.px[3 * 12 + x];
    assert_eq!((row(3)[3], row(4)[3], row(5)[3], row(6)[3], row(7)[3]), (255, 255, 0, 255, 255));
    assert_eq!(row(2)[3], 0);
    let thin = stroke(&bar(), 12, 8, 0.125, [255, 0, 0, 255], true);
    let e = thin.px[3 * 12 + 3][3];
    assert!(e > 20 && e < 40, "a quarter px stroke leaves an eighth outside: {e}");
    assert_eq!(thin.px[3 * 12 + 5][3], 0, "the middle of a stem is untouched");
  }

  #[test]
  fn opaque_fill_hides_the_inner_half() {
    let s = stroke(&bar(), 12, 8, 1.0, [0, 255, 0, 255], false);
    assert!(!s.top);
    assert_eq!((s.px[3 * 12 + 3][3], s.px[3 * 12 + 4][3]), (255, 0));
  }

  #[test]
  fn chained_shadows_cast_the_shadows_before_them() {
    let d = |x: f32| Drop { x, y: 0.0, blur: 0.0, color: [0, 0, 0, 255] };
    let one = chain(&bar(), 12, 8, 1.0, &[d(3.0)]);
    let two = chain(&bar(), 12, 8, 1.0, &[d(3.0), d(3.0)]);
    let a = |v: &[Px], x: usize| v[3 * 12 + x][3];
    assert_eq!((a(&one, 9), a(&one, 12 - 1)), (255, 0));
    // the second shadow is cast from glyph + first shadow, so it reaches 3 further
    assert_eq!((a(&two, 9), a(&two, 11)), (255, 255));
    assert_eq!(a(&one, 11), 0);
  }

  #[test]
  fn hairline_offsets_are_not_rounded_away() {
    let d = Drop { x: 0.5, y: 0.0, blur: 0.0, color: [0, 0, 0, 255] };
    let c = chain(&bar(), 12, 8, 1.0, &[d]);
    let e = c[3 * 12 + 7][3];
    assert!(e > 100 && e < 160, "{e}");
  }
}
