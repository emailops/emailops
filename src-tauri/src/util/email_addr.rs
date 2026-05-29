/// Free / personal email providers whose domain alone is not a useful
/// "company" label — every individual on `gmail.com` is a different person,
/// not the same organisation. When [`company_label_for`] sees one of these
/// domains it falls back to the individual's address so the company tag
/// distinguishes `alice@gmail.com` from `bob@gmail.com`.
///
/// Match is *exact-domain only*: `mail.google.com` (corporate Workspace
/// inbound mail) does NOT match `gmail.com`. Subdomains of these providers
/// are extremely rare for end-user addresses and treating them as personal
/// would be wrong for the few cases where they appear.
pub(crate) const PERSONAL_EMAIL_DOMAINS: &[&str] = &[
    // Google
    "gmail.com",
    "googlemail.com",
    // Microsoft
    "outlook.com",
    "hotmail.com",
    "live.com",
    "msn.com",
    // ↓ Country variants. Not exhaustive — covers the heavy hitters we've
    // actually seen in user mailboxes. Add more as they show up.
    "outlook.es",
    "outlook.fr",
    "outlook.de",
    "outlook.it",
    "outlook.com.br",
    "outlook.com.ar",
    "outlook.com.mx",
    "outlook.co.uk",
    "hotmail.es",
    "hotmail.fr",
    "hotmail.de",
    "hotmail.it",
    "hotmail.com.br",
    "hotmail.com.ar",
    "hotmail.com.mx",
    "hotmail.co.uk",
    "hotmail.co",
    "live.es",
    "live.fr",
    "live.de",
    "live.it",
    "live.com.mx",
    "live.com.ar",
    "live.co.uk",
    // Yahoo
    "yahoo.com",
    "yahoo.co.uk",
    "yahoo.es",
    "yahoo.fr",
    "yahoo.de",
    "yahoo.it",
    "yahoo.com.br",
    "yahoo.com.ar",
    "yahoo.com.mx",
    "yahoo.ca",
    "yahoo.com.au",
    "ymail.com",
    "rocketmail.com",
    // Apple
    "icloud.com",
    "me.com",
    "mac.com",
    // Proton
    "proton.me",
    "protonmail.com",
    "pm.me",
    // AOL
    "aol.com",
    // GMX / Mail.com / Fastmail / Zoho personal
    "gmx.com",
    "gmx.de",
    "gmx.net",
    "gmx.es",
    "gmx.fr",
    "gmx.at",
    "gmx.ch",
    "gmx.co.uk",
    "mail.com",
    "fastmail.com",
    "fastmail.fm",
    "zoho.com",
    "tutanota.com",
    "tutamail.com",
    "tuta.io",
    // Common ES / EU ISPs that act as personal mail
    "telefonica.net",
    "movistar.es",
    "terra.es",
    "ya.com",
];

/// Returns `true` when `domain` is a known free / personal email provider
/// for which a per-individual label is more useful than a per-domain one.
/// Comparison is case-insensitive and exact (no subdomain match) — see the
/// comment on [`PERSONAL_EMAIL_DOMAINS`].
pub fn is_personal_email_domain(domain: &str) -> bool {
    let d = domain.trim().to_ascii_lowercase();
    PERSONAL_EMAIL_DOMAINS.iter().any(|p| *p == d)
}

/// Build the company tag for an envelope side. For a corporate domain this
/// strips the right-most TLD label (`acme.com` → `acme`); for a personal
/// provider it returns the full lowercased address so individuals stay
/// distinct (`alice@gmail.com` → `alice@gmail.com`).
///
/// `anchor_address` is the specific address that contributed `domain`
/// (the sender for inbound, or the dominant recipient for outbound). If
/// the address is unavailable for a personal-domain hit, we fall back to
/// the bare domain rather than producing no label.
pub fn company_label_for(domain: &str, anchor_address: Option<&str>) -> String {
    let d = domain.trim().trim_matches('.').to_ascii_lowercase();
    if is_personal_email_domain(&d) {
        if let Some(addr) = anchor_address {
            let a = addr.trim().to_ascii_lowercase();
            if !a.is_empty() {
                return a;
            }
        }
        return d;
    }
    match d.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem.to_string(),
        _ => d,
    }
}

/// Extract the domain part of an email address.
///
/// - Trims surrounding whitespace.
/// - Strips trailing non-alphanumeric characters (e.g. stray dots / punctuation).
/// - Lowercases the result.
/// - Returns `None` if the input has no `@`, or the domain is empty after cleanup.
pub fn extract_domain(addr: &str) -> Option<String> {
    addr.trim()
        .rsplit_once('@')
        .map(|(_, d)| d.trim_matches(|c: char| !c.is_alphanumeric() && c != '.' && c != '-'))
        .filter(|d| !d.is_empty())
        .map(|d| d.to_ascii_lowercase())
}

/// Parse a JSON array stored in `recipients_json`/`cc_json` into the raw
/// recipient strings (which may include `Name <email@host>` formatting). Best
/// effort: a malformed payload yields an empty vec rather than failing the
/// whole contact aggregation.
pub fn parse_addr_list(json_str: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(json_str).unwrap_or_default()
}

/// Split `Name <email@host>` into `(name, lowercased_email)`. Falls back to
/// treating the whole string as an address when no angle brackets are present.
pub fn split_name_addr(raw: &str) -> (String, String) {
    let trimmed = raw.trim();
    if let (Some(start), Some(end)) = (trimmed.find('<'), trimmed.find('>')) {
        if start < end {
            let name = trimmed[..start].trim().trim_matches('"').to_string();
            let addr = trimmed[start + 1..end].trim().to_lowercase();
            return (name, addr);
        }
    }
    (String::new(), trimmed.to_lowercase())
}

/// Heuristic classification: addresses whose local-part matches a known
/// automated/system/role pattern (or whose domain belongs to a known
/// newsletter platform) get `automated`; everything else is `person`.
/// Multilingual: covers common English + Spanish role names.
pub fn classify_kind(email: &str) -> &'static str {
    let lc = email.to_ascii_lowercase();
    let (local, domain) = match lc.split_once('@') {
        Some((l, d)) => (l, d),
        None => return "person",
    };

    // Exact-match local parts that are unambiguously role / automated.
    const EXACT: &[&str] = &[
        // No-reply variants
        "noreply",
        "no-reply",
        "no_reply",
        "donotreply",
        "do-not-reply",
        "do_not_reply",
        "noresponder",
        "no-responder",
        "no_responder",
        // System
        "mailer-daemon",
        "postmaster",
        "automated",
        "bounces",
        "abuse",
        "admin",
        "webmaster",
        "root",
        "system",
        // Notifications / alerts
        "alert",
        "alerts",
        "alerta",
        "alertas",
        "notification",
        "notifications",
        "notify",
        "notifies",
        "news",
        "newsletter",
        "newsletters",
        "boletin",
        // Generic role (English + Spanish + a few common others)
        "info",
        "hi",
        "hello",
        "hola",
        "support",
        "soporte",
        "help",
        "ayuda",
        "contact",
        "contacto",
        "sales",
        "ventas",
        "tienda",
        "shop",
        "store",
        "office",
        "team",
        "equipo",
        "billing",
        "facturacion",
        "security",
        "seguridad",
        "invoice",
        "invoices",
        "factura",
        "facturas",
        "receipt",
        "receipts",
        "recibo",
        "recibos",
        "payment",
        "payments",
        "pago",
        "pagos",
        "replies",
        "reply",
        "chat",
        // Order / shipping
        "order",
        "orders",
        "pedido",
        "pedidos",
        "shipment",
        "shipments",
        "shipping",
        "envio",
        "envios",
        "order-update",
        "order-updates",
        "delivery",
        // Confirmations / updates
        "confirmar",
        "confirmation",
        "confirmations",
        "confirm",
        "update",
        "updates",
        "actualizacion",
        // Misc
        "feedback",
        "marketing",
        "press",
        "prensa",
        "events",
        "eventos",
        "careers",
        "jobs",
        "hr",
        "rrhh",
        "welcome",
    ];
    if EXACT.contains(&local) {
        return "automated";
    }

    // Substring match for compound forms like `noreply-12345@…`,
    // `ses-bounces-…`, `order-updates@…`, `newsletter-2024@…`.
    const NEEDLES: &[&str] = &[
        "no-reply",
        "noreply",
        "no_reply",
        "donotreply",
        "do-not-reply",
        "do_not_reply",
        "noresponder",
        "no-responder",
        "mailer-daemon",
        "postmaster",
        "bounces",
        "bounce-",
        "automated",
        "notification",
        "notifications",
        "notify",
        "newsletter",
        "newsletters",
        "order-update",
        "order-updates",
        "shipment",
        "shipping",
        "confirmar",
        "confirmation",
        "alerts-",
        "alert-",
        "billing",
        "invoice",
        "invoices",
        "receipts",
    ];
    for n in NEEDLES {
        if local.contains(n) {
            return "automated";
        }
    }

    // Domain-based: known newsletter / no-reply platforms. Match either an
    // exact domain or a `*.<platform>` subdomain.
    const AUTO_DOMAINS: &[&str] = &[
        "substack.com",
        "mailchi.mp",
        "convertkit-mail.com",
        "convertkit-mail2.com",
    ];
    for d in AUTO_DOMAINS {
        if domain == *d || domain.ends_with(&format!(".{d}")) {
            return "automated";
        }
    }

    // Brand pattern: local part equals the registrable domain root
    // (e.g. `sifted@sifted.eu`, `medium@medium.com`, `linkedin@linkedin.com`).
    // Almost always a brand newsletter / corporate sender, not a person.
    // Heuristic: take the second-to-last label of the domain (the SLD), which
    // works for both `sifted.eu` and `mail.sifted.com`. Skip very short local
    // parts (< 3 chars) to avoid `eu@example.eu`-style false positives. This
    // can over-classify rare vanity domains like `john@johndoe.com`, an
    // accepted trade-off for catching the much more common brand pattern.
    let labels: Vec<&str> = domain.split('.').filter(|s| !s.is_empty()).collect();
    if labels.len() >= 2 {
        let root = labels[labels.len() - 2];
        if local.len() >= 3 && !root.is_empty() && local == root {
            return "automated";
        }
    }

    "person"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn personal_domain_matches_exact() {
        assert!(is_personal_email_domain("gmail.com"));
        assert!(is_personal_email_domain("GMAIL.COM"));
        assert!(is_personal_email_domain("  yahoo.co.uk "));
        assert!(is_personal_email_domain("icloud.com"));
        assert!(!is_personal_email_domain("acme.com"));
        // Subdomains do NOT match — corporate Workspace mail must be tagged
        // by domain, not by individual.
        assert!(!is_personal_email_domain("mail.google.com"));
        assert!(!is_personal_email_domain("foo.gmail.com"));
    }

    #[test]
    fn company_label_strips_tld_for_corporate() {
        assert_eq!(company_label_for("acme.com", None), "acme");
        assert_eq!(
            company_label_for("acme.com", Some("alice@acme.com")),
            "acme",
            "address must be ignored for corporate domains"
        );
        assert_eq!(company_label_for("foo.acme.com", None), "foo.acme");
    }

    #[test]
    fn company_label_uses_address_for_personal() {
        assert_eq!(
            company_label_for("gmail.com", Some("Alice@Gmail.com")),
            "alice@gmail.com"
        );
        assert_eq!(
            company_label_for("yahoo.co.uk", Some("bob@yahoo.co.uk")),
            "bob@yahoo.co.uk"
        );
    }

    #[test]
    fn company_label_personal_without_address_falls_back_to_domain() {
        // Should never happen in practice (caller always has the address),
        // but degrade gracefully rather than panic / return "".
        assert_eq!(company_label_for("gmail.com", None), "gmail.com");
    }
}
