// Deterministic structural rubric for shortcut variants.
//
// These checks run unconditionally and decide CI pass/fail for a variant.
// The LLM judge (judge.rs) is layered on top and produces the soft scores
// (structure/faithfulness/usefulness/tone).

use regex::Regex;

use crate::evals::shortcuts::case_loader::StructuralRubric;

#[derive(Debug, Clone)]
pub struct RubricCheck {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Default)]
pub struct RubricReport {
    pub checks: Vec<RubricCheck>,
}

impl RubricReport {
    pub fn all_passed(&self) -> bool {
        self.checks.iter().all(|c| c.passed)
    }
    pub fn passed_count(&self) -> usize {
        self.checks.iter().filter(|c| c.passed).count()
    }
    pub fn total(&self) -> usize {
        self.checks.len()
    }
}

pub fn evaluate(rubric: &StructuralRubric, answer: &str) -> RubricReport {
    let mut checks = Vec::new();

    // 1. Non-empty.
    let trimmed = answer.trim();
    checks.push(RubricCheck {
        name: "answer_nonempty".into(),
        passed: !trimmed.is_empty(),
        detail: if trimmed.is_empty() {
            "assistant produced no text".into()
        } else {
            format!("{} chars", trimmed.chars().count())
        },
    });

    // Parse the table once (if any) and share across downstream checks.
    let table = find_first_markdown_table(answer);

    // 2. Must contain a table.
    if rubric.must_contain_table {
        checks.push(RubricCheck {
            name: "contains_table".into(),
            passed: table.is_some(),
            detail: match &table {
                Some(t) => format!("table with {} row(s)", t.rows.len()),
                None => "no markdown table found".into(),
            },
        });
    }

    // 3. Required columns (case-insensitive substring match against header cells).
    if !rubric.required_columns.is_empty() {
        let (passed, detail) = match &table {
            None => (false, "no table to inspect".to_string()),
            Some(t) => {
                let header_lc: Vec<String> = t.header.iter().map(|c| c.to_lowercase()).collect();
                let missing: Vec<&String> = rubric
                    .required_columns
                    .iter()
                    .filter(|needle| {
                        let n = needle.to_lowercase();
                        !header_lc.iter().any(|h| h.contains(&n))
                    })
                    .collect();
                if missing.is_empty() {
                    (true, format!("header: {}", t.header.join(" | ")))
                } else {
                    let missing_joined = missing.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ");
                    (false, format!("missing: {}", missing_joined))
                }
            }
        };
        checks.push(RubricCheck {
            name: "required_columns".into(),
            passed,
            detail,
        });
    }

    // 4. Minimum row count.
    if rubric.min_rows > 0 {
        let (passed, detail) = match &table {
            None => (false, "no table to inspect".to_string()),
            Some(t) => (
                t.rows.len() >= rubric.min_rows,
                format!("found {} rows, need ≥ {}", t.rows.len(), rubric.min_rows),
            ),
        };
        checks.push(RubricCheck {
            name: "min_rows".into(),
            passed,
            detail,
        });
    }

    // 5. Each row must contain an inline `[n]` citation.
    if rubric.require_row_citations {
        let cite_re = Regex::new(r"\[\d+\]").expect("static regex");
        let (passed, detail) = match &table {
            None => (false, "no table to inspect".to_string()),
            Some(t) => {
                let uncited: Vec<usize> = t
                    .rows
                    .iter()
                    .enumerate()
                    .filter(|(_, row)| !row.iter().any(|cell| cite_re.is_match(cell)))
                    .map(|(i, _)| i + 1)
                    .collect();
                if uncited.is_empty() {
                    (true, "all rows cited".into())
                } else {
                    (
                        false,
                        format!(
                            "uncited rows: {}",
                            uncited.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(", ")
                        ),
                    )
                }
            }
        };
        checks.push(RubricCheck {
            name: "row_citations".into(),
            passed,
            detail,
        });
    }

    // 6. Must end with a prose paragraph (not a table row).
    if rubric.must_end_with_summary_paragraph {
        let tail = answer.trim_end();
        // Strip trailing empty lines, then check the LAST non-empty line.
        let last = tail.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("");
        let last_is_row = last.trim_start().starts_with('|');
        let last_is_sep = last.trim_start().starts_with("|-") || last.trim_start().starts_with("| -");
        let passed = !last_is_row && !last_is_sep && !last.trim().is_empty();
        checks.push(RubricCheck {
            name: "summary_paragraph".into(),
            passed,
            detail: if passed {
                format!("last line: \"{}\"", truncate(last, 80))
            } else {
                format!("last line looked like table row: \"{}\"", truncate(last, 80))
            },
        });
    }

    // 7. Language check (Spanish vs English stop-word ratio).
    //
    // Rationale: we saw gemma4:e2b occasionally emit English verbs inside an
    // otherwise-Spanish reply (e.g. "The sender is asking for..."). A simple
    // stop-word ratio catches wholesale language drift without false-flagging
    // proper nouns or loan words.
    let lang_check = language_heuristic(&rubric.language, answer);
    checks.push(lang_check);

    RubricReport { checks }
}

/// Stop-word based language heuristic. Returns pass when the expected
/// language's stop-word count is ≥ 2× the next-highest other language's count
/// AND ≥ 3 absolute. Short answers with too little signal are treated as
/// pass — they'd fail other rubric checks anyway.
///
/// Supports the four UI languages: `"en"`, `"es"`, `"fr"`, `"de"`. Unknown
/// language codes skip the check (treated as pass).
fn language_heuristic(expected: &str, content: &str) -> RubricCheck {
    // Tokenize on non-alphanumeric, lowercase.
    let tokens: Vec<String> = content
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_lowercase())
        .collect();

    // Stop-word lists per language. Each list is a mix of grammatical
    // function words and domain-specific tokens (email/inbox vocabulary) that
    // a triage answer is overwhelmingly likely to use in that language.
    const ES_STOPS: &[&str] = &[
        "de",
        "la",
        "el",
        "los",
        "las",
        "un",
        "una",
        "y",
        "en",
        "que",
        "por",
        "con",
        "para",
        "es",
        "ha",
        "han",
        "no",
        "se",
        "lo",
        "del",
        "al",
        "como",
        "pero",
        "este",
        "esta",
        "tiene",
        "hoy",
        "ayer",
        "resumen",
        "correos",
        "emails",
        "urgente",
        "pendiente",
        "prioridad",
        "remitente",
        "asunto",
    ];
    const EN_STOPS: &[&str] = &[
        "the",
        "a",
        "an",
        "and",
        "or",
        "is",
        "are",
        "was",
        "were",
        "of",
        "to",
        "in",
        "on",
        "for",
        "with",
        "from",
        "this",
        "that",
        "has",
        "have",
        "not",
        "but",
        "as",
        "by",
        "you",
        "your",
        "today",
        "yesterday",
        "sender",
        "subject",
        "urgent",
        "summary",
    ];
    const FR_STOPS: &[&str] = &[
        "le",
        "la",
        "les",
        "un",
        "une",
        "des",
        "de",
        "du",
        "et",
        "ou",
        "est",
        "sont",
        "était",
        "étaient",
        "à",
        "au",
        "aux",
        "en",
        "dans",
        "sur",
        "pour",
        "avec",
        "par",
        "ce",
        "cette",
        "ces",
        "qui",
        "que",
        "quoi",
        "pas",
        "ne",
        "vous",
        "votre",
        "aujourd",
        "hier",
        "expéditeur",
        "objet",
        "urgent",
        "résumé",
        "courriels",
        "courriel",
    ];
    const DE_STOPS: &[&str] = &[
        "der",
        "die",
        "das",
        "den",
        "dem",
        "des",
        "ein",
        "eine",
        "einen",
        "und",
        "oder",
        "ist",
        "sind",
        "war",
        "waren",
        "in",
        "im",
        "an",
        "am",
        "auf",
        "für",
        "mit",
        "von",
        "zu",
        "zum",
        "zur",
        "nicht",
        "kein",
        "keine",
        "sie",
        "ihr",
        "heute",
        "gestern",
        "absender",
        "betreff",
        "dringend",
        "zusammenfassung",
        "e",
        "mail",
        "mails",
    ];

    let es: usize = tokens.iter().filter(|t| ES_STOPS.contains(&t.as_str())).count();
    let en: usize = tokens.iter().filter(|t| EN_STOPS.contains(&t.as_str())).count();
    let fr: usize = tokens.iter().filter(|t| FR_STOPS.contains(&t.as_str())).count();
    let de: usize = tokens.iter().filter(|t| DE_STOPS.contains(&t.as_str())).count();

    let total = es + en + fr + de;
    let counts_detail = format!("es={es}, en={en}, fr={fr}, de={de}");

    let (target, others_max): (usize, usize) = match expected {
        "es" => (es, [en, fr, de].into_iter().max().unwrap_or(0)),
        "en" => (en, [es, fr, de].into_iter().max().unwrap_or(0)),
        "fr" => (fr, [es, en, de].into_iter().max().unwrap_or(0)),
        "de" => (de, [es, en, fr].into_iter().max().unwrap_or(0)),
        _ => {
            return RubricCheck {
                name: format!("language_{}", expected),
                passed: true,
                detail: format!("unknown expected lang '{expected}', skipping ({counts_detail})"),
            };
        }
    };

    let (passed, detail) = if total < 3 {
        (true, format!("too short to judge ({counts_detail})"))
    } else {
        let p = target >= 3 && target >= others_max.saturating_mul(2);
        (p, counts_detail)
    };

    RubricCheck {
        name: format!("language_{}", expected),
        passed,
        detail,
    }
}

#[derive(Debug, Clone)]
pub struct ParsedTable {
    pub header: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

/// Very small GFM-ish table parser. Finds the FIRST block of lines starting
/// with `|`, requires a separator line (`|---|---|...`) immediately after
/// the header, and returns header + data rows. Good enough for eval rubrics
/// — we don't need to handle escaped pipes or inline HTML.
pub fn find_first_markdown_table(s: &str) -> Option<ParsedTable> {
    let lines: Vec<&str> = s.lines().collect();
    let mut i = 0;
    while i + 1 < lines.len() {
        let head = lines[i].trim();
        let sep = lines[i + 1].trim();
        if head.starts_with('|') && is_separator_line(sep) {
            let header = split_pipe_row(head);
            let mut rows = Vec::new();
            let mut j = i + 2;
            while j < lines.len() {
                let row = lines[j].trim();
                if !row.starts_with('|') {
                    break;
                }
                rows.push(split_pipe_row(row));
                j += 1;
            }
            return Some(ParsedTable { header, rows });
        }
        i += 1;
    }
    None
}

fn is_separator_line(s: &str) -> bool {
    if !s.starts_with('|') {
        return false;
    }
    s.trim_matches('|')
        .split('|')
        .map(|c| c.trim())
        .all(|c| !c.is_empty() && c.chars().all(|ch| ch == '-' || ch == ':' || ch == ' '))
}

fn split_pipe_row(s: &str) -> Vec<String> {
    s.trim_matches('|').split('|').map(|c| c.trim().to_string()).collect()
}

fn truncate(s: &str, n: usize) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i >= n {
            out.push('…');
            break;
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(expected: &str, content: &str) -> RubricCheck {
        language_heuristic(expected, content)
    }

    #[test]
    fn heuristic_passes_when_target_language_dominates() {
        let es =
            "Hoy tienes 3 correos urgentes de remitentes diferentes. El asunto principal es la propuesta de Marta.";
        assert!(run("es", es).passed);

        let en = "You have 3 urgent emails today from different senders. The main subject is the proposal from Marta.";
        assert!(run("en", en).passed);

        let fr = "Aujourd'hui vous avez 3 courriels urgents de différents expéditeurs. Le résumé principal est la proposition de Marta.";
        assert!(run("fr", fr).passed);

        let de = "Heute haben Sie 3 dringende E-Mails von verschiedenen Absendern. Die wichtigste Zusammenfassung ist der Vorschlag von Marta.";
        assert!(run("de", de).passed);
    }

    #[test]
    fn heuristic_fails_when_answer_drifts_to_another_language() {
        // Expected Spanish but answer is entirely English.
        let drift = "The sender is asking for an urgent reply about the proposal. \
                     Today you have three emails to triage from different senders.";
        assert!(!run("es", drift).passed);
    }

    #[test]
    fn heuristic_passes_short_inputs_to_avoid_false_positives() {
        // Below the 3-stopword threshold — treated as pass.
        let short = "OK.";
        assert!(run("fr", short).passed);
    }

    #[test]
    fn heuristic_skips_unknown_language() {
        let r = run("pt", "Hoje você tem três emails.");
        assert!(r.passed, "unknown language should pass-through (skip)");
        assert!(r.detail.contains("unknown expected lang"));
    }
}
