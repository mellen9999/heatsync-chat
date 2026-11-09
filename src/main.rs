// the release exe on windows has no console window; stdio.rs gives its words somewhere to go
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod avatars;
mod badges;
mod bench;
mod chatlog;
mod cosmetics;
mod emotes;
mod fake;
mod heatsync;
mod i18n;
mod images;
mod model;
mod net;
mod store;
mod ui;
mod util;

use i18n::{f, t};
use winit::event_loop::EventLoop;

fn help() -> String {
  let [config, cache] = util::dirs::HELP_FOLDERS;
  f("misc.cli.help", &[("config", config), ("cache", cache)]) + "\n"
}

// the second launch's one plain sentence, for when the running window can't be reached (an older
// version, no socket): said to the terminal and as a desktop notification, since a launcher shows neither

fn already_open(given: &[Box<str>]) {
  if store::wake_running(given) {
    return;
  }
  util::notify::send("heatsync-chat", t("misc.cli.open_title"), t("misc.cli.open_body"));
  println!("{}", t("misc.cli.open_said"));
  util::stdio::alert("heatsync-chat", t("misc.cli.open_said"));
  util::notify::shutdown();
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
  // the terminal text (--help, errors) speaks the machine's language until a saved setting says otherwise
  i18n::apply(0);
  let args: Vec<String> = std::env::args().skip(1).collect();
  if args.iter().any(|a| a.starts_with('-')) {
    util::stdio::attach_terminal();
  }
  util::stdio::log_stderr();
  util::crash::install();
  if let Some(bad) = args.iter().find(|a| a.starts_with('-') && !matches!(a.as_str(), "--flood" | "--bench")) {
    match bad.as_str() {
      "-h" | "--help" => return print!("{}", help()),
      "-V" | "--version" => return println!("heatsync-chat {}", env!("CARGO_PKG_VERSION")),
      _ => {
        eprintln!("{}", f("misc.cli.bad_option", &[("option", bad)]));
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
  net::update::tidy();
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
      util::stdio::alert("heatsync-chat", t("misc.cli.no_window"));
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
  let ran = el.run_app(&mut app);
  util::notify::shutdown();
  if let Err(e) = ran {
    eprintln!("{e}");
    std::process::exit(1);
  }
}
