//! User-editable prompt templates.
//!
//! Built-in prompts (chat / classification / memory) live as Handlebars-style
//! templates in `defaults.rs` and are described by the `registry::PROMPTS`
//! table. At runtime, callers ask this module for the current template via
//! [`get_template`] — which transparently falls back to the registry default
//! when the user has not customised it — then call [`render`] with the
//! per-call variable map.
//!
//! Overrides are persisted in the existing `user_preferences` table under the
//! key `prompt.<id>`. "Reset to default" simply deletes that row.

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

use regex::Regex;
use serde::Serialize;

use crate::db::Database;
use crate::models::error::{AppError, Result};

pub mod defaults;
pub mod registry;

pub use registry::{PromptCategory, PromptDef};

// ── Persistence helpers ─────────────────────────────────────────────────────

fn pref_key(id: &str) -> String {
    format!("prompt.{id}")
}

// ── Process-level (run-scoped) overrides ─────────────────────────────────────
//
// In-memory overrides that take precedence over BOTH the persisted
// `user_preferences` override and the registry default — without touching the
// DB. The desktop app never installs any, so `get_template` behaves exactly as
// before there. `emailops-cli` populates this from `--prompt <id>=<file>` /
// `--system-prompt <file>` so a developer can A/B a prompt for a single run
// without editing code or mutating their real database.

fn overrides() -> &'static RwLock<HashMap<String, String>> {
    static O: OnceLock<RwLock<HashMap<String, String>>> = OnceLock::new();
    O.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Install run-scoped prompt overrides (id → template text), replacing any
/// previously installed set. Ids are NOT validated here — the caller (CLI)
/// validates against the registry before calling so a typo errors loudly.
pub fn install_overrides(map: HashMap<String, String>) {
    if let Ok(mut guard) = overrides().write() {
        *guard = map;
    }
}

/// Drop all run-scoped overrides. Used by tests; harmless in production.
pub fn clear_overrides() {
    if let Ok(mut guard) = overrides().write() {
        guard.clear();
    }
}

/// Ids currently overridden in-memory (for surfacing in logs / traces).
pub fn overridden_ids() -> Vec<String> {
    overrides()
        .read()
        .map(|g| {
            let mut ids: Vec<String> = g.keys().cloned().collect();
            ids.sort();
            ids
        })
        .unwrap_or_default()
}

/// Return the template the user wants for `id`: a run-scoped in-memory override
/// if installed, else their persisted override, else the built-in default.
/// Errors only on an unknown id or database failure.
pub fn get_template(db: &Database, id: &str) -> Result<String> {
    let def = registry::lookup(id).ok_or_else(|| AppError::NotFound(format!("unknown prompt id: {id}")))?;
    if let Ok(guard) = overrides().read() {
        if let Some(override_text) = guard.get(id) {
            return Ok(override_text.clone());
        }
    }
    if let Some(override_text) = db.get_preference(&pref_key(id))? {
        return Ok(override_text);
    }
    Ok(def.default_template.to_string())
}

/// Persist a user override for `id`. Empty templates are rejected so the user
/// can never accidentally blank a prompt out — they should hit "Reset" instead.
pub fn set_template(db: &Database, id: &str, template: &str) -> Result<()> {
    if registry::lookup(id).is_none() {
        return Err(AppError::NotFound(format!("unknown prompt id: {id}")));
    }
    if template.trim().is_empty() {
        return Err(AppError::InvalidInput(
            "prompt template cannot be empty (use reset to restore the default)".into(),
        ));
    }
    db.set_preference(&pref_key(id), template)
}

/// Drop the user override for `id` so subsequent reads return the registry
/// default. Idempotent — clearing an already-default prompt is a no-op.
pub fn reset_template(db: &Database, id: &str) -> Result<()> {
    if registry::lookup(id).is_none() {
        return Err(AppError::NotFound(format!("unknown prompt id: {id}")));
    }
    db.delete_preference(&pref_key(id))
}

// ── Listing API for the settings panel ──────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VariableInfo {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptInfo {
    pub id: String,
    pub label: String,
    pub description: String,
    pub category: PromptCategory,
    pub advanced: bool,
    pub default_template: String,
    pub current_template: String,
    pub is_overridden: bool,
    pub variables: Vec<VariableInfo>,
}

/// Build the rich PromptInfo list for the Settings panel.
pub fn list_prompts(db: &Database) -> Result<Vec<PromptInfo>> {
    let mut out = Vec::with_capacity(registry::PROMPTS.len());
    for def in registry::PROMPTS {
        let override_text = db.get_preference(&pref_key(def.id))?;
        let is_overridden = override_text.is_some();
        let current_template = override_text.unwrap_or_else(|| def.default_template.to_string());
        out.push(PromptInfo {
            id: def.id.to_string(),
            label: def.label.to_string(),
            description: def.description.to_string(),
            category: def.category,
            advanced: def.advanced,
            default_template: def.default_template.to_string(),
            current_template,
            is_overridden,
            variables: def
                .variables
                .iter()
                .map(|v| VariableInfo {
                    name: v.name.to_string(),
                    description: v.description.to_string(),
                })
                .collect(),
        });
    }
    Ok(out)
}

// ── Template rendering ──────────────────────────────────────────────────────

fn placeholder_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // Hard-coded regex literal — infallible.
        #[allow(clippy::unwrap_used)]
        let r = Regex::new(r"\{\{\s*([a-zA-Z_][a-zA-Z0-9_]*)\s*\}\}").unwrap();
        r
    })
}

/// Substitute `{{variable}}` placeholders in `template` using `vars`.
///
/// Unknown placeholders are left in place verbatim and a warning is logged so
/// a user-edited template with a typo is debuggable rather than silently
/// producing garbage. Whitespace inside the braces is allowed.
pub fn render(template: &str, vars: &HashMap<&str, String>) -> String {
    placeholder_re()
        .replace_all(template, |caps: &regex::Captures| match vars.get(&caps[1]) {
            Some(v) => v.clone(),
            None => {
                // Surface to the output panel so a typo in a user-edited
                // template is debuggable. Not a hard error — the placeholder is
                // left intact so the rendered prompt makes the problem obvious
                // to the user too.
                crate::services::logger::log(
                    "debug",
                    "prompts",
                    format!("template references unknown variable: {{{{ {} }}}}", &caps[1]),
                );
                caps[0].to_string()
            }
        })
        .into_owned()
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_substitutes_simple_vars() {
        let mut v = HashMap::new();
        v.insert("name", "Acme".to_string());
        assert_eq!(render("hello {{name}}", &v), "hello Acme");
    }

    #[test]
    fn render_handles_whitespace_and_repeats() {
        let mut v = HashMap::new();
        v.insert("x", "42".to_string());
        assert_eq!(render("{{ x }} and {{x}} again", &v), "42 and 42 again");
    }

    #[test]
    fn render_leaves_unknown_vars_in_place() {
        let v: HashMap<&str, String> = HashMap::new();
        assert_eq!(render("ping {{missing}} pong", &v), "ping {{missing}} pong");
    }

    #[test]
    fn render_does_not_treat_single_braces_as_placeholders() {
        let v: HashMap<&str, String> = HashMap::new();
        // JSON examples in prompt templates rely on this.
        assert_eq!(render(r#"{"a": 1}"#, &v), r#"{"a": 1}"#);
    }

    #[test]
    fn registry_ids_are_unique() {
        let mut ids: Vec<&str> = registry::PROMPTS.iter().map(|p| p.id).collect();
        let before = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), before, "duplicate prompt id in registry");
    }

    /// Snapshot of the classification prompt as it was assembled before the
    /// registry refactor. Guards against accidental drift in
    /// `defaults::CLASSIFY_EMAIL` — if you change the default template
    /// intentionally, update this expected output to match.
    ///
    /// Note: the From/Subject/Preview block is appended programmatically in
    /// `services::classification::classify_email` and is NOT part of the
    /// user-editable template — see that function for the final assembly.
    #[test]
    fn classify_email_default_renders_to_expected_snapshot() {
        let mut vars = HashMap::new();
        vars.insert("today", "2026-04-27".to_string());
        vars.insert("language_clause", "Respond in Spanish.\n".to_string());
        vars.insert("intents", "request, question".to_string());
        vars.insert("topics", "billing, support".to_string());

        let expected = "You are an email classifier for a freelancer / small business owner.\n\
Classify the following email into structured categories.\n\
Today's date is 2026-04-27.\n\
Respond in Spanish.\n\n\
Intent (pick exactly ONE): request, question\n\
Topic (pick exactly ONE): billing, support\n\
Urgency (pick exactly ONE): urgent, normal, low\n\n\
Respond with ONLY a JSON object, no markdown, no explanation:\n\
{\"intent\": \"...\", \"topic\": \"...\", \"urgency\": \"...\", \"confidence\": 0.0-1.0}";

        let rendered = render(defaults::CLASSIFY_EMAIL, &vars);
        assert_eq!(rendered, expected);
    }

    /// Pins the EMAIL LINKS clause + the example that shows the LLM how to
    /// emit `[label](email://EMAIL_ID)` chips. The frontend's
    /// `MarkdownContent` handler depends on the LLM actually emitting this
    /// scheme, so a silent prompt edit that removes the contract would
    /// regress the open-the-email chip feature without any other signal.
    #[test]
    fn chat_system_default_describes_email_link_contract() {
        let tpl = defaults::CHAT_SYSTEM;
        assert!(
            tpl.contains("EMAIL LINKS"),
            "CHAT_SYSTEM lost the EMAIL LINKS section: {tpl}"
        );
        // The contract must spell out both the scheme and the validation
        // semantics so a small model has both the format and a refusal
        // example. Loose substring checks tolerate prose changes around them.
        assert!(
            tpl.contains("email://EMAIL_ID"),
            "CHAT_SYSTEM lost the email://EMAIL_ID scheme"
        );
        assert!(
            tpl.contains("allowlist") || tpl.contains("validates"),
            "CHAT_SYSTEM lost the validation/allowlist disclaimer"
        );
        // An example with `[label](email://...)` is what most local 4B
        // models latch onto — keep it inline.
        assert!(
            tpl.contains("(email://eml-a)"),
            "CHAT_SYSTEM lost the inline [label](email://...) example"
        );
        // The table example was added after a real Qwen 3.5 4B failure case
        // where the model produced a Markdown summary table without any
        // `email://` links — interpreting "table cells" as not being
        // "natural-language mentions". The example + the "inside the cell"
        // wording are what make the rule stick on small models.
        assert!(
            tpl.contains("| Remitente | Asunto"),
            "CHAT_SYSTEM lost the table-format example showing email:// inside a cell"
        );
        assert!(
            tpl.contains("MARKDOWN TABLES") || tpl.contains("tables and lists"),
            "CHAT_SYSTEM lost the 'tables count too' explicit wording"
        );
        // Same shape as EMAIL LINKS, but for the `draft://DRAFT_ID`
        // re-open-the-draft chip. Without this contract a saved draft is
        // invisible after the chat tells the user "draft saved" — they
        // have to navigate manually.
        assert!(
            tpl.contains("DRAFT LINKS"),
            "CHAT_SYSTEM lost the DRAFT LINKS section: {tpl}"
        );
        assert!(
            tpl.contains("draft://DRAFT_ID"),
            "CHAT_SYSTEM lost the draft://DRAFT_ID scheme"
        );
        assert!(
            tpl.contains("(draft://d-1)"),
            "CHAT_SYSTEM lost the inline [label](draft://...) example"
        );
    }

    #[test]
    fn every_default_template_is_non_empty_and_uses_only_declared_vars() {
        let re = placeholder_re();
        for def in registry::PROMPTS {
            assert!(!def.default_template.trim().is_empty(), "{} default empty", def.id);
            let declared: Vec<&str> = def.variables.iter().map(|v| v.name).collect();
            for caps in re.captures_iter(def.default_template) {
                let name = caps.get(1).unwrap().as_str();
                assert!(
                    declared.contains(&name),
                    "prompt {} references undeclared variable {{{{ {} }}}}",
                    def.id,
                    name
                );
            }
        }
    }
}
