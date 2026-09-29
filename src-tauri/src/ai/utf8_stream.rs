// Incremental UTF-8 decoding of a token stream.
//
// A model emits text as byte pieces, one per token, and a multi-byte character
// (an accented letter, CJK, an emoji) is often split across two or more tokens.
// Decoding each piece on its own turns every split character into U+FFFD or
// drops it. `Utf8Stream` keeps the incomplete tail of one piece until the next
// completes it.
//
// Not gated behind `llamacpp`: it is pure, so the `--no-default-features` CI
// jobs test it.

/// Turns a sequence of byte pieces into text, holding back an incomplete
/// UTF-8 sequence at the end of a piece until a later piece completes it.
#[derive(Debug, Default)]
pub struct Utf8Stream {
    pending: Vec<u8>,
}

impl Utf8Stream {
    pub fn new() -> Self {
        Self::default()
    }

    /// The text that `piece` completes. Bytes that can never be valid UTF-8
    /// become U+FFFD; an incomplete sequence at the end is kept for the next
    /// call.
    pub fn push(&mut self, piece: &[u8]) -> String {
        self.pending.extend_from_slice(piece);
        let mut out = String::new();
        let mut rest: &[u8] = &self.pending;
        loop {
            match std::str::from_utf8(rest) {
                Ok(valid) => {
                    out.push_str(valid);
                    rest = &[];
                    break;
                }
                Err(e) => {
                    let (valid, after) = rest.split_at(e.valid_up_to());
                    // `valid_up_to` marks a valid prefix by definition.
                    out.push_str(&String::from_utf8_lossy(valid));
                    match e.error_len() {
                        // An incomplete sequence at the end: wait for more.
                        None => {
                            rest = after;
                            break;
                        }
                        Some(bad) => {
                            out.push(char::REPLACEMENT_CHARACTER);
                            rest = &after[bad..];
                        }
                    }
                }
            }
        }
        self.pending = rest.to_vec();
        out
    }

    /// Whatever is still held back when the stream ends, as U+FFFD — the
    /// sequence was never completed.
    pub fn finish(&mut self) -> String {
        let out = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending.clear();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(pieces: &[&[u8]]) -> (Vec<String>, String) {
        let mut stream = Utf8Stream::new();
        let emitted = pieces.iter().map(|p| stream.push(p)).collect();
        (emitted, stream.finish())
    }

    #[test]
    fn ascii_pieces_pass_straight_through() {
        let (emitted, tail) = decode(&[b"Hel", b"lo"]);
        assert_eq!(emitted, vec!["Hel", "lo"]);
        assert_eq!(tail, "");
    }

    #[test]
    fn a_character_split_across_two_pieces_is_emitted_whole_once_complete() {
        // "é" is C3 A9.
        let (emitted, tail) = decode(&[b"caf\xC3", b"\xA9!"]);
        assert_eq!(emitted, vec!["caf", "é!"]);
        assert_eq!(tail, "");
    }

    #[test]
    fn a_four_byte_emoji_split_one_byte_per_piece_survives() {
        // U+1F600 is F0 9F 98 80.
        let (emitted, tail) = decode(&[b"\xF0", b"\x9F", b"\x98", b"\x80"]);
        assert_eq!(emitted.concat(), "\u{1F600}");
        assert_eq!(emitted[..3], ["", "", ""]);
        assert_eq!(tail, "");
    }

    #[test]
    fn invalid_bytes_become_a_replacement_character_and_decoding_goes_on() {
        let (emitted, tail) = decode(&[b"a\xFFb"]);
        assert_eq!(emitted, vec!["a\u{FFFD}b"]);
        assert_eq!(tail, "");
    }

    #[test]
    fn an_unfinished_sequence_at_the_end_is_flushed_as_a_replacement() {
        let (emitted, tail) = decode(&[b"x\xE2\x82"]);
        assert_eq!(emitted, vec!["x"]);
        assert_eq!(tail, "\u{FFFD}");
    }
}
