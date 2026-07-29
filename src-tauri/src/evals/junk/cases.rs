//! Case format for the junk eval, and the loader that turns a synthetic
//! message into the `JunkSignals` the planner consumes.
//!
//! Cases carry a real RFC 5322 header block rather than pre-digested booleans,
//! so the header-parsing layer is exercised end-to-end by the same corpus that
//! measures the detector. A case that only asserted "spf=fail → phishing" would
//! pass even if the parser never read a header correctly.
//!
//! Every case is synthetic. Domains use the RFC 2606 `.example` TLD; no address,
//! name or subject comes from a real mailbox.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use crate::services::junk::verdict::{JunkAxis, JunkSignals, ReasonCode};
use crate::sync::header_capture::{capture, parse_header_block, split_from_header};

use super::super::{EvalError, EvalResult};

/// Ground truth for one axis. Binary on purpose: `Uncertain` and `Unknown` are
/// detector states, not labels a human can assign to a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpectedBand {
    Clean,
    Junk,
}

impl ExpectedBand {
    fn is_flagged(self) -> bool {
        matches!(self, ExpectedBand::Junk)
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Expectation {
    #[serde(default)]
    pub phishing: Option<ExpectedBand>,
    #[serde(default)]
    pub spam: Option<ExpectedBand>,
    #[serde(default)]
    pub graymail: Option<ExpectedBand>,
    /// Reason codes that must appear on the verdict. Asserting on the *why*
    /// and not only the verdict is what stops a case passing by coincidence.
    #[serde(default)]
    pub reasons_include: Vec<ReasonCode>,
}

impl Expectation {
    /// Only axes the case actually takes a position on. An omitted axis is not
    /// an implicit "clean" — see `metrics::confusion_for`.
    pub fn flagged_map(&self) -> BTreeMap<JunkAxis, bool> {
        let mut map = BTreeMap::new();
        for (axis, band) in [
            (JunkAxis::Phishing, self.phishing),
            (JunkAxis::Spam, self.spam),
            (JunkAxis::Graymail, self.graymail),
        ] {
            if let Some(band) = band {
                map.insert(axis, band.is_flagged());
            }
        }
        map
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct JunkCase {
    pub id: String,
    #[serde(default = "default_tier")]
    pub tier: String,
    /// A raw RFC 5322 header block. Empty means "no headers captured for this
    /// message", which the detector must treat as `Unknown`, not `Clean`.
    #[serde(default)]
    pub raw_headers: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub known_contact_domains: Vec<String>,
    #[serde(default)]
    pub known_contact_names: Vec<String>,
    /// The MTA whose `Authentication-Results` this account accepts.
    ///
    /// The synthetic corpus is written as if `mx.example.com` were the user's
    /// own receiving server, so cases inherit that unless they override it —
    /// which the untrusted-authserv cases do, to prove a forged verdict from an
    /// unexpected MTA is neither believed nor used to incriminate.
    #[serde(default = "default_trusted_authserv")]
    pub trusted_authserv: Option<String>,
    #[serde(default)]
    pub sender_engaged: bool,
    #[serde(default)]
    pub sender_trusted: bool,
    #[serde(default)]
    pub own_thread: bool,
    #[serde(default)]
    pub provider_category: Option<String>,
    #[serde(default)]
    pub attachment_names: Vec<String>,
    #[serde(default)]
    pub user_marked_not_junk: bool,
    /// Messages this sender has sent. Defaults to the recurrence floor, so a
    /// case models a repeat sender unless it says otherwise.
    #[serde(default = "default_sender_message_count")]
    pub sender_message_count: usize,
    /// Simulated output of the account's trained model, so cases can exercise
    /// how the statistical layer composes with the deterministic one without
    /// shipping a trained model in the corpus.
    #[serde(default)]
    pub statistical_spam: Option<f32>,
    #[serde(default)]
    pub statistical_graymail: Option<f32>,
    pub expect: Expectation,
}

fn default_tier() -> String {
    "smoke".to_string()
}

fn default_sender_message_count() -> usize {
    crate::services::junk::verdict::MIN_RECURRENCE
}

fn default_trusted_authserv() -> Option<String> {
    Some("mx.example.com".to_string())
}

impl JunkCase {
    pub fn to_signals(&self) -> JunkSignals {
        // Deliberately through the PRODUCTION capture path, not an eval-local
        // parser: the corpus is only meaningful if the code it exercises is the
        // code that runs during sync.
        let pairs = parse_header_block(&self.raw_headers);
        let headers = if pairs.is_empty() { None } else { Some(capture(&pairs)) };

        let from = headers.as_ref().and_then(|h| h.from_raw.as_deref()).unwrap_or_default();
        let (display_name, sender_email) = split_from_header(from);

        // Subject lives on `emails`, not in the captured header subset, so the
        // eval reads it off the raw block the same way sync reads it off the
        // provider payload.
        let subject = pairs
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("subject"))
            .map(|(_, value)| value.clone())
            .unwrap_or_default();

        JunkSignals {
            headers,
            subject,
            body: self.body.clone(),
            sender_email,
            sender_display_name: display_name,
            known_contact_domains: self.known_contact_domains.clone(),
            known_contact_names: self.known_contact_names.clone(),
            trusted_authserv: self.trusted_authserv.clone(),
            sender_engaged: self.sender_engaged,
            sender_trusted: self.sender_trusted,
            own_thread: self.own_thread,
            provider_category: self.provider_category.clone(),
            attachment_names: self.attachment_names.clone(),
            user_marked_not_junk: self.user_marked_not_junk,
            sender_message_count: self.sender_message_count,
            statistical_spam: self.statistical_spam,
            statistical_graymail: self.statistical_graymail,
        }
    }
}

/// Load every `*.yaml` under `dir`, rejecting duplicate ids.
pub fn load_cases(dir: &Path) -> EvalResult<Vec<JunkCase>> {
    let mut cases: Vec<JunkCase> = Vec::new();
    let mut seen: BTreeMap<String, String> = BTreeMap::new();

    let entries = std::fs::read_dir(dir)?;
    let mut paths: Vec<_> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|ext| ext == "yaml" || ext == "yml"))
        .collect();
    paths.sort();

    for path in paths {
        let raw = std::fs::read_to_string(&path)?;
        let parsed: Vec<JunkCase> = serde_yaml::from_str(&raw)?;
        for case in parsed {
            let file = path.display().to_string();
            if let Some(prev) = seen.insert(case.id.clone(), file.clone()) {
                return Err(EvalError::Config(format!(
                    "duplicate junk case id {:?} in {} (already defined in {})",
                    case.id, file, prev
                )));
            }
            cases.push(case);
        }
    }

    Ok(cases)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_header_block_marks_headers_unavailable() {
        // Drives Band::Unknown rather than a confident Clean: no evidence is
        // not the same as evidence of innocence.
        let case = JunkCase {
            id: "no-headers".into(),
            tier: "smoke".into(),
            raw_headers: String::new(),
            body: "hi".into(),
            known_contact_domains: vec![],
            known_contact_names: vec![],
            trusted_authserv: default_trusted_authserv(),
            sender_engaged: false,
            sender_trusted: false,
            own_thread: false,
            provider_category: None,
            attachment_names: vec![],
            user_marked_not_junk: false,
            sender_message_count: 3,
            statistical_spam: None,
            statistical_graymail: None,
            expect: Expectation::default(),
        };
        assert!(!case.to_signals().headers_available());
        assert!(case.to_signals().headers.is_none());
    }

    #[test]
    fn expectation_omits_axes_the_case_takes_no_position_on() {
        let expectation = Expectation {
            phishing: Some(ExpectedBand::Junk),
            spam: Some(ExpectedBand::Clean),
            graymail: None,
            reasons_include: vec![],
        };
        let map = expectation.flagged_map();
        assert_eq!(map.get(&JunkAxis::Phishing), Some(&true));
        assert_eq!(map.get(&JunkAxis::Spam), Some(&false));
        assert_eq!(map.get(&JunkAxis::Graymail), None);
    }

    #[test]
    fn a_case_parses_from_the_documented_yaml_shape() {
        let yaml = r#"
- id: phish-001-lookalike-reply-to
  tier: smoke
  raw_headers: |
    From: "Accounts Payable" <billing@acme-payments.example>
    Reply-To: ap.acme@mail-secure.example
    Authentication-Results: mx.example.com; spf=fail; dkim=none; dmarc=fail
    Subject: Updated bank details for invoice 4471
  body: "Please update our remittance account before Friday."
  known_contact_domains: [acme.example]
  expect:
    phishing: junk
    spam: clean
    graymail: clean
    reasons_include: [reply_to_mismatch, dmarc_fail, lookalike_domain]
"#;
        let cases: Vec<JunkCase> = serde_yaml::from_str(yaml).expect("parse yaml");
        let case = cases.first().expect("one case");
        assert_eq!(case.id, "phish-001-lookalike-reply-to");

        let signals = case.to_signals();
        assert_eq!(signals.sender_display_name, "Accounts Payable");
        assert_eq!(signals.sender_email, "billing@acme-payments.example");
        assert_eq!(signals.subject, "Updated bank details for invoice 4471");
        assert!(signals.headers_available());
        assert_eq!(signals.known_contact_domains, vec!["acme.example".to_string()]);

        // The case's headers went through the production capture path, so the
        // typed fields the detector will read are populated — not just a bag of
        // strings the eval parsed for itself.
        let headers = signals.headers.as_ref().expect("headers captured");
        assert_eq!(headers.reply_to.as_deref(), Some("ap.acme@mail-secure.example"));
        assert_eq!(headers.authserv_id.as_deref(), Some("mx.example.com"));
        assert!(headers
            .auth_results
            .as_deref()
            .is_some_and(|a| a.contains("dmarc=fail")));

        assert_eq!(case.expect.phishing, Some(ExpectedBand::Junk));
        assert!(case.expect.reasons_include.contains(&ReasonCode::ReplyToMismatch));
    }
}
