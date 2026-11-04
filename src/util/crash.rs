// a crash anywhere leaves one line in crash.log (beside the settings) so it can be reported;
// long letter-digit runs are cut first, so a token caught in a message never lands on disk
use std::panic::PanicHookInfo;

pub const LOG: &str = "crash.log";

pub fn install() {
  let prev = std::panic::take_hook();
  std::panic::set_hook(Box::new(move |info| {
    let _ = crate::store::append(LOG, &line(info), 64 * 1024);
    prev(info);
  }));
}

fn line(info: &PanicHookInfo) -> String {
  let msg = info
    .payload()
    .downcast_ref::<&str>()
    .copied()
    .or_else(|| info.payload().downcast_ref::<String>().map(String::as_str))
    .unwrap_or("?");
  let at = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
  let thread = std::thread::current().name().unwrap_or("?").to_owned();
  format!(
    "{} v{} {thread} {at}: {}\n",
    crate::util::time::now_ms() / 1000,
    env!("CARGO_PKG_VERSION"),
    redact(super::clamp(msg, 512))
  )
}

// any run of 20+ letters, digits, `-` or `_` reads as a secret: tokens, keys, ids. one line
fn redact(s: &str) -> String {
  let mut out = String::with_capacity(s.len());
  let mut run = String::new();
  let flush = |out: &mut String, run: &mut String| {
    out.push_str(if run.len() >= 20 { "[REDACTED]" } else { run });
    run.clear();
  };
  for c in s.chars() {
    if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
      run.push(c);
    } else {
      flush(&mut out, &mut run);
      out.push(if c.is_control() { ' ' } else { c });
    }
  }
  flush(&mut out, &mut run);
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn long_runs_are_cut_and_the_rest_kept() {
    let fake = "k9".repeat(12);
    assert_eq!(redact(&format!("bad token oauth:{fake} here")), "bad token oauth:[REDACTED] here");
    assert_eq!(
      redact("index out of bounds: the len is 3 but the index is 9"),
      "index out of bounds: the len is 3 but the index is 9"
    );
    assert_eq!(redact("a\nb"), "a b");
  }
}
