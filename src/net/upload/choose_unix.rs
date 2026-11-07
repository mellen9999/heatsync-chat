// the file dialog on unix: zenity, then kdialog, run with an argument list and never a shell
use std::path::PathBuf;
use std::process::{Command, Stdio};

use super::{Upload, parse_pick};

enum Ran {
  Picked(PathBuf),
  Closed,
  Missing,
}

fn run_chooser(prog: &str, args: &[&str]) -> Ran {
  let child = Command::new(prog).args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).output();
  match child {
    Err(_) => Ran::Missing,
    Ok(o) if o.status.success() => parse_pick(&String::from_utf8_lossy(&o.stdout)).map_or(Ran::Closed, Ran::Picked),
    // 1 = the user closed it; anything else = it couldn't open (no display, broken): try the next
    Ok(o) if o.status.code() == Some(1) => Ran::Closed,
    Ok(_) => Ran::Missing,
  }
}

// blocks until the dialog closes
pub fn choose() -> Upload {
  let tries: [(&str, &[&str]); 2] = [
    ("zenity", &["--file-selection", "--title=upload a picture or video"]),
    ("kdialog", &["--getopenfilename", ".", "--title", "upload a picture or video"]),
  ];
  for (p, a) in tries {
    match run_chooser(p, a) {
      Ran::Picked(path) => return Upload::Url(path.to_string_lossy().into_owned()),
      Ran::Closed => return Upload::Cancelled,
      Ran::Missing => {}
    }
  }
  Upload::NoChooser
}
