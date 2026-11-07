// where the app's own words go. on unix the terminal that started it has them already: nothing to set up

// --help and friends print to the terminal that started us
pub fn attach_terminal() {}

// diagnostics (`eprintln!`) go wherever stderr already goes
pub fn log_stderr() {}

// a window to say something to someone who started the app from a launcher (the terminal and the desktop
// notification already did, on unix)
pub fn alert(_title: &str, _text: &str) {}
