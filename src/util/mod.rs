pub mod mem;
pub mod open;
pub mod sound;
pub mod time;

// at most `max` bytes of `s`, cut on a char boundary
pub fn clamp(s: &str, max: usize) -> &str {
  if s.len() <= max {
    return s;
  }
  let mut end = max;
  while !s.is_char_boundary(end) {
    end -= 1;
  }
  &s[..end]
}

// one url path segment, RFC 3986 unreserved bytes kept and everything else %XX
pub fn path_seg(s: &str) -> String {
  let mut out = String::with_capacity(s.len());
  for b in s.bytes() {
    if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
      out.push(b as char);
    } else {
      out.push_str(&format!("%{b:02X}"));
    }
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn path_segments_keep_only_unreserved() {
    assert_eq!(path_seg("ab/cd+e=="), "ab%2Fcd%2Be%3D%3D");
    assert_eq!(path_seg("01ABC-x_y.z~"), "01ABC-x_y.z~");
  }
}
