//! Language / locale resolution for AI prompts and UI.
//!
//! The app supports four languages: English, Spanish, French, German. This
//! module is the single source of truth for:
//!
//! * Parsing language preferences from the DB (legacy free-text names like
//!   `"Spanish"` *and* BCP-47 codes like `"es"`, `"es-MX"`).
//! * The fallback chain used by every AI service to decide what language to
//!   emit output in: `ai_output_language_v2` → legacy `ai_output_language` →
//!   `ui_language` → English (default).
//!
//! Before this module landed, six call sites duplicated this resolution and
//! defaulted to Spanish. The new behaviour is: when nothing is configured, AI
//! output follows the UI language; when UI language is also unset, English.
//! See `CHANGELOG.md` for the user-facing impact of the default flip.

use serde::{Deserialize, Serialize};

use crate::db::Database;
use crate::models::error::Result;

/// Supported display & AI-output languages. Keep variants ordered as
/// `En, Es, Fr, De` so iteration order matches the language selector in the
/// UI (which lists English first).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    #[default]
    En,
    Es,
    Fr,
    De,
}

impl Language {
    /// All supported variants in display order.
    pub const ALL: [Language; 4] = [Language::En, Language::Es, Language::Fr, Language::De];

    /// Two-letter language code (`"en"`, `"es"`, `"fr"`, `"de"`). Matches both
    /// the BCP-47 primary subtag and what gets persisted into the
    /// `ui_language` / `ai_output_language_v2` preferences.
    pub fn as_code(self) -> &'static str {
        match self {
            Language::En => "en",
            Language::Es => "es",
            Language::Fr => "fr",
            Language::De => "de",
        }
    }

    /// English name of the language (`"English"`, `"Spanish"`, `"French"`,
    /// `"German"`). Used as the value interpolated into AI prompts —
    /// `format!("Reply in {}.", lang.english_name())` — because LLMs respond
    /// most reliably to English instructions naming the target language.
    pub fn english_name(self) -> &'static str {
        match self {
            Language::En => "English",
            Language::Es => "Spanish",
            Language::Fr => "French",
            Language::De => "German",
        }
    }

    /// Native name of the language (`"English"`, `"Español"`, `"Français"`,
    /// `"Deutsch"`). Used in the language selector so a user accidentally
    /// stuck on a wrong language can still recognise their own.
    pub fn native_name(self) -> &'static str {
        match self {
            Language::En => "English",
            Language::Es => "Español",
            Language::Fr => "Français",
            Language::De => "Deutsch",
        }
    }

    /// Parse a value from a preference / OS locale string into a supported
    /// variant. Accepts:
    ///
    /// * Two-letter codes: `"en"`, `"es"`, `"fr"`, `"de"` (case-insensitive).
    /// * BCP-47 tags: `"en-US"`, `"es-MX"`, `"fr-FR"`, `"de-AT"` — only the
    ///   primary subtag is examined.
    /// * Legacy free-text English names: `"English"`, `"Spanish"`, `"French"`,
    ///   `"German"` (case-insensitive). These predate the typed enum and may
    ///   still be present in users' DBs.
    /// * Common native names: `"español"`, `"français"`, `"deutsch"`
    ///   (accent- and case-insensitive). Cheap to support and avoids surprises
    ///   if a future code path ever stores native names.
    ///
    /// Returns `None` for empty input, unsupported languages (Portuguese,
    /// Italian, Catalan…), or junk. Callers should treat `None` as "fall back
    /// to the next preference in the chain", never panic.
    pub fn from_pref(s: &str) -> Option<Self> {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return None;
        }
        // Take the primary subtag of a BCP-47 tag (`"en-US"` → `"en"`).
        let primary = trimmed.split(['-', '_']).next().unwrap_or(trimmed);
        // Normalise: lowercase + strip a handful of common accents so
        // `"Français"` and `"francais"` both parse.
        let normalised: String = primary
            .chars()
            .flat_map(|c| c.to_lowercase())
            .map(strip_accent)
            .collect();
        match normalised.as_str() {
            "en" | "english" | "ingles" => Some(Language::En),
            "es" | "spanish" | "espanol" | "castellano" => Some(Language::Es),
            "fr" | "french" | "francais" => Some(Language::Fr),
            "de" | "german" | "deutsch" | "aleman" => Some(Language::De),
            _ => None,
        }
    }
}

/// Strip the small set of Latin accents we expect in language-name inputs.
/// Not a full Unicode normalisation — we only care about the few names this
/// module accepts. Keeps the parser dependency-free.
fn strip_accent(c: char) -> char {
    match c {
        'á' | 'à' | 'ä' | 'â' | 'ã' | 'å' => 'a',
        'é' | 'è' | 'ë' | 'ê' => 'e',
        'í' | 'ì' | 'ï' | 'î' => 'i',
        'ó' | 'ò' | 'ö' | 'ô' | 'õ' => 'o',
        'ú' | 'ù' | 'ü' | 'û' => 'u',
        'ñ' => 'n',
        'ç' => 'c',
        other => other,
    }
}

/// Preference key for the typed AI output language (`"en" | "es" | "fr" |
/// "de"`). Absent / empty means "follow the UI language".
pub const PREF_AI_OUTPUT_LANGUAGE_V2: &str = "ai_output_language_v2";

/// Legacy preference key for the AI output language. Stored as a free-text
/// English name (`"Spanish"`, `"English"`, …) by older builds. Read as a
/// fallback for users who upgraded across the i18n change.
pub const PREF_AI_OUTPUT_LANGUAGE_LEGACY: &str = "ai_output_language";

/// Preference key for the UI display language. `"en" | "es" | "fr" | "de"`.
/// Absent means "auto-detect from OS, default to English".
pub const PREF_UI_LANGUAGE: &str = "ui_language";

/// Resolve the language the AI should emit output in, walking the preference
/// fallback chain:
///
/// 1. `ai_output_language_v2` (typed enum, user explicitly chose a language).
/// 2. `ai_output_language` (legacy free-text, parsed loosely).
/// 3. `ui_language` (so AI follows the UI by default).
/// 4. [`Language::default()`] (English).
///
/// Returns `Result` so genuine DB failures propagate; absent / unparseable
/// values silently fall through to the next link in the chain.
pub fn resolve_ai_language(db: &Database) -> Result<Language> {
    if let Some(v) = db.get_preference(PREF_AI_OUTPUT_LANGUAGE_V2)? {
        if let Some(lang) = Language::from_pref(&v) {
            return Ok(lang);
        }
    }
    if let Some(v) = db.get_preference(PREF_AI_OUTPUT_LANGUAGE_LEGACY)? {
        if let Some(lang) = Language::from_pref(&v) {
            return Ok(lang);
        }
    }
    if let Some(v) = db.get_preference(PREF_UI_LANGUAGE)? {
        if let Some(lang) = Language::from_pref(&v) {
            return Ok(lang);
        }
    }
    Ok(Language::default())
}

/// One-time forward migration of the legacy free-text `ai_output_language`
/// preference into the typed `ai_output_language_v2` code, so the legacy read
/// path in [`resolve_ai_language`] can eventually be retired.
///
/// Idempotent — safe to run on every startup:
///
/// * If `ai_output_language_v2` already exists (including the empty
///   "Same as UI" sentinel), the user has used the new selector. Their choice
///   wins; the now-redundant legacy key is removed.
/// * Else if a parseable legacy value is present, it is written to v2 (as a
///   two-letter code) and the legacy key is removed.
/// * Else if the legacy value is present but unparseable (e.g. `"Portuguese"`,
///   a language the new UI doesn't offer), it is removed so it stops shadowing
///   the `ui_language` fallback.
/// * Otherwise (no legacy key) this is a no-op.
///
/// Returns `true` only when a legacy value was promoted into v2, so the caller
/// can emit a one-line startup log.
pub fn migrate_legacy_ai_output_language(db: &Database) -> Result<bool> {
    if db.get_preference(PREF_AI_OUTPUT_LANGUAGE_V2)?.is_some() {
        // v2 exists → the user has interacted with the new selector. Drop the
        // stale legacy key without touching their explicit choice.
        db.delete_preference(PREF_AI_OUTPUT_LANGUAGE_LEGACY)?;
        return Ok(false);
    }

    let Some(legacy) = db.get_preference(PREF_AI_OUTPUT_LANGUAGE_LEGACY)? else {
        return Ok(false);
    };

    match Language::from_pref(&legacy) {
        Some(lang) => {
            db.set_preference(PREF_AI_OUTPUT_LANGUAGE_V2, lang.as_code())?;
            db.delete_preference(PREF_AI_OUTPUT_LANGUAGE_LEGACY)?;
            Ok(true)
        }
        None => {
            db.delete_preference(PREF_AI_OUTPUT_LANGUAGE_LEGACY)?;
            Ok(false)
        }
    }
}

/// Resolve the UI language. Same chain as [`resolve_ai_language`] minus the
/// AI-specific keys: only [`PREF_UI_LANGUAGE`] is consulted, then the
/// default. The caller is responsible for layering OS-locale detection on
/// top — that lives at the Tauri-command boundary.
pub fn resolve_ui_language(db: &Database) -> Result<Option<Language>> {
    if let Some(v) = db.get_preference(PREF_UI_LANGUAGE)? {
        if let Some(lang) = Language::from_pref(&v) {
            return Ok(Some(lang));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Language::from_pref ─────────────────────────────────────────────

    #[test]
    fn from_pref_parses_two_letter_codes() {
        assert_eq!(Language::from_pref("en"), Some(Language::En));
        assert_eq!(Language::from_pref("es"), Some(Language::Es));
        assert_eq!(Language::from_pref("fr"), Some(Language::Fr));
        assert_eq!(Language::from_pref("de"), Some(Language::De));
    }

    #[test]
    fn from_pref_parses_bcp47_tags() {
        assert_eq!(Language::from_pref("en-US"), Some(Language::En));
        assert_eq!(Language::from_pref("en_GB"), Some(Language::En));
        assert_eq!(Language::from_pref("es-MX"), Some(Language::Es));
        assert_eq!(Language::from_pref("fr-CA"), Some(Language::Fr));
        assert_eq!(Language::from_pref("de-AT"), Some(Language::De));
    }

    #[test]
    fn from_pref_parses_legacy_english_names() {
        assert_eq!(Language::from_pref("English"), Some(Language::En));
        assert_eq!(Language::from_pref("Spanish"), Some(Language::Es));
        assert_eq!(Language::from_pref("French"), Some(Language::Fr));
        assert_eq!(Language::from_pref("German"), Some(Language::De));
        // Case insensitivity.
        assert_eq!(Language::from_pref("SPANISH"), Some(Language::Es));
        assert_eq!(Language::from_pref("spanish"), Some(Language::Es));
    }

    #[test]
    fn from_pref_parses_native_names_with_and_without_accents() {
        assert_eq!(Language::from_pref("Español"), Some(Language::Es));
        assert_eq!(Language::from_pref("Espanol"), Some(Language::Es));
        assert_eq!(Language::from_pref("Français"), Some(Language::Fr));
        assert_eq!(Language::from_pref("francais"), Some(Language::Fr));
        assert_eq!(Language::from_pref("Deutsch"), Some(Language::De));
    }

    #[test]
    fn from_pref_returns_none_for_empty_or_unknown() {
        assert_eq!(Language::from_pref(""), None);
        assert_eq!(Language::from_pref("   "), None);
        // Unsupported languages that the legacy AI dropdown used to expose.
        assert_eq!(Language::from_pref("Portuguese"), None);
        assert_eq!(Language::from_pref("Italian"), None);
        assert_eq!(Language::from_pref("Catalan"), None);
        // Junk.
        assert_eq!(Language::from_pref("klingon"), None);
        assert_eq!(Language::from_pref("xx-YY"), None);
    }

    #[test]
    fn from_pref_trims_whitespace() {
        assert_eq!(Language::from_pref("  en  "), Some(Language::En));
        assert_eq!(Language::from_pref("\tSpanish\n"), Some(Language::Es));
    }

    // ── Display helpers ────────────────────────────────────────────────

    #[test]
    fn english_name_matches_expected() {
        assert_eq!(Language::En.english_name(), "English");
        assert_eq!(Language::Es.english_name(), "Spanish");
        assert_eq!(Language::Fr.english_name(), "French");
        assert_eq!(Language::De.english_name(), "German");
    }

    #[test]
    fn native_name_matches_expected() {
        assert_eq!(Language::En.native_name(), "English");
        assert_eq!(Language::Es.native_name(), "Español");
        assert_eq!(Language::Fr.native_name(), "Français");
        assert_eq!(Language::De.native_name(), "Deutsch");
    }

    #[test]
    fn as_code_matches_expected() {
        assert_eq!(Language::En.as_code(), "en");
        assert_eq!(Language::Es.as_code(), "es");
        assert_eq!(Language::Fr.as_code(), "fr");
        assert_eq!(Language::De.as_code(), "de");
    }

    #[test]
    fn as_code_round_trips_via_from_pref() {
        for lang in Language::ALL {
            assert_eq!(Language::from_pref(lang.as_code()), Some(lang));
        }
    }

    #[test]
    fn default_is_english() {
        assert_eq!(Language::default(), Language::En);
    }

    // ── resolve_ai_language: fallback chain ─────────────────────────────

    #[test]
    fn resolve_ai_language_defaults_to_english_on_empty_db() {
        let db = Database::new_for_testing().unwrap();
        assert_eq!(resolve_ai_language(&db).unwrap(), Language::En);
    }

    #[test]
    fn resolve_ai_language_prefers_v2_key() {
        let db = Database::new_for_testing().unwrap();
        db.set_preference(PREF_UI_LANGUAGE, "es").unwrap();
        db.set_preference(PREF_AI_OUTPUT_LANGUAGE_LEGACY, "Spanish").unwrap();
        db.set_preference(PREF_AI_OUTPUT_LANGUAGE_V2, "fr").unwrap();
        assert_eq!(resolve_ai_language(&db).unwrap(), Language::Fr);
    }

    #[test]
    fn resolve_ai_language_falls_back_to_legacy_key() {
        let db = Database::new_for_testing().unwrap();
        db.set_preference(PREF_AI_OUTPUT_LANGUAGE_LEGACY, "Spanish").unwrap();
        assert_eq!(resolve_ai_language(&db).unwrap(), Language::Es);
    }

    #[test]
    fn resolve_ai_language_falls_back_to_ui_language() {
        let db = Database::new_for_testing().unwrap();
        db.set_preference(PREF_UI_LANGUAGE, "de").unwrap();
        assert_eq!(resolve_ai_language(&db).unwrap(), Language::De);
    }

    #[test]
    fn resolve_ai_language_skips_unparseable_values_in_chain() {
        // v2 set to junk, legacy parseable → use legacy.
        let db = Database::new_for_testing().unwrap();
        db.set_preference(PREF_AI_OUTPUT_LANGUAGE_V2, "klingon").unwrap();
        db.set_preference(PREF_AI_OUTPUT_LANGUAGE_LEGACY, "French").unwrap();
        assert_eq!(resolve_ai_language(&db).unwrap(), Language::Fr);
    }

    #[test]
    fn resolve_ai_language_returns_english_when_all_keys_unparseable() {
        let db = Database::new_for_testing().unwrap();
        db.set_preference(PREF_AI_OUTPUT_LANGUAGE_V2, "").unwrap();
        db.set_preference(PREF_AI_OUTPUT_LANGUAGE_LEGACY, "Portuguese").unwrap();
        db.set_preference(PREF_UI_LANGUAGE, "xx").unwrap();
        assert_eq!(resolve_ai_language(&db).unwrap(), Language::En);
    }

    // ── resolve_ui_language ────────────────────────────────────────────

    #[test]
    fn resolve_ui_language_returns_none_when_unset() {
        let db = Database::new_for_testing().unwrap();
        assert_eq!(resolve_ui_language(&db).unwrap(), None);
    }

    #[test]
    fn resolve_ui_language_returns_pref_when_set() {
        let db = Database::new_for_testing().unwrap();
        db.set_preference(PREF_UI_LANGUAGE, "fr").unwrap();
        assert_eq!(resolve_ui_language(&db).unwrap(), Some(Language::Fr));
    }

    #[test]
    fn resolve_ui_language_ignores_ai_keys() {
        let db = Database::new_for_testing().unwrap();
        db.set_preference(PREF_AI_OUTPUT_LANGUAGE_V2, "es").unwrap();
        assert_eq!(resolve_ui_language(&db).unwrap(), None);
    }

    // ── migrate_legacy_ai_output_language ───────────────────────────────

    #[test]
    fn migrate_promotes_parseable_legacy_value_into_v2() {
        let db = Database::new_for_testing().unwrap();
        db.set_preference(PREF_AI_OUTPUT_LANGUAGE_LEGACY, "Spanish").unwrap();

        assert!(migrate_legacy_ai_output_language(&db).unwrap());

        assert_eq!(
            db.get_preference(PREF_AI_OUTPUT_LANGUAGE_V2).unwrap().as_deref(),
            Some("es")
        );
        assert_eq!(db.get_preference(PREF_AI_OUTPUT_LANGUAGE_LEGACY).unwrap(), None);
    }

    #[test]
    fn migrate_does_not_clobber_existing_v2_choice() {
        let db = Database::new_for_testing().unwrap();
        db.set_preference(PREF_AI_OUTPUT_LANGUAGE_V2, "fr").unwrap();
        db.set_preference(PREF_AI_OUTPUT_LANGUAGE_LEGACY, "Spanish").unwrap();

        assert!(!migrate_legacy_ai_output_language(&db).unwrap());

        // v2 untouched, legacy removed as redundant.
        assert_eq!(
            db.get_preference(PREF_AI_OUTPUT_LANGUAGE_V2).unwrap().as_deref(),
            Some("fr")
        );
        assert_eq!(db.get_preference(PREF_AI_OUTPUT_LANGUAGE_LEGACY).unwrap(), None);
    }

    #[test]
    fn migrate_preserves_empty_same_as_ui_sentinel() {
        // An explicit "Same as UI" choice persists as the empty string. The
        // migration must treat that as "user has chosen" and not overwrite it
        // with a legacy value.
        let db = Database::new_for_testing().unwrap();
        db.set_preference(PREF_AI_OUTPUT_LANGUAGE_V2, "").unwrap();
        db.set_preference(PREF_AI_OUTPUT_LANGUAGE_LEGACY, "German").unwrap();

        assert!(!migrate_legacy_ai_output_language(&db).unwrap());

        assert_eq!(
            db.get_preference(PREF_AI_OUTPUT_LANGUAGE_V2).unwrap().as_deref(),
            Some("")
        );
        assert_eq!(db.get_preference(PREF_AI_OUTPUT_LANGUAGE_LEGACY).unwrap(), None);
    }

    #[test]
    fn migrate_drops_unparseable_legacy_value() {
        let db = Database::new_for_testing().unwrap();
        db.set_preference(PREF_AI_OUTPUT_LANGUAGE_LEGACY, "Portuguese").unwrap();

        assert!(!migrate_legacy_ai_output_language(&db).unwrap());

        assert_eq!(db.get_preference(PREF_AI_OUTPUT_LANGUAGE_V2).unwrap(), None);
        assert_eq!(db.get_preference(PREF_AI_OUTPUT_LANGUAGE_LEGACY).unwrap(), None);
    }

    #[test]
    fn migrate_is_noop_when_no_legacy_key() {
        let db = Database::new_for_testing().unwrap();
        assert!(!migrate_legacy_ai_output_language(&db).unwrap());
        assert_eq!(db.get_preference(PREF_AI_OUTPUT_LANGUAGE_V2).unwrap(), None);
    }

    #[test]
    fn migrate_is_idempotent() {
        let db = Database::new_for_testing().unwrap();
        db.set_preference(PREF_AI_OUTPUT_LANGUAGE_LEGACY, "French").unwrap();

        assert!(migrate_legacy_ai_output_language(&db).unwrap());
        // Second run has nothing left to do.
        assert!(!migrate_legacy_ai_output_language(&db).unwrap());
        assert_eq!(
            db.get_preference(PREF_AI_OUTPUT_LANGUAGE_V2).unwrap().as_deref(),
            Some("fr")
        );
    }
}
