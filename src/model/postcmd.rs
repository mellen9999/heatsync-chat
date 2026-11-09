// `/op <text>` posts a thread to your heatsync feed, `/re <text>` answers the last [OP] shown in chat
// (ext slash-registry: op / post, re / reply / opr). they work in any tab
#[derive(Debug, PartialEq, Eq)]
pub enum Cmd {
  // the words; empty = just the usage
  Op(String),
  Re(String),
}

// the keys of the usage lines; show them through `t(OP_USAGE)`
pub const OP_USAGE: &str = "chat2.postcmd.op_usage";
pub const RE_USAGE: &str = "chat2.postcmd.re_usage";

pub fn command(text: &str) -> Option<Cmd> {
  let t = text.trim();
  let (word, rest) = t.split_once(char::is_whitespace).unwrap_or((t, ""));
  let rest = rest.trim().to_owned();
  match word.to_ascii_lowercase().as_str() {
    "/op" | "/post" => Some(Cmd::Op(rest)),
    "/re" | "/reply" | "/opr" => Some(Cmd::Re(rest)),
    _ => None,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn op_and_re_with_their_aliases() {
    assert_eq!(command("/op hello  world"), Some(Cmd::Op("hello  world".into())));
    assert_eq!(command("/POST x"), Some(Cmd::Op("x".into())));
    assert_eq!(command("/re yes"), Some(Cmd::Re("yes".into())));
    assert_eq!(command("/reply yes"), Some(Cmd::Re("yes".into())));
    assert_eq!(command("/opr yes"), Some(Cmd::Re("yes".into())));
    assert_eq!(command("/op"), Some(Cmd::Op(String::new())));
    assert_eq!(command("/open x"), None);
    assert_eq!(command("/r hi"), None, "/r answers a whisper");
    assert_eq!(command("op hi"), None);
  }
}
