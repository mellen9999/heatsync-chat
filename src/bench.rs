// headless render gate: flood a chat view, report frame times + rss
use std::time::Instant;

use crate::fake::Fake;
use crate::ui::canvas::{Canvas, Rect};
use crate::ui::chat_view::{ChatView, Paint};
use crate::ui::res::Res;

pub fn rss_kb() -> (u64, u64, u64, u64) {
  let s = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
  let field = |k: &str| {
    s.lines()
      .find(|l| l.starts_with(k))
      .and_then(|l| l.split_whitespace().nth(1))
      .and_then(|v| v.parse().ok())
      .unwrap_or(0)
  };
  (field("VmRSS:"), field("VmHWM:"), field("RssAnon:"), field("RssFile:"))
}

pub fn run(total: usize) {
  // HSC_BENCH_H: a shorter pane shows fewer rows (and so fewer painted names)
  let h = std::env::var("HSC_BENCH_H").ok().and_then(|v| v.parse().ok()).unwrap_or(1000);
  let w = 480;
  let mut c = Canvas::new(w, h);
  let mut res = Res::new(1.0);
  crate::fake::seed_paints(&mut res.cosmetics);
  res.heats.mute();
  res.flairs.mute();
  res.arts.set_motion(std::env::var("HSC_MOTION").as_deref() != Ok("0"));
  let mut chat = ChatView::new(1000, "bench".into());
  let mut fake = Fake::new(0x5eed);
  let area = Rect::new(0, 0, w, h);
  chat.resize(w);

  let t0 = Instant::now();
  let mut frames = 0u32;
  let mut worst = 0f64;
  let mut ts = 0;
  while chat.len() < 1000 || frames < (total / 3) as u32 {
    for _ in 0..3 {
      ts += 1;
      chat.push(fake.message(ts));
    }
    let f = Instant::now();
    match chat.take_paint(&mut res, h) {
      Paint::Nothing => {}
      Paint::Full => chat.draw(&mut c, &mut res, area, area),
      Paint::Append { shift } => {
        c.scroll_up(area, shift);
        chat.draw(&mut c, &mut res, area, Rect::new(0, h - shift, w, shift));
      }
    }
    let dt = f.elapsed().as_secs_f64() * 1e3;
    worst = worst.max(dt);
    frames += 1;
  }
  dump(&c);
  let append_avg = t0.elapsed().as_secs_f64() * 1e3 / frames as f64;

  let f = Instant::now();
  for _ in 0..100 {
    chat.draw(&mut c, &mut res, area, area);
  }
  let full_avg = f.elapsed().as_secs_f64() * 1e3 / 100.0;

  // painted names: what one animation tick costs, and the cpu that is at the 30fps cap
  if crate::fake::painted_count() > 0 {
    chat.draw(&mut c, &mut res, area, area);
    let names = chat.animated_names();
    let f = Instant::now();
    for _ in 0..300 {
      chat.animate(&mut c, &mut res, area);
    }
    let tick = f.elapsed().as_secs_f64() * 1e3 / 300.0;
    println!("animated names on screen {names}  tick {tick:.3}ms  => ~{:.2}% cpu at 30fps", tick * 30.0 / 10.0);
  }

  let (rss, hwm, anon, file) = rss_kb();
  println!(
    "frames {frames} rows {}  append avg {append_avg:.3}ms worst {worst:.3}ms  full {full_avg:.3}ms",
    chat.len()
  );
  let mb = |k: u64| k as f64 / 1024.0;
  println!("rss {:.1}MB (anon {:.1} file {:.1})  peak {:.1}MB", mb(rss), mb(anon), mb(file), mb(hwm));
}

// HSC_DUMP=path.ppm writes the last frame for eyeballing
pub fn dump(c: &Canvas) {
  let Ok(path) = std::env::var("HSC_DUMP") else { return };
  let mut out = format!("P6 {} {} 255\n", c.w, c.h).into_bytes();
  for p in &c.px {
    out.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, *p as u8]);
  }
  if let Err(e) = std::fs::write(&path, out) {
    eprintln!("dump: {e}");
  }
}
