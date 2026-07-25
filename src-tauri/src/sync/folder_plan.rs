//! Pure planner for interpreting an IMAP `LIST` response: decodes modified
//! UTF-7 names, maps SPECIAL-USE attributes (RFC 6154) to well-known roles,
//! falls back to localized folder-name candidates (en/de/es/fr), and
//! classifies the remaining selectable folders as custom folders to sync.
//!
//! No I/O — `ImapClient` feeds `LIST` results in, the sync orchestrator
//! persists the resulting plan. Unit-tested exhaustively; the executor side
//! is covered by integration tests.

use std::collections::HashMap;

/// Raw `LIST` entry as reported by the server (wire-format name, undecoded).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedFolder {
    /// Modified UTF-7 wire name, used verbatim for `SELECT`.
    pub raw_name: String,
    pub delimiter: Option<String>,
    /// Name attributes as reported, e.g. `["\\HasNoChildren", "\\Sent"]`.
    pub attributes: Vec<String>,
}

/// The three well-known mailboxes the sync pipeline treats specially.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WellKnownFolder {
    Sent,
    Spam,
    Trash,
}

/// A selectable non-role folder that should be synced as a custom folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedCustomFolder {
    /// Wire name for `SELECT` and stable identity.
    pub raw_name: String,
    /// Decoded UTF-8 full path for display.
    pub display_name: String,
    pub delimiter: Option<String>,
}

/// Result of interpreting one `LIST` response.
#[derive(Debug, Clone, Default)]
pub struct FolderPlan {
    /// Well-known role -> raw server name to `SELECT` for that role.
    pub roles: HashMap<WellKnownFolder, String>,
    /// Folders to sync as custom, in server order.
    pub custom: Vec<PlannedCustomFolder>,
}

/// Sent folder candidates across providers and locales, tried in order.
/// English names first (backward compatibility with the historic hardcoded
/// lists), then German / Spanish / French.
pub const SENT_FOLDER_CANDIDATES: &[&str] = &[
    "Sent",
    "Sent Messages", // iCloud
    "Sent Items",    // Outlook/Exchange
    "INBOX.Sent",    // Courier/Dovecot
    "Gesendet",
    "Gesendete Objekte", // IONOS/GMX/Telekom webmail
    "Gesendete Elemente",
    "Enviados",
    "Elementos enviados",
    "Mensajes enviados",
    "Envoyés",
    "Éléments envoyés",
    "Messages envoyés",
];

/// Spam / Junk folder candidates, tried in order.
pub const SPAM_FOLDER_CANDIDATES: &[&str] = &[
    "Spam",
    "Junk",
    "Junk E-mail", // Outlook/Exchange
    "Junk Email",
    "INBOX.Spam",
    "INBOX.Junk",
    "Spamverdacht", // GMX/Web.de
    "Spam-Verdacht",
    "Werbung",
    "Unerwünscht",
    "Correo no deseado",
    "No deseado",
    "Courrier indésirable",
    "Pourriels",
    "Indésirables",
];

/// Trash / Deleted Items folder candidates, tried in order.
pub const TRASH_FOLDER_CANDIDATES: &[&str] = &[
    "Trash",
    "Deleted",
    "Deleted Items",    // Outlook/Exchange
    "Deleted Messages", // iCloud
    "INBOX.Trash",
    "Papierkorb",
    "Gelöscht",
    "Geloescht",
    "Gelöschte Objekte",
    "Gelöschte Elemente",
    "Papelera",
    "Elementos eliminados",
    "Corbeille",
    "Éléments supprimés",
];

/// Drafts folders are excluded from sync entirely (drafts are handled by the
/// dedicated provider-drafts pipeline, not the mail sync).
const DRAFTS_FOLDER_CANDIDATES: &[&str] = &[
    "Drafts",
    "Draft",
    "INBOX.Drafts",
    "Entwürfe",
    "Entwuerfe",
    "Borradores",
    "Brouillons",
];

impl WellKnownFolder {
    pub fn as_str(&self) -> &'static str {
        match self {
            WellKnownFolder::Sent => "sent",
            WellKnownFolder::Spam => "spam",
            WellKnownFolder::Trash => "trash",
        }
    }

    /// RFC 6154 SPECIAL-USE attribute claiming this role.
    fn special_use_attr(&self) -> &'static str {
        match self {
            WellKnownFolder::Sent => "\\Sent",
            WellKnownFolder::Spam => "\\Junk",
            WellKnownFolder::Trash => "\\Trash",
        }
    }

    fn name_candidates(&self) -> &'static [&'static str] {
        match self {
            WellKnownFolder::Sent => SENT_FOLDER_CANDIDATES,
            WellKnownFolder::Spam => SPAM_FOLDER_CANDIDATES,
            WellKnownFolder::Trash => TRASH_FOLDER_CANDIDATES,
        }
    }
}

fn has_attr(entry: &ListedFolder, attr: &str) -> bool {
    entry.attributes.iter().any(|a| a.eq_ignore_ascii_case(attr))
}

/// A folder we must never SELECT or sync from.
fn is_unselectable(entry: &ListedFolder) -> bool {
    has_attr(entry, "\\Noselect") || has_attr(entry, "\\NonExistent")
}

/// Case-insensitive match (full Unicode lowercase — `eq_ignore_ascii_case`
/// would miss "Gelöscht" vs "gelöscht") of a candidate against the decoded
/// folder name or its last path segment (so `INBOX.Gesendet` matches
/// "Gesendet").
fn name_matches(entry: &ListedFolder, candidate: &str) -> bool {
    let decoded = decode_imap_utf7(&entry.raw_name);
    let candidate = candidate.to_lowercase();
    if decoded.to_lowercase() == candidate {
        return true;
    }
    if let Some(delim) = entry.delimiter.as_deref().filter(|d| !d.is_empty()) {
        if let Some(last) = decoded.rsplit(delim).next() {
            if last.to_lowercase() == candidate {
                return true;
            }
        }
    }
    false
}

/// Resolve which server folder fills a well-known role, using the detection
/// ladder: SPECIAL-USE attribute first, then localized name candidates in
/// priority order. Returns the raw wire name to `SELECT`, or `None` when the
/// server exposes no such folder.
pub fn resolve_role_folder(role: WellKnownFolder, entries: &[ListedFolder]) -> Option<String> {
    if let Some(entry) = entries
        .iter()
        .find(|e| !is_unselectable(e) && has_attr(e, role.special_use_attr()))
    {
        return Some(entry.raw_name.clone());
    }
    for candidate in role.name_candidates() {
        if let Some(entry) = entries
            .iter()
            .find(|e| !is_unselectable(e) && name_matches(e, candidate))
        {
            return Some(entry.raw_name.clone());
        }
    }
    None
}

fn is_drafts(entry: &ListedFolder) -> bool {
    has_attr(entry, "\\Drafts")
        || DRAFTS_FOLDER_CANDIDATES
            .iter()
            .any(|candidate| name_matches(entry, candidate))
}

/// Virtual views that duplicate mail present elsewhere — syncing them would
/// double every message.
fn is_virtual_view(entry: &ListedFolder) -> bool {
    has_attr(entry, "\\All") || has_attr(entry, "\\Flagged")
}

/// Interpret one `LIST` response: assign well-known roles and classify every
/// remaining selectable folder as a custom folder to sync. Skips INBOX (the
/// primary pass owns it), role winners, `\Noselect` containers, Drafts, and
/// virtual views. `\Archive` folders ARE included — users file real mail
/// there.
pub fn plan_folders(entries: &[ListedFolder]) -> FolderPlan {
    let mut plan = FolderPlan::default();
    for role in [WellKnownFolder::Sent, WellKnownFolder::Spam, WellKnownFolder::Trash] {
        if let Some(raw) = resolve_role_folder(role, entries) {
            plan.roles.insert(role, raw);
        }
    }
    let role_winners: Vec<&String> = plan.roles.values().collect();
    for entry in entries {
        if is_unselectable(entry)
            || entry.raw_name.eq_ignore_ascii_case("INBOX")
            || role_winners.iter().any(|w| **w == entry.raw_name)
            || is_drafts(entry)
            || is_virtual_view(entry)
        {
            continue;
        }
        plan.custom.push(PlannedCustomFolder {
            raw_name: entry.raw_name.clone(),
            display_name: decode_imap_utf7(&entry.raw_name),
            delimiter: entry.delimiter.clone(),
        });
    }
    plan
}

/// Decode an RFC 3501 modified UTF-7 mailbox name to UTF-8 for display and
/// candidate matching (`"Entw&APw-rfe"` → `"Entwürfe"`). Malformed input is
/// returned unchanged rather than erroring — a display-layer concern must
/// never break sync.
pub fn decode_imap_utf7(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c != '&' {
            out.push(c);
            continue;
        }
        // Shifted section: collect up to the terminating '-'.
        let mut b64 = String::new();
        let mut terminated = false;
        for s in chars.by_ref() {
            if s == '-' {
                terminated = true;
                break;
            }
            b64.push(s);
        }
        if !terminated {
            // Malformed (unterminated shift) — bail out verbatim.
            out.push('&');
            out.push_str(&b64);
            return out;
        }
        if b64.is_empty() {
            out.push('&'); // "&-" is the literal ampersand
            continue;
        }
        match decode_utf7_b64_section(&b64) {
            Some(decoded) => out.push_str(&decoded),
            None => return raw.to_string(), // malformed — display as-is
        }
    }
    out
}

/// Decode one shifted section: modified base64 (',' instead of '/', no
/// padding) holding UTF-16BE code units.
fn decode_utf7_b64_section(section: &str) -> Option<String> {
    use base64::engine::general_purpose::STANDARD_NO_PAD;
    use base64::Engine;
    let standard: String = section.replace(',', "/");
    let bytes = STANDARD_NO_PAD.decode(standard).ok()?;
    // UTF-16 needs an even byte count — anything else is malformed.
    if bytes.len() % 2 != 0 {
        return None;
    }
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
        .collect();
    let decoded: String = char::decode_utf16(units)
        .collect::<std::result::Result<String, _>>()
        .ok()?;
    Some(decoded)
}

/// Encode a UTF-8 mailbox name to RFC 3501 modified UTF-7 for the wire
/// (`"Entwürfe"` → `"Entw&APw-rfe"`). Inverse of [`decode_imap_utf7`];
/// printable US-ASCII passes through, `&` becomes `&-`, everything else is
/// base64(UTF-16BE) with `,` for `/` and no padding.
pub fn encode_imap_utf7(name: &str) -> String {
    use base64::engine::general_purpose::STANDARD_NO_PAD;
    use base64::Engine;

    fn flush(out: &mut String, pending: &mut Vec<u16>) {
        if pending.is_empty() {
            return;
        }
        let bytes: Vec<u8> = pending.iter().flat_map(|u| u.to_be_bytes()).collect();
        let b64 = STANDARD_NO_PAD.encode(&bytes).replace('/', ",");
        out.push('&');
        out.push_str(&b64);
        out.push('-');
        pending.clear();
    }

    let mut out = String::with_capacity(name.len());
    let mut pending: Vec<u16> = Vec::new();
    for c in name.chars() {
        if c == '&' {
            flush(&mut out, &mut pending);
            out.push_str("&-");
        } else if (' '..='~').contains(&c) {
            flush(&mut out, &mut pending);
            out.push(c);
        } else {
            let mut buf = [0u16; 2];
            pending.extend_from_slice(c.encode_utf16(&mut buf));
        }
    }
    flush(&mut out, &mut pending);
    out
}

/// Why a proposed folder display name was rejected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FolderNameError {
    #[error("folder name is empty")]
    Empty,
    #[error("folder name is too long (max {MAX_FOLDER_NAME_CHARS} characters)")]
    TooLong,
    #[error("folder name contains a forbidden character: {0:?}")]
    ForbiddenChar(char),
    #[error("'INBOX' is a reserved folder name")]
    Reserved,
}

const MAX_FOLDER_NAME_CHARS: usize = 100;

/// Validate a user-entered folder display name (a single path segment).
/// Rejects the account's hierarchy delimiter (it would silently create a
/// nested folder), IMAP wildcard/quoting characters, control characters,
/// and the reserved name INBOX. The caller trims whitespace first.
pub fn validate_folder_name(name: &str, delimiter: Option<&str>) -> Result<(), FolderNameError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(FolderNameError::Empty);
    }
    if name.chars().count() > MAX_FOLDER_NAME_CHARS {
        return Err(FolderNameError::TooLong);
    }
    if name.eq_ignore_ascii_case("INBOX") {
        return Err(FolderNameError::Reserved);
    }
    let delim_chars: Vec<char> = delimiter.map(|d| d.chars().collect()).unwrap_or_default();
    for c in name.chars() {
        if c.is_control() {
            return Err(FolderNameError::ForbiddenChar(c));
        }
        if matches!(c, '*' | '%' | '"' | '\\') || delim_chars.contains(&c) {
            return Err(FolderNameError::ForbiddenChar(c));
        }
    }
    Ok(())
}

/// Compose the wire path for a new folder from its (validated) display name.
///
/// Layout heuristic: when every existing folder lives under `INBOX<delim>`
/// (Dovecot/Courier style), the new folder nests there too; otherwise it is
/// created top-level (IONOS/most providers). Non-ASCII names are UTF-7
/// encoded for the wire.
pub fn compose_folder_path(name: &str, delimiter: Option<&str>, existing_paths: &[String]) -> String {
    let encoded = encode_imap_utf7(name.trim());
    let Some(delim) = delimiter.filter(|d| !d.is_empty()) else {
        return encoded;
    };
    let prefix = format!("INBOX{delim}");
    let all_under_inbox = !existing_paths.is_empty()
        && existing_paths
            .iter()
            .filter(|p| !p.eq_ignore_ascii_case("INBOX"))
            .all(|p| {
                p.len() > prefix.len()
                    && p.get(..prefix.len())
                        .is_some_and(|head| head.eq_ignore_ascii_case(&prefix))
            });
    if all_under_inbox {
        format!("{prefix}{encoded}")
    } else {
        encoded
    }
}

/// Compose the wire path for renaming `old_path`'s last segment to `name`,
/// keeping the folder under its current parent. Non-ASCII names are UTF-7
/// encoded for the wire.
pub fn rename_sibling_path(old_path: &str, delimiter: Option<&str>, name: &str) -> String {
    let encoded = encode_imap_utf7(name.trim());
    let Some(delim) = delimiter.filter(|d| !d.is_empty()) else {
        return encoded;
    };
    match old_path.rfind(delim) {
        Some(idx) => format!("{}{delim}{encoded}", &old_path[..idx]),
        None => encoded,
    }
}

/// Parse the raw text of a `LIST` response (one entry per line) into
/// `ListedFolder`s. Used for the `LIST "" "*" RETURN (SPECIAL-USE)` fallback
/// where the typed API of the `imap` crate is unavailable.
///
/// Handles: `* LIST (\HasNoChildren \Sent) "." "Gesendete Objekte"`,
/// unquoted atom names, and `NIL` delimiters. Lines that are not LIST
/// responses are ignored.
pub fn parse_list_response(raw: &str) -> Vec<ListedFolder> {
    let mut folders = Vec::new();
    for line in raw.lines() {
        let line = line.trim().trim_start_matches("* ");
        let Some(rest) = line.strip_prefix("LIST ").or_else(|| line.strip_prefix("list ")) else {
            continue;
        };
        // Attributes: parenthesized list.
        let rest = rest.trim_start();
        let Some(attrs_end) = rest.find(')') else { continue };
        let Some(attrs_body) = rest.get(1..attrs_end) else {
            continue;
        };
        if !rest.starts_with('(') {
            continue;
        }
        let attributes: Vec<String> = attrs_body.split_whitespace().map(|s| s.to_string()).collect();
        let rest = rest[attrs_end + 1..].trim_start();
        // Delimiter: quoted single char or NIL.
        let (delimiter, rest) = if let Some(stripped) = rest.strip_prefix("NIL") {
            (None, stripped.trim_start())
        } else if let Some(stripped) = rest.strip_prefix('"') {
            match stripped.find('"') {
                Some(end) => (Some(stripped[..end].to_string()), stripped[end + 1..].trim_start()),
                None => continue,
            }
        } else {
            continue;
        };
        // Mailbox name: quoted string or bare atom.
        let raw_name = if let Some(stripped) = rest.strip_prefix('"') {
            match stripped.rfind('"') {
                Some(end) => stripped[..end].to_string(),
                None => continue,
            }
        } else {
            rest.trim().to_string()
        };
        if raw_name.is_empty() {
            continue;
        }
        folders.push(ListedFolder {
            raw_name,
            delimiter,
            attributes,
        });
    }
    folders
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, delim: &str, attrs: &[&str]) -> ListedFolder {
        ListedFolder {
            raw_name: name.to_string(),
            delimiter: if delim.is_empty() {
                None
            } else {
                Some(delim.to_string())
            },
            attributes: attrs.iter().map(|a| a.to_string()).collect(),
        }
    }

    // --- role detection ladder ---

    #[test]
    fn special_use_attrs_win_over_name_candidates() {
        // Server marks a non-obvious name with \Sent while another folder is
        // literally called "Sent" — the attribute is authoritative.
        let entries = vec![
            entry("Objetos enviados X", ".", &["\\HasNoChildren", "\\Sent"]),
            entry("Sent", ".", &["\\HasNoChildren"]),
        ];
        assert_eq!(
            resolve_role_folder(WellKnownFolder::Sent, &entries),
            Some("Objetos enviados X".to_string())
        );
    }

    #[test]
    fn german_names_detected_without_special_use() {
        // IONOS-shaped account, no SPECIAL-USE attributes at all.
        let entries = vec![
            entry("INBOX", ".", &["\\HasChildren"]),
            entry("Gesendete Objekte", ".", &["\\HasNoChildren"]),
            entry("Papierkorb", ".", &["\\HasNoChildren"]),
            entry("Spamverdacht", ".", &["\\HasNoChildren"]),
            entry("Entw&APw-rfe", ".", &["\\HasNoChildren"]),
        ];
        assert_eq!(
            resolve_role_folder(WellKnownFolder::Sent, &entries),
            Some("Gesendete Objekte".to_string())
        );
        assert_eq!(
            resolve_role_folder(WellKnownFolder::Trash, &entries),
            Some("Papierkorb".to_string())
        );
        assert_eq!(
            resolve_role_folder(WellKnownFolder::Spam, &entries),
            Some("Spamverdacht".to_string())
        );
    }

    #[test]
    fn spanish_and_french_names_detected() {
        let es = vec![
            entry("Elementos enviados", "/", &[]),
            entry("Papelera", "/", &[]),
            entry("Correo no deseado", "/", &[]),
        ];
        assert_eq!(
            resolve_role_folder(WellKnownFolder::Sent, &es),
            Some("Elementos enviados".to_string())
        );
        assert_eq!(
            resolve_role_folder(WellKnownFolder::Trash, &es),
            Some("Papelera".to_string())
        );
        assert_eq!(
            resolve_role_folder(WellKnownFolder::Spam, &es),
            Some("Correo no deseado".to_string())
        );

        // French names arrive UTF-7 encoded on the wire ("Envoyés" =
        // "Envoy&AOk-s"); matching happens on the decoded form.
        let fr = vec![
            entry("Envoy&AOk-s", "/", &[]),
            entry("Corbeille", "/", &[]),
            entry("Courrier ind&AOk-sirable", "/", &[]),
        ];
        assert_eq!(
            resolve_role_folder(WellKnownFolder::Sent, &fr),
            Some("Envoy&AOk-s".to_string())
        );
        assert_eq!(
            resolve_role_folder(WellKnownFolder::Trash, &fr),
            Some("Corbeille".to_string())
        );
        assert_eq!(
            resolve_role_folder(WellKnownFolder::Spam, &fr),
            Some("Courrier ind&AOk-sirable".to_string())
        );
    }

    #[test]
    fn english_candidates_keep_priority_over_localized() {
        // Backward compatibility: a server exposing both "Sent" and
        // "Gesendet" resolves to "Sent" (the historic behavior).
        let entries = vec![entry("Gesendet", ".", &[]), entry("Sent", ".", &[])];
        assert_eq!(
            resolve_role_folder(WellKnownFolder::Sent, &entries),
            Some("Sent".to_string())
        );
    }

    #[test]
    fn last_path_segment_matches_inbox_dot_gesendet() {
        let entries = vec![entry("INBOX.Gesendet", ".", &["\\HasNoChildren"])];
        assert_eq!(
            resolve_role_folder(WellKnownFolder::Sent, &entries),
            Some("INBOX.Gesendet".to_string())
        );
    }

    #[test]
    fn case_insensitive_match_handles_non_ascii() {
        // "GELÖSCHT" lowercases to "gelöscht" only under full Unicode
        // case-folding — eq_ignore_ascii_case would miss it.
        let entries = vec![entry("GEL&ANY-SCHT", ".", &[])];
        // GELÖSCHT in UTF-7: Ö = U+00D6 -> &ANY-
        assert_eq!(
            resolve_role_folder(WellKnownFolder::Trash, &entries),
            Some("GEL&ANY-SCHT".to_string())
        );
    }

    #[test]
    fn noselect_folders_never_win_roles() {
        let entries = vec![
            entry("Sent", ".", &["\\Noselect", "\\HasChildren"]),
            entry("Sent.2024", ".", &[]),
        ];
        // The container "Sent" is unselectable; nothing else matches a Sent
        // candidate by full name or last segment except the container.
        assert_eq!(resolve_role_folder(WellKnownFolder::Sent, &entries), None);
    }

    #[test]
    fn missing_role_resolves_to_none() {
        let entries = vec![entry("INBOX", ".", &[])];
        assert_eq!(resolve_role_folder(WellKnownFolder::Spam, &entries), None);
    }

    // --- custom folder classification ---

    #[test]
    fn plan_classifies_custom_and_skips_inbox_roles_drafts_virtual() {
        let entries = vec![
            entry("INBOX", ".", &["\\HasChildren"]),
            entry("Gesendete Objekte", ".", &[]),
            entry("Papierkorb", ".", &[]),
            entry("Spamverdacht", ".", &[]),
            entry("Entw&APw-rfe", ".", &[]), // Entwürfe = Drafts, skipped
            entry("Alle", ".", &["\\All"]),  // virtual view, skipped
            entry("Markiert", ".", &["\\Flagged"]),
            entry("Container", ".", &["\\Noselect", "\\HasChildren"]),
            entry("Patienten", ".", &[]),
            entry("Zulieferer", ".", &[]),
        ];
        let plan = plan_folders(&entries);
        assert_eq!(plan.roles.len(), 3);
        let names: Vec<&str> = plan.custom.iter().map(|f| f.raw_name.as_str()).collect();
        assert_eq!(names, vec!["Patienten", "Zulieferer"]);
    }

    #[test]
    fn archive_becomes_custom() {
        let entries = vec![entry("INBOX", ".", &[]), entry("Archiv", ".", &["\\Archive"])];
        let plan = plan_folders(&entries);
        let names: Vec<&str> = plan.custom.iter().map(|f| f.raw_name.as_str()).collect();
        assert_eq!(names, vec!["Archiv"]);
    }

    #[test]
    fn drafts_attr_skipped_even_with_unrecognized_name() {
        let entries = vec![entry("Kladde", ".", &["\\Drafts"])];
        let plan = plan_folders(&entries);
        assert!(plan.custom.is_empty());
    }

    #[test]
    fn custom_display_name_is_utf7_decoded() {
        let entries = vec![entry("INBOX.Vertr&AOQ-ge", ".", &[])];
        let plan = plan_folders(&entries);
        assert_eq!(plan.custom.len(), 1);
        assert_eq!(plan.custom[0].raw_name, "INBOX.Vertr&AOQ-ge");
        assert_eq!(plan.custom[0].display_name, "INBOX.Verträge");
    }

    // --- UTF-7 decoding ---

    #[test]
    fn utf7_decodes_umlauts_and_literal_ampersand() {
        assert_eq!(decode_imap_utf7("Entw&APw-rfe"), "Entwürfe");
        assert_eq!(decode_imap_utf7("Envoy&AOk-s"), "Envoyés");
        assert_eq!(decode_imap_utf7("A&-B"), "A&B");
        assert_eq!(decode_imap_utf7("Plain"), "Plain");
    }

    #[test]
    fn utf7_malformed_input_returned_unchanged() {
        assert_eq!(decode_imap_utf7("Broken&AP"), "Broken&AP");
        assert_eq!(decode_imap_utf7("Bad&!!!-x"), "Bad&!!!-x");
    }

    // --- UTF-7 encoding (folder create/rename) ---

    #[test]
    fn utf7_encodes_umlauts_ampersand_and_plain_ascii() {
        assert_eq!(encode_imap_utf7("Entwürfe"), "Entw&APw-rfe");
        assert_eq!(encode_imap_utf7("Envoyés"), "Envoy&AOk-s");
        assert_eq!(encode_imap_utf7("A&B"), "A&-B");
        assert_eq!(encode_imap_utf7("Plain"), "Plain");
        assert_eq!(encode_imap_utf7("INBOX.Patienten"), "INBOX.Patienten");
    }

    #[test]
    fn utf7_encode_decode_roundtrip() {
        for name in ["Verträge", "Ärzte & Praxen", "日本語", "Corbeille", "a☂b"] {
            assert_eq!(
                decode_imap_utf7(&encode_imap_utf7(name)),
                name,
                "roundtrip failed for {name}"
            );
        }
    }

    // --- folder name validation ---

    #[test]
    fn validate_accepts_reasonable_names() {
        for name in ["Patienten", "Verträge 2026", "Q3 - Rechnungen"] {
            assert!(validate_folder_name(name, Some(".")).is_ok(), "rejected {name}");
        }
    }

    #[test]
    fn validate_rejects_empty_and_whitespace_only() {
        assert!(validate_folder_name("", Some(".")).is_err());
        assert!(validate_folder_name("   ", Some(".")).is_err());
    }

    #[test]
    fn validate_rejects_overlong_names() {
        let long = "x".repeat(101);
        assert!(validate_folder_name(&long, Some(".")).is_err());
        assert!(validate_folder_name(&"x".repeat(100), Some(".")).is_ok());
    }

    #[test]
    fn validate_rejects_delimiter_wildcards_quotes_and_control_chars() {
        // Hierarchy delimiter would silently create a nested folder.
        assert!(validate_folder_name("a.b", Some(".")).is_err());
        assert!(validate_folder_name("a/b", Some("/")).is_err());
        // A '.' is fine when the account's delimiter is '/'.
        assert!(validate_folder_name("a.b", Some("/")).is_ok());
        // IMAP wildcards / quoting characters.
        for name in ["a*b", "a%b", "a\"b", "a\\b"] {
            assert!(validate_folder_name(name, Some(".")).is_err(), "accepted {name}");
        }
        assert!(validate_folder_name("a\u{0007}b", Some(".")).is_err());
    }

    #[test]
    fn validate_rejects_reserved_inbox_name() {
        assert!(validate_folder_name("INBOX", Some(".")).is_err());
        assert!(validate_folder_name("inbox", Some(".")).is_err());
        // ...but INBOX-prefixed display names are fine (nesting is handled by
        // path composition, not the display name).
        assert!(validate_folder_name("Inbox-Archiv", Some(".")).is_ok());
    }

    // --- path composition for create ---

    #[test]
    fn compose_top_level_when_no_existing_folders() {
        assert_eq!(compose_folder_path("Patienten", Some("."), &[]), "Patienten");
    }

    #[test]
    fn compose_nests_under_inbox_when_all_existing_do() {
        // Dovecot/Courier layout: everything lives under INBOX.
        let existing = vec!["INBOX.Patienten".to_string(), "INBOX.Zulieferer".to_string()];
        assert_eq!(compose_folder_path("Neu", Some("."), &existing), "INBOX.Neu");
    }

    #[test]
    fn compose_stays_top_level_when_existing_are_top_level_or_mixed() {
        // IONOS layout: folders are siblings of INBOX.
        let flat = vec!["Patienten".to_string(), "Zulieferer".to_string()];
        assert_eq!(compose_folder_path("Neu", Some("."), &flat), "Neu");

        let mixed = vec!["INBOX.Patienten".to_string(), "Zulieferer".to_string()];
        assert_eq!(compose_folder_path("Neu", Some("."), &mixed), "Neu");
    }

    #[test]
    fn compose_without_delimiter_is_top_level() {
        let existing = vec!["INBOX.Patienten".to_string()];
        assert_eq!(compose_folder_path("Neu", None, &existing), "Neu");
    }

    #[test]
    fn compose_encodes_non_ascii_names() {
        assert_eq!(
            compose_folder_path("Verträge", Some("."), &["INBOX.Alt".to_string()]),
            "INBOX.Vertr&AOQ-ge"
        );
    }

    // --- sibling path for rename ---

    #[test]
    fn rename_sibling_replaces_last_segment() {
        assert_eq!(rename_sibling_path("INBOX.Alt", Some("."), "Neu"), "INBOX.Neu");
        assert_eq!(rename_sibling_path("A.B.C", Some("."), "Neu"), "A.B.Neu");
        assert_eq!(rename_sibling_path("Alt", Some("."), "Neu"), "Neu");
        assert_eq!(rename_sibling_path("Alt", None, "Neu"), "Neu");
    }

    #[test]
    fn rename_sibling_encodes_non_ascii_new_name() {
        assert_eq!(
            rename_sibling_path("INBOX.Alt", Some("."), "Verträge"),
            "INBOX.Vertr&AOQ-ge"
        );
    }

    // --- LIST response parsing (SPECIAL-USE fallback path) ---

    #[test]
    fn parse_list_response_ionos_shaped_fixture() {
        let raw = concat!(
            "* LIST (\\HasNoChildren) \".\" \"INBOX\"\r\n",
            "* LIST (\\HasNoChildren \\Sent) \".\" \"Gesendete Objekte\"\r\n",
            "* LIST (\\HasNoChildren \\Trash) \".\" Papierkorb\r\n",
            "* LIST (\\Noselect \\HasChildren) NIL Container\r\n",
            "A1 OK LIST completed\r\n",
        );
        let folders = parse_list_response(raw);
        assert_eq!(folders.len(), 4);
        assert_eq!(folders[0].raw_name, "INBOX");
        assert_eq!(folders[1].raw_name, "Gesendete Objekte");
        assert_eq!(folders[1].attributes, vec!["\\HasNoChildren", "\\Sent"]);
        assert_eq!(folders[1].delimiter.as_deref(), Some("."));
        assert_eq!(folders[2].raw_name, "Papierkorb");
        assert_eq!(folders[3].raw_name, "Container");
        assert_eq!(folders[3].delimiter, None);
    }
}
