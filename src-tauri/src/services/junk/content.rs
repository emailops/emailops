//! Content-level spam signals: links, shouting, hidden text, attachments.
//!
//! Weakest layer of the three, and deliberately weighted as such. Content
//! heuristics are what every naive spam filter starts and ends with, and they
//! are the main source of false positives: real invoices are link-dense, real
//! security alerts are urgent, real newsletters are all images. Everything here
//! is evidence, never proof.

use std::collections::BTreeSet;

use crate::services::junk::lookalike::{domain_of, registrable_domain};

/// Extensions that execute, or that exist to smuggle something that does.
///
/// `.html` leads because an HTML attachment is the standard credential-phishing
/// delivery vehicle: it renders a local login form, so there is no malicious URL
/// for a scanner to catch.
const DANGEROUS_EXTENSIONS: &[&str] = &[
    "html", "htm", "shtml", "hta", "js", "jse", "vbs", "vbe", "wsf", "wsh", "ps1", "bat", "cmd", "com", "exe", "scr",
    "pif", "cpl", "msi", "msc", "jar", "lnk", "url", "iso", "img", "vhd", "docm", "xlsm", "pptm", "dotm", "xlam",
    "reg",
];

/// Link shorteners hide the destination, which is the point of using one.
const URL_SHORTENERS: &[&str] = &[
    "bit.ly",
    "tinyurl.com",
    "goo.gl",
    "t.co",
    "ow.ly",
    "is.gd",
    "buff.ly",
    "adf.ly",
    "shrt.example",
    "cutt.ly",
    "rebrand.ly",
    "rb.gy",
    "shorturl.at",
    "tiny.cc",
    "bl.ink",
];

/// Pressure tactics, in the languages the app ships.
///
/// English-only lexicons are a trap: a Spanish-speaking user's junk is written
/// in Spanish, and a detector that only reads English scores it at zero. The app
/// ships en/es/fr/de, so the lexicons do too. Matching is done on a lowercased
/// haystack, and accented forms are listed explicitly rather than folded — "más"
/// and "mas" are different words, and stripping accents would create matches
/// that are not there.
const URGENCY_PHRASES: &[&str] = &[
    // English
    "act now",
    "urgent",
    "immediately",
    "within 24 hours",
    "final notice",
    "last chance",
    "before it is too late",
    "before it's too late",
    "account will be suspended",
    "verify your account",
    "confirm your identity",
    "limited time",
    // Spanish
    "urgente",
    "acción requerida",
    "accion requerida",
    "acción necesaria",
    "de inmediato",
    "inmediatamente",
    "último aviso",
    "ultimo aviso",
    "última oportunidad",
    "ultima oportunidad",
    "antes de que sea demasiado tarde",
    "su cuenta será suspendida",
    "su cuenta sera suspendida",
    "será bloqueada",
    "sera bloqueada",
    "serán eliminados",
    "seran eliminados",
    "será eliminado",
    "sera eliminado",
    "tiempo limitado",
    "date prisa",
    "no pierdas",
    // French ("urgent" is already listed above — the same spelling in both
    // languages, and listing it twice used to score one word as two hits)
    "action requise",
    "immédiatement",
    "immediatement",
    "dernier avertissement",
    "dernière chance",
    "derniere chance",
    "votre compte sera suspendu",
    "sera supprimé",
    "sera supprime",
    "temps limité",
    "temps limite",
    // German
    "dringend",
    "sofort handeln",
    "handeln sie jetzt",
    "letzte warnung",
    "letzte chance",
    "ihr konto wird gesperrt",
    "wird gelöscht",
    "wird geloescht",
    "begrenzte zeit",
];

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ContentSignals {
    pub link_count: usize,
    pub distinct_link_domains: usize,
    /// Anchor text that is itself a URL pointing somewhere other than the href.
    pub link_text_href_mismatch: bool,
    pub has_url_shortener: bool,
    pub has_raw_ip_link: bool,
    /// Text hidden with white-on-white or 1px styling — filler used to poison
    /// statistical classifiers with ham-looking tokens.
    pub has_hidden_text: bool,
    pub caps_ratio: f32,
    pub urgency_hits: usize,
    /// The message asks the reader to re-enter credentials, and gives them
    /// somewhere to do it. The payload of credential phishing.
    pub credential_solicitation: bool,
    pub dangerous_attachments: Vec<String>,
}

/// Phrases that ask the reader to hand over access.
///
/// Distinct from the urgency lexicon: urgency is a pressure tactic that plenty
/// of legitimate mail uses, whereas a request to re-enter credentials is the
/// actual payload of credential phishing.
const CREDENTIAL_PHRASES: &[&str] = &[
    // English
    "credentials",
    "re-validate",
    "revalidate",
    "verify your account",
    "confirm your password",
    "update your password",
    "confirm your identity",
    "account will be suspended",
    "sign in to avoid",
    "unlock your account",
    "validate your mailbox",
    // Spanish
    "credenciales",
    "verifique su cuenta",
    "verificar su cuenta",
    "confirme su contraseña",
    "confirme su contrasena",
    "actualice su contraseña",
    "actualice su contrasena",
    "confirme su identidad",
    "inicie sesión para evitar",
    "inicie sesion para evitar",
    "desbloquear su cuenta",
    "validar su buzón",
    "validar su buzon",
    "para conservar sus datos",
    "conservar sus archivos",
    // French
    "identifiants",
    "vérifiez votre compte",
    "verifiez votre compte",
    "confirmez votre mot de passe",
    "mettez à jour votre mot de passe",
    "confirmez votre identité",
    "débloquer votre compte",
    "deverrouiller votre compte",
    // German
    "zugangsdaten",
    "konto bestätigen",
    "konto bestaetigen",
    "passwort bestätigen",
    "passwort aktualisieren",
    "identität bestätigen",
    "konto entsperren",
];

/// How many *distinct* phrases from a lexicon appear in the text.
///
/// Not `filter(contains).count()`. The lexicons are multilingual and their
/// entries genuinely overlap — "urgent" is the same word in English and French,
/// and the Spanish "urgente" contains it — so a naive count turns one occurrence
/// of one word into two or three hits. `urgency_hits >= 2` is written to require
/// two *independent* pressure phrases, and an inflated count defeats exactly
/// that.
///
/// A match that is merely a fragment of another match is dropped. That can
/// undercount when both the long and the short form appear in different places
/// (a body containing "urgente" here and a standalone "urgent" there scores 1),
/// which is the safe direction: this lexicon is the weakest evidence in the
/// detector and its job is to corroborate, never to accuse on its own.
fn distinct_phrase_hits(haystack: &str, phrases: &[&str]) -> usize {
    let mut matched: Vec<&str> = Vec::new();
    for phrase in phrases {
        if haystack.contains(phrase) && !matched.contains(phrase) {
            matched.push(phrase);
        }
    }
    matched
        .iter()
        .filter(|candidate| {
            !matched
                .iter()
                .any(|other| other.len() > candidate.len() && other.contains(*candidate))
        })
        .count()
}

/// Every link in the message: `href` attributes plus bare URLs in plain text.
///
/// Plain text matters — a lot of junk arrives with no HTML at all, and a scanner
/// that only reads anchors sees a message with zero links.
pub fn extract_links(body: &str) -> Vec<String> {
    let mut out = extract_hrefs(body);
    out.extend(extract_bare_urls(body));
    out
}

/// Bare `http(s)://…` runs that are not already inside an `href`.
fn extract_bare_urls(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    // `to_ascii_lowercase`, NOT `to_lowercase`. Byte offsets found in this
    // string are used to slice the ORIGINAL, and a full Unicode lowercasing can
    // change a string's byte length — 'İ' (U+0130) becomes two characters — so
    // the offsets stop lining up and the slice lands mid-character and panics.
    // The literals matched here are all ASCII, so an ASCII fold is both correct
    // and length-preserving.
    let lower = text.to_ascii_lowercase();
    let mut cursor = 0usize;
    while cursor < text.len() {
        let Some(found) = lower[cursor..].find("http") else {
            break;
        };
        let start = cursor + found;
        let rest = &text[start..];
        if !(rest.starts_with("http://") || rest.starts_with("https://")) {
            cursor = start + 4;
            continue;
        }
        // Skip URLs that are the value of an href — already captured, and
        // counting them twice would inflate link density.
        let preceded_by_href = lower[..start]
            .trim_end()
            .trim_end_matches(['"', '\'', '='])
            .ends_with("href");
        let url: String = rest
            .chars()
            .take_while(|c| !c.is_whitespace() && !matches!(c, '"' | '\'' | '<' | '>'))
            .collect();
        let trimmed = url.trim_end_matches(['.', ',', ';', ':', ')']).to_string();
        if !preceded_by_href && !trimmed.is_empty() {
            out.push(trimmed.clone());
        }
        cursor = start + url.len().max(1);
    }
    out
}

/// Extract every `href` value, in document order.
///
/// A deliberately small scanner rather than a full HTML parse: this runs on
/// every synced message and only needs the attribute values.
fn extract_hrefs(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    // `to_ascii_lowercase`, NOT `to_lowercase`. Byte offsets found in this
    // string are used to slice the ORIGINAL, and a full Unicode lowercasing can
    // change a string's byte length — 'İ' (U+0130) becomes two characters — so
    // the offsets stop lining up and the slice lands mid-character and panics.
    // The literals matched here are all ASCII, so an ASCII fold is both correct
    // and length-preserving.
    let lower = html.to_ascii_lowercase();
    let mut cursor = 0usize;
    while let Some(found) = lower[cursor..].find("href=") {
        let start = cursor + found + "href=".len();
        let rest = &html[start..];
        let value = match rest.chars().next() {
            Some(q @ ('"' | '\'')) => rest[1..].split(q).next().unwrap_or_default(),
            _ => rest.split([' ', '>', '\n', '\r', '\t']).next().unwrap_or_default(),
        };
        if !value.trim().is_empty() {
            out.push(value.trim().to_string());
        }
        cursor = start;
    }
    out
}

/// Anchor bodies, paired with their href, for mismatch detection.
fn extract_anchor_pairs(html: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    // `to_ascii_lowercase`, NOT `to_lowercase`. Byte offsets found in this
    // string are used to slice the ORIGINAL, and a full Unicode lowercasing can
    // change a string's byte length — 'İ' (U+0130) becomes two characters — so
    // the offsets stop lining up and the slice lands mid-character and panics.
    // The literals matched here are all ASCII, so an ASCII fold is both correct
    // and length-preserving.
    let lower = html.to_ascii_lowercase();
    let mut cursor = 0usize;
    while let Some(found) = lower[cursor..].find("<a ") {
        let tag_start = cursor + found;
        let Some(tag_end_rel) = lower[tag_start..].find('>') else {
            break;
        };
        let tag_end = tag_start + tag_end_rel;
        let tag = &html[tag_start..=tag_end];
        let href = extract_hrefs(tag).into_iter().next().unwrap_or_default();

        let body_start = tag_end + 1;
        let body = match lower[body_start..].find("</a>") {
            Some(rel) => html[body_start..body_start + rel].trim().to_string(),
            None => String::new(),
        };
        if !href.is_empty() {
            out.push((body, href));
        }
        cursor = body_start;
    }
    out
}

fn is_raw_ip_host(host: &str) -> bool {
    let head = host.split(':').next().unwrap_or(host);
    let octets: Vec<&str> = head.split('.').collect();
    octets.len() == 4
        && octets
            .iter()
            .all(|o| !o.is_empty() && o.chars().all(|c| c.is_ascii_digit()))
}

fn host_of_url(url: &str) -> Option<String> {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let host = rest.split(['/', '?', '#']).next()?.trim();
    // Strip any userinfo — "https://acme.example@attacker.example" is a classic
    // way to make a URL read as one domain while resolving to another.
    let host = host.rsplit('@').next()?.to_lowercase();
    if host.is_empty() {
        None
    } else {
        Some(host)
    }
}

fn strip_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

fn caps_ratio(text: &str) -> f32 {
    let letters: Vec<char> = text.chars().filter(|c| c.is_alphabetic()).collect();
    if letters.len() < 12 {
        // Too short to mean anything — "OK" is not shouting.
        return 0.0;
    }
    let upper = letters.iter().filter(|c| c.is_uppercase()).count();
    upper as f32 / letters.len() as f32
}

/// Hidden-text detection is **deliberately disabled**.
///
/// The intent was to catch Bayes poisoning: a block of ham-looking words hidden
/// with white-on-white or zero-size styling. The problem is that the CSS used to
/// hide it is identical to the CSS every modern HTML newsletter uses for its
/// *preheader* — the short preview line clients show next to the subject:
///
/// ```html
/// <div style="display:none;font-size:0;color:#ffffff">Preview text…</div>
/// ```
///
/// Measured on a real mailbox, the previous check fired on 155 of 613 messages —
/// a quarter of everything, including Upwork, Substack and every ESP template.
/// At a weight of 0.45 that was the single largest content signal, and it was
/// wrong a quarter of the time.
///
/// Separating the two requires measuring how much *text* sits inside the hidden
/// block (a preheader is one short line; poisoning is paragraphs), which needs
/// real DOM parsing rather than substring matching. Until that exists, no signal
/// is better than one that misfires on a quarter of legitimate mail — the false
/// positive budget is the whole point of the feature.
fn has_hidden_text(_html: &str) -> bool {
    false
}

pub fn dangerous_attachments(names: &[String]) -> Vec<String> {
    names
        .iter()
        .filter(|name| {
            name.rsplit_once('.')
                .is_some_and(|(_, ext)| DANGEROUS_EXTENSIONS.contains(&ext.trim().to_lowercase().as_str()))
        })
        .cloned()
        .collect()
}

/// Analyse subject + body + attachments.
pub fn analyse(subject: &str, body: &str, attachment_names: &[String]) -> ContentSignals {
    let hrefs = extract_links(body);
    let mut domains: BTreeSet<String> = BTreeSet::new();
    let mut has_shortener = false;
    let mut has_raw_ip = false;

    for href in &hrefs {
        let Some(host) = host_of_url(href) else {
            continue;
        };
        if is_raw_ip_host(&host) {
            has_raw_ip = true;
            continue;
        }
        let registrable = registrable_domain(&host);
        if URL_SHORTENERS.contains(&registrable.as_str()) {
            has_shortener = true;
        }
        if !registrable.is_empty() {
            domains.insert(registrable);
        }
    }

    // Mismatch only counts when the anchor text is *itself* a URL or a bare
    // domain — "Click here" pointing anywhere is ordinary, and treating it as
    // deception would flag essentially all HTML mail.
    let mismatch = extract_anchor_pairs(body).into_iter().any(|(text, href)| {
        let text = text.trim();
        let looks_like_url = text.starts_with("http://") || text.starts_with("https://") || text.contains(".");
        if !looks_like_url || text.contains(' ') {
            return false;
        }
        match (host_of_url(text), host_of_url(&href)) {
            (Some(shown), Some(actual)) => {
                let shown = registrable_domain(&shown);
                let actual = registrable_domain(&actual);
                !shown.is_empty() && !actual.is_empty() && shown != actual
            }
            _ => false,
        }
    });

    let plain = strip_tags(body);
    let haystack = format!("{} {}", subject, plain).to_lowercase();
    let urgency_hits = distinct_phrase_hits(&haystack, URGENCY_PHRASES);
    // A credential request with nowhere to send them is not an attack.
    let credential_solicitation = CREDENTIAL_PHRASES.iter().any(|p| haystack.contains(*p)) && !hrefs.is_empty();

    ContentSignals {
        link_count: hrefs.len(),
        distinct_link_domains: domains.len(),
        link_text_href_mismatch: mismatch,
        has_url_shortener: has_shortener,
        has_raw_ip_link: has_raw_ip,
        has_hidden_text: has_hidden_text(body),
        caps_ratio: caps_ratio(subject),
        urgency_hits,
        credential_solicitation,
        dangerous_attachments: dangerous_attachments(attachment_names),
    }
}

/// Does the sender's own domain appear among the links?
///
/// Legitimate transactional mail almost always links back to itself; pure
/// link-farm spam almost never does.
pub fn links_include_sender_domain(body: &str, sender_email: &str) -> bool {
    let Some(sender_domain) = domain_of(sender_email).map(|h| registrable_domain(&h)) else {
        return false;
    };
    extract_links(body)
        .iter()
        .filter_map(|h| host_of_url(h))
        .any(|host| registrable_domain(&host) == sender_domain)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn links_are_counted_and_deduplicated_by_registrable_domain() {
        let body = r#"<a href="https://a.example/1">one</a><a href="https://mail.a.example/2">two</a>
                      <a href="https://b.example/3">three</a>"#;
        let s = analyse("", body, &[]);
        assert_eq!(s.link_count, 3);
        assert_eq!(s.distinct_link_domains, 2, "subdomains collapse to one domain");
    }

    #[test]
    fn a_raw_ip_link_is_flagged() {
        let s = analyse("", r#"<a href="http://203.0.113.77/redelivery">go</a>"#, &[]);
        assert!(s.has_raw_ip_link);
    }

    #[test]
    fn a_shortener_is_flagged() {
        let s = analyse("", r#"<a href="https://bit.ly/abc">go</a>"#, &[]);
        assert!(s.has_url_shortener);
    }

    #[test]
    fn a_url_anchor_pointing_elsewhere_is_a_mismatch() {
        let body = r#"<a href="https://collector-host.example/t/9f">https://meridianbank.example/account</a>"#;
        assert!(analyse("", body, &[]).link_text_href_mismatch);
    }

    #[test]
    fn ordinary_call_to_action_text_is_not_a_mismatch() {
        // "Click here" pointing somewhere is how all HTML mail works. Counting
        // it as deception would flag essentially every legitimate message.
        let body = r#"<a href="https://acme.example/pay">Pay now</a>"#;
        assert!(!analyse("", body, &[]).link_text_href_mismatch);
    }

    #[test]
    fn matching_anchor_text_and_href_is_not_a_mismatch() {
        let body = r#"<a href="https://acme.example/pay">https://acme.example/pay</a>"#;
        assert!(!analyse("", body, &[]).link_text_href_mismatch);
    }

    #[test]
    fn userinfo_in_a_url_does_not_disguise_the_real_host() {
        // "https://acme.example@attacker.example" resolves to attacker.example.
        let body = r#"<a href="https://acme.example@attacker.example/x">https://acme.example/x</a>"#;
        assert!(analyse("", body, &[]).link_text_href_mismatch);
    }

    #[test]
    fn executable_and_smuggling_attachments_are_flagged() {
        let s = analyse(
            "",
            "",
            &names(["scan.html", "invoice.pdf", "notes.docx", "setup.exe"].as_ref()),
        );
        assert_eq!(
            s.dangerous_attachments,
            vec!["scan.html".to_string(), "setup.exe".to_string()]
        );
    }

    #[test]
    fn ordinary_documents_are_not_flagged() {
        let s = analyse("", "", &names(["invoice.pdf", "invite.ics", "photo.jpg"].as_ref()));
        assert!(s.dangerous_attachments.is_empty());
    }

    #[test]
    fn shouting_is_measured_only_on_text_long_enough_to_mean_it() {
        assert!(analyse("LIMITED TIME ACT NOW BEFORE IT IS TOO LATE", "", &[]).caps_ratio > 0.9);
        // A short all-caps subject is not evidence of anything.
        assert_eq!(analyse("RE: OK", "", &[]).caps_ratio, 0.0);
    }

    #[test]
    fn a_newsletter_preheader_is_not_treated_as_hidden_text() {
        // REGRESSION, measured on a real mailbox: this exact styling is how
        // every modern HTML newsletter renders its preview line. The old check
        // fired on 155 of 613 messages — a quarter of everything, Upwork and
        // Substack included — at the largest content weight in the detector.
        let preheader = r#"<div style="display:none;font-size:0;color:#ffffff">Your weekly summary</div>"#;
        assert!(!analyse("", preheader, &[]).has_hidden_text);
    }

    #[test]
    fn urgency_phrases_are_counted_across_subject_and_body() {
        let s = analyse("Act now", "Your account will be suspended immediately", &[]);
        assert!(s.urgency_hits >= 2, "got {}", s.urgency_hits);
    }

    #[test]
    fn one_pressure_word_counts_once_however_many_lexicons_list_it() {
        // REGRESSION: the lexicon named "urgent" twice — once as the English
        // entry, once as the French one — so a single occurrence scored two
        // hits and tripped the `urgency_hits >= 2` threshold on its own. That
        // threshold exists precisely to require two *independent* pressure
        // phrases before the signal fires.
        assert_eq!(analyse("", "This is urgent, please read.", &[]).urgency_hits, 1);
    }

    #[test]
    fn a_word_that_merely_contains_a_shorter_entry_is_not_two_hits() {
        // Spanish "urgente" contains the English "urgent". Substring matching
        // scored that as two separate pressure phrases from one word.
        assert_eq!(analyse("", "Es urgente responder.", &[]).urgency_hits, 1);
    }

    #[test]
    fn the_urgency_lexicon_lists_no_phrase_twice() {
        let mut seen = BTreeSet::new();
        let duplicates: Vec<&&str> = URGENCY_PHRASES.iter().filter(|p| !seen.insert(**p)).collect();
        assert!(duplicates.is_empty(), "duplicated entries: {duplicates:?}");
    }

    #[test]
    fn transactional_mail_linking_to_its_own_domain_is_recognised() {
        let body = r#"<a href="https://acme.example/inv/1">invoice</a>"#;
        assert!(links_include_sender_domain(body, "billing@acme.example"));
        assert!(!links_include_sender_domain(body, "blast@offer-network.example"));
    }

    #[test]
    fn a_body_containing_length_changing_characters_does_not_panic() {
        // REGRESSION, found scoring a real mailbox: byte offsets were taken from
        // a `to_lowercase()` copy and used to slice the original. Full Unicode
        // lowercasing is not length-preserving — 'İ' (U+0130) lowercases to two
        // characters — so every offset past one shifted and the slice landed
        // mid-character. That is a panic, not a wrong answer, and it killed the
        // whole scoring batch.
        let body = format!(
            "<p>{}</p><a href=\"https://acme.example/x\">link</a> https://other.example/y",
            "İ".repeat(40)
        );
        let signals = analyse("Konu İstanbul", &body, &[]);
        assert_eq!(signals.link_count, 2, "both links found past the multi-byte run");
    }

    #[test]
    fn upper_case_markup_is_still_matched() {
        // The ASCII fold has to keep working for the thing it is there for.
        let signals = analyse("", r#"<A HREF="https://acme.example/x">go</A>"#, &[]);
        assert_eq!(signals.link_count, 1);
    }

    #[test]
    fn an_empty_body_produces_no_signals() {
        assert_eq!(analyse("", "", &[]), ContentSignals::default());
    }
}
