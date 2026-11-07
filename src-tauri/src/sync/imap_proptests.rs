//! Property tests for the IMAP adapter's pure parts: the id codecs, the
//! inbox pagination, the SEARCH window and the parsers that read server
//! bytes. Server bytes are untrusted input, so "never panics" is a property
//! of its own; the guarded `parse_message` would hide a panic, so the
//! unguarded half is the one checked.

use proptest::prelude::*;

use super::*;
use crate::sync::imap_search::{parse_flags_response, parse_search_response, parse_uid_validity, FetchedMessage};

fn client() -> ImapClient {
    ImapClient::new(
        ImapCredentials {
            host: "imap.example.test".to_string(),
            port: 993,
            username: "u".to_string(),
            password: "p".to_string(),
            smtp_host: "smtp.example.test".to_string(),
            smtp_port: 465,
        },
        "u@example.test".to_string(),
        String::new(),
        "acct".to_string(),
    )
}

/// Arbitrary bytes, biased toward IMAP-looking lines so the parsers get past
/// their first checks.
fn server_bytes() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        proptest::collection::vec(any::<u8>(), 0..400),
        "(\\* (SEARCH|OK|[0-9]{1,4} FETCH|[0-9]{1,3} EXISTS)[ -~]{0,80}\r\n){0,6}".prop_map(String::into_bytes),
    ]
}

/// Timestamps with the extremes over-represented: a corrupt watermark is
/// where arithmetic on them breaks.
fn edge_timestamp() -> impl Strategy<Value = Option<i64>> {
    prop_oneof![
        Just(None),
        any::<i64>().prop_map(Some),
        prop_oneof![Just(i64::MIN), Just(i64::MIN + 1), Just(-1), Just(0), Just(i64::MAX)].prop_map(Some),
    ]
}

proptest! {
    #[test]
    fn any_folder_path_survives_the_id_encoding(path in "\\PC{0,40}") {
        prop_assert_eq!(decode_folder_path(&encode_folder_path(&path)), Some(path));
    }

    #[test]
    fn a_custom_folder_id_parses_back_to_its_folder_and_uid(path in "\\PC{1,40}", uid in any::<u32>()) {
        let client = client();
        let id = client.make_folder_email_id(&path, uid);
        let (folder, uid_str) = client.parse_message_ref(&id);
        prop_assert_eq!(folder, ImapFolder::Custom(path));
        let expected_uid = uid.to_string();
        prop_assert_eq!(uid_str, expected_uid.as_str());
    }

    #[test]
    fn a_custom_folder_id_starts_with_the_folder_prefix(path in "\\PC{1,40}", uid in any::<u32>()) {
        let id = client().make_folder_email_id(&path, uid);
        prop_assert!(id.starts_with(&folder_email_id_prefix("acct", &path)));
    }

    /// Walking the pages visits every UID exactly once, newest first.
    #[test]
    fn paging_the_inbox_visits_every_uid_once_newest_first(
        uids in proptest::collection::btree_set(1u32..100_000, 0..300),
        page in 1u32..60,
    ) {
        let all: Vec<u32> = uids.iter().copied().collect();
        let mut seen = Vec::new();
        let mut token: Option<String> = None;
        for _ in 0..=all.len() {
            let page = plan_inbox_page(all.clone(), token.as_deref(), page);
            prop_assert!(page.uids.windows(2).all(|w| w[0] > w[1]));
            seen.extend(page.uids);
            match page.next_token {
                Some(next) => token = Some(next),
                None => break,
            }
        }
        let mut expected = all;
        expected.reverse();
        prop_assert_eq!(seen, expected);
    }

    /// The window never panics, and pads outward: SINCE the day before, BEFORE the day after.
    #[test]
    fn the_search_window_is_well_formed_for_any_timestamps(after in edge_timestamp(), before in edge_timestamp()) {
        let query = build_search_query(after, before);
        prop_assert!(!query.is_empty());
        prop_assert!(!query.contains('\r') && !query.contains('\n'));
    }

    #[test]
    fn search_responses_never_panic_and_keep_listed_ids(
        ids in proptest::collection::vec(any::<u32>(), 0..30),
        noise in server_bytes(),
    ) {
        let _ = parse_search_response(&noise);
        let line = format!(
            "* SEARCH {}\r\n",
            ids.iter().map(u32::to_string).collect::<Vec<_>>().join(" ")
        );
        let parsed = parse_search_response(line.as_bytes());
        let mut expected = ids;
        expected.sort_unstable();
        expected.dedup();
        prop_assert_eq!(parsed.ids, expected);
        prop_assert!(parsed.saw_search_line);
    }

    #[test]
    fn flags_responses_never_panic(raw in server_bytes()) {
        let _ = parse_flags_response(&raw);
    }

    #[test]
    fn a_flags_line_reports_seen_and_flagged(uid in any::<u32>(), seq in 1u32..10_000, seen: bool, flagged: bool) {
        let mut flags = Vec::new();
        if seen { flags.push("\\Seen"); }
        if flagged { flags.push("\\Flagged"); }
        let raw = format!("* {seq} FETCH (UID {uid} FLAGS ({}))\r\n", flags.join(" "));
        let parsed = parse_flags_response(raw.as_bytes());
        prop_assert_eq!(parsed.len(), 1);
        prop_assert_eq!(parsed[0].0, uid);
        prop_assert_eq!(parsed[0].1.seen, seen);
        prop_assert_eq!(parsed[0].1.flagged, flagged);
    }

    #[test]
    fn uid_validity_never_panics_and_reads_its_code(raw in server_bytes(), value in any::<u32>()) {
        let _ = parse_uid_validity(&raw);
        let response = format!("* OK [UIDVALIDITY {value}] UIDs valid\r\n");
        prop_assert_eq!(parse_uid_validity(response.as_bytes()), Some(value));
    }

    #[test]
    fn address_headers_never_panic(raw in "\\PC{0,200}") {
        let _ = parse_from_header(&raw);
        let _ = parse_address_list(&raw);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// Any bytes a server hands back as a message body parse to an email or an
    /// error, never a panic.
    #[test]
    fn any_message_bytes_parse_without_panicking(
        raw in prop_oneof![
            proptest::collection::vec(any::<u8>(), 0..2_000),
            (
                "[ -~]{0,60}",
                "(text/plain|text/html|multipart/mixed|multipart/alternative|application/pdf)",
                "[ -~\r\n]{0,400}",
            ).prop_map(|(subject, ctype, body)| format!(
                "Subject: {subject}\r\nFrom: x@example.test\r\nContent-Type: {ctype}; boundary=\"b\"\r\n\r\n--b\r\n{body}\r\n--b--\r\n"
            ).into_bytes()),
        ],
        internal_date in any::<Option<i64>>(),
    ) {
        let fetched = FetchedMessage { raw, internal_date, seen: false, flagged: false };
        let _ = ImapClient::parse_message_unguarded(1, &fetched);
    }
}
