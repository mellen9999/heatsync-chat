// a youtube still in chat: the pointer moving over it for many frames costs nothing (no fetch, no player),
// a click asks for one player, and no link shape breaks the row
use super::*;
use crate::images::decode::{Decoded, Frame};
use crate::model::message::{Kind, Message, Platform};
use crate::net::Cmd;

fn boot(body: &str) -> (App, tokio::sync::mpsc::UnboundedReceiver<Cmd>) {
  let (net, mut asked) = Net::tap();
  let mut app = App::new(net, vec!["forsen".into()], false);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.canvas.resize(640, 400);
  app.relayout_body();
  let chat = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
  app.select(chat);
  let mut m = Message::system(0, Platform::Twitch, body);
  m.kind = Kind::Chat;
  m.nick = "bob".into();
  m.login = "bob".into();
  app.on_chat("forsen", Chat::Msg(m));
  for _ in 0..3 {
    app.redraw();
    let wanted: Vec<_> = std::iter::from_fn(|| asked.try_recv().ok())
      .filter_map(|c| if let Cmd::Image(id, _, target) = c { Some((id, target)) } else { None })
      .collect();
    for (id, target) in wanted {
      let rgba = [0, 0, 255, 255].repeat(320 * 180).into();
      app.res.images.loaded(id, target, Ok(Decoded { w: 320, h: 180, frames: vec![Frame { rgba, delay_ms: 0 }] }));
    }
    for t in &mut app.tabs {
      t.view.relayout();
    }
  }
  app.redraw();
  (app, asked)
}

fn still(app: &App) -> Option<(i32, i32)> {
  let (c, a) = (&app.canvas, app.chat_area());
  (a.y..a.bottom()).step_by(4).find_map(|y| {
    (a.x..a.right()).step_by(4).find(|&x| c.px[(y * c.w + x) as usize] == 0x0000ff).map(|x| (x + 8, y + 8))
  })
}

#[test]
fn hovering_a_still_for_many_frames_asks_for_nothing() {
  let (mut app, mut asked) = boot("watch https://youtu.be/dQw4w9WgXcQ?t=5");
  let at = still(&app).expect("the still is on screen");
  while asked.try_recv().is_ok() {}
  for i in 0..120 {
    app.pointer = (at.0 + i % 40, at.1 + i % 30);
    app.cursor_sync();
    app.tip_hover();
    app.chat_hover();
    app.redraw();
  }
  let sent: Vec<_> = std::iter::from_fn(|| asked.try_recv().ok()).collect();
  assert!(sent.is_empty(), "120 frames of hover sent {} commands", sent.len());
}

#[test]
fn every_odd_link_shape_lays_out_and_draws() {
  let long = format!("https://youtu.be/{}", "é".repeat(400));
  for body in [
    "https://youtu.be/".to_owned(),
    "https://youtu.be/dQw4w9WgXc".into(),
    "https://youtu.be/dQw4w9WgXcQé".into(),
    "https://youtu.be/dQw4w9WgXcQ?t=1h2m3s&list=PLx".into(),
    "https://www.youtube.com/shorts/dQw4w9WgXcQ".into(),
    "https://www.youtube.com/live/dQw4w9WgXcQ?feature=share".into(),
    "https://www.youtube.com/playlist?list=PLxxxxxxxxxxx".into(),
    "https://www.youtube.com/watch?v=".into(),
    "https://www.youtube.com/watch?v=dQw4w9WgXcQ#t=é".into(),
    "https://www.youtube.com/@canal-ñ/live".into(),
    "https://m.youtube.com/watch?v=dQw4w9WgXcQ).".into(),
    "http://é.youtube.com/watch?v=dQw4w9WgXcQ".into(),
    "HTTPS://YOUTU.BE/dQw4w9WgXcQ".into(),
    "https://youtu.be/dQw4w9WgXcQ\u{200b}lol".into(),
    "watch?v=dQw4w9WgXcQ shorts/".into(),
    "shorts/é".into(),
    long,
    "https://".into(),
    "https://é".into(),
  ] {
    let (mut app, _) = boot(&body);
    app.redraw();
    let at = still(&app).unwrap_or((20, 20));
    app.pointer = at;
    app.cursor_sync();
    app.click();
  }
}

// a burst of moves, then the batch's hover pass: what the event loop does between two batches
fn batch(app: &mut App, moves: impl Iterator<Item = (i32, i32)>) {
  for (x, y) in moves {
    app.pointer_to(x, y);
  }
  app.hover_flush();
}

#[test]
fn a_burst_of_moves_over_a_still_lays_the_row_out_a_few_times_not_per_move() {
  let (mut app, _asked) = boot("watch https://youtu.be/dQw4w9WgXcQ?t=5");
  let at = still(&app).expect("the still is on screen");
  let before = crate::ui::chat_view::layouts();
  batch(&mut app, (0..200).map(|i| (at.0 + i % 40, at.1 + i % 30)));
  let n = crate::ui::chat_view::layouts() - before;
  eprintln!("layouts for 200 moves: {n}");
  assert!(n <= 8, "200 moves in one batch laid rows out {n} times");
}

#[test]
fn the_pointer_is_a_hand_over_a_still_and_an_arrow_off_it() {
  let (mut app, _asked) = boot("watch https://youtu.be/dQw4w9WgXcQ?t=5");
  let at = still(&app).expect("the still is on screen");
  batch(&mut app, (0..50).map(|i| (at.0 + i % 8, at.1 + i % 8)));
  assert_eq!(app.cursor, winit::window::CursorIcon::Pointer);
  let a = app.chat_area();
  batch(&mut app, (0..50).map(|i| (a.x + 4, a.bottom() - 2 - i % 3)));
  assert_eq!(app.cursor, winit::window::CursorIcon::Default);
}

#[test]
fn a_press_after_moves_in_one_batch_opens_the_link_under_the_last_move() {
  let (mut app, mut asked) = boot("watch https://youtu.be/dQw4w9WgXcQ?t=5");
  let at = still(&app).expect("the still is on screen");
  let a = app.chat_area();
  while asked.try_recv().is_ok() {}
  app.pointer_to(a.x + 4, a.bottom() - 2);
  app.hover_flush();
  // moves onto the still, then the press in the same batch: the loop flushes before any non-move event
  app.pointer_to(at.0, at.1);
  app.hover_flush();
  app.click();
  let sent: Vec<_> = std::iter::from_fn(|| asked.try_recv().ok()).collect();
  assert!(!sent.is_empty(), "the press on the still asked for nothing");
}
