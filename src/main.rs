mod app;
mod avatars;
mod badges;
mod bench;
mod cosmetics;
mod emotes;
mod fake;
mod heatsync;
mod images;
mod model;
mod net;
mod store;
mod ui;
mod util;

use winit::event_loop::EventLoop;

const HELP: &str = "heatsync-chat: the heatsync multichat overlay as its own window

usage: heatsync-chat [channel ...]

  channel        a twitch name (forsen), kick:name or yt:@handle. one tab each; quote
                 several words together to merge them into one tab. they open for this
                 run only: your saved tabs stay as they were
  (no channel)   reopen the tabs you had last time
  -h, --help     show this
  -V, --version  show the version
  --bench [n]    check drawing speed without a window, on n fake lines (default 10000)
  --flood        a window of fake busy chat, for testing

one window at a time: opening a second one brings the first forward (and opens the channel you gave it).
your settings and tabs are in ~/.config/heatsync-chat, the picture cache in
~/.cache/heatsync-chat. both are private to you.
";

// the second launch's one plain sentence, for when the running window can't be reached (an older
// version, no socket): said to the terminal and as a desktop notification, since a launcher shows neither
const RUNNING: &str = "heatsync-chat is already open: look for its window (ctrl-t there adds a channel). \
only one runs at a time, so two can't overwrite each other's saved tabs";

fn already_open(given: &[Box<str>]) {
  if store::wake_running(given) {
    return;
  }
  util::notify::send("heatsync-chat", "heatsync-chat is already open", "look for its window");
  println!("{RUNNING}");
}

// the saved tabs; lines that aren't channels are kept in tabs.bak and said so in chat
fn saved_tabs() -> Vec<Box<str>> {
  let (good, junk): (Vec<_>, Vec<_>) =
    store::load_tabs().into_iter().partition(|l| model::chan::Spec::from_line(l).is_ok());
  if !junk.is_empty() {
    store::salvage_tabs(&good, junk.len());
  }
  good
}

fn main() {
  util::mem::limit_arenas();
  util::crash::install();
  let args: Vec<String> = std::env::args().skip(1).collect();
  if let Some(bad) = args.iter().find(|a| a.starts_with('-') && !matches!(a.as_str(), "--flood" | "--bench")) {
    match bad.as_str() {
      "-h" | "--help" => return print!("{HELP}"),
      "-V" | "--version" => return println!("heatsync-chat {}", env!("CARGO_PKG_VERSION")),
      _ => {
        eprintln!("heatsync-chat: no such option {bad} (heatsync-chat --help lists them)");
        std::process::exit(2);
      }
    }
  }
  if args.first().map(String::as_str) == Some("--bench") {
    let n = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(10_000);
    return bench::run(n);
  }
  let flood = args.iter().any(|a| a == "--flood");
  let given: Vec<Box<str>> = args
    .iter()
    .filter(|a| !a.starts_with('-'))
    .map(|a| a.trim_start_matches('#').to_ascii_lowercase().into())
    .collect();

  store::init();
  images::cache::init();
  // the test window (--flood) may sit beside a real one; a real one may not
  let _lock = if flood {
    None
  } else {
    match store::lock_instance() {
      Some(Err(())) => return already_open(&given),
      Some(Ok(held)) => Some(held),
      None => None,
    }
  };

  // channels on the command line are for this run: the saved tabs are neither read nor overwritten
  let channels = if given.is_empty() {
    saved_tabs()
  } else {
    store::pin_tabs();
    given
  };

  if rustls::crypto::ring::default_provider().install_default().is_err() {
    eprintln!("tls provider already installed");
  }
  let el = match EventLoop::<net::NetEvent>::with_user_event().build() {
    Ok(el) => el,
    Err(e) => {
      eprintln!("event loop: {e}");
      std::process::exit(1);
    }
  };
  // only the lock holder binds, so a second launch can reach this window
  if let (Some(_), Some(l)) = (&_lock, store::bind_wake()) {
    let proxy = el.create_proxy();
    store::serve_wake(l, move |open| proxy.send_event(net::NetEvent::Raise(open)).is_ok());
  }
  let net = net::Net::spawn(el.create_proxy());
  let mut app = app::App::new(net, channels, flood);
  if let Err(e) = el.run_app(&mut app) {
    eprintln!("{e}");
    std::process::exit(1);
  }
}
