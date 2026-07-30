//! Interpreting `Authentication-Results` — the layer everything else leans on.
//!
//! SMTP has no built-in authentication: the `From:` header is a handwritten
//! return address on an envelope. SPF, DKIM and DMARC are three layers bolted on
//! afterwards, and only DMARC checks the thing the user actually sees:
//!
//! * **SPF** validates the *envelope* sender (`Return-Path`) against a DNS
//!   record. The user never sees that address.
//! * **DKIM** proves a domain signed the message. It says nothing about which
//!   `From:` is displayed.
//! * **DMARC** requires *alignment* — the visible `From:` domain must match what
//!   SPF or DKIM validated. Without it an attacker passes SPF perfectly using
//!   their own domain while showing `From: your-bank.example`.
//!
//! All three are evaluated by the *receiving* server, which records the outcome
//! in `Authentication-Results`. That header is plain text, so the trust boundary
//! matters more than the parsing: see [`AuthAssessment::trusted`].

use serde::{Deserialize, Serialize};

use crate::models::headers::RawHeaders;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthResult {
    Pass,
    Fail,
    SoftFail,
    Neutral,
    /// The domain publishes no policy at all.
    None,
    /// Lookup failed — a transient or broken state, never evidence of guilt.
    Error,
    /// Present but unrecognized.
    Unknown,
}

impl AuthResult {
    fn parse(token: &str) -> Self {
        match token.trim().to_lowercase().as_str() {
            "pass" => AuthResult::Pass,
            "fail" => AuthResult::Fail,
            "softfail" => AuthResult::SoftFail,
            "neutral" => AuthResult::Neutral,
            "none" => AuthResult::None,
            "temperror" | "permerror" => AuthResult::Error,
            _ => AuthResult::Unknown,
        }
    }

    /// A hard, actionable failure — not merely "not a pass".
    ///
    /// `SoftFail`, `Neutral`, `None` and `Error` are all deliberately excluded:
    /// enormous amounts of legitimate mail (forwarded messages, mailing lists,
    /// small senders with no DMARC record) lands in those states, and treating
    /// them as guilt is the fastest way to a false positive.
    pub fn is_hard_fail(self) -> bool {
        matches!(self, AuthResult::Fail)
    }
}

/// What the receiving server concluded, plus whether we believe it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AuthAssessment {
    /// `false` when the `Authentication-Results` header is absent, or when its
    /// `authserv-id` is not the MTA we expect for this account.
    ///
    /// This is the security boundary of the whole feature. The header is plain
    /// text and an attacker can paste `Authentication-Results: whatever;
    /// spf=pass; dmarc=pass` into their own message. Our MTA prepends the real
    /// verdict above it — which is why capture keeps only the topmost instance —
    /// but on an account where we cannot identify the expected MTA, a forged
    /// instance could be the topmost one. In that case the correct output is
    /// "we do not know", never "it passed".
    pub trusted: bool,
    pub spf: Option<AuthResult>,
    pub dkim: Option<AuthResult>,
    pub dmarc: Option<AuthResult>,
}

impl AuthAssessment {
    /// Did the message hard-fail the check that actually protects the visible
    /// `From:` domain?
    pub fn dmarc_hard_fail(&self) -> bool {
        self.trusted && self.dmarc.is_some_and(AuthResult::is_hard_fail)
    }

    pub fn spf_hard_fail(&self) -> bool {
        self.trusted && self.spf.is_some_and(AuthResult::is_hard_fail)
    }

    /// Every authentication mechanism the sender's domain publishes came back
    /// clean. The strongest available evidence that a message is what it claims.
    pub fn fully_aligned(&self) -> bool {
        self.trusted
            && self.dmarc == Some(AuthResult::Pass)
            && matches!(self.spf, Some(AuthResult::Pass) | None)
            && matches!(self.dkim, Some(AuthResult::Pass) | None)
    }

    /// Nothing to go on: no header, or one we cannot attribute to our own MTA.
    pub fn is_unknown(&self) -> bool {
        !self.trusted || (self.spf.is_none() && self.dkim.is_none() && self.dmarc.is_none())
    }
}

/// The MTA whose `Authentication-Results` we accept, per provider.
///
/// Returning `None` means "we cannot identify the expected MTA for this
/// account", which makes every verdict on it untrusted. For self-hosted IMAP
/// that is the honest answer until the user's own server is known.
pub fn expected_authserv(provider: &str) -> Option<&'static str> {
    match provider.to_lowercase().as_str() {
        "gmail" => Some("mx.google.com"),
        "outlook" => Some("protection.outlook.com"),
        _ => None,
    }
}

/// Is `authserv_id` the MTA we expect?
///
/// Suffix match so per-tenant hostnames (`acme-example-com.mail.protection.outlook.com`)
/// still resolve to the expected server, anchored on a dot so
/// `evil-protection.outlook.com.attacker.example` cannot masquerade as one.
pub fn authserv_matches(authserv_id: &str, expected: &str) -> bool {
    let id = authserv_id.trim().trim_end_matches('.').to_lowercase();
    let expected = expected.trim().to_lowercase();
    id == expected || id.ends_with(&format!(".{expected}"))
}

/// What the receiving server's own spam scanner concluded.
///
/// The distinction that matters: `X-Spam-*` headers record that a scan *ran*,
/// not that it found something. SpamAssassin and rspamd stamp
/// `X-Spam-Status: No` and `X-Spam-Flag: NO` on essentially every clean message
/// they process, so treating the header's mere presence as a verdict inverts the
/// signal and condemns an entire mailbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerSpamVerdict {
    /// The server says this is spam, decisively — its score is well past its own
    /// threshold.
    Flagged,
    /// Flagged, but only just.
    ///
    /// The symmetric case to [`Self::BarelyCleared`], and it was missing: a
    /// scanner that lands on 5.5 against a threshold of 5.0 is far less certain
    /// than one that lands on 12.7, and the difference is often a single
    /// `SPF_SOFTFAIL` — a state huge amounts of forwarded and small-sender mail
    /// falls into. Treating both as the same hard verdict makes the detector
    /// inherit every marginal call the server made.
    BarelyFlagged,
    /// The server scanned it and cleared it comfortably. Strong exculpatory
    /// evidence: the scanner had RBLs, greylisting history and a trained Bayes
    /// model that we cannot replicate locally.
    Cleared,
    /// Cleared, but only just — the score landed close to the server's own
    /// threshold.
    ///
    /// A bare score means nothing across servers, but `X-Spam-Status` reports
    /// `score` and `required` **together**, so the margin between them is
    /// comparable. "No, score=2.6 required=5.0" is a scanner that nearly fired;
    /// treating it as confidently clean lets a blatant attack through on the
    /// strength of a verdict the server itself was unsure about.
    BarelyCleared,
    /// No scanner ran, or its output is unrecognized.
    Unknown,
}

/// Interpret the collected `X-Spam-*` / rspamd headers.
pub fn parse_server_spam(spam_headers: Option<&str>) -> ServerSpamVerdict {
    let Some(raw) = spam_headers else {
        return ServerSpamVerdict::Unknown;
    };

    /// Fraction of the server's own threshold above which a clearance is
    /// treated as marginal rather than confident.
    const MARGINAL_RATIO: f32 = 0.5;

    /// Below this multiple of the server's own threshold, a flag is marginal.
    const MARGINAL_FLAG_RATIO: f32 = 1.25;

    let mut saw_cleared = false;
    let mut saw_flagged = false;
    let mut margin_ratio: Option<f32> = None;
    for line in raw.lines() {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim().to_lowercase();
        let value = value.trim().to_lowercase();
        // The result is the first token: "Yes, score=9.8 required=5.0".
        let head = value.split([',', ' ']).next().unwrap_or_default();

        match name.as_str() {
            "x-spam-flag" | "x-spam-status" => {
                match head {
                    "yes" | "true" => saw_flagged = true,
                    "no" | "false" => saw_cleared = true,
                    _ => {}
                }
                // "No, score=2.6 required=5.0" — both numbers come from the same
                // scanner, so their ratio is meaningful even though the raw
                // score is not comparable across servers.
                let score = extract_number(&value, "score=");
                let required = extract_number(&value, "required=");
                if let (Some(score), Some(required)) = (score, required) {
                    if required > 0.0 {
                        margin_ratio = Some((score / required).max(margin_ratio.unwrap_or(0.0)));
                    }
                }
            }
            // SpamAssassin's verbose report carries the threshold even when
            // X-Spam-Status omits it: "(5.5 points, 5.0 required)". Without it
            // every flag from such a server looks equally decisive.
            "x-spam-report" => {
                let score = extract_before(&value, " points");
                let required = extract_before(&value, " required");
                if let (Some(score), Some(required)) = (score, required) {
                    if required > 0.0 {
                        margin_ratio = Some((score / required).max(margin_ratio.unwrap_or(0.0)));
                    }
                }
            }
            "x-rspamd-action" => match head {
                "reject" | "add_header" | "rewrite_subject" => return ServerSpamVerdict::Flagged,
                "no_action" | "greylist" => saw_cleared = true,
                _ => {}
            },
            // X-Spam-Score / X-Spam-Level / X-Spam-Bar carry a magnitude with no
            // threshold attached, and every server scales them differently.
            // Reading them without the site's `required` value would be guessing.
            _ => {}
        }
    }

    if saw_flagged {
        // A flag with no score attached is taken at full strength: without the
        // numbers there is no basis for discounting it.
        if margin_ratio.is_some_and(|r| r < MARGINAL_FLAG_RATIO) {
            ServerSpamVerdict::BarelyFlagged
        } else {
            ServerSpamVerdict::Flagged
        }
    } else if saw_cleared {
        if margin_ratio.is_some_and(|r| r >= MARGINAL_RATIO) {
            ServerSpamVerdict::BarelyCleared
        } else {
            ServerSpamVerdict::Cleared
        }
    } else {
        ServerSpamVerdict::Unknown
    }
}

/// Pull the number immediately preceding `marker`, e.g. the `5.0` in
/// "(5.5 points, 5.0 required)".
fn extract_before(value: &str, marker: &str) -> Option<f32> {
    let idx = value.find(marker)?;
    let head = &value[..idx];
    let token: String = head
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    token.parse::<f32>().ok()
}

/// Pull a `key=<number>` value out of an `X-Spam-Status` line.
fn extract_number(value: &str, key: &str) -> Option<f32> {
    let rest = value.split(key).nth(1)?;
    let token: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
        .collect();
    token.parse::<f32>().ok()
}

/// Read the authentication verdicts out of captured headers.
///
/// `trusted_authserv` is the MTA whose claims we accept; `None` means we cannot
/// identify one, so nothing is trusted.
pub fn assess(headers: Option<&RawHeaders>, trusted_authserv: Option<&str>) -> AuthAssessment {
    let Some(headers) = headers else {
        return AuthAssessment::default();
    };
    let Some(raw) = headers.auth_results.as_deref() else {
        return AuthAssessment::default();
    };

    let trusted = match (headers.authserv_id.as_deref(), trusted_authserv) {
        (Some(id), Some(expected)) => authserv_matches(id, expected),
        _ => false,
    };

    let mut out = AuthAssessment {
        trusted,
        ..Default::default()
    };

    // Skip the leading authserv-id segment; each remaining segment is
    // "method=result" plus optional properties.
    for segment in raw.split(';').skip(1) {
        let segment = segment.trim();
        let Some((method, rest)) = segment.split_once('=') else {
            continue;
        };
        // "spf=pass smtp.mailfrom=x.example" — the result is the first token.
        let result = AuthResult::parse(rest.split_whitespace().next().unwrap_or_default());
        match method.trim().to_lowercase().as_str() {
            "spf" => out.spf.get_or_insert(result),
            "dkim" => out.dkim.get_or_insert(result),
            "dmarc" => out.dmarc.get_or_insert(result),
            _ => continue,
        };
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(auth: &str) -> RawHeaders {
        let pairs = crate::sync::header_capture::parse_header_block(&format!("Authentication-Results: {auth}\n"));
        crate::sync::header_capture::capture(&pairs)
    }

    const OURS: Option<&str> = Some("mx.example.com");

    #[test]
    fn absent_headers_yield_an_unknown_assessment() {
        let a = assess(None, OURS);
        assert!(a.is_unknown());
        assert!(!a.trusted);
        assert!(!a.dmarc_hard_fail());
    }

    #[test]
    fn results_are_parsed_per_method() {
        let a = assess(Some(&headers("mx.example.com; spf=fail; dkim=none; dmarc=fail")), OURS);
        assert!(a.trusted);
        assert_eq!(a.spf, Some(AuthResult::Fail));
        assert_eq!(a.dkim, Some(AuthResult::None));
        assert_eq!(a.dmarc, Some(AuthResult::Fail));
    }

    #[test]
    fn properties_after_the_result_are_ignored() {
        let a = assess(
            Some(&headers("mx.example.com; spf=pass smtp.mailfrom=acme.example")),
            OURS,
        );
        assert_eq!(a.spf, Some(AuthResult::Pass));
    }

    #[test]
    fn a_verdict_from_an_unexpected_server_is_never_trusted() {
        // The core defence. An attacker can write any Authentication-Results
        // they like into their own message; if we cannot attribute the topmost
        // instance to our own MTA, the honest output is "unknown".
        let a = assess(Some(&headers("forged.example; spf=pass; dmarc=pass")), OURS);
        assert!(!a.trusted);
        assert!(a.is_unknown());
        assert!(!a.fully_aligned(), "a forged pass must not read as aligned");
    }

    #[test]
    fn an_untrusted_verdict_cannot_incriminate_either() {
        // Symmetry matters: an untrusted header must not be usable to frame a
        // message any more than to clear one.
        let a = assess(Some(&headers("forged.example; spf=fail; dmarc=fail")), OURS);
        assert!(!a.dmarc_hard_fail());
        assert!(!a.spf_hard_fail());
    }

    #[test]
    fn an_account_with_no_known_mta_trusts_nothing() {
        let a = assess(Some(&headers("mx.example.com; spf=pass; dmarc=pass")), None);
        assert!(!a.trusted);
        assert!(a.is_unknown());
    }

    #[test]
    fn only_a_hard_fail_counts_as_a_failure() {
        // Forwarded mail routinely softfails SPF, and huge numbers of small
        // senders publish no DMARC at all. Treating those as guilt is the
        // fastest route to a false positive on real mail.
        for token in ["softfail", "neutral", "none", "temperror", "permerror"] {
            let a = assess(Some(&headers(&format!("mx.example.com; dmarc={token}"))), OURS);
            assert!(!a.dmarc_hard_fail(), "{token} must not be a hard fail");
        }
        let a = assess(Some(&headers("mx.example.com; dmarc=fail")), OURS);
        assert!(a.dmarc_hard_fail());
    }

    #[test]
    fn full_alignment_requires_a_trusted_dmarc_pass() {
        let aligned = assess(Some(&headers("mx.example.com; spf=pass; dkim=pass; dmarc=pass")), OURS);
        assert!(aligned.fully_aligned());

        let no_dmarc = assess(Some(&headers("mx.example.com; spf=pass; dkim=pass")), OURS);
        assert!(!no_dmarc.fully_aligned(), "SPF+DKIM without DMARC is not alignment");
    }

    #[test]
    fn tenant_subdomains_of_the_expected_mta_are_accepted() {
        assert!(authserv_matches(
            "acme-example-com.mail.protection.outlook.com",
            "protection.outlook.com"
        ));
        assert!(authserv_matches("mx.google.com", "mx.google.com"));
    }

    #[test]
    fn a_lookalike_authserv_is_rejected() {
        // Anchored on a dot, so an attacker cannot register a host that merely
        // ends with the expected string.
        assert!(!authserv_matches(
            "evil-mx.google.com.attacker.example",
            "mx.google.com"
        ));
        assert!(!authserv_matches("notmx.google.com", "mx.google.com"));
    }

    // ── Server spam verdict ───────────────────────────────────────────────

    #[test]
    fn a_negative_spam_status_clears_the_message() {
        // Regression: SpamAssassin stamps "No" on every clean message it scans.
        // Reading the header's PRESENCE as a verdict flagged 93% of a real
        // mailbox — the scanner had cleared all of them.
        assert_eq!(
            parse_server_spam(Some("x-spam-status: No, score=-2.1 required=5.0\nx-spam-flag: NO")),
            ServerSpamVerdict::Cleared
        );
    }

    #[test]
    fn a_positive_spam_status_flags_the_message() {
        assert_eq!(
            parse_server_spam(Some("x-spam-flag: YES\nx-spam-status: Yes, score=9.8 required=5.0")),
            ServerSpamVerdict::Flagged
        );
    }

    #[test]
    fn a_flag_anywhere_outranks_a_clearance() {
        // Mixed output from chained scanners: any single "yes" wins.
        assert_eq!(
            parse_server_spam(Some("x-spam-status: No\nx-spam-flag: YES")),
            ServerSpamVerdict::Flagged
        );
    }

    #[test]
    fn a_flag_that_barely_crossed_the_threshold_is_marked_marginal() {
        // REGRESSION, from a real mailbox: a message a human called legitimate
        // was flagged at 5.4 against the server's own threshold of 5.0, with
        // more than half the points coming from an SPF softfail and a valid DKIM
        // signature on the message. Treating that as the same verdict as a
        // decisive flag makes the detector inherit every borderline call the
        // server ever made.
        assert_eq!(
            parse_server_spam(Some("x-spam-flag: YES\nx-spam-status: Yes, score=5.4 required=5.0")),
            ServerSpamVerdict::BarelyFlagged
        );
    }

    #[test]
    fn a_decisive_flag_stays_decisive() {
        assert_eq!(
            parse_server_spam(Some("x-spam-status: Yes, score=12.7 required=5.0")),
            ServerSpamVerdict::Flagged
        );
    }

    #[test]
    fn the_threshold_is_read_from_the_verbose_report_when_the_status_omits_it() {
        // Second half of the same regression. This server writes no `required=`
        // in X-Spam-Status and only states the threshold inside X-Spam-Report,
        // in a different shape. Reading one header and not the other made the
        // fix above a no-op on exactly the mailbox that motivated it.
        assert_eq!(
            parse_server_spam(Some(
                "x-spam-status: Yes, score=5.6\nx-spam-report: Content analysis details: (5.6 points, 5.0 required) pts rule name"
            )),
            ServerSpamVerdict::BarelyFlagged
        );
    }

    #[test]
    fn a_flag_with_no_numbers_at_all_is_taken_at_full_strength() {
        // No score, no threshold, no basis for discounting the server's call.
        assert_eq!(parse_server_spam(Some("x-spam-flag: YES")), ServerSpamVerdict::Flagged);
    }

    #[test]
    fn a_bare_score_is_not_a_verdict() {
        // X-Spam-Score has no threshold attached and every site scales it
        // differently. A score of 10 is spam on one server and normal on
        // another; acting on it without `required` would be guessing.
        assert_eq!(
            parse_server_spam(Some("x-spam-score: 10\nx-spam-bar: ++++++++++")),
            ServerSpamVerdict::Unknown
        );
    }

    #[test]
    fn rspamd_actions_are_understood() {
        assert_eq!(
            parse_server_spam(Some("x-rspamd-action: reject")),
            ServerSpamVerdict::Flagged
        );
        assert_eq!(
            parse_server_spam(Some("x-rspamd-action: no_action")),
            ServerSpamVerdict::Cleared
        );
    }

    #[test]
    fn no_scanner_output_is_unknown_not_clean() {
        assert_eq!(parse_server_spam(None), ServerSpamVerdict::Unknown);
        assert_eq!(
            parse_server_spam(Some("x-spam-checker-version: 4.0")),
            ServerSpamVerdict::Unknown
        );
    }

    #[test]
    fn self_hosted_imap_has_no_assumed_mta() {
        assert_eq!(expected_authserv("imap"), None);
        assert_eq!(expected_authserv("gmail"), Some("mx.google.com"));
    }
}
