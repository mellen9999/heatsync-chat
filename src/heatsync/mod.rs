// heatsync's own data and the rules for when it renders
pub mod content;
pub mod senders;

pub const MAX_NAME: usize = 100;

// an emote name is one token: no whitespace/control bytes, not a command (`/me`) or bot prefix (`.`)
pub fn valid_name(w: &str) -> bool {
  !w.is_empty()
    && w.len() <= MAX_NAME
    && !w.starts_with(['/', '.'])
    && !w.chars().any(|c| c.is_whitespace() || c.is_control())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn names() {
    assert!(valid_name("PogU"));
    for bad in ["", "a b", "/me", ".w", "x\u{7}", &"x".repeat(101)] {
      assert!(!valid_name(bad), "{bad:?}");
    }
  }
}
