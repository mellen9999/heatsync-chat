// heatsync's own data and the rules for when it renders
pub mod senders;

use serde_json::Value;

pub const MAX_NAME: usize = 100;

// an emote name is one token: no whitespace/control bytes, not a command (`/me`) or bot prefix (`.`)
pub fn valid_name(w: &str) -> bool {
  !w.is_empty()
    && w.len() <= MAX_NAME
    && !w.starts_with(['/', '.'])
    && !w.chars().any(|c| c.is_whitespace() || c.is_control())
}

// content-warning gate: nsfw or sexual/gore categories never render
pub fn cw_blocked(e: &Value) -> bool {
  e.get("nsfw").and_then(Value::as_bool) == Some(true)
    || e
      .get("cw_cats")
      .and_then(Value::as_array)
      .is_some_and(|c| c.iter().any(|x| matches!(x.as_str(), Some("sexual" | "gore"))))
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn names() {
    assert!(valid_name("PogU"));
    for bad in ["", "a b", "/me", ".w", "x\u{7}", &"x".repeat(101)] {
      assert!(!valid_name(bad), "{bad:?}");
    }
  }

  #[test]
  fn cw() {
    assert!(cw_blocked(&json!({"nsfw": true})));
    assert!(cw_blocked(&json!({"cw_cats": ["gore"]})));
    assert!(!cw_blocked(&json!({"cw_cats": ["flashing"]})));
    assert!(!cw_blocked(&json!({})));
  }
}
