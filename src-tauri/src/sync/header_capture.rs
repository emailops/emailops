//! Provider-agnostic RFC 5322 header capture.
//!
//! One pure function, three call sites (Gmail, IMAP, Graph). Gmail and IMAP
//! already fetch the full header set and discard it; only Graph needs the fetch
//! widened. Keeping the normalization in one place means the three providers
//! cannot drift apart in how they read an authentication result.

use crate::models::headers::RawHeaders;

/// Headers copied into `RawHeaders::extra` verbatim. Anything not named here
/// and not mapped to a typed field is dropped — we store what the detector
/// needs, not everything the sender sent.
const EXTRA_ALLOWLIST: &[&str] = &[
    "x-priority",
    "x-msmail-priority",
    "x-originating-ip",
    "x-authenticated-sender",
    "auto-submitted",
    "feedback-id",
];

fn is_spam_header(name: &str) -> bool {
    name.starts_with("x-spam") || name == "x-rspamd-score" || name == "x-rspamd-action"
}

/// Normalize a provider's header list into the stored subset.
///
/// `headers` must be in message order (topmost first). Both properties that
/// matter are order-dependent:
///
/// * `Authentication-Results` takes the **first** instance — see
///   [`RawHeaders::auth_results`].
/// * `first_received` takes the **last** `Received`, which is the origin hop.
///   Relays prepend, so the bottom-most is the one furthest from us and the one
///   an attacker cannot push down the list.
pub fn capture(headers: &[(String, String)]) -> RawHeaders {
    let mut out = RawHeaders::default();
    let mut spam_bits: Vec<String> = Vec::new();

    for (raw_name, raw_value) in headers {
        let name = raw_name.trim().to_lowercase();
        let value = raw_value.trim();
        if value.is_empty() {
            continue;
        }

        match name.as_str() {
            // First instance wins.
            "authentication-results" => {
                if out.auth_results.is_none() {
                    out.auth_results = Some(value.to_string());
                    out.authserv_id = parse_authserv_id(value);
                }
            }
            "received-spf" => set_once(&mut out.received_spf, value),
            "dkim-signature" => {
                if let Some(domain) = parse_dkim_domain(value) {
                    out.dkim_domains.push(domain);
                }
            }
            "received" => {
                out.received_count += 1;
                // Last one wins: the bottom-most Received is the origin hop.
                out.first_received = Some(value.to_string());
            }
            "return-path" => set_once(&mut out.return_path, value),
            "reply-to" => set_once(&mut out.reply_to, value),
            "from" => set_once(&mut out.from_raw, value),
            "to" => set_once(&mut out.to_raw, value),
            "list-id" => set_once(&mut out.list_id, value),
            "list-unsubscribe" => set_once(&mut out.list_unsubscribe, value),
            "list-unsubscribe-post" => set_once(&mut out.list_unsubscribe_post, value),
            "precedence" => set_once(&mut out.precedence, value),
            "x-mailer" => set_once(&mut out.x_mailer, value),
            "content-type" => set_once(&mut out.content_type, value),
            other => {
                if is_spam_header(other) {
                    spam_bits.push(format!("{other}: {value}"));
                } else if EXTRA_ALLOWLIST.contains(&other) {
                    out.extra.entry(other.to_string()).or_insert_with(|| value.to_string());
                }
            }
        }
    }

    if !spam_bits.is_empty() {
        out.spam_headers = Some(spam_bits.join("\n"));
    }

    out
}

fn set_once(slot: &mut Option<String>, value: &str) {
    if slot.is_none() {
        *slot = Some(value.to_string());
    }
}

/// Extract the `authserv-id` — the leading token of an `Authentication-Results`
/// value, before the first `;`.
fn parse_authserv_id(value: &str) -> Option<String> {
    let head = value.split(';').next()?.trim();
    // The authserv-id may be followed by a version number ("mx.example.com 1").
    let id = head.split_whitespace().next()?.trim();
    if id.is_empty() {
        return None;
    }
    Some(id.to_lowercase())
}

/// Pull the `d=` signing domain out of a `DKIM-Signature` value.
fn parse_dkim_domain(value: &str) -> Option<String> {
    for part in value.split(';') {
        let part = part.trim();
        if let Some(rest) = part.strip_prefix("d=") {
            let domain = rest.trim().trim_end_matches(';').trim();
            if !domain.is_empty() {
                return Some(domain.to_lowercase());
            }
        }
    }
    None
}

/// Parse a raw RFC 5322 header block into ordered `(name, value)` pairs,
/// unfolding continuation lines.
///
/// Providers that hand back structured headers (Gmail, Graph) do not need this;
/// it exists for raw-message sources — IMAP `RFC822` fetches and the eval
/// corpus, which stores cases as real header blocks so this path is exercised
/// by the same tests that measure the detector.
pub fn parse_header_block(raw: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();

    for line in raw.lines() {
        if line.trim().is_empty() {
            continue;
        }
        // A leading space or tab continues the previous header (RFC 5322 folding).
        // Authentication-Results is folded by essentially every MTA, so a parser
        // that stops at the newline reads a truncated verdict.
        if line.starts_with(' ') || line.starts_with('\t') {
            if let Some((_, value)) = out.last_mut() {
                value.push(' ');
                value.push_str(line.trim());
            }
            continue;
        }
        if let Some((name, value)) = line.split_once(':') {
            out.push((name.trim().to_string(), value.trim().to_string()));
        }
    }

    out
}

/// Split a `From:`-style value into (display name, addr-spec).
///
/// Handles `"Name" <addr>`, `Name <addr>` and a bare `addr`. The display name is
/// kept separately because impersonation lives there: `"Sam Okafor"
/// <attacker@elsewhere.example>` is the whole BEC trick, and it is invisible if
/// you only ever look at the address.
pub fn split_from_header(from: &str) -> (String, String) {
    let from = from.trim();
    match (from.rfind('<'), from.rfind('>')) {
        (Some(open), Some(close)) if close > open => {
            let addr = from[open + 1..close].trim().to_string();
            let name = from[..open].trim().trim_matches('"').trim().to_string();
            (name, addr)
        }
        _ => (String::new(), from.trim_matches('"').to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(items: &[(&str, &str)]) -> Vec<(String, String)> {
        items
            .iter()
            .map(|(n, v)| ((*n).to_string(), (*v).to_string()))
            .collect()
    }

    #[test]
    fn empty_input_captures_nothing() {
        let captured = capture(&[]);
        assert!(captured.is_empty());
    }

    #[test]
    fn header_names_are_matched_case_insensitively() {
        let captured = capture(&pairs(&[("REPLY-TO", "a@x.example"), ("List-Id", "l.x.example")]));
        assert_eq!(captured.reply_to.as_deref(), Some("a@x.example"));
        assert_eq!(captured.list_id.as_deref(), Some("l.x.example"));
    }

    #[test]
    fn the_topmost_authentication_results_wins() {
        // Security property. The attacker's forged instance sits below the one
        // our own MTA prepended; taking anything but the first would let them
        // declare their own message authentic.
        let captured = capture(&pairs(&[
            ("Authentication-Results", "mx.example.com; spf=fail; dmarc=fail"),
            ("Authentication-Results", "forged.example; spf=pass; dmarc=pass"),
        ]));
        let auth = captured.auth_results.as_deref().expect("captured");
        assert!(auth.contains("spf=fail"), "got {auth:?}");
        assert!(!auth.contains("spf=pass"), "forged instance leaked: {auth:?}");
    }

    #[test]
    fn authserv_id_is_the_leading_token_of_the_auth_results() {
        let captured = capture(&pairs(&[(
            "Authentication-Results",
            "mx.google.com; spf=pass smtp.mailfrom=x.example",
        )]));
        assert_eq!(captured.authserv_id.as_deref(), Some("mx.google.com"));
    }

    #[test]
    fn authserv_id_drops_a_trailing_version_number() {
        // RFC 8601 allows "authserv-id version", e.g. "mx.example.com 1".
        let captured = capture(&pairs(&[("Authentication-Results", "mx.example.com 1; spf=pass")]));
        assert_eq!(captured.authserv_id.as_deref(), Some("mx.example.com"));
    }

    #[test]
    fn the_bottom_most_received_is_kept_as_the_origin_hop() {
        // Relays prepend, so the last Received in the list is the furthest from
        // us — the one an attacker cannot push down by adding more.
        let captured = capture(&pairs(&[
            ("Received", "from relay2.example by mx.example.com"),
            ("Received", "from relay1.example by relay2.example"),
            ("Received", "from origin.example by relay1.example"),
        ]));
        assert_eq!(captured.received_count, 3);
        let first = captured.first_received.as_deref().expect("captured");
        assert!(first.contains("origin.example"), "got {first:?}");
    }

    #[test]
    fn every_dkim_signing_domain_is_collected_in_order() {
        let captured = capture(&pairs(&[
            ("DKIM-Signature", "v=1; a=rsa-sha256; d=first.example; s=sel; b=AAA"),
            ("DKIM-Signature", "v=1; a=rsa-sha256; d=Second.Example; s=sel; b=BBB"),
        ]));
        assert_eq!(
            captured.dkim_domains,
            vec!["first.example".to_string(), "second.example".to_string()]
        );
    }

    #[test]
    fn a_dkim_signature_without_a_domain_is_skipped_not_faked() {
        let captured = capture(&pairs(&[("DKIM-Signature", "v=1; a=rsa-sha256; s=sel; b=AAA")]));
        assert!(captured.dkim_domains.is_empty());
    }

    #[test]
    fn spam_headers_from_the_receiving_server_are_all_collected() {
        // On IMAP this is where SpamAssassin / rspamd put their verdict, and it
        // carries most of the achievable recall for free.
        let captured = capture(&pairs(&[
            ("X-Spam-Flag", "YES"),
            ("X-Spam-Status", "Yes, score=9.8 required=5.0"),
            ("X-Spam-Level", "*********"),
        ]));
        let spam = captured.spam_headers.as_deref().expect("captured");
        assert!(spam.contains("x-spam-flag: YES"), "got {spam:?}");
        assert!(spam.contains("score=9.8"), "got {spam:?}");
    }

    #[test]
    fn headers_outside_the_allowlist_are_dropped() {
        // We store what the detector needs, not everything the sender wrote.
        let captured = capture(&pairs(&[
            ("X-Custom-Tracking-Id", "user-12345"),
            ("X-Some-Vendor-Header", "internal-value"),
        ]));
        assert!(captured.is_empty(), "unexpected capture: {captured:?}");
    }

    #[test]
    fn allowlisted_long_tail_headers_land_in_extra() {
        let captured = capture(&pairs(&[("Auto-Submitted", "auto-generated")]));
        assert_eq!(
            captured.extra.get("auto-submitted").map(String::as_str),
            Some("auto-generated")
        );
    }

    #[test]
    fn empty_values_are_not_captured() {
        let captured = capture(&pairs(&[("Reply-To", "   "), ("List-Id", "")]));
        assert!(captured.is_empty());
    }

    // ── parse_header_block ────────────────────────────────────────────────

    #[test]
    fn folded_continuation_lines_are_unfolded_into_one_value() {
        let raw = concat!(
            "Authentication-Results: mx.example.com;\n",
            "\tspf=fail smtp.mailfrom=evil.example;\n",
            "\tdkim=none; dmarc=fail\n",
        );
        let parsed = parse_header_block(raw);
        assert_eq!(parsed.len(), 1);
        let (_, value) = &parsed[0];
        assert!(value.contains("spf=fail"), "got {value:?}");
        assert!(value.contains("dmarc=fail"), "got {value:?}");
    }

    #[test]
    fn a_parsed_block_feeds_capture_and_preserves_order() {
        // The eval corpus stores cases as raw blocks precisely so this path —
        // parse then capture — is the one under test.
        let raw = concat!(
            "Authentication-Results: mx.example.com; spf=fail; dmarc=fail\n",
            "Authentication-Results: forged.example; spf=pass\n",
            "Received: from relay.example by mx.example.com\n",
            "Received: from origin.example by relay.example\n",
        );
        let captured = capture(&parse_header_block(raw));
        let auth = captured.auth_results.as_deref().expect("captured");
        assert!(auth.contains("spf=fail"), "topmost must survive the round trip");
        assert_eq!(captured.received_count, 2);
        let first = captured.first_received.as_deref().expect("captured");
        assert!(first.contains("origin.example"));
    }

    // ── split_from_header ─────────────────────────────────────────────────

    #[test]
    fn from_header_splits_into_display_name_and_address() {
        let (name, addr) = split_from_header("\"Accounts Payable\" <billing@acme-payments.example>");
        assert_eq!(name, "Accounts Payable");
        assert_eq!(addr, "billing@acme-payments.example");
    }

    #[test]
    fn from_header_without_a_display_name_yields_just_the_address() {
        let (name, addr) = split_from_header("billing@acme.example");
        assert_eq!(name, "");
        assert_eq!(addr, "billing@acme.example");
    }

    #[test]
    fn a_display_name_that_looks_like_an_address_is_kept_separate() {
        // The impersonation trick: the name is written to read as the address in
        // clients that truncate. Collapsing them would erase the signal.
        let (name, addr) = split_from_header("\"security@acme.example\" <bounce77@mailer-host.example>");
        assert_eq!(name, "security@acme.example");
        assert_eq!(addr, "bounce77@mailer-host.example");
    }

    #[test]
    fn a_line_without_a_colon_is_ignored_rather_than_corrupting_the_block() {
        let parsed = parse_header_block("From: a@x.example\ngarbage line\nTo: b@y.example");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].0, "From");
        assert_eq!(parsed[1].0, "To");
    }
}
