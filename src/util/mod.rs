pub mod mem;
pub mod time;

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
