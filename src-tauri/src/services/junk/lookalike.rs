//! Detecting domains that impersonate one the user actually corresponds with.
//!
//! Two genuinely different attacks live here, and a detector that implements
//! only one misses most of the field:
//!
//! * **Typosquat** — `meridianbnk.example` for `meridianbank.example`. Caught by
//!   edit distance on the registrable domain.
//! * **Cousin domain** — `acme-payments.example` for `acme.example`. Edit
//!   distance is useless here (nine edits), but the victim's brand token appears
//!   as a label inside an unrelated registrable domain. This is the dominant
//!   shape in real BEC, because it reads as plausible rather than as a typo.
//!
//! Plus the Unicode tricks: punycode/homoglyph substitution, mixed scripts, and
//! invisible characters used to disguise the whole thing.

use std::collections::BTreeSet;

/// Multi-label public suffixes common enough to matter.
///
/// A full implementation would use the Public Suffix List, which is a ~10k-entry
/// dependency that has to be kept current. This covers the suffixes that appear
/// in practice; anything unlisted falls back to "last two labels", which is
/// correct for the overwhelming majority of domains. The failure mode is a
/// slightly wider registrable domain, which makes lookalike matching *less*
/// eager — the safe direction.
const MULTI_LABEL_SUFFIXES: &[&str] = &[
    "co.uk", "org.uk", "ac.uk", "gov.uk", "me.uk", "net.uk", "sch.uk", "com.au", "net.au", "org.au", "edu.au",
    "gov.au", "co.nz", "net.nz", "org.nz", "co.jp", "ne.jp", "or.jp", "ac.jp", "go.jp", "com.br", "net.br", "org.br",
    "gov.br", "com.mx", "com.ar", "com.tr", "com.cn", "net.cn", "org.cn", "gov.cn", "co.in", "net.in", "org.in",
    "co.za", "org.za", "co.kr", "com.sg", "com.hk", "com.tw", "com.my", "com.ph", "co.il", "com.pl", "com.ua",
    "com.pe", "com.co", "com.ve", "com.ec", "com.uy", "gob.es", "com.es", "org.es",
];

/// Characters that carry no visible width and exist in a domain or display name
/// only to disguise it.
const INVISIBLE: &[char] = &[
    '\u{200B}', '\u{200C}', '\u{200D}', '\u{200E}', '\u{200F}', '\u{202A}', '\u{202B}', '\u{202C}', '\u{202D}',
    '\u{202E}', '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}', '\u{FEFF}', '\u{00AD}',
];

/// Map a homoglyph to the ASCII letter it imitates.
///
/// Covers the Cyrillic and Greek letters that render identically (or near
/// enough) to Latin in the fonts mail clients use — the substitutions actually
/// used in the wild, not the full Unicode confusables table.
fn fold_confusable(c: char) -> char {
    match c {
        // Cyrillic
        'а' => 'a',
        'е' => 'e',
        'о' => 'o',
        'р' => 'p',
        'с' => 'c',
        'х' => 'x',
        'у' => 'y',
        'і' => 'i',
        'ј' => 'j',
        'ѕ' => 's',
        'һ' => 'h',
        'ԁ' => 'd',
        'ν' => 'v',
        'А' => 'A',
        'Е' => 'E',
        'О' => 'O',
        'Р' => 'P',
        'С' => 'C',
        'Х' => 'X',
        'В' => 'B',
        'Н' => 'H',
        'М' => 'M',
        'Т' => 'T',
        'К' => 'K',
        // Greek
        'ο' => 'o',
        'α' => 'a',
        'ρ' => 'p',
        'τ' => 't',
        'υ' => 'u',
        'κ' => 'k',
        'Ο' => 'O',
        'Α' => 'A',
        'Ρ' => 'P',
        'Τ' => 'T',
        'Κ' => 'K',
        'Ι' => 'I',
        'Β' => 'B',
        'Ε' => 'E',
        'Ζ' => 'Z',
        'Η' => 'H',
        // Digit/letter substitutions
        '０' => '0',
        '１' => '1',
        other => other,
    }
}

/// Normalize for comparison: lowercase, drop invisibles, fold homoglyphs.
pub fn normalize(input: &str) -> String {
    input
        .chars()
        .filter(|c| !INVISIBLE.contains(c))
        .map(fold_confusable)
        .flat_map(char::to_lowercase)
        .collect()
}

/// Does the text mix alphabets? Legitimate domains do not.
pub fn has_mixed_scripts(input: &str) -> bool {
    let mut scripts: BTreeSet<&'static str> = BTreeSet::new();
    for c in input.chars() {
        if !c.is_alphabetic() {
            continue;
        }
        let script = match c {
            'a'..='z' | 'A'..='Z' => "latin",
            '\u{0400}'..='\u{04FF}' => "cyrillic",
            '\u{0370}'..='\u{03FF}' => "greek",
            '\u{0590}'..='\u{05FF}' => "hebrew",
            '\u{0600}'..='\u{06FF}' => "arabic",
            // Accented Latin and non-alphabetic scripts (CJK etc.) are not a
            // signal on their own — plenty of legitimate mail uses them.
            _ => continue,
        };
        scripts.insert(script);
    }
    scripts.len() > 1
}

pub fn has_invisible_chars(input: &str) -> bool {
    input.chars().any(|c| INVISIBLE.contains(&c))
}

/// The host part of an address, lowercased.
pub fn domain_of(address: &str) -> Option<String> {
    let addr = address.trim().trim_matches(|c| c == '<' || c == '>').trim();
    let (_, host) = addr.rsplit_once('@')?;
    let host = host.trim().trim_end_matches('.').to_lowercase();
    if host.is_empty() {
        None
    } else {
        Some(host)
    }
}

/// The domain of an embedded address, but only when the text really is an
/// address rather than prose that happens to contain an `@`.
///
/// `"Blake @ Flippa"` is an extremely common legitimate display-name style; so
/// is `"Support @ Acme"`. Treating either as an embedded address makes the
/// impersonation check fire on ordinary marketing mail. Requiring a dot in the
/// host is what separates `"security@acme.example"` (an address, and a real
/// impersonation trick) from a company name after an at-sign.
pub fn embedded_address_domain(text: &str) -> Option<String> {
    let host = domain_of(text)?;
    let host = host.trim();
    // A bare label is a word, not a hostname.
    if !host.contains('.') {
        return None;
    }
    // The TLD has to look like one.
    let tld = host.rsplit('.').next()?;
    if tld.len() < 2 || !tld.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    Some(registrable_domain(host))
}

/// Reduce a host to its registrable domain (roughly eTLD+1).
pub fn registrable_domain(host: &str) -> String {
    let host = host.trim().trim_end_matches('.').to_lowercase();
    let labels: Vec<&str> = host.split('.').filter(|l| !l.is_empty()).collect();
    if labels.len() <= 2 {
        return labels.join(".");
    }
    let last_two = labels[labels.len() - 2..].join(".");
    if MULTI_LABEL_SUFFIXES.contains(&last_two.as_str()) && labels.len() >= 3 {
        return labels[labels.len() - 3..].join(".");
    }
    last_two
}

/// Is this an internationalized domain encoded as punycode?
pub fn is_punycode(host: &str) -> bool {
    host.split('.').any(|label| label.starts_with("xn--"))
}

/// Best-effort ASCII skeleton of a punycode label.
///
/// Punycode encodes the ASCII characters first, then a `-` separator, then the
/// encoded non-ASCII insertions. Taking the part before the final separator
/// recovers the ASCII letters an attacker kept — which is exactly what makes
/// `xn--meridinbank-9db` (a Cyrillic 'а' in "meridianbank") comparable to the
/// real thing. This is a heuristic, not a punycode decoder: it recovers enough
/// for distance comparison without pulling in an IDNA dependency.
fn punycode_skeleton(label: &str) -> Option<String> {
    let rest = label.strip_prefix("xn--")?;
    let ascii = rest.rsplit_once('-').map(|(head, _)| head).unwrap_or(rest);
    if ascii.is_empty() {
        None
    } else {
        Some(ascii.to_string())
    }
}

/// The brand token of a registrable domain: the label before the suffix.
pub fn brand_token(registrable: &str) -> String {
    registrable.split('.').next().unwrap_or(registrable).to_string()
}

/// Damerau–Levenshtein distance (optimal string alignment).
///
/// Transpositions count as one edit, which matters because `acme` → `acem` is a
/// single slip of the fingers, not two.
pub fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }

    let mut prev_prev: Vec<usize> = vec![0; b.len() + 1];
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr: Vec<usize> = vec![0; b.len() + 1];

    for i in 1..=a.len() {
        curr[0] = i;
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                curr[j] = curr[j].min(prev_prev[j - 2] + cost);
            }
        }
        std::mem::swap(&mut prev_prev, &mut prev);
        std::mem::swap(&mut prev, &mut curr);
    }

    prev[b.len()]
}

/// Why a domain was judged to imitate another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LookalikeKind {
    /// A near-miss spelling of the real domain.
    Typosquat,
    /// The real domain's brand token embedded in an unrelated domain.
    Cousin,
    /// Punycode whose ASCII skeleton is a near-miss of the real domain.
    Homoglyph,
}

/// Edit-distance budget, scaled to length: short brands must match tighter,
/// because at distance 2 almost every four-letter word is "close" to every other.
fn distance_budget(len: usize) -> usize {
    match len {
        0..=4 => 1,
        5..=8 => 2,
        _ => 3,
    }
}

/// Does `candidate` imitate any of `references`?
///
/// An exact match is never a lookalike — that is just the real domain.
pub fn detect(candidate_host: &str, references: &[String]) -> Option<(LookalikeKind, String)> {
    let candidate = registrable_domain(&normalize(candidate_host));
    if candidate.is_empty() {
        return None;
    }
    let candidate_brand = brand_token(&candidate);

    // Punycode is compared on its recovered ASCII skeleton.
    let skeleton = candidate_host
        .split('.')
        .find_map(punycode_skeleton)
        .map(|s| normalize(&s));

    for reference in references {
        let reference = registrable_domain(&normalize(reference));
        if reference.is_empty() || reference == candidate {
            continue;
        }
        let reference_brand = brand_token(&reference);
        if reference_brand.is_empty() {
            continue;
        }

        if let Some(skeleton) = &skeleton {
            if edit_distance(skeleton, &reference_brand) <= distance_budget(reference_brand.len()) {
                return Some((LookalikeKind::Homoglyph, reference));
            }
        }

        if edit_distance(&candidate_brand, &reference_brand) <= distance_budget(reference_brand.len()) {
            return Some((LookalikeKind::Typosquat, reference));
        }

        // Cousin domain: the brand appears as a separate token. Split on the
        // separators an attacker uses to build a plausible-looking hostname, so
        // "acme-payments" and "secure.acme" match but "acmeworks" — a different
        // company whose name merely starts the same way — does not.
        let is_cousin = candidate
            .split(['-', '.'])
            .any(|token| token == reference_brand && reference_brand.len() >= 3);
        if is_cousin {
            return Some((LookalikeKind::Cousin, reference));
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refs(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_string()).collect()
    }

    // ── registrable domain ────────────────────────────────────────────────

    #[test]
    fn subdomains_reduce_to_the_registrable_domain() {
        assert_eq!(registrable_domain("mail.corp.acme.example"), "acme.example");
        assert_eq!(registrable_domain("acme.example"), "acme.example");
    }

    #[test]
    fn multi_label_suffixes_keep_three_labels() {
        // Reducing "shop.co.uk" to "co.uk" would make every .co.uk domain look
        // like every other.
        assert_eq!(registrable_domain("shop.co.uk"), "shop.co.uk");
        assert_eq!(registrable_domain("mail.shop.co.uk"), "shop.co.uk");
        assert_eq!(registrable_domain("tienda.com.es"), "tienda.com.es");
    }

    #[test]
    fn a_name_after_an_at_sign_is_not_an_embedded_address() {
        // Regression from a real mailbox: "Blake @ Flippa" is a normal display
        // name used by marketing senders. Reading "Flippa" as a domain made the
        // impersonation check fire on ordinary legitimate mail.
        assert_eq!(embedded_address_domain("Blake @ Flippa"), None);
        assert_eq!(embedded_address_domain("Support @ Acme"), None);
        assert_eq!(embedded_address_domain("Tory @ Flippa"), None);
    }

    #[test]
    fn a_real_address_in_a_display_name_is_extracted() {
        // The actual trick: a display name written to read as an address, so
        // clients that truncate show the wrong identity.
        assert_eq!(
            embedded_address_domain("security@acme.example").as_deref(),
            Some("acme.example")
        );
        // Spaces around the at-sign must not defeat it.
        assert_eq!(
            embedded_address_domain("security @ acme.example").as_deref(),
            Some("acme.example")
        );
    }

    #[test]
    fn text_with_no_at_sign_has_no_embedded_address() {
        assert_eq!(embedded_address_domain("Northwind Billing"), None);
    }

    #[test]
    fn domain_is_extracted_from_an_address() {
        assert_eq!(domain_of("a@Mail.Acme.Example").as_deref(), Some("mail.acme.example"));
        assert_eq!(domain_of("<bounce@x.example>").as_deref(), Some("x.example"));
        assert_eq!(domain_of("not-an-address"), None);
    }

    // ── the two attack shapes ─────────────────────────────────────────────

    #[test]
    fn a_cousin_domain_embedding_the_brand_is_detected() {
        // The dominant BEC shape, and the one edit distance cannot see: nine
        // edits from the real domain, yet instantly plausible to a human.
        let hit = detect("acme-payments.example", &refs(&["acme.example"]));
        assert_eq!(hit, Some((LookalikeKind::Cousin, "acme.example".to_string())));
    }

    #[test]
    fn a_typosquat_within_the_distance_budget_is_detected() {
        let hit = detect("meridianbnk.example", &refs(&["meridianbank.example"]));
        assert_eq!(
            hit,
            Some((LookalikeKind::Typosquat, "meridianbank.example".to_string()))
        );
    }

    #[test]
    fn a_punycode_homoglyph_is_compared_on_its_ascii_skeleton() {
        // xn--meridinbank-9db is "meridianbank" with a Cyrillic 'а'.
        let hit = detect("xn--meridinbank-9db.example", &refs(&["meridianbank.example"]));
        assert_eq!(
            hit,
            Some((LookalikeKind::Homoglyph, "meridianbank.example".to_string()))
        );
    }

    #[test]
    fn the_real_domain_is_never_its_own_lookalike() {
        assert_eq!(detect("acme.example", &refs(&["acme.example"])), None);
        assert_eq!(detect("mail.acme.example", &refs(&["acme.example"])), None);
    }

    #[test]
    fn an_unrelated_domain_is_not_flagged() {
        assert_eq!(detect("northwind.example", &refs(&["acme.example"])), None);
        assert_eq!(detect("partnerco.example", &refs(&["acme.example"])), None);
    }

    #[test]
    fn a_company_whose_name_merely_starts_the_same_is_not_a_cousin() {
        // "acmeworks" is a different company, not "acme" wearing a disguise.
        // Requiring a separator is what keeps this from being a false positive
        // generator.
        assert_eq!(detect("acmeworks.example", &refs(&["acme.example"])), None);
    }

    #[test]
    fn short_brands_get_a_tighter_distance_budget() {
        // At distance 2 nearly every four-letter brand is "close" to every
        // other, which would badge huge amounts of unrelated legitimate mail.
        assert!(detect("bank.example", &refs(&["bark.example"])).is_some());
        assert_eq!(detect("bond.example", &refs(&["bark.example"])), None);
    }

    #[test]
    fn no_references_means_no_verdict() {
        assert_eq!(detect("anything.example", &[]), None);
    }

    // ── unicode tricks ────────────────────────────────────────────────────

    #[test]
    fn cyrillic_homoglyphs_fold_to_their_latin_shapes() {
        // "асme" with Cyrillic а and с.
        assert_eq!(normalize("\u{0430}\u{0441}me"), "acme");
    }

    #[test]
    fn invisible_characters_are_stripped_and_reported() {
        let sneaky = "acme\u{200B}.example";
        assert!(has_invisible_chars(sneaky));
        assert_eq!(normalize(sneaky), "acme.example");
    }

    #[test]
    fn mixing_alphabets_is_detected() {
        assert!(has_mixed_scripts("\u{0430}cme"), "cyrillic + latin");
        assert!(!has_mixed_scripts("acme"));
        // Accented Latin is ordinary, not an attack.
        assert!(!has_mixed_scripts("Müller GmbH"));
    }

    // ── edit distance ─────────────────────────────────────────────────────

    #[test]
    fn a_transposition_costs_one_edit() {
        assert_eq!(edit_distance("acme", "acem"), 1);
    }

    #[test]
    fn distance_is_symmetric_and_zero_for_equal_strings() {
        assert_eq!(edit_distance("acme", "acme"), 0);
        assert_eq!(edit_distance("abc", "abd"), edit_distance("abd", "abc"));
    }

    #[test]
    fn empty_strings_cost_their_full_length() {
        assert_eq!(edit_distance("", "acme"), 4);
        assert_eq!(edit_distance("acme", ""), 4);
    }
}
