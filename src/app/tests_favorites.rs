// favorite emotes, the whole app headless: the account's list leads tab and the picker, a pin made here
// goes to the account whole, and the list leaves with the login
use super::*;
use crate::emotes::favorites::parse;
use crate::emotes::{Emote, Provider};
use crate::model::picker::Sec;
use crate::net::emote_ops::EmoteCmd;
use crate::net::{Cmd, NetEvent};
use serde_json::{Value, json};
use winit::keyboard::{Key, ModifiersState, NamedKey};

fn me() -> crate::net::auth::Me {
  crate::net::auth::Me {
    username: "mellen".into(),
    platform: "twitch".into(),
    twitch: Some("mellen".into()),
    twitch_id: Some(1),
    names: vec!["mellen".into()],
    kick: None,
    youtube: None,
    packs: vec![],
  }
}

fn stv(id: &str, size: &str) -> String {
  format!("https://cdn.7tv.app/emote/{id}/{size}.webp")
}

fn emote(id: &str) -> Emote {
  Emote { url: stv(id, "1x").into(), w: 28, h: 28, zero_width: false }
}

// the room holds zqPog (and the account pinned that picture at another size), a global zqPogU; the account
// also pinned zqPogAway (held nowhere here, an overlay) and a twitch emote this login cannot use
fn app() -> (App, tokio::sync::mpsc::UnboundedReceiver<Cmd>) {
  let (net, rx) = Net::tap();
  let mut app = App::new(net, vec!["forsen".into()], false);
  app.me = Some(me());
  app.res.emotes.set(Some("forsen"), Provider::SevenTv, [("zqPog".into(), emote("ROOM"))].into());
  app.res.emotes.set(
    None,
    Provider::SevenTv,
    [("zqPogU".into(), emote("GLOB")), ("zqAaa".into(), emote("AAA"))].into(),
  );
  (app, rx)
}

fn account() -> Value {
  json!([
    { "name": "zqPogAway", "url": stv("AWAY", "2x"), "provider": "7tv", "id": "AWAY", "zeroWidth": true },
    { "name": "zqPog", "url": stv("ROOM", "2x"), "provider": "7tv", "id": "ROOM" },
    { "name": "zqSub", "url": "https://static-cdn.jtvnw.net/emoticons/v2/999/default/dark/2.0", "provider": "twitch" },
  ])
}

fn sent(rx: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>) -> Vec<Value> {
  std::iter::from_fn(|| rx.try_recv().ok())
    .filter_map(|c| match c {
      Cmd::Emote(EmoteCmd::Favorites(v)) => Some(v),
      _ => None,
    })
    .collect()
}

fn tiles(app: &App) -> Vec<(Sec, String, String)> {
  let p = app.picker.as_ref().unwrap();
  (0..p.len()).filter_map(|i| p.item(i)).map(|it| (it.sec, it.name.to_string(), it.url.to_string())).collect()
}

fn favs(app: &App) -> Vec<String> {
  tiles(app).into_iter().filter(|t| t.0 == Sec::Favorite).map(|t| t.1).collect()
}

fn alt_f(app: &mut App) {
  app.mods = ModifiersState::ALT;
  app.picker_key(Key::Character("f"), Some("f"));
  app.mods = ModifiersState::empty();
}

// the law's favorites: where the name starts with what you typed, yours lead (after the exact name), in pin
// order. the one this room holds is the room's copy, said in the room's words; the one it does not is
// offered from its record; a platform's own emote you are not handed here is not offered at all
#[test]
fn tab_leads_with_the_favorites_the_word_starts() {
  let (mut app, _rx) = app();
  let before: Vec<String> =
    app.candidates("zqP", false).into_iter().filter(|c| c.emote).map(|c| c.name.into()).collect();
  assert_eq!(before, ["zqPog", "zqPogU"], "setup: the room's, then the global");
  app.on_net(NetEvent::Favorites(parse(&account()).unwrap()));
  let got: Vec<_> = app.candidates("zqP", false).into_iter().filter(|c| c.emote).collect();
  let names: Vec<&str> = got.iter().map(|c| &*c.name).collect();
  assert_eq!(names, ["zqPogAway", "zqPog", "zqPogU"], "pin order leads, the global follows");
  assert_eq!(got[0].what, "favorites · heatsync only");
  let away = got[0].pic.as_ref().unwrap();
  assert_eq!((&*away.url, away.zero_width, away.provider), (&*stv("AWAY", "2x"), true, "7tv"));
  assert_eq!(got[1].what, "channel · 7tv users", "the room's own copy, once");
  assert_eq!(&*got[1].pic.as_ref().unwrap().url, &*stv("ROOM", "1x"));
  assert!(app.candidates("zqS", false).iter().all(|c| !c.emote), "twitch's emote is not offered where it is not held");
  // typed whole, a favorite of that name is the emote typing it sends only when nothing here holds the name
  assert_eq!(&*app.candidates("zqPog", false)[0].pic.as_ref().unwrap().url, &*stv("ROOM", "1x"));
}

#[test]
fn the_picker_leads_with_your_favorites_and_an_away_one_is_picked_from_its_record() {
  let (mut app, _rx) = app();
  app.on_net(NetEvent::Favorites(parse(&account()).unwrap()));
  app.toggle_picker();
  let all = tiles(&app);
  assert_eq!(
    all[..2],
    [(Sec::Favorite, "zqPogAway".into(), stv("AWAY", "2x")), (Sec::Favorite, "zqPog".into(), stv("ROOM", "1x")),],
    "pin order; the one held here is the room's own tile, and twitch's is not shown"
  );
  assert_eq!(all[2].0, Sec::Channel, "then the sections");
  // HSC_DUMP=$d/x.ppm cargo test -j2 --release the_picker_leads: a "favorites 2" band above "channel 1"
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.canvas.resize(720, 360);
  app.redraw();
  super::tests_pickers::dump_frame(&app);
  // enter on the first tile: the away favorite is this draft's pick, with its record's overlay flag
  app.picker_key(Key::Named(NamedKey::Enter), None);
  let rec = app.line.picks.get("zqPogAway").expect("picked");
  assert_eq!((&*rec.url, rec.zero_width, rec.provider), (&*stv("AWAY", "2x"), true, "7tv"));
  assert_eq!(app.line.text.trim(), "zqPogAway");
  // a word typed: the law's list, one tile per picture (no second tile for the favorite held here)
  app.toggle_picker();
  for c in ["z", "q", "P"] {
    app.picker_key(Key::Character(c), Some(c));
  }
  let names: Vec<String> = tiles(&app).into_iter().map(|t| t.1).collect();
  assert_eq!(names, ["zqPogAway", "zqPog", "zqPogU"], "tab's order");
  assert_eq!(app.picker.as_ref().unwrap().selected_what(), Some("favorites · heatsync only"));
}

// alt-f on a tile pins its picture: the account gets the whole list, with the records it already held
// untouched, and the tile leads the grid at once. again unpins. logged out there is no list
#[test]
fn alt_f_pins_and_unpins_the_picked_tile() {
  let (mut app, mut rx) = app();
  let list = parse(&account()).unwrap();
  app.on_net(NetEvent::Favorites(list));
  app.toggle_picker();
  for c in ["z", "q", "A"] {
    app.picker_key(Key::Character(c), Some(c));
  }
  assert_eq!(tiles(&app)[0].1, "zqAaa");
  alt_f(&mut app);
  let got = sent(&mut rx);
  assert_eq!(got.len(), 1, "one write");
  let rows = got[0].as_array().unwrap();
  assert_eq!(rows.len(), 4);
  assert_eq!(rows[..3], account().as_array().unwrap()[..], "what the account held is sent back as it was");
  assert_eq!(rows[3], json!({ "name": "zqAaa", "url": stv("AAA", "2x"), "provider": "7tv", "id": "AAA" }));
  app.picker_key(Key::Named(NamedKey::Escape), None);
  assert_eq!(favs(&app), ["zqPogAway", "zqPog", "zqAaa"], "it leads the grid, last pinned last");
  // the account's older answer (a login re-check) does not undo it while the write is on its way
  app.on_net(NetEvent::Favorites(parse(&account()).unwrap()));
  assert_eq!(favs(&app).len(), 3);
  app.on_net(NetEvent::FavoritesSaved);
  // unpin it from its favorites tile
  let at = tiles(&app).iter().position(|t| t.1 == "zqAaa").unwrap();
  app.picker.as_mut().unwrap().sel = at;
  alt_f(&mut app);
  let got = sent(&mut rx);
  assert_eq!(got[0].as_array().unwrap().len(), 3);
  assert_eq!(favs(&app), ["zqPogAway", "zqPog"]);
  app.on_net(NetEvent::FavoritesSaved);
  // refused by the account: its own list follows, and it is the list again
  alt_f(&mut app);
  assert_eq!(sent(&mut rx).len(), 1);
  app.on_net(NetEvent::FavoritesSaved);
  app.on_net(NetEvent::Favorites(parse(&account()).unwrap()));
  assert_eq!(favs(&app), ["zqPogAway", "zqPog"]);
  // logged out: nothing is pinned, nothing is sent
  app.on_net(NetEvent::Me(None));
  assert!(favs(&app).is_empty(), "the list left with the login");
  alt_f(&mut app);
  assert!(sent(&mut rx).is_empty());
  assert!(app.res.favorites.list().is_empty());
}

// a pin made on heatsync.org or in the extension shows in the open picker at once, and another account's
// login starts with its own list
#[test]
fn the_accounts_list_follows_the_account() {
  let (mut app, _rx) = app();
  app.toggle_picker();
  assert!(favs(&app).is_empty());
  app.on_net(NetEvent::Favorites(parse(&account()).unwrap()));
  assert_eq!(favs(&app), ["zqPogAway", "zqPog"]);
  app.on_net(NetEvent::Me(Some(crate::net::auth::Me { username: "other".into(), ..me() })));
  assert!(favs(&app).is_empty(), "another account: not its list");
  // a list that arrives with nobody logged in is nobody's
  app.on_net(NetEvent::Me(None));
  app.on_net(NetEvent::Favorites(parse(&account()).unwrap()));
  assert!(app.res.favorites.list().is_empty());
}

// the emote menu (a right-click on an emote in chat or in the box) pins the exact picture under the pointer
#[test]
fn the_emote_menu_pins_the_picture_clicked() {
  let (mut app, mut rx) = app();
  app.fav_toggle("zqPog", &stv("ROOM", "1x"), false);
  let got = sent(&mut rx);
  assert_eq!(got[0], json!([{ "name": "zqPog", "url": stv("ROOM", "2x"), "provider": "7tv", "id": "ROOM" }]));
  assert!(app.res.favorites.has("zqPog", &stv("ROOM", "4x")));
  assert!(!app.res.favorites.has("zqPog", &stv("OTHER", "1x")), "another zqPog is not the favorite");
}
