//! Turning one model reply into a filled form.
//!
//! Split per the repo's planner/executor rule:
//!   - **pure** [`parse_fill`] (model text → [`FormFill`]) — exhaustively
//!     unit-tested, no I/O. This is where every small-model quirk is absorbed:
//!     fenced blocks, `<think>` preambles, nested objects, stringified numbers,
//!     invented keys, out-of-set enum values.
//!   - **thin** [`fill_form`] executor — renders the prompt, calls the provider,
//!     parses. Mirrors `chat::planner::plan_search`: it runs as its own focused
//!     completion on the scratch sequence with `cache_prompt = false`, so the
//!     form definition never enters the chat system prompt and never touches
//!     the chat KV prefix.
//!
//! A fill is always *partial-friendly*: a missing required field is reported,
//! never an error. The user reviews and submits the form themselves, so the
//! worst case is a form that opens half-filled — not a failed turn.

use super::registry::{FieldDef, FieldKind, FormDef};
use crate::ai::provider::{AIProvider, CompletionOptions};
use serde_json::{Map, Value};

/// What the model produced for a form, after filtering and coercion.
#[derive(Debug, Clone, PartialEq)]
pub struct FormFill {
    pub form_id: String,
    /// Only keys the form declares, coerced to the declared kind.
    pub values: Map<String, Value>,
    /// Declared-required keys the model did not supply (or supplied
    /// unusably). The chat answer names these so the user knows what to add.
    pub missing_required: Vec<String>,
    /// Keys the model invented, and declared keys whose value could not be
    /// coerced. Surfaced in the trace, never shown to the user.
    pub dropped: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FillError {
    #[error("no JSON object in the model reply")]
    NoJsonObject,
}

/// Split a camelCase form key into the nested path a model might have used
/// instead: `scopeMailboxes` → `("scope", "mailboxes")`.
///
/// The registry flattens nested inputs because small models nest unreliably —
/// but when one *does* nest, the data is right there and throwing it away
/// would be perverse. `None` for a key with no internal capital.
fn nested_fallback(key: &str) -> Option<(String, String)> {
    let idx = key.char_indices().find(|(i, c)| *i > 0 && c.is_uppercase())?.0;
    let (head, tail) = key.split_at(idx);
    let mut tail_chars = tail.chars();
    let first = tail_chars.next()?.to_lowercase().to_string();
    Some((head.to_string(), format!("{first}{}", tail_chars.as_str())))
}

/// The first balanced JSON object in `raw`, ignoring braces inside strings.
///
/// Models wrap the object in prose, ``` fences, or a `<think>` block; some
/// emit a second object afterwards. Taking the first balanced one handles all
/// three without a regex that would choke on nested objects.
fn extract_json_object(raw: &str) -> Option<&str> {
    let bytes = raw.as_bytes();
    let start = raw.find('{')?;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for i in start..bytes.len() {
        let c = bytes[i] as char;
        if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&raw[start..=i]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Truthiness for a checkbox, across the words small models actually emit.
fn coerce_bool(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        Value::Number(n) => n.as_f64().map(|f| f != 0.0),
        Value::String(s) => match s.trim().to_lowercase().as_str() {
            "true" | "yes" | "y" | "1" | "si" | "sí" | "oui" | "ja" => Some(true),
            "false" | "no" | "n" | "0" | "non" | "nein" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn coerce_number(v: &Value) -> Option<Value> {
    match v {
        Value::Number(_) => Some(v.clone()),
        Value::String(s) => serde_json::from_str::<serde_json::Number>(s.trim())
            .ok()
            .map(Value::Number),
        _ => None,
    }
}

fn coerce_text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// A list of strings, from a JSON array, or from the comma/semicolon-separated
/// string a model reaches for when it forgets the array.
fn coerce_string_list(v: &Value) -> Option<Vec<String>> {
    let items: Vec<String> = match v {
        Value::Array(a) => a.iter().filter_map(coerce_text).collect(),
        Value::String(s) => s
            .split([',', ';'])
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .collect(),
        _ => return None,
    };
    (!items.is_empty()).then_some(items)
}

/// Match an enum value case-insensitively and return the *declared* spelling,
/// so `"Inbox"` becomes `"inbox"` and the frontend never sees a variant the
/// backing Rust enum cannot deserialize.
fn canonical_enum(raw: &str, options: &[&str]) -> Option<String> {
    let needle = raw.trim().to_lowercase();
    options
        .iter()
        .find(|o| o.to_lowercase() == needle)
        .map(|o| (*o).to_string())
}

/// Coerce one raw value to one declared field kind. `None` means the value is
/// unusable and the field is dropped.
fn coerce_field(field: &FieldDef, raw: &Value) -> Option<Value> {
    match field.kind {
        FieldKind::Text | FieldKind::LongText => coerce_text(raw).map(Value::String),
        FieldKind::Number => coerce_number(raw),
        FieldKind::Bool => coerce_bool(raw).map(Value::Bool),
        FieldKind::Enum { options } => match raw {
            Value::String(s) => canonical_enum(s, options).map(Value::String),
            _ => None,
        },
        FieldKind::StringList => {
            coerce_string_list(raw).map(|items| Value::Array(items.into_iter().map(Value::String).collect()))
        }
        FieldKind::EnumList { options } => {
            let items: Vec<Value> = coerce_string_list(raw)?
                .iter()
                .filter_map(|s| canonical_enum(s, options))
                .map(Value::String)
                .collect();
            (!items.is_empty()).then_some(Value::Array(items))
        }
        FieldKind::ObjectList { fields } => {
            // A model that produced exactly one item often skips the array.
            let entries: Vec<&Value> = match raw {
                Value::Array(a) => a.iter().collect(),
                Value::Object(_) => vec![raw],
                _ => return None,
            };
            let rows: Vec<Value> = entries
                .into_iter()
                .filter_map(|entry| {
                    let obj = entry.as_object()?;
                    let mut row = Map::new();
                    for sub in fields {
                        if let Some(v) = pick(obj, sub).and_then(|v| coerce_field(sub, &v)) {
                            row.insert(sub.key.to_string(), v);
                        }
                    }
                    // A row that lost every required sub-field is noise, not data.
                    let complete = fields.iter().filter(|f| f.required).all(|f| row.contains_key(f.key));
                    complete.then_some(Value::Object(row))
                })
                .collect();
            (!rows.is_empty()).then_some(Value::Array(rows))
        }
    }
}

/// Read a field's raw value out of the model's object: by its declared key,
/// then by the nested path the model might have used instead.
fn pick(obj: &Map<String, Value>, field: &FieldDef) -> Option<Value> {
    if let Some(v) = obj.get(field.key) {
        if !v.is_null() {
            return Some(v.clone());
        }
    }
    let (outer, inner) = nested_fallback(field.key)?;
    obj.get(&outer)?
        .as_object()?
        .get(&inner)
        .filter(|v| !v.is_null())
        .cloned()
}

/// Model reply → a filled form. Pure.
pub fn parse_fill(raw: &str, form: &FormDef) -> Result<FormFill, FillError> {
    let json = extract_json_object(raw).ok_or(FillError::NoJsonObject)?;
    let parsed: Value = serde_json::from_str(json).map_err(|_| FillError::NoJsonObject)?;
    let obj = parsed.as_object().ok_or(FillError::NoJsonObject)?;

    let mut values = Map::new();
    let mut dropped = Vec::new();
    let mut missing_required = Vec::new();

    for field in form.fields {
        match pick(obj, field) {
            Some(raw_value) => match coerce_field(field, &raw_value) {
                Some(v) => {
                    values.insert(field.key.to_string(), v);
                }
                None => {
                    dropped.push(field.key.to_string());
                    if field.required {
                        missing_required.push(field.key.to_string());
                    }
                }
            },
            None => {
                if field.required {
                    missing_required.push(field.key.to_string());
                }
            }
        }
    }

    // Keys the model invented. Reported, never applied — the frontend renders
    // the declared fields only, so an unknown key could not be shown anyway.
    let declared: Vec<&str> = form.fields.iter().map(|f| f.key).collect();
    for key in obj.keys() {
        let is_container = form
            .fields
            .iter()
            .filter_map(|f| nested_fallback(f.key))
            .any(|(outer, _)| &outer == key);
        if !declared.contains(&key.as_str()) && !is_container {
            dropped.push(key.clone());
        }
    }
    dropped.sort_unstable();
    dropped.dedup();

    Ok(FormFill {
        form_id: form.id.to_string(),
        values,
        missing_required,
        dropped,
    })
}

/// One fill attempt: the decision plus what the provider charged, mirroring
/// `chat::planner::PlanRun` so the trace reports both the same way.
#[derive(Debug)]
pub struct FillRun {
    /// `None` when the model produced nothing usable. The turn then falls back
    /// to opening the form empty, which is still better than an error.
    pub fill: Option<FormFill>,
    pub error: Option<FillError>,
    pub prompt_tokens: u32,
    pub prefill_ms: Option<i64>,
}

/// Split the template at `{{form_id}}`: everything above is identical on every
/// call (the instructions), everything below is per-call (the form's fields,
/// the current values, the request).
///
/// Splitting the TEMPLATE rather than the rendered text keeps the halves exact
/// — the cut lands on a placeholder boundary, so no `{{var}}` straddles it.
/// Same trick as `chat::planner::split_planner_prompt`.
pub(crate) fn split_fill_prompt(
    template: &str,
    form: &FormDef,
    language: &str,
    today: &str,
    current_values: &Value,
    request: &str,
) -> (String, String) {
    let mut vars = std::collections::HashMap::new();
    vars.insert("language", language.to_string());
    vars.insert("today", today.to_string());
    vars.insert("form_id", form.id.to_string());
    vars.insert(
        "fields",
        serde_json::to_string_pretty(&form.fields).unwrap_or_else(|_| "[]".to_string()),
    );
    vars.insert(
        "current_values",
        if current_values.as_object().is_some_and(|o| !o.is_empty()) {
            serde_json::to_string_pretty(current_values).unwrap_or_else(|_| "{}".to_string())
        } else {
            "(none — this is a new form)".to_string()
        },
    );
    vars.insert("request", request.to_string());

    const MARKER: &str = "Form: {{form_id}}";
    let (head, tail) = match template.find(MARKER) {
        Some(idx) => template.split_at(idx),
        // A user-edited template that dropped the marker still works; it just
        // forfeits the one-shot prefix slot.
        None => (template, ""),
    };
    (
        crate::services::prompts::render(head, &vars),
        crate::services::prompts::render(tail, &vars),
    )
}

/// Thin executor: render, call the provider, parse. Never fails the turn — a
/// provider error becomes a `FillRun` with no fill, and the caller opens the
/// form empty.
pub async fn fill_form(
    provider: &dyn AIProvider,
    template: &str,
    form: &FormDef,
    language: &str,
    today: &str,
    current_values: &Value,
    request: &str,
) -> FillRun {
    let (prefix, suffix) = split_fill_prompt(template, form, language, today, current_values, request);
    let opts = CompletionOptions {
        temperature: Some(0.0),
        // A lens with a handful of columns is ~400 tokens of JSON; 1024 leaves
        // room for a verbose model without letting a runaway generation stall
        // the turn.
        max_tokens: Some(1024),
        think: Some(false),
    };
    match provider.complete_with_prefix(&prefix, &suffix, opts).await {
        Ok(result) => match parse_fill(&result.text, form) {
            Ok(fill) => FillRun {
                fill: Some(fill),
                error: None,
                prompt_tokens: result.prompt_tokens,
                prefill_ms: result.prefill_ms,
            },
            Err(e) => FillRun {
                fill: None,
                error: Some(e),
                prompt_tokens: result.prompt_tokens,
                prefill_ms: result.prefill_ms,
            },
        },
        Err(_) => FillRun {
            fill: None,
            error: Some(FillError::NoJsonObject),
            prompt_tokens: 0,
            prefill_ms: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::forms::registry::LENS_CREATE;

    /// The happy path a well-behaved model produces.
    fn good_reply() -> &'static str {
        r#"{
            "name": "Facturas de proveedores",
            "icon": "🧾",
            "scopeMailboxes": ["inbox"],
            "scopeSenderDomains": ["stripe.com", "fly.io"],
            "columns": [
                {"key": "supplier", "label": "Proveedor", "type": "string"},
                {"key": "amount", "label": "Importe", "type": "currency"},
                {"key": "issued_at", "label": "Fecha", "type": "date"}
            ],
            "promptText": "Extrae proveedor, importe y fecha de cada factura."
        }"#
    }

    #[test]
    fn maps_a_clean_reply_onto_the_declared_fields() {
        let fill = parse_fill(good_reply(), &LENS_CREATE).expect("parses");
        assert_eq!(fill.form_id, "lens.create");
        assert_eq!(
            fill.values.get("name").and_then(Value::as_str),
            Some("Facturas de proveedores")
        );
        assert!(fill.values.get("promptText").and_then(Value::as_str).is_some());
    }

    #[test]
    fn a_clean_reply_leaves_nothing_missing_and_nothing_dropped() {
        let fill = parse_fill(good_reply(), &LENS_CREATE).expect("parses");
        assert!(fill.missing_required.is_empty(), "missing: {:?}", fill.missing_required);
        assert!(fill.dropped.is_empty(), "dropped: {:?}", fill.dropped);
    }

    #[test]
    fn keeps_every_column_of_an_object_list() {
        let fill = parse_fill(good_reply(), &LENS_CREATE).expect("parses");
        let columns = fill.values.get("columns").and_then(Value::as_array).expect("columns");
        assert_eq!(columns.len(), 3);
        assert_eq!(columns[1].get("type").and_then(Value::as_str), Some("currency"));
    }

    #[test]
    fn reads_a_reply_wrapped_in_a_fenced_code_block() {
        let raw = format!("Claro, aquí tienes:\n```json\n{}\n```\n", good_reply());
        let fill = parse_fill(&raw, &LENS_CREATE).expect("parses");
        assert_eq!(
            fill.values.get("name").and_then(Value::as_str),
            Some("Facturas de proveedores")
        );
    }

    #[test]
    fn reads_a_reply_preceded_by_a_thinking_block() {
        let raw = format!(
            "<think>The user wants invoices. I'll use columns.</think>\n{}",
            good_reply()
        );
        let fill = parse_fill(&raw, &LENS_CREATE).expect("parses");
        assert_eq!(fill.missing_required, Vec::<String>::new());
    }

    #[test]
    fn rejects_a_reply_with_no_json_at_all() {
        assert_eq!(
            parse_fill("No puedo crear eso.", &LENS_CREATE),
            Err(FillError::NoJsonObject)
        );
    }

    #[test]
    fn rejects_a_reply_whose_json_is_a_bare_array() {
        assert_eq!(parse_fill("[1, 2, 3]", &LENS_CREATE), Err(FillError::NoJsonObject));
    }

    #[test]
    fn names_required_fields_the_model_omitted_instead_of_failing() {
        let fill = parse_fill(r#"{"icon": "🧾"}"#, &LENS_CREATE).expect("partial fills still parse");
        assert_eq!(fill.missing_required, vec!["name", "columns", "promptText"]);
        assert_eq!(fill.values.get("icon").and_then(Value::as_str), Some("🧾"));
    }

    #[test]
    fn drops_keys_the_model_invented() {
        let raw = r#"{"name": "X", "columns": [{"key":"a","label":"A","type":"string"}],
                      "promptText": "p", "deleteEverything": true}"#;
        let fill = parse_fill(raw, &LENS_CREATE).expect("parses");
        assert_eq!(fill.dropped, vec!["deleteEverything"]);
        assert!(!fill.values.contains_key("deleteEverything"));
    }

    #[test]
    fn finds_a_scope_field_the_model_nested_instead_of_flattening() {
        let raw = r#"{"name": "X", "promptText": "p",
                      "columns": [{"key":"a","label":"A","type":"string"}],
                      "scope": {"mailboxes": ["sent"], "senderDomains": ["acme.com"]}}"#;
        let fill = parse_fill(raw, &LENS_CREATE).expect("parses");
        assert_eq!(fill.values.get("scopeMailboxes"), Some(&serde_json::json!(["sent"])));
        assert_eq!(
            fill.values.get("scopeSenderDomains"),
            Some(&serde_json::json!(["acme.com"]))
        );
    }

    #[test]
    fn a_nested_container_is_not_reported_as_an_invented_key() {
        let raw = r#"{"name": "X", "promptText": "p",
                      "columns": [{"key":"a","label":"A","type":"string"}],
                      "scope": {"mailboxes": ["sent"]}}"#;
        let fill = parse_fill(raw, &LENS_CREATE).expect("parses");
        assert!(
            !fill.dropped.contains(&"scope".to_string()),
            "dropped: {:?}",
            fill.dropped
        );
    }

    #[test]
    fn normalizes_an_enum_the_model_capitalized() {
        let raw = r#"{"name":"X","promptText":"p","scopeDirection":"Inbound",
                      "columns":[{"key":"a","label":"A","type":"String"}]}"#;
        let fill = parse_fill(raw, &LENS_CREATE).expect("parses");
        assert_eq!(
            fill.values.get("scopeDirection").and_then(Value::as_str),
            Some("inbound")
        );
        let columns = fill.values.get("columns").and_then(Value::as_array).expect("columns");
        assert_eq!(columns[0].get("type").and_then(Value::as_str), Some("string"));
    }

    #[test]
    fn drops_an_enum_value_outside_the_declared_set() {
        let raw = r#"{"name":"X","promptText":"p","scopeDirection":"sideways",
                      "columns":[{"key":"a","label":"A","type":"string"}]}"#;
        let fill = parse_fill(raw, &LENS_CREATE).expect("parses");
        assert!(!fill.values.contains_key("scopeDirection"));
        assert!(fill.dropped.contains(&"scopeDirection".to_string()));
    }

    #[test]
    fn splits_a_comma_separated_string_into_a_list() {
        let raw = r#"{"name":"X","promptText":"p","scopeSenderDomains":"stripe.com, fly.io",
                      "columns":[{"key":"a","label":"A","type":"string"}]}"#;
        let fill = parse_fill(raw, &LENS_CREATE).expect("parses");
        assert_eq!(
            fill.values.get("scopeSenderDomains"),
            Some(&serde_json::json!(["stripe.com", "fly.io"]))
        );
    }

    #[test]
    fn keeps_only_the_declared_members_of_an_enum_list() {
        let raw = r#"{"name":"X","promptText":"p","scopeMailboxes":["inbox","outbox"],
                      "columns":[{"key":"a","label":"A","type":"string"}]}"#;
        let fill = parse_fill(raw, &LENS_CREATE).expect("parses");
        assert_eq!(fill.values.get("scopeMailboxes"), Some(&serde_json::json!(["inbox"])));
    }

    #[test]
    fn wraps_a_single_object_the_model_forgot_to_put_in_an_array() {
        let raw = r#"{"name":"X","promptText":"p",
                      "columns":{"key":"a","label":"A","type":"string"}}"#;
        let fill = parse_fill(raw, &LENS_CREATE).expect("parses");
        let columns = fill.values.get("columns").and_then(Value::as_array).expect("columns");
        assert_eq!(columns.len(), 1);
    }

    #[test]
    fn discards_a_column_missing_its_required_sub_fields() {
        let raw = r#"{"name":"X","promptText":"p",
                      "columns":[{"key":"a","label":"A","type":"string"},{"label":"orphan"}]}"#;
        let fill = parse_fill(raw, &LENS_CREATE).expect("parses");
        let columns = fill.values.get("columns").and_then(Value::as_array).expect("columns");
        assert_eq!(columns.len(), 1, "the incomplete column must be discarded");
    }

    #[test]
    fn reports_columns_missing_when_every_column_was_discarded() {
        let raw = r#"{"name":"X","promptText":"p","columns":[{"label":"orphan"}]}"#;
        let fill = parse_fill(raw, &LENS_CREATE).expect("parses");
        assert!(fill.missing_required.contains(&"columns".to_string()));
    }

    #[test]
    fn coerces_a_stringified_boolean_in_a_column() {
        let raw = r#"{"name":"X","promptText":"p",
                      "columns":[{"key":"a","label":"A","type":"string","required":"yes"}]}"#;
        let fill = parse_fill(raw, &LENS_CREATE).expect("parses");
        let columns = fill.values.get("columns").and_then(Value::as_array).expect("columns");
        assert_eq!(columns[0].get("required").and_then(Value::as_bool), Some(true));
    }

    #[test]
    fn treats_an_explicit_null_as_absent() {
        let raw = r#"{"name":"X","icon":null,"promptText":"p",
                      "columns":[{"key":"a","label":"A","type":"string"}]}"#;
        let fill = parse_fill(raw, &LENS_CREATE).expect("parses");
        assert!(!fill.values.contains_key("icon"));
        assert!(!fill.missing_required.contains(&"icon".to_string()), "icon is optional");
    }

    #[test]
    fn treats_an_empty_string_as_absent_for_a_required_field() {
        let raw = r#"{"name":"   ","promptText":"p",
                      "columns":[{"key":"a","label":"A","type":"string"}]}"#;
        let fill = parse_fill(raw, &LENS_CREATE).expect("parses");
        assert!(fill.missing_required.contains(&"name".to_string()));
    }

    #[test]
    fn ignores_a_brace_inside_a_string_when_finding_the_object() {
        let raw = r#"{"name":"A } B","promptText":"p",
                     "columns":[{"key":"a","label":"A","type":"string"}]}"#;
        let fill = parse_fill(raw, &LENS_CREATE).expect("parses");
        assert_eq!(fill.values.get("name").and_then(Value::as_str), Some("A } B"));
    }

    #[test]
    fn nested_fallback_splits_a_camel_case_key() {
        assert_eq!(
            nested_fallback("scopeMailboxes"),
            Some(("scope".into(), "mailboxes".into()))
        );
        assert_eq!(nested_fallback("name"), None);
    }
}

#[cfg(test)]
mod executor_tests {
    use super::*;
    use crate::ai::provider::FakeAiProvider;
    use crate::services::forms::registry::LENS_CREATE;
    use crate::services::prompts::defaults::FORMS_FILL;

    fn split() -> (String, String) {
        split_fill_prompt(
            FORMS_FILL,
            &LENS_CREATE,
            "Spanish",
            "2026-09-23",
            &serde_json::json!({}),
            "facturas de proveedores con importe y fecha",
        )
    }

    #[test]
    fn the_prefix_is_identical_for_two_different_requests() {
        // The whole point of the split: the instructions half must not vary,
        // or the one-shot prefix slot re-seeds on every call.
        let (a, _) = split();
        let (b, _) = split_fill_prompt(
            FORMS_FILL,
            &LENS_CREATE,
            "Spanish",
            "2026-09-23",
            &serde_json::json!({}),
            "otra cosa completamente distinta",
        );
        assert_eq!(a, b);
    }

    #[test]
    fn the_suffix_carries_the_form_the_fields_and_the_request() {
        let (_, suffix) = split();
        assert!(suffix.contains("lens.create"));
        assert!(suffix.contains("promptText"), "field keys must reach the model");
        assert!(suffix.contains("facturas de proveedores"));
    }

    #[test]
    fn the_two_halves_leave_no_placeholder_unrendered() {
        let (prefix, suffix) = split();
        let whole = format!("{prefix}{suffix}");
        assert!(!whole.contains("{{"), "unrendered placeholder in: {whole}");
    }

    #[test]
    fn a_fresh_form_says_so_instead_of_showing_an_empty_object() {
        let (_, suffix) = split();
        assert!(suffix.contains("this is a new form"));
    }

    #[test]
    fn an_open_form_sends_its_current_values() {
        let (_, suffix) = split_fill_prompt(
            FORMS_FILL,
            &LENS_CREATE,
            "Spanish",
            "2026-09-23",
            &serde_json::json!({"name": "Facturas"}),
            "añade una columna de IVA",
        );
        assert!(suffix.contains("\"name\""));
        assert!(suffix.contains("Facturas"));
    }

    #[tokio::test]
    async fn a_usable_reply_becomes_a_fill() {
        let provider = FakeAiProvider::new();
        provider.push_completion(
            r#"{"name":"Facturas","promptText":"extrae","columns":[{"key":"a","label":"A","type":"string"}]}"#,
        );
        let run = fill_form(
            &provider,
            FORMS_FILL,
            &LENS_CREATE,
            "Spanish",
            "2026-09-23",
            &serde_json::json!({}),
            "facturas",
        )
        .await;
        let fill = run.fill.expect("a parseable reply fills the form");
        assert_eq!(fill.form_id, "lens.create");
        assert!(fill.missing_required.is_empty());
    }

    #[tokio::test]
    async fn an_unusable_reply_yields_no_fill_instead_of_failing_the_turn() {
        let provider = FakeAiProvider::new();
        provider.push_completion("Lo siento, no puedo.");
        let run = fill_form(
            &provider,
            FORMS_FILL,
            &LENS_CREATE,
            "Spanish",
            "2026-09-23",
            &serde_json::json!({}),
            "facturas",
        )
        .await;
        assert!(run.fill.is_none());
        assert_eq!(run.error, Some(FillError::NoJsonObject));
    }

    #[tokio::test]
    async fn a_provider_error_yields_no_fill_instead_of_failing_the_turn() {
        let provider = FakeAiProvider::new();
        provider.fail_completions(Some("model unavailable"));
        let run = fill_form(
            &provider,
            FORMS_FILL,
            &LENS_CREATE,
            "Spanish",
            "2026-09-23",
            &serde_json::json!({}),
            "facturas",
        )
        .await;
        assert!(run.fill.is_none());
    }
}
