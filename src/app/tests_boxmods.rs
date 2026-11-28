// modifier words in the message box: folded into the emote they bind to and drawn as the change they
// make, by the rule chat lines read the sent text with (ext scanAndApplyModifiersInInput, nl_kripp's
// `Okayge p!`)

use super::*;
use crate::emotes::motion::Motion;
use crate::emotes::{Emote, Provider};
use crate::images::decode::{Decoded, Frame};
use crate::net::Cmd;
use winit::keyboard::{Key, NamedKey};

const OKAYGE: &str = "https://cdn.7tv.app/emote/01F7GJ0N4R00074A83FVHRDMDB/1x.webp";
const P: &str = "https://cdn.betterttv.net/emote/65cbe7dbaed093b2eaf87c65/1x";

// nl_kripp's room: Okayge (7tv) and betterttv's global p! placeholder (an overlay)
fn kripp(w: u32) -> (App, tokio::sync::mpsc::UnboundedReceiver<Cmd>) {
  let (net, asked) = Net::tap();
  let mut app = App::new(net, vec!["nl_kripp".into()], false);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.resized(w as i32, 300);
  let ok = Emote { url: OKAYGE.into(), w: 28, h: 28, zero_width: false };
  app.res.emotes.set(None, Provider::SevenTv, [("Okayge".into(), ok)].into());
  let p = Emote { url: P.into(), w: 28, h: 28, zero_width: true };
  app.res.emotes.set(None, Provider::Bttv, [("p!".into(), p)].into());
  (app, asked)
}

fn space(app: &mut App) {
  app.key_insert(Key::Named(NamedKey::Space), Some(" "));
}

// the base picked with tab, as a person does: a picture in the box
fn okayge(app: &mut App) {
  app.line.insert("Oka");
  app.tab_complete(false);
  app.complete = None;
  assert!(app.line.text.ends_with("Okayge "), "{}", app.line.text);
}

// every picture asked for answers: Okayge a solid square of `rgb` (w×28), p! green
fn pictures(app: &mut App, asked: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>, w: u32) {
  for _ in 0..3 {
    app.input_dirty = true;
    app.redraw();
    while let Ok(c) = asked.try_recv() {
      let Cmd::Image(id, url, h) = c else { continue };
      let (w, rgb) = if url.contains("betterttv") { (28, [0, 255, 0]) } else { (w, [255, 0, 0]) };
      let px: Vec<u8> = (0..w * 28).flat_map(|_| [rgb[0], rgb[1], rgb[2], 255]).collect();
      let img = Decoded { w, h: 28, frames: vec![Frame { rgba: px.into(), delay_ms: 0 }] };
      app.on_net(NetEvent::Image(id, h & 0xffff, Ok(img)));
    }
  }
}

#[test]
fn okayge_p_is_one_partying_okayge_in_the_box_never_the_placeholder() {
  let (mut app, mut asked) = kripp(520);
  okayge(&mut app);
  app.line.insert("p!");
  let open = app.input_chips();
  assert_eq!(open.len(), 1);
  assert_eq!(open[0].range, 0..6, "the word being typed is still text");
  assert!(open[0].fx.is_empty());
  space(&mut app);
  assert_eq!(app.line.text, "Okayge p! ", "the words sent are unchanged");
  let chips = app.input_chips();
  assert_eq!(chips.len(), 1, "p! is folded into Okayge, not a chip of its own");
  assert_eq!(chips[0].range, 0.."Okayge p!".len());
  assert_eq!(chips[0].layers.len(), 1, "no placeholder layer");
  assert_eq!(chips[0].fx.len(), 1);
  assert_eq!(chips[0].fx[0].1.motion(), Some(Motion::Party));
  pictures(&mut app, &mut asked, 28);
  let r = app.input_area();
  let green = |app: &App| {
    {
      (r.y..r.bottom()).flat_map(|y| (r.x..r.right()).map(move |x| (x, y))).filter(|&(x, y)| {
        let p = app.canvas.px[(y * app.canvas.w + x) as usize];
        p == 0x00ff00
      })
    }
    .count()
  };
  assert_eq!(green(&app), 0, "the placeholder is never drawn");
  assert!(app.input_next.is_some(), "a moving chip asks for its next frame");
}

#[test]
fn a_betterttv_modifier_before_the_emote_folds_forward() {
  let (mut app, _asked) = kripp(520);
  app.line.insert("w! ");
  okayge(&mut app);
  let chips = app.input_chips();
  assert_eq!(chips.len(), 1);
  assert_eq!(chips[0].range, 0.."w! Okayge".len());
  assert_eq!(chips[0].w, 2 * chips[0].h, "w! is twice as wide");
}

#[test]
fn tab_on_a_modifier_settles_it_and_never_completes_the_placeholder() {
  let (mut app, mut asked) = kripp(520);
  okayge(&mut app);
  while asked.try_recv().is_ok() {}
  app.line.insert("p!");
  app.tab_complete(false);
  assert_eq!(app.line.text, "Okayge p! ");
  assert!(app.complete.is_none(), "no cycle opens");
  let catalog = std::iter::from_fn(|| asked.try_recv().ok()).filter(|c| matches!(c, Cmd::Catalog { .. })).count();
  assert_eq!(catalog, 0, "the catalogs are not asked for p!");
  assert_eq!(app.input_chips().len(), 1);
  // a bare letter after a picture is its modifier
  app.line.set_text("Okayge p".into());
  app.line.chips.insert("Okayge".into());
  app.line.cursor = app.line.text.len();
  app.tab_complete(false);
  assert_eq!(app.line.text, "Okayge p! ");
}

#[test]
fn a_stretched_chip_stays_inside_the_box_and_the_caret_in_view() {
  let (mut app, mut asked) = kripp(320);
  app.line.insert("w! ");
  okayge(&mut app);
  pictures(&mut app, &mut asked, 200);
  let (r, caret) = (app.input_area(), app.ime_win.area.expect("caret"));
  assert!(caret.x >= r.x && caret.right() <= r.right(), "caret {caret:?} in {r:?}");
}
