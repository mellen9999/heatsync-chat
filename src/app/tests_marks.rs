// who sees a tile, the whole app headless (law::mark, the site's emote-mark vectors): the picker draws no tile
// for a picture its room answers a name with another of, and a corner letter on the rest; tab offers one
// picture per name, with the same letter on the `:` list's row
use super::*;
use crate::emotes::{Emote, Provider};
use crate::heatsync::senders::parse_set;
use crate::model::picker::Sec;
use crate::net::auth::Me;
use serde_json::json;

fn me() -> Me {
  Me {
    username: "mellen".into(),
    platform: "twitch".into(),
    twitch: Some("mellen".into()),
    twitch_id: Some(1),
    names: vec!["mellen".into()],
    kick: Some("mellenk".into()),
    youtube: None,
    packs: vec![],
  }
}

fn stv(id: &str) -> String {
  format!("https://cdn.7tv.app/emote/{id}/1x.webp")
}

fn emote(url: &str) -> Emote {
  Emote { url: url.into(), w: 28, h: 28, zero_width: false }
}

fn yours(app: &mut App, rows: &[(&str, &str)]) {
  let rows: Vec<_> = rows
    .iter()
    .map(
      |(n, u)| json!({"custom_name": n, "url": u, "width": 32, "height": 32, "added_at": "2026-10-30T22:26:51.099Z"}),
    )
    .collect();
  app.res.senders.lookup(1, "x", 0);
  app.res.senders.put(1, parse_set(&json!(rows)));
}

// forsen's room holds SusgeBinoculars (7tv); you saved an ffz picture under that name, and one of your own
// uploads; a 7tv global and twitch's Kappa are the globals
fn room(tabs: &str) -> App {
  let (net, _rx) = Net::tap();
  let mut app = App::new(net, vec![tabs.into()], false);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.canvas.resize(720, 360);
  app.me = Some(me());
  let chan = [("SusgeBinoculars".into(), emote(&stv("S1"))), ("SusgeBinocularsBig".into(), emote(&stv("S2")))];
  app.res.emotes.set(Some("forsen"), Provider::SevenTv, chan.into());
  app.res.emotes.set(None, Provider::SevenTv, [("GlobalSeven".into(), emote(&stv("G1")))].into());
  let kappa = "https://static-cdn.jtvnw.net/emoticons/v2/25/default/dark/2.0";
  app.res.emotes.set(None, Provider::Twitch, [("Kappa".into(), emote(kappa))].into());
  yours(
    &mut app,
    &[
      ("SusgeBinoculars", "https://cdn.frankerfacez.com/emote/9/2"),
      ("MineOnly", "https://cdn.heatsync.org/e/mine.webp"),
    ],
  );
  app
}

// the tiles the grid shows: section, name, corner letter
fn shown(app: &App) -> Vec<(Sec, String, Option<char>)> {
  let p = app.picker.as_ref().expect("the picker is open");
  (0..p.len()).filter_map(|i| p.item(i)).map(|i| (i.sec, i.name.to_string(), i.verdict.mark())).collect()
}

#[test]
fn the_picker_draws_no_tile_for_a_name_the_room_answers_with_another_picture_and_marks_the_rest() {
  let mut app = room("forsen");
  app.toggle_picker();
  let tiles = shown(&app);
  assert!(
    !tiles.iter().any(|t| t.0 == Sec::Mine && t.1 == "SusgeBinoculars"),
    "your ffz copy is not a tile here: {tiles:?}"
  );
  for want in [
    (Sec::Channel, "SusgeBinoculars", Some('7')),
    (Sec::Channel, "SusgeBinocularsBig", Some('7')),
    (Sec::Mine, "MineOnly", Some('H')),
    (Sec::Global, "GlobalSeven", Some('7')),
    (Sec::Global, "Kappa", Some('T')),
  ] {
    assert!(tiles.contains(&(want.0, want.1.to_string(), want.2)), "{want:?} in {tiles:?}");
  }
  let p = app.picker.as_ref().unwrap();
  assert!(p.unjudged().is_empty(), "every tile was asked once");
}

#[test]
fn a_name_stays_one_tile_when_the_room_has_none() {
  let mut app = room("forsen");
  app.res.emotes.set(Some("forsen"), Provider::SevenTv, Default::default());
  app.toggle_picker();
  let tiles = shown(&app);
  assert!(
    tiles.contains(&(Sec::Mine, "SusgeBinoculars".to_string(), Some('H'))),
    "yours, heatsync users only: {tiles:?}"
  );
}

#[test]
fn tab_offers_one_picture_per_name_and_the_groups_end_sooner() {
  let app = room("forsen");
  let (list, exact, rooms) = app.offer("SusgeBinoculars", None);
  let names: Vec<(&str, &str)> = list.iter().map(|p| (&*p.name, &*p.pic.as_ref().unwrap().url)).collect();
  assert_eq!(
    names,
    [("SusgeBinoculars", &*stv("S1")), ("SusgeBinocularsBig", &*stv("S2"))],
    "your ffz copy is not offered"
  );
  assert_eq!((exact, rooms), (1, 2), "the exact name is one tile, the room's names two");
  assert_eq!(list[0].mark, Some('7'));
  // the cap counts what is left: two tiles, not the room's copy and yours cut to one
  assert_eq!(app.offer("SusgeBinoculars", Some(2)).0.len(), 2);
}

#[test]
fn a_twitch_emote_is_t_where_the_send_reaches_twitch_and_a_picture_where_it_does_not() {
  let sub = "https://static-cdn.jtvnw.net/emoticons/v2/999/default/dark/2.0";
  let mark_in = |tabs: &str| {
    let mut app = room(tabs);
    app.res.emotes.set_subs([("zqSub".into(), emote(sub))].into());
    app.toggle_picker();
    let tiles = shown(&app);
    tiles.into_iter().find(|t| t.1 == "zqSub").map(|t| t.2)
  };
  assert_eq!(mark_in("forsen kick:xqc"), Some(Some('T')), "a merged tab's twitch leg draws it");
  assert_eq!(mark_in("kick:xqc"), Some(Some('H')), "no twitch leg: it goes out as a picture");
}

#[test]
fn the_list_row_ends_in_the_tiles_letter() {
  let mut app = room("forsen");
  let chat = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
  app.select(chat);
  app.enter_insert();
  app.line.insert("so :Susge");
  app.input_dirty = true;
  app.redraw();
  let s = app.sug.list.as_ref().expect("the list opens");
  let marks: Vec<Option<char>> = s.items.iter().filter(|p| p.emote).map(|p| p.mark).collect();
  assert_eq!(marks, [Some('7'), Some('7')], "both of the room's, none of yours");
  // the second row is not the pick: white letter on black at its right end, and gone when it has none
  let ends = |app: &App| {
    let r = app.sug.rows[1].1;
    let end = crate::ui::canvas::Rect::new(r.right() - 24, r.y, 20, r.h);
    let c = &app.canvas;
    (end.y..end.bottom())
      .flat_map(|y| (end.x..end.right()).map(move |x| (x, y)))
      .filter(|&(x, y)| c.px[(y * c.w + x) as usize] == 0xffffff)
      .count()
  };
  assert!(ends(&app) > 0, "the letter is drawn");
  app.sug.list.as_mut().unwrap().items[1].mark = None;
  app.input_dirty = true;
  app.redraw();
  assert_eq!(ends(&app), 0, "and only the letter");
}
