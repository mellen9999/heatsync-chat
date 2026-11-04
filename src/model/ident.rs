// the one place that says what a login, a channel name, a slug or a relay key may look like. these
// gates stand between hostile text (a server frame, a typed word) and a url, a request body or an irc
// line, so each rule lives here once, named for what it checks. call sites keep their own trimming and
// lowercasing: every fn takes the word as it is and answers yes or no

// a byte of a twitch name: a-z A-Z 0-9 _
pub fn name_byte(b: u8) -> bool {
  b.is_ascii_alphanumeric() || b == b'_'
}

// a byte of a slug (kick names, ids, archive rooms): a name byte or `-`
pub fn slug_byte(b: u8) -> bool {
  name_byte(b) || b == b'-'
}

// 1..=max slug bytes, either case: ids, rooms and hostname labels that only need to be one safe word
pub fn slug(s: &str, max: usize) -> bool {
  (1..=max).contains(&s.len()) && s.bytes().all(slug_byte)
}

// a twitch login as twitch hands it out or as typed, either case: 1-25 of a-z A-Z 0-9 _
pub fn twitch_login(s: &str) -> bool {
  (1..=25).contains(&s.len()) && s.bytes().all(name_byte)
}

// a twitch room as irc joins it: a login in lowercase (irc wants it so), 1-25 of a-z 0-9 _
pub fn twitch_room(s: &str) -> bool {
  (1..=25).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

// a kick channel slug as the add-channel form takes it, lowercase: twitch_room's letters plus `-`
pub fn kick_slug(s: &str) -> bool {
  (1..=25).contains(&s.len())
    && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

// a youtube handle without its `@`: 3-30 of a-z A-Z 0-9 _ . -
pub fn yt_handle(s: &str) -> bool {
  (3..=30).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
}

// a person as the archive names them, without a youtube `@`: 1-40 slug bytes or `.` (handles carry dots)
pub fn archive_login(s: &str) -> bool {
  (1..=40).contains(&s.len()) && s.bytes().all(|b| slug_byte(b) || b == b'.')
}

// a room as the archive and the pin api key it, on any platform: lowercase, 1-50 of a-z 0-9 _ -
pub fn room_key(s: &str) -> bool {
  (1..=50).contains(&s.len())
    && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

// a twitch or kick login in a lookup request, either case: 1-50 slug bytes (the longer of the two)
pub fn chatter_login(s: &str) -> bool {
  slug(s, 50)
}

// a key heatsync's relay knows a room or a youtube handle by: 1-100 of a-z A-Z 0-9 _ - . @
pub fn relay_key(s: &str) -> bool {
  (1..=100).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"_-.@".contains(&b))
}

#[cfg(test)]
mod tests {
  use super::*;

  // (the word, does each rule take it) for the rules that differ only by case, dash and length
  #[test]
  fn twitch_names_take_either_case_but_rooms_only_lowercase() {
    assert!(twitch_login("Forsen_2") && twitch_room("forsen_2"));
    assert!(twitch_login("a") && twitch_room("a"));
    assert!(twitch_login(&"a".repeat(25)) && twitch_room(&"a".repeat(25)));
    assert!(!twitch_login(&"a".repeat(26)) && !twitch_room(&"a".repeat(26)));
    assert!(!twitch_login("") && !twitch_room(""));
    assert!(twitch_login("Forsen") && !twitch_room("Forsen"), "a room is lowercase");
    for no in ["a b", "a-b", "a.b", "@a", "#a", "a\n", "ä", "a/b"] {
      assert!(!twitch_login(no) && !twitch_room(no), "{no:?}");
    }
  }

  #[test]
  fn kick_slugs_add_the_dash_and_stay_lowercase() {
    assert!(kick_slug("some-one_2") && !twitch_room("some-one_2"));
    assert!(!kick_slug("Some-One") && !kick_slug("") && !kick_slug(&"a".repeat(26)));
    assert!(!kick_slug("a b") && !kick_slug("a.b"));
  }

  #[test]
  fn youtube_handles_are_3_to_30_with_dots_and_dashes() {
    assert!(yt_handle("Kripp.arrian-1_x") && yt_handle("abc"));
    assert!(!yt_handle("ab") && !yt_handle(&"a".repeat(31)) && !yt_handle("a b c") && !yt_handle("@abc"));
  }

  #[test]
  fn archive_rooms_and_chatter_logins_run_to_50() {
    assert!(room_key("some-one_2") && room_key(&"a".repeat(50)));
    assert!(!room_key(&"a".repeat(51)) && !room_key("") && !room_key("Forsen") && !room_key("a.b"));
    assert!(chatter_login("Some-One_2") && chatter_login(&"a".repeat(50)));
    assert!(!chatter_login(&"a".repeat(51)) && !chatter_login("") && !chatter_login("a b") && !chatter_login("a/b"));
  }

  #[test]
  fn archive_names_allow_dots_up_to_40() {
    assert!(archive_login("kripp.arrian-1_x") && archive_login(&"a".repeat(40)));
    assert!(!archive_login(&"a".repeat(41)) && !archive_login("") && !archive_login("@a") && !archive_login("a b"));
  }

  #[test]
  fn relay_keys_allow_handles_and_dots() {
    assert!(relay_key("forsen") && relay_key("@yt.handle-1") && relay_key(&"a".repeat(100)));
    assert!(
      !relay_key("") && !relay_key(&"a".repeat(101)) && !relay_key("a b") && !relay_key("a/b") && !relay_key("a#b")
    );
  }

  #[test]
  fn slugs_are_one_safe_word_of_a_given_length() {
    assert!(slug("a-b_C9", 6) && !slug("a-b_C9", 5) && !slug("", 5));
    assert!(!slug("a.b", 9) && !slug("a b", 9) && !slug("a/b", 9) && !slug("é", 9));
    assert!(slug_byte(b'-') && name_byte(b'_') && !name_byte(b'-') && !slug_byte(b'.'));
  }
}
