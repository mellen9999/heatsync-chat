// one row of tabs: active reversed (white bg, black text), unread bold in its call color, each ended
// by a 1px white line.
// a `+` after the last tab adds one; hover reverses it like the active tab
use super::canvas::{Canvas, Rect};
use super::mode_label::{self, Seg};
use super::text::{Ink, Span, Text};
use super::theme;

pub struct TabLabel<'a> {
  pub name: &'a str,
  // the counter after the name (ext tab-counter), empty for none
  pub count: &'a str,
  pub active: bool,
  pub unread: bool,
  // the color of an unread tab: white for chat, yellow after a stream line, red for mentions, cyan
  // for whispers (the ext's)
  pub hot: u32,
  // tab mode's cursor is on it: underlined
  pub cursor: bool,
  // its channel is live: a red dot at its right end, centered on the line (ext data-live)
  pub live: bool,
  // the one unsaved tab: its name is underlined
  pub temp: bool,
  // its pings are muted: a ⊘ after the name
  pub muted: bool,
  // a platform is hidden on it (merged tabs): a ◐ after the name
  pub filtered: bool,
}

// the buttons after the tabs: which is under the pointer (or open), and a newer version's label
pub struct Hot<'a> {
  // the tab under the pointer (its position in `tabs`): reversed like the active one
  pub tab: Option<usize>,
  pub plus: bool,
  pub gear: bool,
  pub keys: bool,
  pub update: Option<(&'a str, bool)>,
  // streamer mode's marker, after the buttons (not a button)
  pub mark: Option<&'a str>,
}

// the height of a bar of `rows` lines of tabs: each line and the 1px white line toward the chat or
// between the lines (the site's tabbar wraps like flex-wrap, its tabs share borders)
pub fn height(line: i32, rows: i32) -> i32 {
  rows.max(1) * (line + 1)
}

// where everything sits, worked out once: `rows` measures with it and `draw` paints from it, so the room
// the layout keeps for the bar and the bar itself can't drift
struct Plan {
  // each tab's rect (a live tab's includes its dot), with the label drawn in it
  tabs: Vec<(Rect, String)>,
  btn: [Rect; 3],
  update: Option<(Rect, String)>,
  mark: Option<(Rect, String)>,
  // lines used (a column is one)
  rows: i32,
  // after the last thing: where the mode line starts
  end: (i32, i32),
}

fn plan(
  text: &mut Text,
  inner: Rect,
  column: bool,
  tabs: &[TabLabel],
  update: Option<&str>,
  mark: Option<&str>,
) -> Plan {
  let line = text.line_px as i32;
  let step = line + 1;
  let d = (line * 2 / 5).max(4);
  let (mut x, mut y) = (inner.x, inner.y);
  // a row: the next thing starts a new line when it would pass the right edge (the first on a line is
  // clipped, never wrapped again)
  let fit = |x: &mut i32, y: &mut i32, w: i32| {
    if !column && *x > inner.x && *x + w > inner.right() {
      (*x, *y) = (inner.x, *y + step);
    }
  };
  let width = |text: &mut Text, label: &str, bold: bool| {
    text.layout(label, &[Span { range: 0..label.len(), ink: Ink(theme::WHITE), bold }], &[], f32::MAX).width().ceil()
      as i32
  };
  let mut out = Plan {
    tabs: Vec::with_capacity(tabs.len()),
    btn: [Rect::new(0, 0, 0, 0); 3],
    update: None,
    mark: None,
    rows: 1,
    end: (x, y),
  };
  for t in tabs {
    let count = if t.count.is_empty() { String::new() } else { format!(" {}", t.count) };
    let count = if t.muted { format!("{count} ⊘") } else { count };
    let count = if t.filtered { format!("{count} ◐") } else { count };
    let label = if column { format!(" {}{count}", t.name) } else { format!(" {}{count} ", t.name) };
    let w = width(text, &label, t.unread || t.active) + if t.live && !column { d + 2 } else { 0 };
    // the 1px line after it goes with it
    fit(&mut x, &mut y, w + 1);
    let r = Rect::new(x, y, if column { inner.w } else { w }, line);
    out.tabs.push((r, label));
    if column {
      y += line;
    } else {
      x += w + 1;
    }
  }
  let bw = width(text, " + ", false);
  for k in 0..3 {
    fit(&mut x, &mut y, bw + 1);
    out.btn[k] = Rect::new(x, y, bw, line);
    x += if column { bw } else { bw + 1 };
  }
  if column {
    (x, y) = (inner.x, y + line);
  }
  for (slot, what) in [(&mut out.update, update), (&mut out.mark, mark)] {
    let Some(w) = what else { continue };
    let label = format!(" {w} ");
    let lw = width(text, &label, true);
    fit(&mut x, &mut y, lw);
    *slot = Some((Rect::new(x, y, if column { inner.w } else { lw }, line), label));
    if column {
      y += line;
    } else {
      x += lw;
    }
  }
  out.rows = if column { 1 } else { (y - inner.y) / step + 1 };
  out.end = (x, y);
  out
}

// how many lines of tabs a bar `width` wide needs: what `draw` will wrap to
pub fn rows(text: &mut Text, width: i32, tabs: &[TabLabel], update: Option<&str>, mark: Option<&str>) -> i32 {
  plan(text, Rect::new(0, 0, width, text.line_px as i32), false, tabs, update, mark).rows
}

// returns each tab's rect, then the `+`, the gear and the `?` (for clicks). hot: pointer on a button, or its screen open.
// a row wraps onto a new line when the next tab would pass the edge (`rows` says how many); `column` is
// tabs on the left or right (ext tabPosition): a tab a row, full width, a 1px white edge toward the chat
pub fn draw(
  c: &mut Canvas,
  text: &mut Text,
  area: Rect,
  column: bool,
  tabs: &[TabLabel],
  // the vi mode line when auto-hide folded the input bar away
  mode: (&[Seg], &str),
  hot: Hot,
) -> Vec<Rect> {
  c.fill(area, theme::BLACK);
  let line = text.line_px as i32;
  let step = line + 1;
  let top = area.y == 0;
  let inner = if !column {
    // the site's tabbar border: a 1px white line on the edge toward the chat (a bar at the top of the
    // window has the chat below it), and one between wrapped lines
    let y0 = if top { area.y } else { area.y + 1 };
    Rect::new(area.x, y0, area.w, area.h - 1)
  } else {
    // on the left the edge is its right side, on the right its left side
    let left = area.x == 0;
    c.fill(Rect::new(if left { area.right() - 1 } else { area.x }, area.y, 1, area.h), theme::WHITE);
    Rect::new(if left { area.x } else { area.x + 1 }, area.y, area.w - 1, area.h)
  };
  let p = plan(text, inner, column, tabs, hot.update.map(|u| u.0), hot.mark);
  if !column {
    for k in 0..area.h / step {
      let y = if top { inner.y + k * step + line } else { inner.y + k * step - 1 };
      c.fill(Rect::new(area.x, y, area.w, 1), theme::WHITE);
    }
  }
  let mut hits = Vec::with_capacity(tabs.len() + 3);
  // the live dot: the ext's 8px at a 20px line, kept clear of the label by its own width
  let d = (line * 2 / 5).max(4);
  let cell = |c: &mut Canvas, text: &mut Text, r: Rect, label: &str, ink: u32, bold: bool, on: bool| {
    let l = text.layout(
      label,
      &[Span { range: 0..label.len(), ink: Ink(if on { theme::BLACK } else { ink }), bold }],
      &[],
      f32::MAX,
    );
    if on {
      c.fill(r.intersect(inner), theme::WHITE);
    }
    text.draw(&l, c, r.x as f32, r.y as f32, inner, |_, _| {});
  };
  for (i, (t, (r, label))) in tabs.iter().zip(&p.tabs).enumerate() {
    let (r, rev) = (*r, t.active || hot.tab == Some(i));
    let ink = if t.unread { t.hot } else { theme::DIM };
    cell(c, text, r, label, ink, t.unread || t.active, rev);
    if t.live {
      dot(c, Rect::new(r.right() - d - 2, r.y + (r.h - d) / 2, d, d).intersect(inner), d, theme::RED);
    }
    if t.temp && !t.cursor {
      // under the name, not the padding
      let pad = text.layout(" ", &[], &[], f32::MAX).width().ceil() as i32;
      let u =
        Rect::new(r.x + pad, r.bottom() - 2, (r.w - 2 * pad - if t.live && !column { d + 2 } else { 0 }).max(1), 1);
      c.fill(u.intersect(inner), if rev { theme::BLACK } else { theme::WHITE });
    }
    if t.cursor {
      let u = Rect::new(r.x + 2, r.bottom() - 2, (r.w - 4).max(1), 1);
      c.fill(u.intersect(inner), if rev { theme::BLACK } else { theme::WHITE });
    }
    hits.push(r);
    // a row of tabs is split by a 1px white line after each one (the site's and the ext's bordered
    // tabs): "quin69 999+ ●" ran into the next name. a column puts each tab on its own row already
    if !column {
      c.fill(Rect::new(r.right(), r.y, 1, line).intersect(inner), theme::WHITE);
    }
  }
  // the three buttons: same width, each glyph centered on its own ink (the gear comes from a fallback
  // font that sits lower than the `+`). in a column they share one row
  for (k, (glyph, on)) in [("+", hot.plus), ("⚙", hot.gear), ("?", hot.keys)].into_iter().enumerate() {
    let r = p.btn[k];
    icon(c, text, r.intersect(inner), glyph, on);
    hits.push(r);
    // boxed like the site's `+` `cfg` buttons: a 1px white line after each, none in a column
    if !column {
      c.fill(Rect::new(r.right(), r.y, 1, line).intersect(inner), theme::WHITE);
    }
  }
  if let (Some((r, label)), Some((_, on))) = (&p.update, hot.update) {
    cell(c, text, *r, label, theme::YELLOW, true, on);
    hits.push(*r);
  }
  if let Some((r, label)) = &p.mark {
    cell(c, text, *r, label, theme::GREEN, true, false);
  }
  let (mut x, mut y) = p.end;
  // the mode line: after the buttons on a row, wrapped under them in a column
  // a column indents every wrapped line by one cell, like the tab names
  let indent = if column { text.layout(" ", &[], &[], f32::MAX).width().ceil() as i32 } else { 0 };
  let wrap = if column { (inner.w - 2 * indent) as f32 } else { f32::MAX };
  let lead = if column { "" } else { "  " };
  let mut put = |text: &mut Text, s: &str, spans: &[Span], x: &mut i32, y: &mut i32| {
    let l = text.layout(s, spans, &[], wrap);
    text.draw(&l, c, (*x + indent) as f32, *y as f32, inner, |_, _| {});
    if column {
      *y += l.height().ceil() as i32;
    } else {
      *x += l.width().ceil() as i32;
    }
  };
  if !mode.0.is_empty() {
    let s = format!("{lead}{}{}{}", mode_label::text(mode.0), if mode.1.is_empty() { "" } else { "  " }, mode.1);
    let mut spans = mode_label::spans(mode.0);
    for sp in &mut spans {
      sp.range = sp.range.start + lead.len()..sp.range.end + lead.len();
    }
    put(text, &s, &spans, &mut x, &mut y);
  }
  hits
}

// a filled round status dot of diameter `d` at `r` (the one round shape the design allows)
fn dot(c: &mut Canvas, r: Rect, d: i32, ink: u32) {
  let rad = d as f32 / 2.0;
  for y in 0..r.h.min(d) {
    for x in 0..r.w.min(d) {
      let (dx, dy) = (x as f32 + 0.5 - rad, y as f32 + 0.5 - rad);
      if dx * dx + dy * dy <= rad * rad {
        c.fill(Rect::new(r.x + x, r.y + y, 1, 1), ink);
      }
    }
  }
}

// one glyph bold in the middle of `r` by its drawn pixels, not its font's line box. hover reverses it
fn icon(c: &mut Canvas, text: &mut Text, r: Rect, glyph: &str, on: bool) {
  let (bg, ink) = if on { (theme::WHITE, theme::BLACK) } else { (theme::BLACK, theme::WHITE) };
  let l = text.layout(glyph, &[Span { range: 0..glyph.len(), ink: Ink(ink), bold: true }], &[], f32::MAX);
  let mut s = Canvas::new(r.w.max(l.width().ceil() as i32), r.h);
  s.fill(s.bounds(), bg);
  let sb = s.bounds();
  text.draw(&l, &mut s, 0.0, 0.0, sb, |_, _| {});
  c.fill(r, bg);
  // a pixel counts once it is a quarter of the way to the ink: faint antialiasing doesn't pull the middle
  let ink_at = |x: i32, y: i32| {
    let p = s.px[(y * s.w + x) as usize];
    [16, 8, 0].iter().any(|sh| ((p >> sh) & 0xff).abs_diff((bg >> sh) & 0xff) > 0x40)
  };
  let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, -1, -1);
  for y in 0..s.h {
    for x in 0..s.w {
      if ink_at(x, y) {
        (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
      }
    }
  }
  if x1 < 0 {
    return;
  }
  let (dx, dy) = (r.x + (r.w - (x1 - x0 + 1)) / 2 - x0, r.y + (r.h - (y1 - y0 + 1)) / 2 - y0);
  for y in y0..=y1 {
    for x in x0..=x1 {
      let p = s.px[(y * s.w + x) as usize];
      if r.contains(x + dx, y + dy) && c.bounds().contains(x + dx, y + dy) {
        c.px[((y + dy) * c.w + x + dx) as usize] = p;
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn tab(name: &'static str, count: &'static str, live: bool) -> TabLabel<'static> {
    TabLabel {
      name,
      count,
      active: false,
      unread: false,
      hot: theme::WHITE,
      cursor: false,
      live,
      temp: false,
      muted: false,
      filtered: false,
    }
  }

  // "quin69 999+ ●" ran into the next tab's name: every tab in a row ends on a 1px white line,
  // like the site's and the ext's bordered tabs, and a click on the line belongs to no tab
  #[test]
  fn row_tabs_are_split_by_a_one_pixel_white_line() {
    let mut text = Text::new(1.0);
    let line = text.line_px as i32;
    let (w, h) = (600, height(line, 1));
    let mut c = Canvas::new(w, h);
    let tabs = [tab("quin69", "999+", true), tab("forsen", "", false), tab("xqc", "12", false)];
    let none = Hot { tab: None, plus: false, gear: false, keys: false, update: None, mark: None };
    let hits = draw(&mut c, &mut text, Rect::new(0, 0, w, h), false, &tabs, (&[], ""), none);
    for r in &hits[..tabs.len()] {
      let x = r.right();
      assert!((0..line).all(|y| c.px[(y * w + x) as usize] == theme::WHITE), "a white line at x={x} after the tab");
      assert!((0..line).all(|y| c.px[(y * w + x - 1) as usize] != theme::WHITE || r.w == 0), "the line is 1px");
    }
    for p in hits[..tabs.len()].windows(2) {
      assert_eq!(p[1].x, p[0].right() + 1, "the next tab starts after the line, so the line is no tab's");
    }
  }

  // a column already puts each tab on its own row: no line between them
  #[test]
  fn column_tabs_draw_no_split_line() {
    let mut text = Text::new(1.0);
    let line = text.line_px as i32;
    let (w, h) = (200, line * 6);
    let mut c = Canvas::new(w, h);
    let tabs = [tab("quin69", "999+", true), tab("forsen", "", false)];
    let none = Hot { tab: None, plus: false, gear: false, keys: false, update: None, mark: None };
    let hits = draw(&mut c, &mut text, Rect::new(0, 0, w, h), true, &tabs, (&[], ""), none);
    assert_eq!(hits[1].y, hits[0].bottom());
  }

  // the site's tabbar: a 1px white line under the tabs (toward the chat), and `+` `cfg` boxed like tabs
  #[test]
  fn a_row_bar_has_its_edge_line_and_boxed_buttons() {
    let mut text = Text::new(1.0);
    let line = text.line_px as i32;
    let (w, h) = (600, height(line, 1));
    let mut c = Canvas::new(w, h);
    let tabs = [tab("forsen", "", false)];
    let none = Hot { tab: None, plus: false, gear: false, keys: false, update: None, mark: None };
    let hits = draw(&mut c, &mut text, Rect::new(0, 0, w, h), false, &tabs, (&[], ""), none);
    assert!((0..w).all(|x| c.px[((h - 1) * w + x) as usize] == theme::WHITE), "the edge line under the tabs");
    assert!((0..w).any(|x| c.px[((h - 2) * w + x) as usize] != theme::WHITE), "the tabs sit above it");
    for r in &hits[1..4] {
      let x = r.right();
      assert!((0..line).all(|y| c.px[(y * w + x) as usize] == theme::WHITE), "a line after the button at x={x}");
    }
    // at the bottom of the window the edge is the top of the bar
    let mut c = Canvas::new(w, h + 10);
    let none = Hot { tab: None, plus: false, gear: false, keys: false, update: None, mark: None };
    draw(&mut c, &mut text, Rect::new(0, 10, w, h), false, &tabs, (&[], ""), none);
    assert!((0..w).all(|x| c.px[(10 * w + x) as usize] == theme::WHITE), "the edge line over the tabs");
  }

  fn none() -> Hot<'static> {
    Hot { tab: None, plus: false, gear: false, keys: false, update: None, mark: None }
  }

  // tabs that don't fit one line wrap onto the next (the site's flex-wrap); every rect stays in the bar
  #[test]
  fn tabs_that_do_not_fit_wrap_to_a_second_line() {
    let mut text = Text::new(1.0);
    let line = text.line_px as i32;
    let tabs = [tab("quin69", "999+", true), tab("forsen", "", false), tab("xqc", "12", false), tab("nymn", "", false)];
    let w = 300;
    let n = rows(&mut text, w, &tabs, None, None);
    assert_eq!(n, 2, "wraps to two lines");
    let h = height(line, n);
    let mut c = Canvas::new(w, h);
    let hits = draw(&mut c, &mut text, Rect::new(0, 0, w, h), false, &tabs, (&[], ""), none());
    let area = Rect::new(0, 0, w, h);
    assert!(hits.iter().all(|r| r.x >= 0 && r.right() <= w && area.intersect(*r) == *r), "every hit inside the bar");
    assert!(hits.iter().any(|r| r.y == 0) && hits.iter().any(|r| r.y == line + 1), "tabs on both lines");
    assert!(hits[..4].windows(2).all(|p| p[1].y == p[0].y && p[1].x > p[0].x || p[1].y > p[0].y && p[1].x == 0));
    // a white line closes each line of tabs
    assert!((0..w).all(|x| c.px[(line * w + x) as usize] == theme::WHITE), "line between");
    assert!((0..w).all(|x| c.px[((h - 1) * w + x) as usize] == theme::WHITE), "edge line");
  }

  // one tab keeps a one-line bar, and a tab wider than the bar sits alone on its line, clipped
  #[test]
  fn a_single_tab_keeps_one_line_and_a_huge_one_is_clipped() {
    let mut text = Text::new(1.0);
    assert_eq!(rows(&mut text, 600, &[tab("forsen", "", false)], None, None), 1);
    let big = tab("an_enormous_channel_name_that_will_never_fit_in_the_bar", "", false);
    assert_eq!(rows(&mut text, 120, &[big], None, None), 2, "the buttons wrap under it");
  }
}
