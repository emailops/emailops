/// Truncate `s` to at most `max_bytes` bytes, walking back to the previous
/// UTF-8 char boundary so we never split a multibyte character.
pub fn truncate_utf8(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}
