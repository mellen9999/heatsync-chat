// the one-look sweep: the desktop's screens drawn headless, to compare with the site's chat tile.
// HSC_PNG=/tmp/claude-1000/onelook/desk cargo test -j2 --release onelook_shots writes <prefix>-main.png ...
use super::*;
use crate::model::message::{Kind, Message, Notice, flag};

pub(super) fn png(c: &crate::ui::canvas::Canvas, path: &str) {
  let mut raw = Vec::with_capacity(c.px.len() * 3);
  c.px.iter().for_each(|p| raw.extend([(p >> 16) as u8, (p >> 8) as u8, *p as u8]));
  let mut enc =
    png::Encoder::new(std::io::BufWriter::new(std::fs::File::create(path).unwrap()), c.w as u32, c.h as u32);
  enc.set_color(png::ColorType::Rgb);
  enc.set_depth(png::BitDepth::Eight);
  enc.write_header().unwrap().write_image_data(&raw).unwrap();
}

fn said(plat: Platform, id: &str, nick: &str, color: u32, body: &str) -> Message {
  let mut m = Message::system(now_ms(), plat, body);
  m.kind = Kind::Chat;
  m.id = id.into();
  m.nick = nick.into();
  m.login = nick.to_lowercase().into();
  m.color = color;
  m
}

pub(super) fn scene() -> App {
  let (net, _asked) = Net::tap();
  let mut app = App::new(net, vec!["xqc".into(), "kaicenat".into(), "forsen".into()], false);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.canvas.resize(900, 640);
  app.relayout_body();
  let t = |app: &App, n: &str| app.tabs.iter().position(|t| &*t.name == n).unwrap();
  let (xqc, kai) = (t(&app, "xqc"), t(&app, "kaicenat"));
  app.select(kai);
  let tw = Platform::Twitch;
  app.on_chat("kaicenat", Chat::Msg(said(tw, "1", "averycool", 0xffff00, "Kai")));
  app.on_chat("kaicenat", Chat::Msg(said(tw, "2", "idk_82o1", 0xff00ff, "!followage")));
  let mut r = said(tw, "3", "Fossabot", 0x8888ff, "@idk_82o1 has been following for 2 years");
  r.reply_to = Some("idk_82o1".into());
  r.reply_text = Some("!followage".into());
  app.on_chat("kaicenat", Chat::Msg(r));
  app.on_chat("kaicenat", Chat::Msg(said(tw, "4", "averycool", 0xffff00, "Meeeeee")));
  let mut sub = Message::system(now_ms(), tw, "★ tico_xo subscribed at Tier 1. They've subscribed for 19 months!");
  sub.extra().notice = Notice::Sub;
  app.on_chat("kaicenat", Chat::Msg(sub));
  let mut first = said(tw, "5", "newbie", 0x00ff00, "hi first time here");
  first.flags |= flag::FIRST;
  app.on_chat("kaicenat", Chat::Msg(first));
  let mut me = said(tw, "6", "bob", 0x00ffff, "@mellen look at this");
  me.flags |= flag::ALERT;
  app.on_chat("kaicenat", Chat::Msg(me));
  app.on_chat(
    "kaicenat",
    Chat::Msg(said(Platform::Kick, "7", "bmac2525", 0x00ff00, "prob a break. until a new contract")),
  );
  app.on_chat("kaicenat", Chat::Msg(said(tw, "8", "sun_tres", 0xff8700, "yo chat")));
  app.on_chat("xqc", Chat::Msg(said(tw, "9", "x", 0xff0000, "unread in xqc")));
  let _ = xqc;
  for t in &mut app.tabs {
    t.view.resize(900);
  }
  app.redraw();
  app
}

fn shot(app: &App, name: &str) {
  if let Some(p) = std::env::var_os("HSC_PNG") {
    png(&app.canvas, &format!("{}-{name}.png", p.to_string_lossy()));
  }
}

#[test]
fn onelook_shots() {
  let mut app = scene();
  shot(&app, "main");
  // the chat is bottom-anchored: a line near the bottom of the chat area
  let chat = app.chat_area();
  app.pointer = (chat.x + 100, chat.bottom() - 70);
  app.right_click();
  app.redraw();
  shot(&app, "rowmenu");
  app.close_menu();
  app.redraw();
  let r = app.bar();
  app.pointer = (r.x + 60, r.y + 5);
  app.right_click();
  app.redraw();
  shot(&app, "tabmenu");
  app.close_tab_menu();
  app.find_open();
  app.redraw();
  shot(&app, "find");
}
