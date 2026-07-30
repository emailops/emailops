//! Tolerant handling of untagged IMAP responses that `imap` 2.4.1 parses too
//! strictly.
//!
//! `imap::Session::uid_search`, `::select` and `::uid_fetch` all fail the
//! *entire* command when the server interleaves any untagged response their
//! typed parser does not specifically expect (`parse_ids` / `parse_mailbox` /
//! `parse_fetches`, respectively). Each has its own allowlist — e.g.
//! `parse_ids`'s `handle_unilateral` only tolerates `STATUS`, `RECENT`,
//! `FLAGS`, `EXISTS` and `EXPUNGE` — and anything outside it, most commonly an
//! unsolicited `* n FETCH (FLAGS (\Seen) UID n)` emitted when another client
//! touches the same mailbox, is turned into
//! `Error::Parse(ParseError::Unexpected(_))`, which renders as the opaque
//! *"Encountered unexpected parse response"*.
//!
//! RFC 3501 §7 is explicit that "the client MUST be prepared to accept any
//! response at all times", so the strictness is a bug in the client library,
//! not a server fault. This module works around each command differently,
//! chosen by how safe it is to bypass the typed parser for that command:
//!
//! - [`uid_search`]: reads the raw response and picks out `* SEARCH` lines
//!   itself. Safe because a SEARCH response is plain whitespace-separated
//!   decimal ids — no literals, nothing that needs protocol-aware parsing.
//! - [`select`]: reads the raw response and checks only for the tagged
//!   completion, discarding the untagged `Mailbox` payload (UIDVALIDITY,
//!   UIDNEXT, flags, …) unparsed. Safe because none of this codebase's
//!   `select_folder_blocking` callers use that payload — they only care
//!   whether SELECT succeeded.
//! - [`uid_fetch_rfc822`]: still calls the crate's typed `uid_fetch` (and
//!   therefore its literal-boundary-aware parsing of the RFC822 body — the
//!   one piece of this bug class it is not safe to hand-roll, since getting a
//!   `{n}` byte count wrong risks silently truncating or corrupting email
//!   content), but retries on `ParseError::Unexpected`. The interleave is a
//!   race against another client mutating the mailbox at that instant, so an
//!   immediate retry essentially never repeats it.

use std::io::{Read, Write};

use crate::models::error::{AppError, Result};

/// Run `UID SEARCH <query>` on an open session, tolerating any untagged
/// response the server interleaves.
///
/// Use this instead of [`imap::Session::uid_search`] — see the module docs for
/// why that method fails whole syncs. `run_command_and_read_response` still
/// turns a tagged `NO`/`BAD` into an `Err`, so reaching the parser means the
/// server reported the search as successful and its result is authoritative
/// (including "no matches").
pub(crate) fn uid_search<T: Read + Write>(session: &mut imap::Session<T>, query: &str) -> Result<Vec<u32>> {
    let raw = session
        .run_command_and_read_response(format!("UID SEARCH {query}"))
        .map_err(|e| AppError::SyncError(format!("IMAP SEARCH failed: {e}")))?;

    let parsed = parse_search_response(&raw);

    // Interleaved untagged responses are ordinary protocol traffic, so they are
    // dropped silently — a chatty server emits them on most syncs and logging
    // each one would only bury the output panel.
    //
    // A search that returned no result line *and* something we could not
    // classify is a different matter: it is not a genuine empty mailbox, and
    // treating it as one is how a sync silently stops fetching mail.
    if !parsed.saw_search_line {
        if let Some(line) = parsed.ignored.first() {
            return Err(AppError::SyncError(format!(
                "IMAP SEARCH returned no result line; server said: {}",
                truncate_for_error(line)
            )));
        }
    }

    Ok(parsed.ids)
}

/// SELECT a mailbox, tolerating any untagged response the server interleaves.
/// See the module docs for why this bypasses the crate's typed `Mailbox`
/// parsing rather than making it lenient.
pub(crate) fn select<T: Read + Write>(session: &mut imap::Session<T>, mailbox_name: &str) -> Result<()> {
    if mailbox_name.contains(['\r', '\n']) {
        return Err(AppError::SyncError(
            "IMAP SELECT rejected: mailbox name contains a line break".to_string(),
        ));
    }
    let quoted = format!("\"{}\"", mailbox_name.replace('\\', "\\\\").replace('"', "\\\""));
    session
        .run_command_and_read_response(format!("SELECT {quoted}"))
        .map(|_| ())
        .map_err(|e| AppError::SyncError(format!("IMAP SELECT failed: {e}")))
}

/// How many times to retry `UID FETCH … RFC822` after the typed parser
/// rejects an interleaved untagged response before giving up. See the module
/// docs for why this is a retry rather than a reimplementation.
const FETCH_RETRY_ATTEMPTS: u32 = 3;

/// Fetch the raw RFC822 body of `uid`, retrying past interleaved-response
/// failures. See the module docs for the tradeoff behind this approach.
pub(crate) fn uid_fetch_rfc822<T: Read + Write>(session: &mut imap::Session<T>, uid: u32) -> Result<Vec<u8>> {
    let mut last_err: Option<imap::Error> = None;
    for attempt in 1..=FETCH_RETRY_ATTEMPTS {
        match session.uid_fetch(uid.to_string(), "RFC822") {
            Ok(messages) => {
                return messages
                    .iter()
                    .next()
                    .and_then(|f| f.body())
                    .map(<[u8]>::to_vec)
                    .ok_or_else(|| AppError::NotFound(format!("IMAP UID {uid}: body not found")));
            }
            // Interleaved untagged responses are ordinary protocol traffic
            // (see the module docs) — not logged, for the same reason
            // `uid_search` doesn't log its ignored lines: a chatty server
            // would bury the output panel on most syncs.
            Err(imap::Error::Parse(imap::error::ParseError::Unexpected(_))) if attempt < FETCH_RETRY_ATTEMPTS => {}
            Err(e) => {
                last_err = Some(e);
                break;
            }
        }
    }
    Err(AppError::SyncError(format!(
        "IMAP FETCH failed: {}",
        last_err
            .map(|e| e.to_string())
            .unwrap_or_else(|| "server kept returning interleaved responses".to_string())
    )))
}

/// Outcome of scanning the untagged portion of a `SEARCH` response.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct SearchResponse {
    /// Message ids (UIDs for `UID SEARCH`), sorted ascending and deduplicated.
    pub ids: Vec<u32>,
    /// Whether at least one untagged `* SEARCH` line was present. A compliant
    /// server always sends one — bare (`* SEARCH\r\n`) when nothing matched.
    pub saw_search_line: bool,
    /// Untagged lines that were not `* SEARCH` results, kept verbatim for
    /// diagnostics. Never surfaced wholesale to the user.
    pub ignored: Vec<String>,
}

/// Scan the untagged lines of a `SEARCH` response.
///
/// `raw` is what `Session::run_command_and_read_response` returns: every
/// untagged line of the response, with the tagged completion line already
/// stripped. A tagged `NO`/`BAD` has already been turned into an `Err` by the
/// caller, so reaching this function means the server executed the search.
///
/// Lines are split on CRLF. A literal (`{n}`) payload inside an interleaved
/// response would therefore be scanned as if it were response lines; during a
/// `SEARCH` no server sends unsolicited literals, and the worst case is an
/// extra entry in `ignored`.
pub(crate) fn parse_search_response(raw: &[u8]) -> SearchResponse {
    let text = String::from_utf8_lossy(raw);
    let mut out = SearchResponse::default();

    for line in text.split("\r\n").flat_map(|l| l.split('\n')) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match search_ids_in_line(line) {
            Some(ids) => {
                out.saw_search_line = true;
                out.ids.extend(ids);
            }
            None => out.ignored.push(line.to_string()),
        }
    }

    out.ids.sort_unstable();
    out.ids.dedup();
    out
}

/// Parse one untagged line, returning its ids when it is a `* SEARCH` result
/// line and `None` for every other response.
fn search_ids_in_line(line: &str) -> Option<Vec<u32>> {
    let rest = line.strip_prefix('*')?.trim_start();
    let rest = strip_keyword(rest, "SEARCH")?;

    let mut ids = Vec::new();
    for token in rest.split_whitespace() {
        // RFC 4551 appends a `(MODSEQ n)` trailer to the SEARCH response when
        // the search used a MODSEQ key. Anything parenthesised ends the ids.
        if token.starts_with('(') {
            break;
        }
        match token.parse::<u32>() {
            Ok(id) => ids.push(id),
            // A non-numeric token means this is not a plain result line after
            // all (e.g. a future extension). Keep what we read and let the
            // caller see the line in `ignored`.
            Err(_) => return None,
        }
    }
    Some(ids)
}

/// Consume `keyword` case-insensitively when it is followed by whitespace or
/// the end of the line, returning the remainder. `None` when the line starts
/// with a different (or longer) atom — `SEARCHRES` must not match `SEARCH`.
fn strip_keyword<'a>(input: &'a str, keyword: &str) -> Option<&'a str> {
    let (head, tail) = input.split_at_checked(keyword.len())?;
    if !head.eq_ignore_ascii_case(keyword) {
        return None;
    }
    if tail.is_empty() || tail.starts_with(char::is_whitespace) {
        Some(tail)
    } else {
        None
    }
}

/// Shorten a server response line for inclusion in a user-facing error.
/// Server text can echo mailbox names, so only a bounded prefix is kept.
pub(crate) fn truncate_for_error(line: &str) -> String {
    const MAX: usize = 120;
    let mut end = MAX.min(line.len());
    while end > 0 && !line.is_char_boundary(end) {
        end -= 1;
    }
    if end < line.len() {
        format!("{}…", &line[..end])
    } else {
        line.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A canned IMAP server: hands back `script` on reads and swallows writes.
    /// Enough to drive `Client::login` + one command, which is all these tests
    /// need to exercise the real `imap` crate's response handling.
    struct ScriptedStream {
        script: std::io::Cursor<Vec<u8>>,
    }

    impl ScriptedStream {
        fn new(script: &str) -> Self {
            Self {
                script: std::io::Cursor::new(script.as_bytes().to_vec()),
            }
        }
    }

    impl Read for ScriptedStream {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.script.read(buf)
        }
    }

    impl Write for ScriptedStream {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// `login` consumes tag `a1`, so the first command under test answers to
    /// `a2`, the second (if any, concatenated in `remaining`) to `a3`, etc.
    fn session_for(remaining: &str) -> imap::Session<ScriptedStream> {
        let script = format!("a1 OK Logged in.\r\n{remaining}");
        match imap::Client::new(ScriptedStream::new(&script)).login("u", "p") {
            Ok(session) => session,
            Err((e, _)) => panic!("scripted login failed: {e}"),
        }
    }

    /// The reported bug, reproduced against the real crate: an unsolicited
    /// FETCH interleaved with the results makes `Session::uid_search` fail the
    /// whole command with "Encountered unexpected parse response".
    #[test]
    fn the_crate_search_still_rejects_an_interleaved_fetch() {
        let response = "* SEARCH 1 2 3\r\n\
                        * 7 FETCH (FLAGS (\\Seen) UID 91)\r\n\
                        a2 OK Search completed.\r\n";
        let err = match session_for(response).uid_search("ALL") {
            Ok(ids) => panic!("expected the crate to reject this response, got {ids:?}"),
            Err(e) => e.to_string(),
        };
        assert_eq!(err, "Encountered unexpected parse response");
    }

    /// …and the same response goes through our path intact.
    #[test]
    fn lenient_uid_search_survives_an_interleaved_fetch() {
        let response = "* SEARCH 1 2 3\r\n\
                        * 7 FETCH (FLAGS (\\Seen) UID 91)\r\n\
                        a2 OK Search completed.\r\n";
        let ids = match uid_search(&mut session_for(response), "ALL") {
            Ok(ids) => ids,
            Err(e) => panic!("lenient search failed: {e}"),
        };
        assert_eq!(ids, vec![1, 2, 3]);
    }

    #[test]
    fn lenient_uid_search_reports_an_empty_mailbox_as_empty() {
        let response = "* SEARCH\r\na2 OK Search completed.\r\n";
        assert_eq!(uid_search(&mut session_for(response), "ALL").ok(), Some(vec![]));
    }

    #[test]
    fn lenient_uid_search_propagates_a_tagged_no() {
        let response = "a2 NO [SERVERBUG] Internal error\r\n";
        let err = match uid_search(&mut session_for(response), "ALL") {
            Ok(ids) => panic!("expected a failure, got {ids:?}"),
            Err(e) => e.to_string(),
        };
        assert!(err.contains("IMAP SEARCH failed"), "got: {err}");
    }

    #[test]
    fn lenient_uid_search_errors_rather_than_reporting_a_bogus_empty_result() {
        // No `* SEARCH` line at all, but the server did say something: this is
        // not an empty mailbox, and pretending it is would silently halt sync.
        let response = "* 7 FETCH (FLAGS (\\Seen))\r\na2 OK Search completed.\r\n";
        let err = match uid_search(&mut session_for(response), "ALL") {
            Ok(ids) => panic!("expected a failure, got {ids:?}"),
            Err(e) => e.to_string(),
        };
        assert!(err.contains("no result line"), "got: {err}");
    }

    /// The same bug class, reproduced against `Session::select`: an
    /// unsolicited FETCH mid-SELECT makes the crate reject the whole command.
    #[test]
    fn the_crate_select_still_rejects_an_interleaved_fetch() {
        let response = "* 12 EXISTS\r\n\
                        * 7 FETCH (FLAGS (\\Seen) UID 91)\r\n\
                        a2 OK [READ-WRITE] Select completed.\r\n";
        let err = match session_for(response).select("INBOX") {
            Ok(mailbox) => panic!("expected the crate to reject this response, got {mailbox:?}"),
            Err(e) => e.to_string(),
        };
        assert_eq!(err, "Encountered unexpected parse response");
    }

    #[test]
    fn lenient_select_survives_an_interleaved_fetch() {
        let response = "* 12 EXISTS\r\n\
                        * 7 FETCH (FLAGS (\\Seen) UID 91)\r\n\
                        a2 OK [READ-WRITE] Select completed.\r\n";
        assert!(select(&mut session_for(response), "INBOX").is_ok());
    }

    #[test]
    fn lenient_select_propagates_a_tagged_no() {
        let response = "a2 NO Mailbox does not exist\r\n";
        let err = match select(&mut session_for(response), "Nonexistent") {
            Ok(()) => panic!("expected a failure"),
            Err(e) => e.to_string(),
        };
        assert!(err.contains("IMAP SELECT failed"), "got: {err}");
    }

    #[test]
    fn lenient_select_rejects_a_mailbox_name_with_a_line_break() {
        // A raw newline in the mailbox name would let it inject a second IMAP
        // command; reject it before it ever reaches the wire.
        let err = match select(&mut session_for("a2 OK done\r\n"), "INBOX\r\nA2 LOGOUT") {
            Ok(()) => panic!("expected a failure"),
            Err(e) => e.to_string(),
        };
        assert!(err.contains("line break"), "got: {err}");
    }

    #[test]
    fn lenient_select_quotes_and_escapes_the_mailbox_name() {
        // A folder literally named `Alice's "Inbox"` must round-trip through
        // IMAP's quoted-string escaping without breaking the command.
        let mut session = session_for("a2 OK done\r\n");
        assert!(select(&mut session, "Alice's \"Inbox\"").is_ok());
    }

    /// The same bug class, reproduced against `Session::uid_fetch`: an
    /// unsolicited SEARCH mid-FETCH makes the crate reject the whole command,
    /// even though the FETCH data itself parsed fine.
    #[test]
    fn the_crate_fetch_still_rejects_an_interleaved_search() {
        let response = "* 1 FETCH (UID 91 RFC822 {5}\r\nhello)\r\n\
                        * SEARCH 1 2\r\n\
                        a2 OK Fetch completed.\r\n";
        let err = match session_for(response).uid_fetch("91", "RFC822") {
            Ok(fetches) => panic!("expected the crate to reject this response, got {fetches:?}"),
            Err(e) => e.to_string(),
        };
        assert_eq!(err, "Encountered unexpected parse response");
    }

    #[test]
    fn uid_fetch_rfc822_retries_past_an_interleaved_search_and_returns_the_body() {
        let response = "* 1 FETCH (UID 91 RFC822 {5}\r\nhello)\r\n\
                        * SEARCH 1 2\r\n\
                        a2 OK Fetch completed.\r\n\
                        * 1 FETCH (UID 91 RFC822 {5}\r\nhello)\r\n\
                        a3 OK Fetch completed.\r\n";
        let body = match uid_fetch_rfc822(&mut session_for(response), 91) {
            Ok(body) => body,
            Err(e) => panic!("expected the retry to succeed, got: {e}"),
        };
        assert_eq!(body, b"hello");
    }

    #[test]
    fn uid_fetch_rfc822_gives_up_after_exhausting_its_retries() {
        let failing = "* 1 FETCH (UID 91 RFC822 {5}\r\nhello)\r\n\
                       * SEARCH 1 2\r\n\
                       aN OK Fetch completed.\r\n";
        // FETCH_RETRY_ATTEMPTS attempts, tags a2..a4.
        let script: String = (2..=4).map(|tag| failing.replace("aN", &format!("a{tag}"))).collect();
        let err = match uid_fetch_rfc822(&mut session_for(&script), 91) {
            Ok(body) => panic!("expected every attempt to fail, got {body:?}"),
            Err(e) => e.to_string(),
        };
        assert!(err.contains("IMAP FETCH failed"), "got: {err}");
    }

    #[test]
    fn uid_fetch_rfc822_propagates_a_tagged_no_without_retrying() {
        let response = "a2 NO [SERVERBUG] Internal error\r\n";
        let err = match uid_fetch_rfc822(&mut session_for(response), 91) {
            Ok(body) => panic!("expected a failure, got {body:?}"),
            Err(e) => e.to_string(),
        };
        assert!(err.contains("IMAP FETCH failed"), "got: {err}");
    }

    #[test]
    fn uid_fetch_rfc822_reports_a_missing_body_as_not_found() {
        let response = "* 1 FETCH (UID 91 FLAGS (\\Seen))\r\na2 OK Fetch completed.\r\n";
        let err = match uid_fetch_rfc822(&mut session_for(response), 91) {
            Ok(body) => panic!("expected a failure, got {body:?}"),
            Err(e) => e.to_string(),
        };
        assert!(err.contains("body not found"), "got: {err}");
    }

    #[test]
    fn collects_ids_from_a_plain_search_line() {
        let parsed = parse_search_response(b"* SEARCH 23 42 4711\r\n");
        assert_eq!(parsed.ids, vec![23, 42, 4711]);
        assert!(parsed.saw_search_line);
        assert!(parsed.ignored.is_empty());
    }

    #[test]
    fn merges_ids_split_across_several_search_lines() {
        let parsed = parse_search_response(b"* SEARCH 1 2 3\r\n* SEARCH 4 5\r\n");
        assert_eq!(parsed.ids, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn an_empty_result_still_counts_as_a_search_line() {
        let parsed = parse_search_response(b"* SEARCH\r\n");
        assert!(parsed.ids.is_empty());
        assert!(parsed.saw_search_line);
    }

    /// The reported failure: another client touching the mailbox makes the
    /// server interleave an unsolicited FETCH, which `uid_search` rejects with
    /// "Encountered unexpected parse response".
    #[test]
    fn tolerates_an_unsolicited_fetch_between_search_lines() {
        let raw = b"* SEARCH 1 2 3\r\n\
                    * 7 FETCH (FLAGS (\\Seen) UID 91)\r\n\
                    * SEARCH 4 5\r\n";
        let parsed = parse_search_response(raw);
        assert_eq!(parsed.ids, vec![1, 2, 3, 4, 5]);
        assert_eq!(parsed.ignored, vec!["* 7 FETCH (FLAGS (\\Seen) UID 91)"]);
    }

    #[test]
    fn tolerates_the_other_untagged_responses_a_server_may_interleave() {
        let raw = b"* 12 EXISTS\r\n\
                    * 1 RECENT\r\n\
                    * OK [UIDNEXT 4392] Predicted next UID\r\n\
                    * 3 EXPUNGE\r\n\
                    * FLAGS (\\Answered \\Seen)\r\n\
                    * SEARCH 9\r\n";
        let parsed = parse_search_response(raw);
        assert_eq!(parsed.ids, vec![9]);
        assert_eq!(parsed.ignored.len(), 5);
    }

    #[test]
    fn drops_the_condstore_modseq_trailer() {
        let parsed = parse_search_response(b"* SEARCH 2 5 6 (MODSEQ 917162500)\r\n");
        assert_eq!(parsed.ids, vec![2, 5, 6]);
    }

    #[test]
    fn sorts_and_deduplicates_ids() {
        let parsed = parse_search_response(b"* SEARCH 5 1 5\r\n* SEARCH 1 3\r\n");
        assert_eq!(parsed.ids, vec![1, 3, 5]);
    }

    #[test]
    fn a_response_with_no_search_line_is_reported_as_such() {
        let parsed = parse_search_response(b"* 7 FETCH (FLAGS (\\Seen))\r\n");
        assert!(!parsed.saw_search_line);
        assert!(parsed.ids.is_empty());
        assert_eq!(parsed.ignored.len(), 1);
    }

    #[test]
    fn an_entirely_empty_response_is_an_empty_result() {
        let parsed = parse_search_response(b"");
        assert_eq!(parsed, SearchResponse::default());
    }

    #[test]
    fn matches_the_search_keyword_case_insensitively() {
        assert_eq!(parse_search_response(b"* search 8\r\n").ids, vec![8]);
    }

    #[test]
    fn does_not_treat_a_longer_atom_as_a_search_result() {
        let parsed = parse_search_response(b"* SEARCHRES 1 2\r\n");
        assert!(!parsed.saw_search_line);
        assert_eq!(parsed.ignored, vec!["* SEARCHRES 1 2"]);
    }

    #[test]
    fn a_search_line_with_a_non_numeric_token_is_not_treated_as_results() {
        // `* ESEARCH`-style extensions and anything else unforeseen must not
        // silently contribute a partial id list.
        let parsed = parse_search_response(b"* SEARCH 1 2 THREE\r\n");
        assert!(!parsed.saw_search_line);
        assert!(parsed.ids.is_empty());
    }

    #[test]
    fn tolerates_bare_lf_line_endings() {
        let parsed = parse_search_response(b"* SEARCH 1 2\n* SEARCH 3\n");
        assert_eq!(parsed.ids, vec![1, 2, 3]);
    }

    #[test]
    fn ids_at_the_u32_boundary_survive() {
        let parsed = parse_search_response(b"* SEARCH 4294967295\r\n");
        assert_eq!(parsed.ids, vec![u32::MAX]);
    }

    #[test]
    fn an_out_of_range_id_is_not_silently_dropped() {
        // 2^32 does not fit a UID; treat the line as unrecognised rather than
        // returning a short list that looks like a legitimate result.
        let parsed = parse_search_response(b"* SEARCH 4294967296\r\n");
        assert!(!parsed.saw_search_line);
    }

    #[test]
    fn truncate_for_error_bounds_long_lines() {
        let long = "* OK ".to_string() + &"x".repeat(400);
        let short = truncate_for_error(&long);
        assert!(short.len() <= 124, "got {} bytes", short.len());
        assert!(short.ends_with('…'));
    }

    #[test]
    fn truncate_for_error_leaves_short_lines_alone() {
        assert_eq!(truncate_for_error("* OK done"), "* OK done");
    }

    #[test]
    fn truncate_for_error_never_splits_a_multibyte_character() {
        let line = "á".repeat(200);
        let short = truncate_for_error(&line);
        assert!(short.ends_with('…'));
        // Would have panicked on a non-boundary slice.
        assert!(short.chars().all(|c| c == 'á' || c == '…'));
    }
}
