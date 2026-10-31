mod app;
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

fn main() {
  util::mem::limit_arenas();
  let args: Vec<String> = std::env::args().skip(1).collect();
  if args.first().map(String::as_str) == Some("--bench") {
    let n = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(10_000);
    return bench::run(n);
  }
  let flood = args.iter().any(|a| a == "--flood");
  // channels on the command line replace the saved tabs; none = reopen last session
  let given: Vec<Box<str>> = args
    .iter()
    .filter(|a| !a.starts_with("--"))
    .map(|a| a.trim_start_matches('#').to_ascii_lowercase().into())
    .collect();
  let channels = if given.is_empty() { store::load_tabs() } else { given };
  if !channels.is_empty() {
    app::persist(&channels);
  }

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
  let net = net::Net::spawn(el.create_proxy());
  let mut app = app::App::new(net, channels, flood);
  if let Err(e) = el.run_app(&mut app) {
    eprintln!("{e}");
    std::process::exit(1);
  }
}
