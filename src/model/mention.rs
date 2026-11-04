// who counts as "you" and which lines mention you (ext mentions.js isMention): the sender is not
// you, a reply to your line, `@name` anywhere, or `name` as a whole word, case-insensitive.
// names are your heatsync name plus the twitch / kick / youtube names linked to it
use super::ident;
use crate::model::message::Message;

#[derive(Default)]
pub struct Mentions {
  // lowercase, unique, word characters only (a name with anything else could not be matched safely)
  names: Vec<Box<str>>,
}

// `\bname\b`, ascii case-insensitive
fn whole_word(text: &str, name: &str) -> bool {
  let (t, n) = (text.as_bytes(), name.as_bytes());
  if n.is_empty() || n.len() > t.len() {
    return false;
  }
  t.windows(n.len()).enumerate().any(|(i, w)| {
    w.eq_ignore_ascii_case(n)
      && (i == 0 || !ident::name_byte(t[i - 1]))
      && t.get(i + n.len()).is_none_or(|&b| !ident::name_byte(b))
  })
}

// `@name`, with no boundary after it (the ext's `includes`)
fn at_name(text: &str, name: &str) -> bool {
  let (t, n) = (text.as_bytes(), name.as_bytes());
  t.windows(n.len() + 1).any(|w| w[0] == b'@' && w[1..].eq_ignore_ascii_case(n))
}

impl Mentions {
  pub fn set<'a>(&mut self, names: impl IntoIterator<Item = &'a str>) {
    self.names.clear();
    for n in names {
      let n = n.trim().to_ascii_lowercase();
      if (1..=64).contains(&n.len()) && n.bytes().all(ident::name_byte) && !self.names.iter().any(|x| **x == *n) {
        self.names.push(n.into());
      }
    }
  }

  pub fn is_me(&self, who: &str) -> bool {
    !who.is_empty() && self.names.iter().any(|n| n.eq_ignore_ascii_case(who))
  }

  pub fn hit(&self, m: &Message) -> bool {
    if self.names.is_empty() || !m.kind.said() {
      return false;
    }
    // your own lines never ping you
    if self.is_me(&m.login) || self.is_me(&m.nick) {
      return false;
    }
    if m.reply_to.as_deref().is_some_and(|r| self.is_me(r)) {
      return true;
    }
    self.names.iter().any(|n| at_name(&m.body, n) || whole_word(&m.body, n))
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::model::message::Platform;

  fn line(login: &str, body: &str) -> Message {
    let mut m = Message::system(1, Platform::Twitch, "x");
    m.kind = crate::model::message::Kind::Chat;
    m.login = login.into();
    m.nick = login.into();
    m.body = body.into();
    m
  }

  fn me() -> Mentions {
    let mut m = Mentions::default();
    m.set(["Mellen", "mellen9999", "bad name", "mellen"]);
    m
  }

  #[test]
  fn names_are_cleaned() {
    assert_eq!(me().names.len(), 2);
    let mut m = Mentions::default();
    m.set(["", "a-b", "x".repeat(65).as_str()]);
    assert!(m.names.is_empty());
  }

  #[test]
  fn whole_words_and_at_signs() {
    let m = me();
    assert!(m.hit(&line("bob", "hey MELLEN how are you")));
    assert!(m.hit(&line("bob", "mellen, hi")));
    assert!(m.hit(&line("bob", "@Mellen9999")));
    // no boundary needed after an @: the ext uses includes
    assert!(m.hit(&line("bob", "@mellenfan lol")));
    // a bare name inside a longer word is not you
    assert!(!m.hit(&line("bob", "mellens cat")));
    assert!(!m.hit(&line("bob", "nomellen")));
    assert!(!m.hit(&line("bob", "mellen_x")));
    assert!(m.hit(&line("bob", "(mellen)")));
    assert!(!m.hit(&line("bob", "hello world")));
  }

  #[test]
  fn not_yourself_not_system_replies_count() {
    let m = me();
    assert!(!m.hit(&line("mellen9999", "@mellen test")));
    assert!(!m.hit(&Message::system(1, Platform::Twitch, "mellen was timed out")));
    let mut r = line("bob", "yeah agreed");
    assert!(!m.hit(&r));
    r.reply_to = Some("Mellen".into());
    assert!(m.hit(&r));
    r.reply_to = Some("someoneelse".into());
    assert!(!m.hit(&r));
    // no names yet (logged out): nothing pings
    assert!(!Mentions::default().hit(&line("bob", "@mellen")));
  }

  #[test]
  fn non_ascii_text_is_safe() {
    let m = me();
    assert!(m.hit(&line("bob", "こんにちは mellen です")));
    assert!(!m.hit(&line("bob", "日本語だけ")));
    assert!(m.hit(&line("bob", "é@mellen")));
  }
}
