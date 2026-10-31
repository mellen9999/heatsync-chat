// the scene catalog: every backdrop and weather of the ext's scene-spec.js, ported 1:1 (same
// gradients, same pixel silhouettes, same tile sizes and loop travel). colors come from the user's
// tint through the ext's hsl ladder, so a free pick keeps each plate's designed lightness
use super::raster::Svg;
use super::scene::{Band, Img, LayerDef, SceneArt, backdrop_period, l, weather_period};
use crate::cosmetics::fx::{Backdrop, Scene, Weather};

// ---- tint kit ---------------------------------------------------------------------------------

struct K {
  h: f32,
  s: f32,
  l: f32,
  tint: u32,
}

const NIGHT: f32 = 245.0;

fn hex(n: u32) -> String {
  format!("#{n:06x}")
}

fn hsl_hex(h: f32, s: f32, l: f32) -> String {
  let (s, l) = (s.clamp(0.0, 1.0), l.clamp(0.0, 1.0));
  let f = |n: f32| {
    let kk = (n + h / 30.0).rem_euclid(12.0);
    let c = l - s * l.min(1.0 - l) * (-1.0f32).max((kk - 3.0).min(9.0 - kk).min(1.0));
    (c * 255.0).round() as u32
  };
  format!("#{:02x}{:02x}{:02x}", f(0.0), f(8.0), f(4.0))
}

impl K {
  fn new(tint: u32) -> K {
    let (r, g, b) = ((tint >> 16 & 255) as f32 / 255.0, (tint >> 8 & 255) as f32 / 255.0, (tint & 255) as f32 / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let d = max - min;
    let l = (max + min) / 2.0;
    if d == 0.0 {
      return K { h: 0.0, s: 0.0, l, tint };
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let h = if max == r {
      ((g - b) / d + 6.0) % 6.0
    } else if max == g {
      (b - r) / d + 2.0
    } else {
      (r - g) / d + 4.0
    };
    K { h: h * 60.0, s, l, tint }
  }

  // k(l)
  fn k(&self, l: f32) -> String {
    self.kh(l, 1.0, 0.0)
  }

  // k(l, sat)
  fn ks(&self, l: f32, sat: f32) -> String {
    self.kh(l, sat, 0.0)
  }

  // k(l, sat, hue)
  fn kh(&self, l: f32, sat: f32, hue: f32) -> String {
    hsl_hex((self.h + hue + 360.0).rem_euclid(360.0), self.s * sat, l)
  }

  // k.to(l, sat, target, amount)
  fn to(&self, l: f32, sat: f32, target: f32, amount: f32) -> String {
    let delta = (target - self.h + 540.0).rem_euclid(360.0) - 180.0;
    hsl_hex((self.h + delta * amount + 360.0).rem_euclid(360.0), self.s * sat, l)
  }

  fn tint(&self) -> String {
    hex(self.tint)
  }
}

fn g(s: String) -> Img {
  Img::G(s)
}

// ---- svg helpers ------------------------------------------------------------------------------

fn deck_path(steps: &[(u32, u32)]) -> String {
  let mut d = format!("M0 {}", steps[0].1);
  for (w, h) in steps {
    d += &format!("V{h}h{w}");
  }
  d + "V0H0Z"
}

fn band_path(steps: &[(u32, u32)], hh: u32) -> String {
  let mut d = format!("M0 {hh}");
  for (w, h) in steps {
    d += &format!("V{}h{w}", hh - h);
  }
  d + &format!("V{hh}Z")
}

fn cloud_deck(steps: &[(u32, u32)], w: u32, color: &str, op: f32) -> Svg {
  Svg::new(w as f32, 24.0).path(&deck_path(steps), color, op)
}

fn band(steps: &[(u32, u32)], w: u32, hh: u32, color: &str) -> Svg {
  Svg::new(w as f32, hh as f32).path(&band_path(steps, hh), color, 1.0)
}

fn city_deck(steps: &[(u32, u32)], w: u32, hh: u32, color: &str, win: &str) -> Svg {
  let mut svg = band(steps, w, hh, color);
  let mut x = 0u32;
  for (i, &(bw, bh)) in steps.iter().enumerate() {
    if bh >= 8 {
      let mut y = hh - bh + 2;
      while y < hh - 2 {
        let mut wx = x + 1;
        while wx < x + bw - 1 {
          if (wx * 7 + y * 3 + i as u32) % 5 < 2 {
            svg = svg.rect(wx as f32, y as f32, 1.0, 1.0, win, 0.85);
          }
          wx += 2;
        }
        y += 3;
      }
    }
    x += bw;
  }
  svg
}

fn ridge(steps: &[(u32, u32)], w: u32, hh: u32, color: &str, cap: &str) -> Svg {
  let mut svg = band(steps, w, hh, color);
  let mut x = 0;
  for &(bw, bh) in steps {
    if bh >= 9 {
      svg = svg.rect(x as f32, (hh - bh) as f32, bw as f32, 2.0, cap, 0.9);
    }
    x += bw;
  }
  svg
}

const CLOUD_FAR: [(u32, u32); 7] = [(5, 7), (6, 10), (4, 6), (7, 11), (5, 8), (6, 12), (7, 7)];
const CLOUD_NEAR: [(u32, u32); 8] = [(7, 6), (6, 10), (8, 5), (7, 12), (9, 8), (6, 13), (6, 7), (7, 6)];
const SWELL_FAR: [(u32, u32); 11] =
  [(4, 5), (5, 7), (4, 9), (5, 7), (4, 5), (5, 4), (4, 5), (5, 7), (4, 9), (4, 7), (4, 5)];
const SWELL_NEAR: [(u32, u32); 11] =
  [(3, 4), (4, 6), (3, 8), (4, 6), (3, 4), (4, 3), (3, 4), (4, 6), (3, 8), (3, 6), (2, 4)];
const ICE_FAR: [(u32, u32); 10] = [(6, 6), (5, 9), (4, 7), (7, 10), (5, 6), (6, 8), (5, 11), (4, 7), (6, 9), (4, 6)];
const ICE_NEAR: [(u32, u32); 9] = [(5, 4), (4, 6), (5, 5), (4, 7), (5, 4), (4, 3), (5, 4), (4, 6), (4, 4)];
const CITY_FAR: [(u32, u32); 13] =
  [(6, 10), (2, 6), (7, 15), (3, 8), (8, 12), (2, 5), (6, 18), (4, 9), (7, 13), (3, 6), (6, 11), (2, 7), (8, 10)];
const CITY_NEAR: [(u32, u32); 13] = [
  (8, 14),
  (3, 9),
  (10, 20),
  (4, 11),
  (9, 16),
  (3, 8),
  (11, 22),
  (5, 12),
  (8, 17),
  (3, 10),
  (10, 19),
  (6, 13),
  (8, 14),
];
const PEAKS: [(u32, u32); 24] = [
  (4, 6),
  (4, 9),
  (4, 13),
  (4, 17),
  (4, 21),
  (4, 17),
  (4, 12),
  (4, 9),
  (4, 13),
  (4, 18),
  (4, 22),
  (4, 18),
  (4, 13),
  (4, 9),
  (4, 7),
  (4, 10),
  (4, 14),
  (4, 11),
  (4, 8),
  (4, 6),
  (4, 8),
  (4, 11),
  (4, 8),
  (4, 6),
];
const RIDGE_FAR: [(u32, u32); 15] = [
  (5, 8),
  (5, 12),
  (5, 16),
  (5, 13),
  (5, 9),
  (5, 11),
  (5, 15),
  (5, 19),
  (5, 15),
  (5, 11),
  (5, 8),
  (5, 10),
  (5, 13),
  (5, 10),
  (5, 8),
];
const RIDGE_NEAR: [(u32, u32); 14] =
  [(4, 4), (4, 7), (4, 10), (4, 13), (4, 10), (4, 7), (4, 5), (4, 8), (4, 12), (4, 9), (4, 6), (4, 4), (4, 6), (4, 4)];
const CASTLE: [(u32, u32); 37] = [
  (3, 7),
  (2, 9),
  (3, 7),
  (2, 9),
  (3, 7),
  (2, 9),
  (3, 7),
  (5, 15),
  (2, 18),
  (2, 15),
  (2, 18),
  (2, 15),
  (5, 15),
  (3, 7),
  (2, 9),
  (3, 7),
  (2, 9),
  (3, 7),
  (7, 21),
  (2, 24),
  (2, 21),
  (2, 24),
  (2, 21),
  (7, 21),
  (3, 7),
  (2, 9),
  (3, 7),
  (2, 9),
  (3, 7),
  (2, 9),
  (3, 7),
  (5, 13),
  (2, 16),
  (2, 13),
  (2, 16),
  (5, 13),
  (3, 7),
];
const MOON: [(u32, u32); 10] = [(6, 5), (6, 6), (6, 5), (6, 7), (6, 6), (6, 5), (6, 6), (6, 7), (6, 6), (6, 5)];

fn sum_w(s: &[(u32, u32)]) -> u32 {
  s.iter().map(|x| x.0).sum()
}

fn sil_dunes(c: &str) -> Svg {
  Svg::new(96.0, 20.0).path("M0 20v-8h6v-2h8v-2h10v2h8v2h10v4h6v-2h10v-4h8v-2h10v2h8v4h6v2h6v4z", c, 1.0)
}

fn sil_graveyard(c: &str) -> Svg {
  Svg::new(140.0, 26.0).path(
    "M0 26v-2h140v2z M4 24v-7h2v7z M11 24v-7h2v7z M18 24v-7h2v7z M2 19h20v2H2z M34 24v-6h2v-2h6v2h2v6z \
     M52 24v-8h-3v-2h3v-4h2v4h3v2h-3v8z M68 24v-2h-2v-2h2v-9h4v9h2v2h-2v2z M96 24v-10h-4v-2h4v-4h2v2h5v2h-5v12z \
     M116 24v-5h2v-2h5v2h2v5z M130 24v-7h2v7z M137 24v-7h2v7z M128 19h12v2h-12z",
    c,
    1.0,
  )
}

fn sil_reef(c: &str) -> Svg {
  Svg::new(90.0, 14.0).path("M0 14v-4h8v-2h10v2h10v2h10v-4h8v-2h8v2h8v4h10v-2h10v2h8v2z", c, 1.0)
}

fn sil_pines(c: &str) -> Svg {
  Svg::new(84.0, 24.0).path(
    "M0 24v-3h84v3z M8 21v-3H5v-4h3v-4h2v-3h2v3h2v4h3v4h-3v3z M30 21v-2h-2v-4h2v-3h2v-3h2v3h2v3h2v4h-2v2z \
     M52 21v-3h-3v-4h3v-4h2v-4h2v4h2v4h3v4h-3v3z M74 21v-2h-2v-3h2v-3h2v3h2v3h-2v2z",
    c,
    1.0,
  )
}

fn sil_sakura(sil: &str, bloom: &str) -> Svg {
  let mut s = Svg::new(120.0, 26.0).path(
    "M0 26v-2h120v2z M50 24v-9h-4v-2h4v-3h2v3h5v-2h3v2h-3v2h-5v9z M96 24v-6h-2v-2h2v-2h2v2h3v2h-3v6z",
    sil,
    1.0,
  );
  for (x, y, r) in [
    (46.0, 11.0, 3.0),
    (52.0, 8.0, 3.4),
    (58.0, 11.0, 3.0),
    (49.0, 15.0, 2.2),
    (56.0, 15.0, 2.0),
    (95.0, 14.0, 2.2),
    (99.0, 12.0, 2.4),
    (102.0, 16.0, 1.8),
  ] {
    s = s.circle(x, y, r, bloom, 1.0);
  }
  s
}

fn sil_reeds(c: &str) -> Svg {
  Svg::new(72.0, 22.0).path(
    "M0 22v-1h72v1z M6 21V9h1v12z M5 6h3v4H5z M15 21V12h1v9z M14 9h3v4h-3z M28 21V7h1v14z M27 4h3v4h-3z \
     M39 21V13h1v8z M52 21V8h1v13z M51 5h3v4h-3z M62 21V11h1v10z M61 8h3v4h-3z",
    c,
    1.0,
  )
}

fn sil_bamboo(c: &str) -> Svg {
  let mut s = Svg::new(40.0, 24.0);
  for y in [0.0, 6.0, 12.0, 18.0] {
    s = s.rect(7.0, y, 3.0, 5.0, c, 1.0).rect(24.0, y + 2.0, 3.0, 5.0, c, 1.0).rect(34.0, y + 1.0, 2.0, 5.0, c, 1.0);
  }
  s.rect(7.0, 0.0, 3.0, 24.0, c, 0.55).rect(24.0, 0.0, 3.0, 24.0, c, 0.55).rect(34.0, 0.0, 2.0, 24.0, c, 0.55).path(
    "M10 7h6v1h-4v1h-2z M22 14h-6v1h4v1h2z M36 10h5v1h-3v1h-2z",
    c,
    1.0,
  )
}

fn sil_pyramids(c: &str) -> Svg {
  Svg::new(120.0, 24.0).path("M0 24v-2h120v2z M18 22l26-19 26 19z M78 22l15-11 15 11z", c, 1.0)
}

fn fg_dunes(c: &str) -> Svg {
  Svg::new(64.0, 8.0).path("M0 8V6h6V4h10v2h8v2h8V6h10V4h8v2h6v2z", c, 1.0)
}

fn fg_graveyard(c: &str) -> Svg {
  Svg::new(72.0, 8.0).path(
    "M0 8V7h72v1z M8 7V3h2V1h5v2h2v4z M30 7V2h-2V0h2v-1h2v1h2v2h-2v5z M50 7V3h2v4z M56 7V3h2v4z M62 7V3h2v4z M48 4h18v1H48z",
    c,
    1.0,
  )
}

fn fg_reef(c: &str) -> Svg {
  Svg::new(56.0, 6.0).path("M0 6V4h5V2h6v2h7v2h9V3h6v3h8V4h7v2h8z", c, 1.0)
}

fn fg_pines(c: &str) -> Svg {
  Svg::new(60.0, 9.0).path(
    "M0 9V8h60v1z M9 8V5H7V3h2V1h2v2h2v2h-2v3z M33 8V6h-2V4h2V2h2v2h2v2h-2v2z M50 8V5h-2V3h2V2h2v1h2v2h-2v3z",
    c,
    1.0,
  )
}

fn fg_sakura(c: &str) -> Svg {
  Svg::new(48.0, 6.0).path("M0 6V5h48v1z M6 5V3h2v2z M15 5V4h2v1z M27 5V3h2v2z M38 5V4h2v1z", c, 1.0)
}

fn fg_swell(c: &str) -> Svg {
  Svg::new(36.0, 6.0).path("M0 6V4h4V2h4v2h4v2h6V3h4v3h6V4h4v2z", c, 1.0)
}

fn fg_ice(c: &str) -> Svg {
  Svg::new(40.0, 6.0).path("M0 6V4h5V2h6v2h5v2h8V3h5v3h6V4h5v2z", c, 1.0)
}

fn fg_roof(c: &str) -> Svg {
  Svg::new(64.0, 8.0).path("M0 8V7h64v1z M12 7V1h1v6z M10 3h5v1h-5z M40 7V4h-1V1h6v3h-1v3z M41 4h4v3h-4z", c, 1.0)
}

fn fg_rock(c: &str) -> Svg {
  Svg::new(48.0, 6.0).path("M0 6V5h48v1z M4 5V3h3V1h4v2h2v2z M22 5V2h5v3z M35 5V3h4V2h3v1h2v2z", c, 1.0)
}

fn fg_reeds(c: &str) -> Svg {
  Svg::new(40.0, 7.0).path(
    "M0 7V6h40v1z M5 6V1h1v5z M4 0h3v2H4z M17 6V2h1v4z M29 6V0h1v6z M28 -2h3v3h-3z M36 6V3h1v3z",
    c,
    1.0,
  )
}

fn fg_wall(c: &str) -> Svg {
  Svg::new(48.0, 6.0).path("M0 6V3h3V1h3v2h4V1h3v2h4V1h3v2h4V1h3v2h4V1h3v2h4V1h3v2h4V1h3v2h1v3z", c, 1.0)
}

fn fg_rocks(c: &str) -> Svg {
  Svg::new(40.0, 5.0).path("M0 5V4h40v1z M6 4V2h5v2z M19 4V3h3v1z M29 4V1h4v3z", c, 1.0)
}

fn fg_grass(c: &str) -> Svg {
  Svg::new(32.0, 6.0).path(
    "M0 6V5h32v1z M3 5V2h1v3z M7 5V3h1v2z M12 5V1h1v4z M17 5V3h1v2z M21 5V2h1v3z M26 5V3h1v2z M29 5V1h1v4z",
    c,
    1.0,
  )
}

fn craters(color: &str) -> impl Fn(Svg) -> Svg {
  let color = color.to_string();
  move |mut s| {
    for (cx, cy, r) in [(9.0, 12.0, 2.5), (27.0, 11.5, 1.6), (44.0, 12.5, 2.8), (54.0, 11.6, 1.2)] {
      s = s.circle(cx, cy, r, &color, 0.55).arc_top(cx, cy, r, 0.6, "#ffffff", 0.35);
    }
    s
  }
}

fn lava_cracks(s: Svg, color: &str) -> Svg {
  let mut s = s;
  for (x, y, h) in
    [(17.0, 3.0, 9.0), (18.0, 8.0, 6.0), (41.0, 2.0, 10.0), (42.0, 9.0, 5.0), (66.0, 10.0, 6.0), (65.0, 13.0, 4.0)]
  {
    s = s.rect(x, y, 1.0, h, color, 0.9);
  }
  s
}

// ---- backdrops --------------------------------------------------------------------------------

struct Built {
  layers: Vec<LayerDef>,
  fg: Option<LayerDef>,
  alternate: bool,
  // the breathe's resting value shown while nothing animates
  rest_k: f32,
}

fn tilepx(w: u32) -> String {
  format!("{w}px")
}

const STARS_A: &str = "radial-gradient(circle,#ffffffcc 0 .5px,transparent 1px)";
const STARS_B: &str = "radial-gradient(circle,#ffffff66 0 .5px,transparent 1px)";

fn sweep(haze: &str, a: &str, b: &str, c: &str) -> String {
  format!("linear-gradient(90deg,transparent 0%,{haze}{a} 35%,{haze}{b} 50%,{haze}{c} 65%,transparent 100%)")
}

fn backdrop(id: Backdrop, k: &K) -> Built {
  use Backdrop::*;
  let plain = |layers: Vec<LayerDef>, fg: Option<LayerDef>| Built { layers, fg, alternate: false, rest_k: 0.0 };
  let r = "no-repeat";
  let full = "100% 100%";
  match id {
    Dawn => {
      let (haze, bloom) = (k.k(0.84), k.k(0.69));
      plain(
        vec![
          l(Img::S(sil_dunes(&k.ks(0.04, 0.8))), "repeat-x", "auto 42%", "0 100%").to("0 100%"),
          l(g(sweep(&haze, "38", "55", "38")), r, "220% 58%", "200% 78%").to("-100% 78%"),
          l(
            g(format!("radial-gradient(90% 90% at 50% 108%,{bloom}66 0%,{bloom}22 40%,transparent 70%)")),
            r,
            full,
            "0 0",
          ),
          l(
            g(format!(
              "linear-gradient(0deg,{} 0%,{} 22%,{} 55%,{} 82%,{} 100%)",
              k.k(0.5),
              k.k(0.35),
              k.to(0.33, 0.35, NIGHT, 0.55),
              k.to(0.26, 0.3, NIGHT, 0.9),
              k.to(0.19, 0.3, NIGHT, 1.0)
            )),
            r,
            full,
            "0 0",
          ),
        ],
        Some(l(Img::S(fg_dunes(&k.ks(0.02, 0.8))), "repeat-x", "auto 26%", "0 100%")),
      )
    }
    Graveyard => {
      let near = cloud_deck(&CLOUD_NEAR, 56, &k.ks(0.22, 0.3), 1.0);
      let far = cloud_deck(&CLOUD_FAR, 40, &k.ks(0.18, 0.35), 0.9);
      plain(
        vec![
          l(Img::S(sil_graveyard(&k.ks(0.04, 0.3))), "repeat-x", "auto 52%", "0 100%").to("0 100%"),
          l(Img::S(near), "repeat-x", "56px 58%", "0 0").to("-56px 0"),
          l(Img::S(far), "repeat-x", "40px 44%", "0 0").to("-40px 0"),
          l(g(format!("radial-gradient(45% 75% at 62% 6%,{}2e 0%,transparent 68%)", k.ks(0.8, 0.5))), r, full, "0 0"),
          l(
            g(format!(
              "linear-gradient(0deg,{} 0%,{} 40%,{} 75%,{} 100%)",
              k.ks(0.2, 0.35),
              k.ks(0.15, 0.35),
              k.ks(0.09, 0.35),
              k.ks(0.06, 0.35)
            )),
            r,
            full,
            "0 0",
          ),
        ],
        Some(l(Img::S(fg_graveyard("#000000")), "repeat-x", "auto 30%", "0 100%")),
      )
    }
    Abyss => {
      let ray = k.tint();
      plain(
        vec![
          l(Img::S(sil_reef(&k.ks(0.02, 0.8))), "repeat-x", "auto 26%", "0 100%").to("0 100%"),
          l(
            g(format!(
              "linear-gradient(104deg,transparent 30%,{ray}14 42%,transparent 50%,{ray}0e 62%,transparent 72%)"
            )),
            r,
            "260% 100%",
            "-90% 0",
          )
          .to("190% 0"),
          l(g(format!("radial-gradient(80% 60% at 50% -10%,{ray}20 0%,transparent 60%)")), r, full, "0 0"),
          l(
            g(format!("linear-gradient(180deg,{} 0%,{} 45%,{} 100%)", k.k(0.15), k.k(0.09), k.k(0.04))),
            r,
            full,
            "0 0",
          ),
        ],
        Some(l(Img::S(fg_reef(&k.k(0.01))), "repeat-x", "auto 22%", "0 100%")),
      )
    }
    Nightfall => {
      let (a1, a2) = (k.tint(), k.kh(k.l, 1.0, 40.0));
      let n = |l: f32| k.to(l, 0.4, 240.0, 0.75);
      plain(
        vec![
          l(Img::S(sil_pines(&k.to(0.03, 0.45, 240.0, 0.75))), "repeat-x", "auto 46%", "0 100%").to("0 100%"),
          l(
            g(format!("linear-gradient(100deg,transparent 15%,{a1}30 35%,{a2}2e 50%,{a1}24 62%,transparent 82%)")),
            r,
            "240% 90%",
            "-90% 0",
          )
          .to("190% 0"),
          l(g(STARS_A.into()), "repeat", "17px 13px", "0 0"),
          l(g(STARS_B.into()), "repeat", "23px 19px", "5px 7px"),
          l(g(format!("linear-gradient(0deg,{} 0%,{} 55%,{} 100%)", n(0.06), n(0.12), n(0.07))), r, full, "0 0"),
        ],
        Some(l(Img::S(fg_pines(&k.to(0.01, 1.0, 240.0, 0.75))), "repeat-x", "auto 30%", "0 100%")),
      )
    }
    Terminal => {
      let ph = k.tint();
      plain(
        vec![
          l(g(format!("linear-gradient(0deg,transparent 38%,{ph}16 50%,transparent 62%)")), r, "100% 300%", "0 0")
            .to("0 100%"),
          l(
            g(format!("repeating-linear-gradient(0deg,{ph}0d 0 1px,transparent 1px 3px)")),
            "repeat",
            "100% auto",
            "0 0",
          ),
          l(g(format!("linear-gradient({},{})", k.ks(0.05, 0.55), k.ks(0.02, 0.55))), r, full, "0 0"),
        ],
        None,
      )
    }
    Furnace => {
      let glow = k.tint();
      let sky =
        l(g(format!("linear-gradient(0deg,{} 0%,{} 55%,#050505 100%)", k.k(0.05), k.ks(0.03, 0.75))), r, full, "0 0");
      let breathe = |c: &str| format!("radial-gradient(120% 90% at 50% 115%,{c} 0%,transparent 65%)");
      Built {
        layers: vec![
          l(Img::Breathe(breathe(&format!("{glow}55")), breathe(&format!("{glow}a8"))), r, full, "0 0"),
          sky,
        ],
        fg: None,
        alternate: true,
        rest_k: (0x66 - 0x55) as f32 / (0xa8 - 0x55) as f32,
      }
    }
    Ocean => {
      let near = band(&SWELL_NEAR, 36, 12, &k.to(0.17, 0.7, 195.0, 0.8));
      let far = band(&SWELL_FAR, 48, 16, &k.to(0.23, 0.65, 195.0, 0.8));
      let (bloom, glit) = (k.k(0.69), k.k(0.84));
      plain(
        vec![
          l(Img::S(near), "repeat-x", "36px 34%", "0 100%").to("-36px 100%"),
          l(Img::S(far), "repeat-x", "48px 46%", "0 100%").to("48px 100%"),
          l(
            g(format!("repeating-linear-gradient(90deg,transparent 0 3px,{glit}66 3px 4px,transparent 4px 7px)")),
            "repeat-x",
            "100% 8%",
            "0 60%",
          )
          .to("7px 60%"),
          l(
            g(format!("radial-gradient(70% 70% at 50% 62%,{bloom}88 0%,{bloom}22 35%,transparent 60%)")),
            r,
            full,
            "0 0",
          ),
          l(
            g(format!(
              "linear-gradient(0deg,{} 0%,{} 24%,{} 58%,{} 100%)",
              k.k(0.5),
              k.to(0.49, 0.55, NIGHT, 0.25),
              k.to(0.3, 0.4, NIGHT, 0.7),
              k.to(0.19, 0.35, NIGHT, 1.0)
            )),
            r,
            full,
            "0 0",
          ),
        ],
        Some(l(Img::S(fg_swell(&k.to(0.08, 0.7, 195.0, 0.8))), "repeat-x", "auto 22%", "0 100%")),
      )
    }
    Skyline => {
      let (win, glow) = ("#ffd75f", k.tint());
      let near = city_deck(&CITY_NEAR, 88, 24, &k.ks(0.05, 0.5), win);
      let far = city_deck(&CITY_FAR, 64, 24, &k.ks(0.13, 0.5), win);
      plain(
        vec![
          l(Img::S(near), "repeat-x", "88px 78%", "0 100%").to("-88px 100%"),
          l(Img::S(far), "repeat-x", "64px 60%", "0 100%").to("-64px 100%"),
          l(
            g(format!("radial-gradient(80% 60% at 50% 100%,{glow}40 0%,{glow}14 40%,transparent 70%)")),
            r,
            full,
            "0 0",
          ),
          l(
            g(format!(
              "linear-gradient(0deg,{} 0%,{} 40%,{} 100%)",
              k.ks(0.19, 0.65),
              k.ks(0.11, 0.65),
              k.ks(0.05, 0.6)
            )),
            r,
            full,
            "0 0",
          ),
        ],
        Some(l(Img::S(fg_roof(&k.ks(0.03, 0.5))), "repeat-x", "auto 30%", "0 100%")),
      )
    }
    Orbit => {
      let (neb, rim) = (k.tint(), k.k(0.69));
      plain(
        vec![
          l(
            g(format!("radial-gradient(160% 110% at 50% 168%,{} 0 44%,{rim}99 45%,transparent 50%)", k.ks(0.28, 0.7))),
            r,
            full,
            "0 0",
          ),
          l(g(STARS_A.into()), "repeat", "17px 13px", "0 0").to("-17px 0"),
          l(g(STARS_B.into()), "repeat", "23px 19px", "5px 7px").to("-18px 7px"),
          l(
            g(format!("linear-gradient(100deg,transparent 20%,{neb}26 40%,{neb}14 52%,transparent 70%)")),
            r,
            "240% 100%",
            "-90% 0",
          )
          .to("190% 0"),
          l(g(format!("linear-gradient(0deg,{} 0%,{} 100%)", k.ks(0.06, 0.4), k.ks(0.03, 0.4))), r, full, "0 0"),
        ],
        None,
      )
    }
    Synth => {
      let (sun, line) = (k.tint(), k.k(0.6));
      plain(
        vec![
          l(
            g(format!(
              "radial-gradient(circle at 50% 100%,{sun} 0 5px,{} 5.5px 14px,{} 100%)",
              k.ks(0.16, 0.7),
              k.ks(0.07, 0.75)
            )),
            r,
            "100% 54%",
            "0 0",
          ),
          l(
            g(format!("repeating-linear-gradient(0deg,{line}cc 0 1px,transparent 1px 5px)")),
            "repeat",
            "100% calc(46% + 5px)",
            "0 100%",
          )
          .to("0 calc(100% + 5px)"),
          l(g(format!("repeating-linear-gradient(90deg,{line}55 0 1px,transparent 1px 9px)")), r, "100% 46%", "0 100%"),
          l(g(format!("linear-gradient(180deg,{} 0%,#000000 100%)", k.ks(0.11, 0.7))), r, "100% 46%", "0 100%"),
          l(g("linear-gradient(#000000,#000000)".into()), r, full, "0 0"),
        ],
        None,
      )
    }
    Glacier => {
      let near = band(&ICE_NEAR, 40, 10, &k.ks(0.38, 0.41));
      let far = band(&ICE_FAR, 52, 14, &k.ks(0.28, 0.42));
      let (haze, bloom) = (k.k(0.88), k.k(0.94));
      plain(
        vec![
          l(Img::S(near), "repeat-x", "40px 32%", "0 100%").to("-40px 100%"),
          l(Img::S(far), "repeat-x", "52px 48%", "0 100%").to("-52px 100%"),
          l(g(sweep(&haze, "30", "4a", "30")), r, "220% 40%", "200% 60%").to("-100% 60%"),
          l(
            g(format!("radial-gradient(60% 60% at 50% 70%,{bloom}55 0%,{bloom}18 40%,transparent 65%)")),
            r,
            full,
            "0 0",
          ),
          l(
            g(format!(
              "linear-gradient(0deg,{} 0%,{} 35%,{} 100%)",
              k.ks(0.48, 0.3),
              k.ks(0.27, 0.37),
              k.ks(0.12, 0.4)
            )),
            r,
            full,
            "0 0",
          ),
        ],
        Some(l(Img::S(fg_ice(&k.ks(0.19, 0.47))), "repeat-x", "auto 22%", "0 100%")),
      )
    }
    Sakura => {
      let (bloom, haze) = (k.k(0.92), k.k(0.92));
      plain(
        vec![
          l(Img::S(sil_sakura(&k.ks(0.07, 0.45), &bloom)), "repeat-x", "auto 62%", "0 100%").to("0 100%"),
          l(g(sweep(&haze, "2e", "44", "2e")), r, "220% 50%", "-100% 30%").to("200% 30%"),
          l(g(format!("radial-gradient(50% 60% at 30% 20%,{bloom}55 0%,transparent 65%)")), r, full, "0 0"),
          l(
            g(format!(
              "linear-gradient(0deg,{} 0%,{} 30%,{} 65%,{} 100%)",
              k.k(0.84),
              k.ks(0.64, 0.6),
              k.to(0.33, 0.35, 250.0, 0.6),
              k.to(0.19, 0.37, 250.0, 1.0)
            )),
            r,
            full,
            "0 0",
          ),
        ],
        Some(l(Img::S(fg_sakura(&k.ks(0.04, 0.45))), "repeat-x", "auto 22%", "0 100%")),
      )
    }
    Volcano => {
      let lava = k.tint();
      let sil = k.kh(0.03, 0.5, -15.0);
      let peaks = lava_cracks(band(&PEAKS, 96, 24, &sil), &lava);
      plain(
        vec![
          l(Img::S(peaks), "repeat-x", "96px 58%", "0 100%").to("0 100%"),
          l(
            g(format!("linear-gradient(90deg,transparent 0%,{sil}99 40%,{sil}bb 50%,{sil}99 60%,transparent 100%)")),
            r,
            "220% 36%",
            "-100% 30%",
          )
          .to("200% 30%"),
          l(
            g(format!("radial-gradient(80% 70% at 50% 110%,{lava}77 0%,{lava}22 40%,transparent 70%)")),
            r,
            full,
            "0 0",
          ),
          l(
            g(format!(
              "linear-gradient(0deg,{} 0%,{} 30%,{} 70%,{} 100%)",
              k.kh(0.19, 1.0, -15.0),
              k.kh(0.09, 0.8, -15.0),
              k.kh(0.04, 0.6, -15.0),
              k.kh(0.02, 0.45, -15.0)
            )),
            r,
            full,
            "0 0",
          ),
        ],
        Some(l(Img::S(fg_rock(&k.kh(0.01, 0.6, -15.0))), "repeat-x", "auto 24%", "0 100%")),
      )
    }
    Alpine => {
      let near = ridge(&RIDGE_NEAR, 56, 14, &k.to(0.18, 0.27, NIGHT, 0.85), "#ffffff");
      let far = ridge(&RIDGE_FAR, 75, 20, &k.to(0.38, 0.25, NIGHT, 0.85), "#ffffff");
      let haze = k.k(0.84);
      plain(
        vec![
          l(Img::S(near), "repeat-x", "56px 40%", "0 100%").to("-56px 100%"),
          l(Img::S(far), "repeat-x", "75px 62%", "0 100%").to("-75px 100%"),
          l(g(sweep(&haze, "40", "66", "40")), r, "220% 22%", "200% 48%").to("-100% 48%"),
          l(
            g(format!(
              "linear-gradient(0deg,{} 0%,{} 22%,{} 55%,{} 100%)",
              k.k(0.69),
              k.to(0.61, 0.6, NIGHT, 0.2),
              k.to(0.45, 0.2, NIGHT, 0.9),
              k.to(0.17, 0.35, NIGHT, 1.0)
            )),
            r,
            full,
            "0 0",
          ),
        ],
        Some(l(Img::S(fg_pines(&k.to(0.06, 0.33, NIGHT, 0.9))), "repeat-x", "auto 30%", "0 100%")),
      )
    }
    Swamp => {
      let (wisp, sil, water) = (k.tint(), k.kh(0.05, 0.4, 20.0), k.kh(0.07, 0.45, 30.0));
      plain(
        vec![
          l(Img::S(sil_reeds(&sil)), "repeat-x", "auto 60%", "0 100%").to("0 100%"),
          l(
            g(format!("radial-gradient(22% 60% at 50% 60%,{wisp}66 0%,{wisp}22 45%,transparent 70%)")),
            r,
            "140% 100%",
            "-60% 0",
          )
          .to("160% 0"),
          l(g(format!("linear-gradient(0deg,{water} 0%,{water} 60%,transparent 100%)")), r, "100% 22%", "0 100%"),
          l(
            g(format!("linear-gradient(90deg,transparent 0%,{sil}66 40%,{sil}88 50%,{sil}66 60%,transparent 100%)")),
            r,
            "220% 40%",
            "200% 70%",
          )
          .to("-100% 70%"),
          l(
            g(format!(
              "linear-gradient(0deg,{} 0%,{} 35%,{} 75%,{} 100%)",
              k.ks(0.27, 0.4),
              k.ks(0.14, 0.35),
              k.ks(0.07, 0.3),
              k.ks(0.03, 0.25)
            )),
            r,
            full,
            "0 0",
          ),
        ],
        Some(l(Img::S(fg_reeds(&k.ks(0.02, 0.35))), "repeat-x", "auto 28%", "0 100%")),
      )
    }
    Castle => {
      let cw = sum_w(&CASTLE);
      let keep = city_deck(&CASTLE, cw, 24, &k.ks(0.06, 0.4), "#ffd75f");
      let near = cloud_deck(&CLOUD_NEAR, 56, &k.ks(0.18, 0.4), 1.0);
      let far = cloud_deck(&CLOUD_FAR, 40, &k.ks(0.16, 0.45), 0.9);
      let moon = k.k(0.93);
      plain(
        vec![
          l(Img::S(keep), "repeat-x", &format!("{} 70%", tilepx(cw)), "0 100%").to("0 100%"),
          l(Img::S(near), "repeat-x", "56px 40%", "0 0").to("-56px 0"),
          l(Img::S(far), "repeat-x", "40px 30%", "0 0").to("-40px 0"),
          l(
            g(format!("radial-gradient(circle at 70% 18%,{moon} 0 3px,{moon}44 3.5px 7px,transparent 12px)")),
            r,
            full,
            "0 0",
          ),
          l(
            g(format!(
              "linear-gradient(0deg,{} 0%,{} 40%,{} 75%,{} 100%)",
              k.ks(0.27, 0.4),
              k.ks(0.18, 0.42),
              k.ks(0.1, 0.44),
              k.ks(0.05, 0.43)
            )),
            r,
            full,
            "0 0",
          ),
        ],
        Some(l(Img::S(fg_wall(&k.ks(0.04, 0.6))), "repeat-x", "auto 24%", "0 100%")),
      )
    }
    Moon => {
      let ground = craters(&k.ks(0.19, 0.3))(band(&MOON, 60, 14, &k.ks(0.37, 0.25)));
      let earth_b = k.ks(0.33, 0.65);
      plain(
        vec![
          l(Img::S(ground), "repeat-x", "60px 44%", "0 100%").to("0 100%"),
          l(
            g(format!(
              "radial-gradient(circle at 80% 0%,{} 0 3px,{earth_b} 3.5px 6px,{earth_b}44 6.5px 7px,transparent 7.5px)",
              k.tint()
            )),
            r,
            full,
            "0 0",
          ),
          l(g(STARS_A.into()), "repeat", "17px 13px", "0 0").to("-17px 0"),
          l(g(STARS_B.into()), "repeat", "23px 19px", "5px 7px").to("-18px 7px"),
          l(g(format!("linear-gradient(0deg,{} 0%,{} 100%)", k.ks(0.04, 0.3), k.ks(0.02, 0.3))), r, full, "0 0"),
        ],
        Some(l(Img::S(fg_rocks(&k.ks(0.15, 0.25))), "repeat-x", "auto 20%", "0 100%")),
      )
    }
    Circuit => {
      let c = k.tint();
      let sig = format!("linear-gradient(90deg,transparent 0%,{c} 50%,transparent 100%)");
      plain(
        vec![
          l(g(sig.clone()), r, "40% 1px", "-40% 6px").to("140% 6px"),
          l(g(sig), r, "40% 1px", "140% 18px").to("-40% 18px"),
          l(g(format!("radial-gradient(circle,{c}55 0 .7px,transparent 1.1px)")), "repeat", "18px 12px", "4px 3px"),
          l(
            g(format!("repeating-linear-gradient(0deg,{c}22 0 1px,transparent 1px 6px)")),
            "repeat",
            "100% auto",
            "0 0",
          ),
          l(
            g(format!("repeating-linear-gradient(90deg,{c}22 0 1px,transparent 1px 9px)")),
            "repeat",
            "auto 100%",
            "0 0",
          ),
          l(g(format!("linear-gradient({},{})", k.ks(0.06, 0.65), k.ks(0.04, 0.7))), r, full, "0 0"),
        ],
        None,
      )
    }
    Eclipse => {
      let corona = k.tint();
      let disc = |c: &str| format!("radial-gradient(circle at 50% 50%,#000000 0 6px,{c} 6.5px 8px,transparent 13px)");
      Built {
        layers: vec![
          l(Img::Breathe(disc(&format!("{corona}66")), disc(&format!("{corona}cc"))), r, full, "0 0"),
          l(g("radial-gradient(circle,#ffffff99 0 .5px,transparent 1px)".into()), "repeat", "19px 15px", "3px 2px"),
          l(g(format!("linear-gradient({},#000000)", k.ks(0.02, 0.45))), r, full, "0 0"),
        ],
        fg: None,
        alternate: true,
        rest_k: (0x88 - 0x66) as f32 / (0xcc - 0x66) as f32,
      }
    }
    Arcade => {
      let (c, c1, c2) = (k.tint(), k.ks(0.1, 0.72), k.ks(0.18, 0.68));
      plain(
        vec![
          l(
            g(format!("repeating-linear-gradient(90deg,{c} 0 3px,transparent 3px 7px)")),
            "repeat-x",
            "7px 2px",
            "0 10%",
          )
          .to("-7px 10%"),
          l(g(format!("linear-gradient(180deg,{c1} 0%,{c1} 100%)")), r, "100% 60%", "0 0"),
          l(g(format!("radial-gradient(80% 40% at 50% 60%,{c}55 0%,transparent 70%)")), r, full, "0 0"),
          l(g(format!("repeating-conic-gradient({c1} 0 25%,{c2} 0 50%)")), "repeat", "8px 8px", "0 0").to("-8px 0"),
          l(g(format!("linear-gradient({},{c2})", k.ks(0.07, 0.75))), r, full, "0 0"),
        ],
        None,
      )
    }
    Bamboo => {
      let haze = k.k(0.92);
      plain(
        vec![
          l(Img::S(sil_bamboo(&k.ks(0.07, 0.45))), "repeat-x", "auto 100%", "0 0").to("0 0"),
          l(g(sweep(&haze, "30", "4a", "30")), r, "220% 50%", "-100% 40%").to("200% 40%"),
          l(
            g(format!(
              "linear-gradient(0deg,{} 0%,{} 35%,{} 75%,{} 100%)",
              k.ks(0.45, 0.2),
              k.ks(0.3, 0.25),
              k.ks(0.15, 0.27),
              k.ks(0.07, 0.3)
            )),
            r,
            full,
            "0 0",
          ),
        ],
        Some(l(Img::S(fg_grass(&k.ks(0.03, 0.45))), "repeat-x", "auto 24%", "0 100%")),
      )
    }
    Giza => {
      let (sun, haze) = (k.kh(0.69, 1.0, 15.0), k.k(0.84));
      plain(
        vec![
          l(Img::S(sil_pyramids(&k.ks(0.1, 0.65))), "repeat-x", "auto 70%", "0 100%").to("0 100%"),
          l(
            g(format!("radial-gradient(circle at 74% 70%,{sun} 0 3px,{sun}44 3.5px 7px,transparent 10px)")),
            r,
            full,
            "0 0",
          ),
          l(g(sweep(&haze, "38", "55", "38")), r, "220% 40%", "200% 90%").to("-100% 90%"),
          l(
            g(format!(
              "linear-gradient(0deg,{} 0%,{} 28%,{} 62%,{} 100%)",
              k.k(0.5),
              k.to(0.46, 0.5, NIGHT, 0.2),
              k.to(0.28, 0.35, NIGHT, 0.65),
              k.to(0.19, 0.35, NIGHT, 1.0)
            )),
            r,
            full,
            "0 0",
          ),
        ],
        Some(l(Img::S(fg_dunes(&k.ks(0.03, 0.65))), "repeat-x", "auto 26%", "0 100%")),
      )
    }
  }
}

// ---- weather tiles ----------------------------------------------------------------------------

type Tile = (Svg, u32, u32);

fn rain_drop(s: Svg, c: &str, x: f32, y: f32, h: f32) -> Svg {
  let at = (x + 1.0, y + h / 2.0);
  s.rect_rot((x, y, 2.0, h), 12.0, at, c, 0.5).rect_rot((x, y + h - 3.0, 2.0, 3.0), 12.0, at, c, 0.95)
}

fn tile(w: u32, h: u32) -> Svg {
  Svg::new(w as f32, h as f32)
}

fn rain_tile(c: &str, d: u8) -> Tile {
  match d {
    3.. => {
      (rain_drop(rain_drop(rain_drop(tile(20, 24), c, 2.0, 2.0, 9.0), c, 10.0, 8.0, 9.0), c, 15.0, 1.0, 8.0), 20, 24)
    }
    2 => (rain_drop(rain_drop(tile(28, 28), c, 4.0, 3.0, 10.0), c, 18.0, 13.0, 10.0), 28, 28),
    _ => (rain_drop(rain_drop(tile(40, 32), c, 7.0, 4.0, 11.0), c, 27.0, 16.0, 10.0), 40, 32),
  }
}

fn dots(mut s: Svg, c: &str, list: &[(f32, f32, f32, f32)]) -> Svg {
  for &(x, y, r, o) in list {
    s = s.circle(x, y, r, c, o);
  }
  s
}

fn snow_tile(c: &str, d: u8) -> Tile {
  match d {
    3.. => (
      dots(
        tile(14, 18),
        c,
        &[(3.0, 3.0, 1.0, 0.9), (10.0, 8.0, 0.7, 0.6), (6.0, 13.0, 1.0, 0.8), (12.0, 15.0, 0.6, 0.5)],
      ),
      14,
      18,
    ),
    2 => (dots(tile(18, 22), c, &[(4.0, 5.0, 1.0, 0.9), (12.0, 14.0, 0.7, 0.6), (8.0, 19.0, 0.6, 0.5)]), 18, 22),
    _ => (dots(tile(24, 28), c, &[(6.0, 6.0, 1.0, 0.85), (16.0, 18.0, 0.7, 0.55)]), 24, 28),
  }
}

fn rects(mut s: Svg, list: &[(f32, f32, f32, f32, &str, f32)]) -> Svg {
  for &(x, y, w, h, c, o) in list {
    s = s.rect(x, y, w, h, c, o);
  }
  s
}

fn ember_tile(c1: &str, c2: &str, d: u8) -> Tile {
  match d {
    3.. => (
      rects(
        tile(14, 20),
        &[
          (3.0, 15.0, 1.0, 2.0, c1, 0.9),
          (9.0, 9.0, 1.0, 1.0, c2, 0.8),
          (6.0, 4.0, 1.0, 1.0, c1, 0.5),
          (12.0, 17.0, 1.0, 1.0, c2, 0.7),
        ],
      ),
      14,
      20,
    ),
    2 => (rects(tile(16, 24), &[(4.0, 18.0, 1.0, 2.0, c1, 0.9), (11.0, 8.0, 1.0, 1.0, c2, 0.7)]), 16, 24),
    _ => (rects(tile(20, 28), &[(5.0, 21.0, 1.0, 2.0, c1, 0.85), (14.0, 9.0, 1.0, 1.0, c2, 0.6)]), 20, 28),
  }
}

fn glyph_col(mut s: Svg, c: &str, x: f32, ys: &[(f32, f32)], head: f32) -> Svg {
  for &(y, h) in ys {
    s = s.rect(x, y, 1.0, h, c, 0.28);
  }
  s.rect(x, head, 1.0, 4.0, c, 0.95)
}

fn glyph_tile(c: &str, d: u8) -> Tile {
  match d {
    3.. => {
      let s = glyph_col(tile(12, 22), c, 2.0, &[(1.0, 3.0), (6.0, 2.0), (10.0, 3.0)], 15.0);
      (glyph_col(s, c, 8.0, &[(3.0, 2.0), (8.0, 3.0), (13.0, 2.0)], 17.0), 12, 22)
    }
    2 => {
      let s = glyph_col(tile(14, 22), c, 3.0, &[(1.0, 3.0), (6.0, 2.0), (10.0, 3.0)], 15.0);
      (glyph_col(s, c, 10.0, &[(4.0, 2.0), (9.0, 3.0)], 16.0), 14, 22)
    }
    _ => (glyph_col(tile(18, 24), c, 5.0, &[(2.0, 3.0), (8.0, 2.0), (13.0, 3.0)], 18.0), 18, 24),
  }
}

// small repeated marks: (x, y, opacity) through a stamp
fn stamps(mut s: Svg, list: &[(f32, f32, f32)], f: impl Fn(Svg, f32, f32, f32) -> Svg) -> Svg {
  for &(x, y, o) in list {
    s = f(s, x, y, o);
  }
  s
}

fn idx(d: u8) -> usize {
  match d {
    3.. => 0,
    2 => 1,
    _ => 2,
  }
}

type Marks = [&'static [(f32, f32, f32)]; 3];

fn marks_tile(d: u8, sizes: [(u32, u32); 3], m: Marks, f: impl Fn(Svg, f32, f32, f32) -> Svg) -> Tile {
  let i = idx(d);
  let (w, h) = sizes[i];
  (stamps(tile(w, h), m[i], f), w, h)
}

fn petal_tile(c: &str, d: u8) -> Tile {
  marks_tile(
    d,
    [(16, 18), (20, 22), (26, 28)],
    [
      &[(2.0, 2.0, 0.9), (9.0, 7.0, 0.7), (5.0, 12.0, 0.85), (12.0, 15.0, 0.6)],
      &[(4.0, 4.0, 0.9), (13.0, 13.0, 0.65), (8.0, 18.0, 0.5)],
      &[(6.0, 6.0, 0.85), (17.0, 18.0, 0.6)],
    ],
    |s, x, y, o| s.rect(x, y, 2.0, 1.0, c, o).rect(x + 1.0, y + 1.0, 1.0, 1.0, c, o),
  )
}

fn bubble_tile(c: &str, d: u8) -> Tile {
  let big = |s: Svg, x, y, o| s.ring(x, y, 1.5, 1.0, c, o);
  let small = |s: Svg, x, y, o| s.ring(x, y, 1.0, 1.0, c, o);
  match d {
    3.. => {
      let s = big(tile(14, 20), 3.0, 4.0, 0.8);
      (big(small(s, 10.0, 10.0, 0.6), 6.0, 16.0, 0.7), 14, 20)
    }
    2 => (small(big(tile(18, 24), 4.0, 5.0, 0.8), 13.0, 15.0, 0.55), 18, 24),
    _ => (small(big(tile(22, 30), 6.0, 7.0, 0.75), 16.0, 21.0, 0.5), 22, 30),
  }
}

fn firefly_tile(c: &str, d: u8) -> Tile {
  marks_tile(
    d,
    [(16, 20), (20, 26), (26, 32)],
    [
      &[(3.0, 4.0, 0.95), (11.0, 9.0, 0.7), (6.0, 15.0, 0.85)],
      &[(4.0, 6.0, 0.9), (14.0, 17.0, 0.65)],
      &[(7.0, 8.0, 0.9), (18.0, 22.0, 0.55)],
    ],
    |s, x, y, o| s.circle(x, y, 2.0, c, o * 0.3).circle(x, y, 0.8, c, o),
  )
}

fn leaf_tile(c: &str, d: u8) -> Tile {
  marks_tile(
    d,
    [(16, 18), (20, 22), (26, 28)],
    [
      &[(2.0, 2.0, 0.9), (9.0, 8.0, 0.7), (4.0, 13.0, 0.85)],
      &[(4.0, 4.0, 0.9), (13.0, 14.0, 0.65)],
      &[(6.0, 6.0, 0.85), (17.0, 19.0, 0.55)],
    ],
    |s, x, y, o| s.path(&format!("M{x} {y}h1v1h2v1h1v1h-1v-1h-2v-1h-1z"), c, o),
  )
}

fn spark_tile(c: &str, d: u8) -> Tile {
  marks_tile(
    d,
    [(12, 16), (16, 22), (22, 28)],
    [
      &[(2.0, 2.0, 0.95), (8.0, 7.0, 0.7), (5.0, 12.0, 0.85)],
      &[(3.0, 4.0, 0.9), (11.0, 14.0, 0.65)],
      &[(6.0, 7.0, 0.9), (15.0, 20.0, 0.55)],
    ],
    |s, x, y, o| s.rect(x, y, 1.0, 3.0, c, o),
  )
}

fn dust_tile(c: &str, d: u8) -> Tile {
  marks_tile(
    d,
    [(14, 14), (18, 18), (24, 22)],
    [
      &[(2.0, 3.0, 0.8), (9.0, 6.0, 0.6), (5.0, 11.0, 0.7), (12.0, 1.0, 0.5)],
      &[(4.0, 5.0, 0.8), (13.0, 12.0, 0.55), (8.0, 16.0, 0.45)],
      &[(6.0, 7.0, 0.75), (17.0, 16.0, 0.5)],
    ],
    |s, x, y, o| s.rect(x, y, 1.0, 1.0, c, o),
  )
}

fn confetti_tile(cs: [&str; 3], d: u8) -> Tile {
  let r = |s: Svg, x: f32, y: f32, c: &str, o: f32, tall: bool| {
    s.rect(x, y, if tall { 1.0 } else { 2.0 }, if tall { 2.0 } else { 1.0 }, c, o)
  };
  match d {
    3.. => {
      let s = r(tile(16, 18), 2.0, 2.0, cs[0], 0.95, false);
      let s = r(s, 9.0, 6.0, cs[1], 0.85, true);
      let s = r(s, 5.0, 11.0, cs[2], 0.9, false);
      (r(s, 12.0, 15.0, cs[0], 0.7, true), 16, 18)
    }
    2 => {
      let s = r(tile(20, 22), 4.0, 4.0, cs[0], 0.95, false);
      let s = r(s, 13.0, 12.0, cs[1], 0.8, true);
      (r(s, 8.0, 18.0, cs[2], 0.85, false), 20, 22)
    }
    _ => {
      let s = r(tile(26, 28), 6.0, 6.0, cs[0], 0.9, false);
      (r(s, 17.0, 19.0, cs[1], 0.7, true), 26, 28)
    }
  }
}

fn meteor_tile(c: &str, d: u8) -> Tile {
  let m = |s: Svg, x: f32, y: f32, o: f32| {
    s.line((x + 8.0, y), (x, y + 5.0), 1.0, c, o * 0.55).rect(x, y + 4.0, 2.0, 2.0, c, o)
  };
  match d {
    3.. => (m(m(tile(24, 20), 2.0, 2.0, 0.95), 12.0, 11.0, 0.7), 24, 20),
    2 => (m(m(tile(32, 26), 4.0, 4.0, 0.9), 18.0, 16.0, 0.5), 32, 26),
    _ => (m(tile(40, 30), 8.0, 8.0, 0.9), 40, 30),
  }
}

fn heart_tile(c: &str, d: u8) -> Tile {
  marks_tile(
    d,
    [(16, 18), (20, 22), (26, 28)],
    [
      &[(2.0, 2.0, 0.9), (9.0, 8.0, 0.7), (4.0, 13.0, 0.85)],
      &[(4.0, 4.0, 0.9), (13.0, 14.0, 0.65)],
      &[(6.0, 6.0, 0.85), (17.0, 19.0, 0.55)],
    ],
    |s, x, y, o| s.path(&format!("M{} {y}h1v1h1v-1h1v2h-1v1h-1v1h-1v-1h-1v-1h-1v-2h1z", x + 1.0), c, o),
  )
}

// ---- weathers ---------------------------------------------------------------------------------

struct WBuilt {
  layers: Vec<LayerDef>,
  // the far plane's tile and its signed travel in tile heights (None = no far plane)
  far: Option<(Tile, i32)>,
  behind: bool,
  alternate: bool,
  flash: bool,
}

fn fall(t: &Tile, dir: i32, sway: Option<(i32, i32)>) -> Vec<LayerDef> {
  let (w, h) = (t.1 as f32, t.2 as f32);
  let (w2, h2) = ((w * 1.4).round() as i32, (h * 1.4).round() as i32);
  let mk = |tw: i32, th: i32, sx: Option<i32>| {
    let mut d = l(Img::S(t.0.clone()), "repeat", &format!("{tw}px {th}px"), "0 0").to(&format!("0 {}px", dir * th));
    if let Some(x) = sx {
      d = d.mid(&format!("{x}px {}px", dir * ((th as f32 / 2.0).round() as i32)));
    }
    d
  };
  vec![mk(w as i32, h as i32, sway.map(|s| s.0)), mk(w2, h2, sway.map(|s| s.1))]
}

fn weather(id: Weather, k: &K, d: u8) -> WBuilt {
  use Weather::*;
  let c = k.tint();
  let d3 = (d + 1).min(3);
  let plain = |layers: Vec<LayerDef>, far: Option<(Tile, i32)>| WBuilt {
    layers,
    far,
    behind: false,
    alternate: false,
    flash: false,
  };
  // the second copy of a tile at `mul` scale, from `p2`, to `to2`
  let two = |t: &Tile, mul: f32, to1: String, p2: &str, to2: &dyn Fn(i32, i32) -> String| {
    let (w2, h2) = ((t.1 as f32 * mul).round() as i32, (t.2 as f32 * mul).round() as i32);
    vec![
      l(Img::S(t.0.clone()), "repeat", &format!("{}px {}px", t.1, t.2), "0 0").to(&to1),
      l(Img::S(t.0.clone()), "repeat", &format!("{w2}px {h2}px"), p2).to(&to2(w2, h2)),
    ]
  };
  match id {
    Rain => plain(fall(&rain_tile(&c, d), 1, None), Some((rain_tile(&c, d3), 1))),
    Snow => plain(fall(&snow_tile(&c, d), 1, Some((2, -3))), Some((snow_tile(&c, d3), 1))),
    Fog => {
      let a = match d {
        3.. => ["80", "4d"],
        2 => ["66", "38"],
        _ => ["40", "26"],
      };
      WBuilt {
        layers: vec![
          l(
            g(format!("radial-gradient(55% 130% at 50% 60%,{c}{} 0%,{c}{} 45%,transparent 72%)", a[0], a[1])),
            "no-repeat",
            "160% 100%",
            "-60% 40%",
          )
          .to("160% 40%"),
          l(
            g(format!("radial-gradient(65% 150% at 50% 40%,{c}{} 0%,transparent 70%)", a[1])),
            "no-repeat",
            "200% 100%",
            "160% 70%",
          )
          .to("-60% 70%"),
        ],
        far: None,
        behind: true,
        alternate: true,
        flash: false,
      }
    }
    Embers => {
      let hot = k.k(k.l + 0.2);
      plain(fall(&ember_tile(&c, &hot, d), -1, Some((2, -2))), Some((ember_tile(&c, &hot, d3), -1)))
    }
    Glyphs => {
      let t = glyph_tile(&c, d);
      let layers = two(&t, 1.5, format!("0 {}px", t.2), "7px 0", &|_, h2| format!("7px {h2}px"));
      plain(layers, Some((glyph_tile(&c, d3), 1)))
    }
    Storm => WBuilt {
      layers: std::iter::once(l(Img::Flash(0xe8f4ff), "no-repeat", "100% 100%", "0 0"))
        .chain(fall(&rain_tile(&c, d), 1, None))
        .collect(),
      far: Some((rain_tile(&c, d3), 1)),
      behind: false,
      alternate: false,
      flash: true,
    },
    Petals => plain(fall(&petal_tile(&c, d), 1, Some((4, -5))), Some((petal_tile(&c, d3), 1))),
    Bubbles => plain(fall(&bubble_tile(&c, d), -1, Some((2, -3))), Some((bubble_tile(&c, d3), -1))),
    Fireflies => plain(fall(&firefly_tile(&c, d), -1, Some((5, -6))), Some((firefly_tile(&c, d3), -1))),
    Leaves => plain(fall(&leaf_tile(&c, d), 1, Some((5, -6))), Some((leaf_tile(&c, d3), 1))),
    Sparks => {
      let t = spark_tile(&c, d);
      let layers = two(&t, 1.4, format!("0 -{}px", t.2), "3px 0", &|_, h2| format!("3px -{h2}px"));
      plain(layers, Some((spark_tile(&c, d3), -1)))
    }
    Dust => {
      let t = dust_tile(&c, d);
      let (w, w2) = (t.1 as i32, (t.1 as f32 * 1.4).round() as i32);
      let layers = two(&t, 1.4, format!("-{w}px 0"), "0 0", &|w2, _| format!("-{w2}px 0"));
      let layers = layers
        .into_iter()
        .zip([(w, 1), (w2, -1)])
        .map(|(d, (ww, y))| d.mid(&format!("-{}px {y}px", (ww as f32 / 2.0).round() as i32)))
        .collect();
      plain(layers, None)
    }
    Confetti => {
      let cs = [c.clone(), k.kh(k.l, 1.0, 120.0), k.kh(k.l, 1.0, 240.0)];
      let refs = [cs[0].as_str(), cs[1].as_str(), cs[2].as_str()];
      plain(fall(&confetti_tile(refs, d), 1, Some((4, -5))), Some((confetti_tile(refs, d3), 1)))
    }
    Meteors => {
      let t = meteor_tile(&c, d);
      let w = t.1 as i32;
      let layers = two(&t, 1.4, format!("-{w}px {}px", t.2), "9px 0", &|w2, h2| format!("{}px {h2}px", 9 - w2));
      plain(layers, None)
    }
    Hearts => plain(fall(&heart_tile(&c, d), -1, Some((3, -4))), Some((heart_tile(&c, d3), -1))),
  }
}

// ---- assembly ---------------------------------------------------------------------------------

// the ext rounds periods to the millisecond and floors brightness-changing ones at 1s
fn period(base: f32, speed: f32, lum: bool) -> f32 {
  let s = base / speed;
  ((if lum { s.max(1.0) } else { s }) * 1000.0).round() / 1000.0
}

pub fn build(spec: &Scene, w: usize, h: usize) -> SceneArt {
  let s = (h as f32 / 22.0).clamp(0.5, 6.0);
  let mut art = SceneArt { back: None, behind: None, front: None };
  let wb = spec.weather.map(|ws| {
    let (base, lum) = weather_period(ws.id);
    (weather(ws.id, &K::new(ws.tint), ws.density), period(base, ws.speed, lum), ws, lum)
  });
  let wperiod = wb.as_ref().map_or(0.0, |x| x.1);
  let mut fg = None;
  if let Some(bs) = spec.backdrop {
    let (base, lum) = backdrop_period(bs.id);
    let bp = period(base, bs.speed, lum);
    let b = backdrop(bs.id, &K::new(bs.tint));
    let mut defs = b.layers;
    if let Some((wbuilt, _, _, _)) = &wb
      && !wbuilt.behind
      && let Some(((svg, tw, th), dir)) = &wbuilt.far
    {
      let cycles = (bp / wperiod.max(0.2)).round().max(1.0) as i32;
      let (fw, fh) = ((*tw as f32 * 0.7).round() as i32, (*th as f32 * 0.7).round() as i32);
      let travel = if b.alternate { "0 0".to_string() } else { format!("0 {}px", dir * fh * cycles) };
      defs.insert(0, l(Img::S(svg.clone()), "repeat", &format!("{fw}px {fh}px"), "0 0").to(&travel));
    }
    let mut band = Band::new(&defs, w, h, s, bp, b.alternate, lum);
    band.set_rest(b.rest_k);
    art.back = Some(band);
    fg = b.fg;
  }
  let behind_text = wb.as_ref().is_some_and(|x| x.0.behind);
  let mut front: Vec<LayerDef> = Vec::new();
  if !behind_text && let Some(f) = fg {
    front.push(f);
  }
  if let Some((wbuilt, wp, ws, _)) = wb {
    // storm: the rain loops on the rain's clock, the lightning on its own
    let pos_period = if wbuilt.flash { period(weather_period(Weather::Rain).0, ws.speed, false) } else { wp };
    front.extend(wbuilt.layers);
    let band = Band::new(&front, w, h, s, pos_period, wbuilt.alternate, false).with_flash_period(wp);
    if wbuilt.behind {
      art.behind = Some(band);
    } else {
      art.front = Some(band);
    }
  } else if !front.is_empty() {
    art.front = Some(Band::new(&front, w, h, s, 1.0, false, false));
  }
  art
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::cosmetics::fx::{BackdropSpec, WeatherSpec};

  #[test]
  fn tint_ladder_matches_the_ext() {
    // green #0ca43d: hue 138 deg, hsl lightness ladder keeps the hue
    let k = K::new(0x0ca43d);
    assert_eq!(k.k(0.5).len(), 7);
    assert_eq!(K::new(0xffffff).k(0.5), "#808080");
    assert_eq!(K::new(0xff0000).k(0.5), "#ff0000");
    assert_eq!(K::new(0xff0000).kh(0.5, 1.0, 120.0), "#00ff00");
  }

  #[test]
  fn every_backdrop_and_weather_builds_and_paints() {
    for &b in Backdrop::ALL {
      for &(w, h) in &[(60usize, 22usize), (130, 30)] {
        let spec = Scene { backdrop: Some(BackdropSpec { id: b, tint: 0x4080c0, speed: 1.0 }), weather: None };
        let art = SceneArt::new(&spec, w, h);
        let mut buf = vec![[0u8; 4]; w * h];
        art.paint_under(Some(1.234), &mut buf);
        art.paint_over(Some(1.234), &mut buf);
        assert!(buf.iter().any(|p| p[3] == 255), "{b:?} paints an opaque plate");
      }
    }
    for &wd in Weather::ALL {
      for d in 1..=3u8 {
        let spec = Scene {
          backdrop: Some(BackdropSpec { id: Backdrop::Terminal, tint: 0x40c080, speed: 1.0 }),
          weather: Some(WeatherSpec { id: wd, tint: 0xffffff, density: d, speed: 1.0 }),
        };
        let art = SceneArt::new(&spec, 70, 22);
        let (mut a, mut b) = (vec![[0u8; 4]; 70 * 22], vec![[0u8; 4]; 70 * 22]);
        art.paint_under(Some(0.3), &mut a);
        art.paint_over(Some(0.3), &mut a);
        art.paint_under(None, &mut b);
        art.paint_over(None, &mut b);
        assert!(art.animated(), "{wd:?}");
        assert!(a.iter().all(|p| p[3] > 0));
        // the resting frame differs from a mid-loop one for anything that moves
        assert_ne!(a, b, "{wd:?} d{d} moves");
      }
    }
  }

  #[test]
  fn frames_are_deterministic_and_phase_locked() {
    let spec = Scene {
      backdrop: Some(BackdropSpec { id: Backdrop::Synth, tint: 0x0ca43d, speed: 1.0 }),
      weather: Some(WeatherSpec { id: Weather::Petals, tint: 0xd7408c, density: 1, speed: 1.0 }),
    };
    let art = SceneArt::new(&spec, 64, 22);
    let frame = |t: f32| {
      let mut b = vec![[0u8; 4]; 64 * 22];
      art.paint_under(Some(t), &mut b);
      art.paint_over(Some(t), &mut b);
      b.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, p| {
        p.iter().fold(h, |h, &c| (h ^ u64::from(c)).wrapping_mul(0x100_0000_01b3))
      })
    };
    assert_eq!(frame(2.5), frame(2.5));
    assert_ne!(frame(2.5), frame(2.6));
    // one backdrop loop later (synth loops in 1.4s, petals in 3.8s: lcm 53.2s) the picture is back
    assert_eq!(frame(1.0), frame(1.0 + 53.2));
  }

  #[test]
  fn synth_rests_with_a_half_sun_on_the_horizon() {
    let spec =
      Scene { backdrop: Some(BackdropSpec { id: Backdrop::Synth, tint: 0xff40af, speed: 1.0 }), weather: None };
    let art = SceneArt::new(&spec, 60, 22);
    let mut b = vec![[0u8; 4]; 60 * 22];
    art.paint_under(None, &mut b);
    // the tint itself is the disc, centred on the sky box's bottom edge (54% of 22 = 12)
    let sun = b[11 * 60 + 30];
    assert!(sun[0] > 200 && sun[1] < 110, "{sun:?}");
  }
}
