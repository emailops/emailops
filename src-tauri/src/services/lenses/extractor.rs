//! Per-email extraction. Uses the provider's `chat_with_tools` to force a
//! single function call whose JSON-schema parameters mirror the Lens schema.
//! Works on all four AI providers (Ollama / OpenRouter / llamacpp / vllm-metal)
//! because all of them declare `capabilities().tools = true`.

use std::sync::Arc;

use crate::services::app_handle::AppHandle;
use serde_json::json;

use crate::ai::provider::{AIProvider, AiMessage, CompletionOptions};
use crate::db::Database;
use crate::models::error::{AppError, Result};
use crate::models::lens::{Lens, LensColumn, LensColumnType, LensSchema};

use super::emit_log;

/// Default cap on body characters fed to the model. Configurable via the
/// `lenses.max_body_chars` user preference (settable from the Settings UI).
pub const DEFAULT_MAX_BODY_CHARS: usize = 4_000;

/// Result of a single extraction attempt.
pub struct ExtractionResult {
    /// The JSON object returned by the model, validated against the schema.
    /// On `status = "failed"`, this is `serde_json::Value::Null`.
    pub data: serde_json::Value,
    pub status: ExtractionStatus,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtractionStatus {
    Ok,
    Failed,
}

impl ExtractionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            ExtractionStatus::Ok => "ok",
            ExtractionStatus::Failed => "failed",
        }
    }
}

/// Extract structured data from a single email.
pub async fn extract_email(
    db: &Database,
    provider: Arc<dyn AIProvider>,
    lens: &Lens,
    email_id: &str,
    app: Option<&AppHandle>,
) -> Result<ExtractionResult> {
    // 1. Load email metadata + body.
    let email = db
        .get_email_by_id(email_id)?
        .ok_or_else(|| AppError::NotFound(format!("email {email_id}")))?;
    let body = db.get_email_body(email_id).unwrap_or_default();

    let max_body_chars = db
        .get_preference("lenses.max_body_chars")
        .ok()
        .flatten()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(DEFAULT_MAX_BODY_CHARS);

    let body_text = clean_and_trim_body(&body, max_body_chars);

    // 2. Build the tool definition from the Lens schema.
    let tool = build_tool_definition(&lens.schema);

    // 3. Build messages. Some chat templates (e.g. Qwen 3's Jinja template)
    // reject multiple consecutive system messages, so we concatenate the
    // per-Lens prompt into a single system message.
    let prompt = lens.prompt_text.trim();
    let system_msg = if prompt.is_empty() {
        "You extract structured information from a single email. Call \
         `submit_extraction` exactly once. Use null for fields you cannot \
         determine. Do not include explanations or additional text."
            .to_string()
    } else {
        format!(
            "You extract structured information from a single email. Call \
             `submit_extraction` exactly once. Use null for fields you cannot \
             determine. Do not include explanations or additional text.\n\n{prompt}"
        )
    };
    let user_msg = format!(
        "From: {sender} <{from_email}>\nDate: {ts}\nSubject: {subject}\n\n{body}",
        sender = email.sender,
        from_email = email.sender_email,
        ts = format_date_from_unix(email.timestamp),
        subject = email.subject,
        body = body_text,
    );

    let messages = vec![
        AiMessage {
            role: "system".to_string(),
            content: system_msg.clone(),
            tool_calls: None,
        },
        AiMessage {
            role: "user".to_string(),
            content: user_msg.clone(),
            tool_calls: None,
        },
    ];

    // 4. Call the model. Tool-calling is still the primary path, but several
    // local models under-fill tool arguments while answering the same prompt
    // well in plain chat. Keep a text fallback for sparse or unsupported tool
    // responses.
    let tool_result = provider.chat_with_tools(&messages, std::slice::from_ref(&tool)).await;
    let tool_extracted = match tool_result {
        Ok(response) => response
            .tool_calls
            .as_ref()
            .and_then(|calls| calls.first())
            .map(|c| c.function.arguments.clone())
            .or_else(|| extract_text_response_to_json(&response.content, &lens.schema))
            .ok_or_else(|| "model did not return a structured response".to_string()),
        Err(e) => {
            emit_log(
                app,
                "warn",
                format!("Lens '{}': tool extraction failed, retrying as text: {e}", lens.name),
            );
            Err(format!("model error: {e}"))
        }
    };

    let mut best_data = match tool_extracted {
        Ok(extracted) => match validate_against_schema(&extracted, &lens.schema) {
            Ok(coerced) => coerced,
            Err(msg) => match extract_via_text_prompt(provider.as_ref(), lens, &system_msg, &user_msg).await {
                Ok(text_data) => {
                    return Ok(ExtractionResult {
                        data: text_data,
                        status: ExtractionStatus::Ok,
                        error_message: None,
                    });
                }
                Err(text_msg) => {
                    return Ok(ExtractionResult {
                        data: extracted,
                        status: ExtractionStatus::Failed,
                        error_message: Some(format!("{msg}; text retry failed: {text_msg}")),
                    });
                }
            },
        },
        Err(tool_msg) => {
            return match extract_via_text_prompt(provider.as_ref(), lens, &system_msg, &user_msg).await {
                Ok(text_data) => Ok(ExtractionResult {
                    data: text_data,
                    status: ExtractionStatus::Ok,
                    error_message: None,
                }),
                Err(text_msg) => Ok(ExtractionResult {
                    data: serde_json::Value::Null,
                    status: ExtractionStatus::Failed,
                    error_message: Some(format!("{tool_msg}; text retry failed: {text_msg}")),
                }),
            };
        }
    };

    if extraction_is_sparse(&best_data, &lens.schema) {
        if let Ok(text_data) = extract_via_text_prompt(provider.as_ref(), lens, &system_msg, &user_msg).await {
            if extraction_score(&text_data, &lens.schema) > extraction_score(&best_data, &lens.schema) {
                best_data = text_data;
            }
        }
    }

    Ok(ExtractionResult {
        data: best_data,
        status: ExtractionStatus::Ok,
        error_message: None,
    })
}

/// Build the JSON-schema tool definition. PRD §7.4 maps each `LensColumnType`
/// to its JSON-Schema shape; non-required columns also accept `null`.
pub fn build_tool_definition(schema: &LensSchema) -> serde_json::Value {
    let mut props = serde_json::Map::new();
    let mut required: Vec<String> = Vec::new();

    for col in &schema.columns {
        props.insert(col.key.clone(), column_to_json_schema(col));
        if col.required {
            required.push(col.key.clone());
        }
    }

    json!({
        "type": "function",
        "function": {
            "name": "submit_extraction",
            "description": "Submit the extracted fields for this email.",
            "parameters": {
                "type": "object",
                "required": required,
                "properties": props,
            }
        }
    })
}

async fn extract_via_text_prompt(
    provider: &dyn AIProvider,
    lens: &Lens,
    system_msg: &str,
    user_msg: &str,
) -> std::result::Result<serde_json::Value, String> {
    let prompt = build_text_extraction_prompt(lens, system_msg, user_msg);
    let response = provider
        .complete(
            &prompt,
            CompletionOptions {
                temperature: Some(0.0),
                max_tokens: Some(1024),
                think: Some(false),
            },
        )
        .await
        .map_err(|e| e.to_string())?;

    let extracted = extract_text_response_to_json(&response.text, &lens.schema)
        .ok_or_else(|| "model did not return parseable extraction text".to_string())?;
    validate_against_schema(&extracted, &lens.schema)
}

fn build_text_extraction_prompt(lens: &Lens, system_msg: &str, user_msg: &str) -> String {
    let fields = lens
        .schema
        .columns
        .iter()
        .map(|col| {
            let type_name = match col.column_type {
                LensColumnType::String => "string",
                LensColumnType::Text => "text",
                LensColumnType::Number => "number",
                LensColumnType::Currency => "currency object {\"amount\": number, \"currency\": \"ISO-4217\"}",
                LensColumnType::Date => "date string YYYY-MM-DD",
                LensColumnType::Boolean => "boolean",
                LensColumnType::Enum => "enum",
                LensColumnType::Email => "email",
                LensColumnType::Url => "url",
            };
            let enum_values = col
                .enum_values
                .as_ref()
                .filter(|values| !values.is_empty())
                .map(|values| format!(" Allowed values: {}.", values.join(", ")))
                .unwrap_or_default();
            format!(
                "- {} ({}): {}{}",
                col.key,
                type_name,
                col.description.trim(),
                enum_values
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "{system_msg}\n\nReturn exactly one JSON object and no prose. \
         Include every field key below. Use null for unknown values.\n\nFields:\n{fields}\n\nEmail:\n{user_msg}"
    )
}

fn extract_text_response_to_json(content: &str, schema: &LensSchema) -> Option<serde_json::Value> {
    try_parse_json_object(content).or_else(|| parse_markdown_field_list(content, schema))
}

fn column_to_json_schema(col: &LensColumn) -> serde_json::Value {
    // We deliberately keep `type` a single string rather than the JSON-Schema
    // `["string", "null"]` array form. Several chat templates (notably Qwen
    // 3.x's Jinja template) fail to render the array form and emit ffi
    // error -3 from `apply_chat_template_oaicompat`. Optionality is conveyed
    // entirely via the parent object's `required` array, which is the shape
    // OpenAI's tool-calling spec recommends.
    match col.column_type {
        LensColumnType::String | LensColumnType::Text => {
            json!({"type": "string", "description": col.description})
        }
        LensColumnType::Email => {
            json!({"type": "string", "format": "email", "description": col.description})
        }
        LensColumnType::Url => {
            json!({"type": "string", "format": "uri", "description": col.description})
        }
        LensColumnType::Number => {
            json!({"type": "number", "description": col.description})
        }
        LensColumnType::Currency => json!({
            "type": "object",
            "required": ["amount", "currency"],
            "properties": {
                "amount": {"type": "number"},
                "currency": {"type": "string"}
            },
            "description": col.description,
        }),
        LensColumnType::Date => {
            json!({"type": "string", "format": "date", "description": col.description})
        }
        LensColumnType::Boolean => {
            json!({"type": "boolean", "description": col.description})
        }
        LensColumnType::Enum => {
            let values = col.enum_values.clone().unwrap_or_default();
            json!({"type": "string", "enum": values, "description": col.description})
        }
    }
}

/// Validate the extracted object against the schema and coerce values where
/// safe (e.g. wrap stray currency strings into the `{amount, currency}` shape).
/// On validation failure returns the first error message encountered.
fn validate_against_schema(
    extracted: &serde_json::Value,
    schema: &LensSchema,
) -> std::result::Result<serde_json::Value, String> {
    let obj = extracted
        .as_object()
        .ok_or_else(|| "extraction response is not a JSON object".to_string())?;

    let mut out = serde_json::Map::new();
    for col in &schema.columns {
        let val = obj.get(&col.key).cloned().unwrap_or(serde_json::Value::Null);

        if val.is_null() {
            // Store null and continue — even for 'required' columns.
            // 'required' guides the model to try harder (via the JSON-schema
            // tool definition) but is not a hard gate on the output: discarding
            // an otherwise valid extraction because one field came back null is
            // worse UX than showing the row with a visible null in the
            // spreadsheet. The user can then refine the prompt or make the
            // column optional.
            out.insert(col.key.clone(), serde_json::Value::Null);
            continue;
        }

        let coerced = match col.column_type {
            LensColumnType::String | LensColumnType::Text | LensColumnType::Email | LensColumnType::Url => match val {
                serde_json::Value::String(s) => serde_json::Value::String(s),
                other => serde_json::Value::String(other.to_string()),
            },
            LensColumnType::Number => match val.as_f64() {
                Some(n) => json!(n),
                None => return Err(format!("column '{}' is not a number", col.key)),
            },
            LensColumnType::Boolean => match val.as_bool() {
                Some(b) => json!(b),
                None => return Err(format!("column '{}' is not a boolean", col.key)),
            },
            LensColumnType::Date => match val.as_str() {
                Some(s) => serde_json::Value::String(s.to_string()),
                None => return Err(format!("column '{}' is not a date string", col.key)),
            },
            LensColumnType::Enum => match val.as_str() {
                Some(s) => {
                    if let Some(values) = col.enum_values.as_ref() {
                        if !values.iter().any(|v| v == s) {
                            return Err(format!("column '{}' value '{s}' is not one of {values:?}", col.key));
                        }
                    }
                    serde_json::Value::String(s.to_string())
                }
                None => return Err(format!("column '{}' is not a string", col.key)),
            },
            LensColumnType::Currency => match val {
                serde_json::Value::Object(mut m) => {
                    let amount = m
                        .remove("amount")
                        .and_then(|v| v.as_f64())
                        .ok_or_else(|| format!("column '{}' missing numeric amount", col.key))?;
                    let currency = m
                        .remove("currency")
                        .and_then(|v| v.as_str().map(|s| s.to_string()))
                        .unwrap_or_default();
                    json!({"amount": amount, "currency": currency})
                }
                // Models often collapse the nested object to a bare number
                // (e.g. Ollama returns `42.50` for an "amount" field instead of
                // `{"amount": 42.50, "currency": "EUR"}`). Wrap it rather than
                // failing — the currency code will be empty but the amount is
                // preserved and visible in the spreadsheet.
                serde_json::Value::Number(n) => {
                    json!({"amount": n.as_f64().unwrap_or(0.0), "currency": ""})
                }
                serde_json::Value::String(s) => parse_currency_value(&s)
                    .map(|(amount, currency)| json!({"amount": amount, "currency": currency}))
                    .ok_or_else(|| format!("column '{}' is not a currency object", col.key))?,
                _ => return Err(format!("column '{}' is not a currency object", col.key)),
            },
        };
        out.insert(col.key.clone(), coerced);
    }

    Ok(serde_json::Value::Object(out))
}

fn extraction_is_sparse(data: &serde_json::Value, schema: &LensSchema) -> bool {
    let Some(obj) = data.as_object() else {
        return true;
    };
    schema
        .columns
        .iter()
        .filter(|col| col.required)
        .any(|col| obj.get(&col.key).is_none_or(value_is_empty))
}

fn extraction_score(data: &serde_json::Value, schema: &LensSchema) -> usize {
    let Some(obj) = data.as_object() else {
        return 0;
    };
    schema
        .columns
        .iter()
        .filter(|col| obj.get(&col.key).is_some_and(|v| !value_is_empty(v)))
        .count()
}

fn value_is_empty(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Null => true,
        serde_json::Value::String(s) => s.trim().is_empty(),
        serde_json::Value::Object(m) => m.is_empty() || m.values().all(value_is_empty),
        serde_json::Value::Array(a) => a.is_empty(),
        _ => false,
    }
}

/// Strip HTML, collapse whitespace, and truncate to `max_chars` (from the
/// bottom — header context at the top is more important than trailing
/// footers/quoted blocks).
pub(crate) fn clean_and_trim_body(body: &str, max_chars: usize) -> String {
    let stripped = crate::util::html::strip_html_for_fts(body);
    let normalised = stripped.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalised.chars().count() <= max_chars {
        return normalised;
    }
    // Take the first `max_chars` codepoints; we keep the head because subject
    // + greeting tend to hold the most signal for short prompts.
    normalised.chars().take(max_chars).collect::<String>()
}

/// Convert a Unix timestamp (seconds) to an ISO 8601 date string (`YYYY-MM-DD`).
/// Used to give the model a human-readable date rather than a raw integer.
/// Implemented without external crates using Hinnant's civil-date algorithm.
fn format_date_from_unix(secs: i64) -> String {
    if secs <= 0 {
        return String::new();
    }
    let days = secs / 86_400;
    let z = days + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

/// Best-effort: extract a top-level JSON object from a free-text response.
/// Used when the model ignores the tool schema (PRD §7.4 documented fallback).
fn try_parse_json_object(content: &str) -> Option<serde_json::Value> {
    let trimmed = content.trim();
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if v.is_object() {
            return Some(v);
        }
    }
    // Find the first '{' ... matching '}' span and try parsing that.
    let start = trimmed.find('{')?;
    let end = trimmed.rfind('}')?;
    if end > start {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&trimmed[start..=end]) {
            if v.is_object() {
                return Some(v);
            }
        }
    }
    None
}

fn parse_markdown_field_list(content: &str, schema: &LensSchema) -> Option<serde_json::Value> {
    let mut out = serde_json::Map::new();

    for raw_line in content.lines() {
        let Some((name, value)) = split_field_line(raw_line) else {
            continue;
        };
        let Some(col) = schema.columns.iter().find(|col| field_name_matches(&name, col)) else {
            continue;
        };
        out.insert(col.key.clone(), parse_scalar_text_value(&value, col));
    }

    if out.is_empty() {
        None
    } else {
        Some(serde_json::Value::Object(out))
    }
}

fn split_field_line(line: &str) -> Option<(String, String)> {
    let trimmed = line
        .trim()
        .trim_start_matches(|c: char| matches!(c, '-' | '*' | '•') || c.is_whitespace())
        .trim();
    let (left, right) = trimmed.split_once(':')?;
    let name = clean_markdown_token(left);
    if name.is_empty() {
        return None;
    }
    Some((name, right.trim().to_string()))
}

fn clean_markdown_token(token: &str) -> String {
    token.trim().trim_matches('*').trim_matches('`').trim().to_string()
}

fn field_name_matches(name: &str, col: &LensColumn) -> bool {
    let name = normalise_field_name(name);
    name == normalise_field_name(&col.key) || name == normalise_field_name(&col.label)
}

fn normalise_field_name(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn parse_scalar_text_value(value: &str, col: &LensColumn) -> serde_json::Value {
    let cleaned = clean_markdown_token(value)
        .trim()
        .trim_end_matches('.')
        .trim()
        .to_string();
    let lower = cleaned.to_ascii_lowercase();
    let is_unknown_enum_value = col.column_type == LensColumnType::Enum
        && col
            .enum_values
            .as_ref()
            .is_some_and(|values| values.iter().any(|value| value.eq_ignore_ascii_case(&cleaned)));
    if cleaned.is_empty()
        || (!is_unknown_enum_value
            && matches!(
                lower.as_str(),
                "null" | "none" | "n/a" | "unknown" | "desconocido" | "no indicado" | "no consta"
            ))
    {
        serde_json::Value::Null
    } else {
        serde_json::Value::String(cleaned)
    }
}

fn parse_currency_value(value: &str) -> Option<(f64, String)> {
    let cleaned = value.trim();
    let currency = detect_currency(cleaned).unwrap_or_default();
    let amount = parse_first_number(cleaned)?;
    Some((amount, currency))
}

fn detect_currency(value: &str) -> Option<String> {
    let upper = value.to_ascii_uppercase();
    for code in [
        "USD", "EUR", "GBP", "CAD", "AUD", "CHF", "JPY", "CNY", "SEK", "NOK", "DKK",
    ] {
        if upper.contains(code) {
            return Some(code.to_string());
        }
    }
    if value.contains('€') {
        Some("EUR".to_string())
    } else if value.contains('$') {
        Some("USD".to_string())
    } else if value.contains('£') {
        Some("GBP".to_string())
    } else {
        None
    }
}

fn parse_first_number(value: &str) -> Option<f64> {
    let mut buf = String::new();
    let mut started = false;
    for ch in value.chars() {
        if ch.is_ascii_digit() || ch == ',' || ch == '.' || (ch == '-' && !started) {
            buf.push(ch);
            started = true;
        } else if started {
            break;
        }
    }
    parse_localised_number(&buf)
}

fn parse_localised_number(raw: &str) -> Option<f64> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }

    let comma = raw.rfind(',');
    let dot = raw.rfind('.');
    let normalised = match (comma, dot) {
        (Some(c), Some(d)) if c > d => raw.replace('.', "").replace(',', "."),
        (Some(_), Some(_)) => raw.replace(',', ""),
        (Some(c), None) => {
            let digits_after = raw[c + 1..].chars().filter(|ch| ch.is_ascii_digit()).count();
            if digits_after == 2 {
                raw.replace(',', ".")
            } else {
                raw.replace(',', "")
            }
        }
        _ => raw.to_string(),
    };
    normalised.parse::<f64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::ai::provider::FakeAiProvider;
    use crate::db::Database;
    use crate::models::lens::{Direction, LensColumn, LensColumnType, LensSchema, LensScope};

    fn schema() -> LensSchema {
        LensSchema {
            columns: vec![
                LensColumn {
                    key: "vendor".into(),
                    label: "Vendor".into(),
                    column_type: LensColumnType::String,
                    description: "Vendor name".into(),
                    enum_values: None,
                    required: true,
                    is_unique_key: false,
                },
                LensColumn {
                    key: "amount".into(),
                    label: "Amount".into(),
                    column_type: LensColumnType::Number,
                    description: "Total amount".into(),
                    enum_values: None,
                    required: false,
                    is_unique_key: false,
                },
                LensColumn {
                    key: "status".into(),
                    label: "Status".into(),
                    column_type: LensColumnType::Enum,
                    description: "Invoice status".into(),
                    enum_values: Some(vec!["paid".into(), "unpaid".into()]),
                    required: false,
                    is_unique_key: false,
                },
            ],
        }
    }

    fn invoice_schema() -> LensSchema {
        LensSchema {
            columns: vec![
                LensColumn {
                    key: "vendor".into(),
                    label: "Vendor".into(),
                    column_type: LensColumnType::String,
                    description: "Company or person issuing the invoice.".into(),
                    enum_values: None,
                    required: true,
                    is_unique_key: false,
                },
                LensColumn {
                    key: "amount".into(),
                    label: "Amount".into(),
                    column_type: LensColumnType::Currency,
                    description: "Total amount due, including currency.".into(),
                    enum_values: None,
                    required: true,
                    is_unique_key: false,
                },
                LensColumn {
                    key: "invoice_number".into(),
                    label: "Invoice #".into(),
                    column_type: LensColumnType::String,
                    description: "Vendor's invoice number / reference.".into(),
                    enum_values: None,
                    required: false,
                    is_unique_key: false,
                },
                LensColumn {
                    key: "due_date".into(),
                    label: "Due".into(),
                    column_type: LensColumnType::Date,
                    description: "Date by which the invoice must be paid. ISO 8601.".into(),
                    enum_values: None,
                    required: false,
                    is_unique_key: false,
                },
                LensColumn {
                    key: "status".into(),
                    label: "Status".into(),
                    column_type: LensColumnType::Enum,
                    description: "Best guess from the email content.".into(),
                    enum_values: Some(vec!["unpaid".into(), "paid".into(), "overdue".into(), "unknown".into()]),
                    required: true,
                    is_unique_key: false,
                },
            ],
        }
    }

    #[test]
    fn tool_definition_marks_required_columns() {
        let def = build_tool_definition(&schema());
        let params = &def["function"]["parameters"];
        let required = params["required"].as_array().unwrap();
        assert_eq!(required.len(), 1);
        assert_eq!(required[0], "vendor");
        let props = params["properties"].as_object().unwrap();
        assert!(props.contains_key("vendor"));
        assert!(props.contains_key("amount"));
        assert!(props.contains_key("status"));
    }

    #[test]
    fn validate_passes_through_null_for_required_field() {
        // 'required' is a hint to the model, not a hard gate on the output.
        // A required field coming back null still produces a successful
        // validation result (with null stored) so the row appears in the
        // spreadsheet. The user can then refine the prompt or make the field
        // optional rather than having the extraction silently discarded.
        let res = validate_against_schema(&json!({"amount": 10}), &schema());
        assert!(res.is_ok(), "null required field must not hard-fail validation");
        let out = res.unwrap();
        assert!(out["vendor"].is_null(), "missing required field must be stored as null");
        assert_eq!(out["amount"], 10.0);
    }

    #[test]
    fn validate_rejects_unknown_enum_value() {
        let res = validate_against_schema(&json!({"vendor": "Acme", "status": "pending"}), &schema());
        assert!(res.is_err());
    }

    #[test]
    fn validate_accepts_null_for_optional_fields() {
        let coerced =
            validate_against_schema(&json!({"vendor": "Acme", "amount": null, "status": null}), &schema()).unwrap();
        assert_eq!(coerced["vendor"], "Acme");
        assert!(coerced["amount"].is_null());
        assert!(coerced["status"].is_null());
    }

    #[test]
    fn validate_coerces_currency_object() {
        let schema = LensSchema {
            columns: vec![LensColumn {
                key: "price".into(),
                label: "Price".into(),
                column_type: LensColumnType::Currency,
                description: "".into(),
                enum_values: None,
                required: true,
                is_unique_key: false,
            }],
        };
        let coerced = validate_against_schema(&json!({"price": {"amount": 12.5, "currency": "USD"}}), &schema).unwrap();
        assert_eq!(coerced["price"]["amount"], 12.5);
        assert_eq!(coerced["price"]["currency"], "USD");
    }

    #[test]
    fn validate_coerces_bare_number_for_currency_column() {
        // Regression: Ollama (and other local models) often collapse a Currency
        // column to a bare number instead of the expected nested object
        // {"amount": N, "currency": "EUR"}.  The validator must wrap the number
        // rather than failing, so the row appears in the spreadsheet with the
        // amount preserved (currency code left empty for the user to fill in).
        let schema = LensSchema {
            columns: vec![LensColumn {
                key: "amount".into(),
                label: "Amount".into(),
                column_type: LensColumnType::Currency,
                description: "Invoice amount".into(),
                enum_values: None,
                required: false,
                is_unique_key: false,
            }],
        };
        let coerced = validate_against_schema(&json!({"amount": 42.5}), &schema).unwrap();
        assert_eq!(coerced["amount"]["amount"], 42.5, "amount must be preserved");
        assert_eq!(
            coerced["amount"]["currency"], "",
            "currency defaults to empty when model omits it"
        );
    }

    #[test]
    fn validate_coerces_integer_bare_number_for_currency_column() {
        let schema = LensSchema {
            columns: vec![LensColumn {
                key: "total".into(),
                label: "Total".into(),
                column_type: LensColumnType::Currency,
                description: "".into(),
                enum_values: None,
                required: false,
                is_unique_key: false,
            }],
        };
        let coerced = validate_against_schema(&json!({"total": 100}), &schema).unwrap();
        assert_eq!(coerced["total"]["amount"], 100.0);
    }

    #[test]
    fn validate_coerces_localised_currency_string() {
        let schema = LensSchema {
            columns: vec![LensColumn {
                key: "amount".into(),
                label: "Amount".into(),
                column_type: LensColumnType::Currency,
                description: "".into(),
                enum_values: None,
                required: true,
                is_unique_key: false,
            }],
        };

        let coerced = validate_against_schema(&json!({"amount": "24,20 EUR"}), &schema).unwrap();
        assert_eq!(coerced["amount"]["amount"], 24.2);
        assert_eq!(coerced["amount"]["currency"], "EUR");
    }

    #[test]
    fn parses_markdown_field_list_from_manual_model_answer() {
        let answer = "Basado en el hilo de conversación proporcionado:\n\
            - **vendor**: Barceló\n\
            - **amount**: 24,20 EUR\n\
            - **invoice_number**: BCL-0010144\n\
            - **due_date**: 2026-05-19\n\
            - **status**: unpaid";

        let parsed = extract_text_response_to_json(answer, &invoice_schema()).expect("parse field list");
        let coerced = validate_against_schema(&parsed, &invoice_schema()).expect("validate field list");
        assert_eq!(coerced["vendor"], "Barceló");
        assert_eq!(coerced["amount"]["amount"], 24.2);
        assert_eq!(coerced["amount"]["currency"], "EUR");
        assert_eq!(coerced["invoice_number"], "BCL-0010144");
        assert_eq!(coerced["due_date"], "2026-05-19");
        assert_eq!(coerced["status"], "unpaid");
    }

    #[tokio::test]
    async fn extract_email_retries_sparse_tool_result_with_text_prompt() {
        let db = Database::new_for_testing().expect("test db");
        db.connection()
            .execute(
                "INSERT INTO accounts (id, provider, email, name, created_at)
                 VALUES ('acct1', 'gmail', 'me@example.com', 'Me', 0)",
                [],
            )
            .expect("insert account");
        db.connection()
            .execute(
                "INSERT INTO emails
                 (id, account_id, thread_id, subject, sender, sender_email, sender_domain,
                  recipients_json, cc_json, snippet, timestamp, is_read, category, mailbox, created_at)
                 VALUES
                 ('email1', 'acct1', 'thread1', 'Nueva factura de Barceló', 'Impact Hub Barceló',
                  'madrid.barcelo@impacthub.net', 'impacthub.net', '[]', '[]', 'factura', 1779187200,
                  0, 'primary', 'inbox', 1779187200)",
                [],
            )
            .expect("insert email");
        db.connection()
            .execute(
                "INSERT INTO email_bodies (email_id, body)
                 VALUES ('email1', 'Factura BCL-0010144 por 24,20 EUR. Vence el 2026-05-19.')",
                [],
            )
            .expect("insert body");

        let provider = Arc::new(FakeAiProvider::new());
        provider.push_chat_response(r#"{"amount": 24.2}"#);
        provider.push_completion(
            "- vendor: Barceló\n\
             - amount: 24,20 EUR\n\
             - invoice_number: BCL-0010144\n\
             - due_date: 2026-05-19\n\
             - status: unpaid",
        );

        let lens = Lens {
            id: "lens1".into(),
            name: "Invoices received".into(),
            icon: None,
            template_key: Some("invoices_received".into()),
            account_id: None,
            scope: LensScope {
                direction: Some(Direction::Inbound),
                ..Default::default()
            },
            schema: invoice_schema(),
            prompt_text: "Extract invoice fields.".into(),
            prompt_version: 1,
            model_provider: None,
            model_name: None,
            is_enabled: true,
            sort_order: 1,
            created_at: 0,
            updated_at: 0,
        };

        let result = extract_email(&db, provider, &lens, "email1", None)
            .await
            .expect("extract");

        assert_eq!(result.status, ExtractionStatus::Ok);
        assert_eq!(result.data["vendor"], "Barceló");
        assert_eq!(result.data["amount"]["amount"], 24.2);
        assert_eq!(result.data["amount"]["currency"], "EUR");
        assert_eq!(result.data["invoice_number"], "BCL-0010144");
        assert_eq!(result.data["due_date"], "2026-05-19");
        assert_eq!(result.data["status"], "unpaid");
    }

    #[test]
    fn body_is_trimmed_to_max_chars() {
        let body = "abc ".repeat(2000);
        let trimmed = clean_and_trim_body(&body, 50);
        assert_eq!(trimmed.chars().count(), 50);
    }

    #[test]
    fn parse_embedded_json_from_free_text() {
        let s = "Sure! Here you go:\n{\"vendor\": \"Acme\"}\nlet me know.";
        let parsed = try_parse_json_object(s).expect("should parse");
        assert_eq!(parsed["vendor"], "Acme");
    }
}
