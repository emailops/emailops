//! The shape a structured completion must take, described once and rendered
//! for each provider: a GBNF grammar for the embedded llama.cpp runtime (it
//! masks every token that would leave the shape, so the reply always parses)
//! and a JSON Schema for Ollama's `format` and OpenRouter's `response_format`.
//!
//! Deliberately small — objects with ordered fields (all required), arrays,
//! bounded strings and closed sets of strings — because that is all the
//! callers need, and a small shape renders to a grammar that is easy to check.

use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq)]
pub enum JsonShape {
    /// Fields in the order the model writes them; every field is required.
    Object(Vec<(String, JsonShape)>),
    Array {
        items: Box<JsonShape>,
        min: usize,
        max: usize,
    },
    /// A string of at most `max_len` characters.
    String { max_len: usize },
    /// One of these strings.
    Enum(Vec<String>),
}

impl JsonShape {
    pub fn object(fields: Vec<(&str, JsonShape)>) -> Self {
        JsonShape::Object(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    pub fn array(items: JsonShape, min: usize, max: usize) -> Self {
        JsonShape::Array {
            items: Box::new(items),
            min,
            max,
        }
    }

    pub fn one_of<S: AsRef<str>>(values: &[S]) -> Self {
        JsonShape::Enum(values.iter().map(|v| v.as_ref().to_string()).collect())
    }

    /// The shape as a JSON Schema (draft 2020-12 subset) for providers that
    /// take one.
    pub fn to_json_schema(&self) -> Value {
        match self {
            JsonShape::Object(fields) => {
                let properties: serde_json::Map<String, Value> =
                    fields.iter().map(|(k, v)| (k.clone(), v.to_json_schema())).collect();
                let required: Vec<&String> = fields.iter().map(|(k, _)| k).collect();
                json!({
                    "type": "object",
                    "properties": properties,
                    "required": required,
                    "additionalProperties": false,
                })
            }
            JsonShape::Array { items, min, max } => json!({
                "type": "array",
                "items": items.to_json_schema(),
                "minItems": min,
                "maxItems": max,
            }),
            JsonShape::String { max_len } => json!({ "type": "string", "maxLength": max_len }),
            JsonShape::Enum(values) => json!({ "type": "string", "enum": values }),
        }
    }

    /// The shape as a llama.cpp GBNF grammar whose root is the whole reply.
    pub fn to_gbnf(&self) -> String {
        let mut rules: Vec<String> = Vec::new();
        let root = self.rule(&mut rules);
        let mut out = format!("root ::= space {root}\n");
        for (i, body) in rules.iter().enumerate() {
            out.push_str(&format!("r{i} ::= {body}\n"));
        }
        out.push_str("space ::= | \" \" | \"\\n\" [ \\t]{0,20}\n");
        out.push_str("char ::= [^\"\\\\\\x7F\\x00-\\x1F] | [\\\\] ([\"\\\\bfnrt] | \"u\" [0-9a-fA-F]{4})\n");
        out
    }

    /// Adds this shape's rule (and its children's) to `rules`; returns its name.
    fn rule(&self, rules: &mut Vec<String>) -> String {
        let body = match self {
            JsonShape::Object(fields) => {
                let parts: Vec<String> = fields
                    .iter()
                    .map(|(key, value)| {
                        let value_rule = value.rule(rules);
                        format!("{} space \":\" space {value_rule}", gbnf_literal(&json_string(key)))
                    })
                    .collect();
                format!("\"{{\" space {} \"}}\" space", parts.join(" \",\" space "))
            }
            JsonShape::Array { items, min, max } => {
                let item = items.rule(rules);
                let rest = max.saturating_sub(1);
                let more = format!("(\",\" space {item}){{0,{rest}}}");
                if *min == 0 {
                    format!("\"[\" space ({item} {more})? \"]\" space")
                } else {
                    format!("\"[\" space {item} {more} \"]\" space")
                }
            }
            JsonShape::String { max_len } => format!("\"\\\"\" char{{0,{max_len}}} \"\\\"\" space"),
            JsonShape::Enum(values) => {
                let alternatives: Vec<String> = values.iter().map(|v| gbnf_literal(&json_string(v))).collect();
                format!("({}) space", alternatives.join(" | "))
            }
        };
        rules.push(body);
        format!("r{}", rules.len() - 1)
    }
}

/// `value` as a JSON string literal, quotes included.
fn json_string(value: &str) -> String {
    Value::String(value.to_string()).to_string()
}

/// `text` as a GBNF double-quoted literal.
fn gbnf_literal(text: &str) -> String {
    let escaped: String = text
        .chars()
        .map(|c| match c {
            '"' => "\\\"".to_string(),
            '\\' => "\\\\".to_string(),
            '\n' => "\\n".to_string(),
            c => c.to_string(),
        })
        .collect();
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finding() -> JsonShape {
        JsonShape::object(vec![
            ("tag", JsonShape::one_of(&["match", "context"])),
            ("text", JsonShape::String { max_len: 200 }),
            ("emails", JsonShape::array(JsonShape::one_of(&["E1", "E2"]), 1, 3)),
        ])
    }

    #[test]
    fn a_shape_renders_to_a_json_schema() {
        let schema = JsonShape::array(finding(), 0, 10).to_json_schema();
        assert_eq!(schema["type"], "array");
        assert_eq!(schema["maxItems"], 10);
        let item = &schema["items"];
        assert_eq!(item["required"], json!(["tag", "text", "emails"]));
        assert_eq!(item["properties"]["tag"]["enum"], json!(["match", "context"]));
        assert_eq!(item["properties"]["text"]["maxLength"], 200);
        assert_eq!(item["properties"]["emails"]["minItems"], 1);
    }

    #[test]
    fn a_shape_renders_to_a_grammar_with_fields_in_order() {
        let grammar = JsonShape::object(vec![("findings", JsonShape::array(finding(), 0, 10))]).to_gbnf();
        assert!(grammar.starts_with("root ::= space r"), "{grammar}");
        // Fields in the declared order, not alphabetical.
        let tag = grammar.find("\"\\\"tag\\\"\"").expect("tag key");
        let text = grammar.find("\"\\\"text\\\"\"").expect("text key");
        let emails = grammar.find("\"\\\"emails\\\"\"").expect("emails key");
        assert!(tag < text && text < emails, "{grammar}");
        assert!(
            grammar.contains("(\"\\\"match\\\"\" | \"\\\"context\\\"\") space"),
            "{grammar}"
        );
        assert!(grammar.contains("char{0,200}"), "{grammar}");
        assert!(grammar.contains("{0,9}"), "an array of at most 10: {grammar}");
        assert!(
            grammar.contains("{0,2}"),
            "at most 3 emails, the first required: {grammar}"
        );
    }

    #[test]
    fn every_rule_a_grammar_references_is_defined() {
        let grammar = JsonShape::object(vec![("findings", JsonShape::array(finding(), 0, 10))]).to_gbnf();
        let defined: Vec<&str> = grammar
            .lines()
            .filter_map(|l| l.split_once(" ::= ").map(|(name, _)| name))
            .collect();
        // The generated rules only: `char`'s classes hold quotes of their own.
        let bodies: String = grammar
            .lines()
            .filter_map(|l| l.split_once(" ::= "))
            .filter(|(name, _)| *name == "root" || name.starts_with('r'))
            .map(|(_, body)| body)
            .collect();
        // Rule names outside string literals.
        let mut outside = String::new();
        let mut in_literal = false;
        let mut escaped = false;
        for c in bodies.chars() {
            match (in_literal, escaped, c) {
                (true, true, _) => escaped = false,
                (true, false, '\\') => escaped = true,
                (_, false, '"') => in_literal = !in_literal,
                (false, _, c) => outside.push(c),
                _ => {}
            }
        }
        let referenced: Vec<&str> = outside
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .filter(|w| w.starts_with('r') && w[1..].chars().all(|c| c.is_ascii_digit()) && w.len() > 1)
            .chain(["space", "char"])
            .collect();
        for name in referenced {
            assert!(defined.contains(&name), "{name} is used but not defined:\n{grammar}");
        }
    }

    #[test]
    fn literals_escape_quotes_and_backslashes() {
        assert_eq!(gbnf_literal(&json_string("a\"b")), "\"\\\"a\\\\\\\"b\\\"\"");
    }
}
