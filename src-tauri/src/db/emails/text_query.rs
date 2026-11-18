//! The free text of a list search, read as a boolean query: words, quoted
//! phrases, `AND` / `OR` / `NOT` (uppercase), `-word` and parentheses. `NOT`
//! binds tighter than `AND`, `AND` tighter than `OR`, and words side by side
//! are ANDed. Pure: it only builds the FTS5 query, or the condition, to run.

/// A parsed text query.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum TextExpr {
    /// A word; it matches as a prefix, like the rest of the search.
    Term(String),
    /// A quoted phrase; it matches exactly.
    Phrase(String),
    And(Vec<TextExpr>),
    Or(Vec<TextExpr>),
    Not(Box<TextExpr>),
}

/// How the list applies a text query.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum TextFilter {
    /// One FTS5 MATCH expression that finds every matching email.
    Match(String),
    /// A query FTS5 cannot run as one MATCH: a negation with nothing to take it
    /// away from (`-draft`, `a OR -b`). [`TextExpr::sql`] turns it into a
    /// condition with one MATCH per part.
    Condition(TextExpr),
}

/// The filter for `text`, or `None` when it holds nothing that can match (only
/// symbols, or only operators).
pub(crate) fn text_filter(text: &str) -> Option<TextFilter> {
    let tokens = lex(text);
    let mut parser = Parser { tokens, pos: 0 };
    // A stray `)` ends a group early; what follows it still counts.
    let mut parts = Vec::new();
    loop {
        parts.extend(parser.or());
        if parser.pos >= parser.tokens.len() {
            break;
        }
        parser.pos += 1;
    }
    let expr = group(parts, true)?;
    Some(match expr.fts() {
        Some(query) => TextFilter::Match(query),
        None => TextFilter::Condition(expr),
    })
}

impl TextExpr {
    /// The FTS5 query for this expression, when FTS5 can run it as one MATCH:
    /// its `NOT` is binary, so a negation needs something to take mail from.
    fn fts(&self) -> Option<String> {
        match self {
            TextExpr::Term(word) => Some(format!("\"{word}\"*")),
            TextExpr::Phrase(phrase) => Some(format!("\"{phrase}\"")),
            TextExpr::Or(items) => {
                let parts = items.iter().map(TextExpr::fts).collect::<Option<Vec<_>>>()?;
                Some(format!("({})", parts.join(" OR ")))
            }
            TextExpr::And(items) => {
                let (negated, kept): (Vec<&TextExpr>, Vec<&TextExpr>) =
                    items.iter().partition(|item| matches!(item, TextExpr::Not(_)));
                let kept = kept.into_iter().map(TextExpr::fts).collect::<Option<Vec<_>>>()?;
                let mut query = match kept.len() {
                    0 => return None,
                    1 => kept.join(""),
                    _ => format!("({})", kept.join(" AND ")),
                };
                for item in negated {
                    let TextExpr::Not(inner) = item else { continue };
                    query = format!("({query} NOT {})", inner.fts()?);
                }
                Some(query)
            }
            TextExpr::Not(_) => None,
        }
    }

    /// A condition on `{alias}.id`. Each MATCH value is pushed to `binds` and
    /// numbered from `idx`.
    pub(crate) fn sql(&self, alias: &str, idx: &mut usize, binds: &mut Vec<String>) -> String {
        if let Some(query) = self.fts() {
            let sql = format!("{alias}.id IN (SELECT email_id FROM emails_fts WHERE emails_fts MATCH ?{idx})");
            binds.push(query);
            *idx += 1;
            return sql;
        }
        let (items, joiner) = match self {
            TextExpr::Not(inner) => return format!("NOT ({})", inner.sql(alias, idx, binds)),
            TextExpr::And(items) => (items, " AND "),
            TextExpr::Or(items) => (items, " OR "),
            // Words and phrases always have an FTS form, handled above.
            TextExpr::Term(_) | TextExpr::Phrase(_) => return String::new(),
        };
        let parts: Vec<String> = items.iter().map(|item| item.sql(alias, idx, binds)).collect();
        format!("({})", parts.join(joiner))
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Open,
    Close,
    And,
    Or,
    Not,
    Word(String),
    Phrase(String),
}

/// Split the text into tokens. A `-` starts a negation only at the start of a
/// token; inside a word (`e-mail`) it is part of the word.
fn lex(text: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(&c) = chars.peek() {
        match c {
            c if c.is_whitespace() => {
                chars.next();
            }
            '(' | ')' => {
                chars.next();
                tokens.push(if c == '(' { Token::Open } else { Token::Close });
            }
            '"' => {
                chars.next();
                let phrase: String = chars.by_ref().take_while(|&c| c != '"').collect();
                let phrase = phrase.split_whitespace().collect::<Vec<_>>().join(" ");
                if phrase.chars().any(char::is_alphanumeric) {
                    tokens.push(Token::Phrase(phrase));
                }
            }
            '-' => {
                chars.next();
                if chars.peek().is_some_and(|&next| !next.is_whitespace() && next != ')') {
                    tokens.push(Token::Not);
                }
            }
            _ => {
                let mut word = String::new();
                while let Some(&c) = chars.peek() {
                    if c.is_whitespace() || matches!(c, '(' | ')' | '"') {
                        break;
                    }
                    word.push(c);
                    chars.next();
                }
                match word.as_str() {
                    "AND" => tokens.push(Token::And),
                    "OR" => tokens.push(Token::Or),
                    "NOT" => tokens.push(Token::Not),
                    _ => {
                        // The characters the search has always kept in a word.
                        let word: String = word
                            .chars()
                            .filter(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '@' | '.'))
                            .collect();
                        if word.chars().any(char::is_alphanumeric) {
                            tokens.push(Token::Word(word));
                        }
                    }
                }
            }
        }
    }
    tokens
}

/// `items` ANDed (`all`) or ORed, with nested groups of the same kind
/// flattened; `None` when nothing is left.
fn group(items: Vec<TextExpr>, all: bool) -> Option<TextExpr> {
    let mut flat = Vec::new();
    for item in items {
        match item {
            TextExpr::And(inner) if all => flat.extend(inner),
            TextExpr::Or(inner) if !all => flat.extend(inner),
            item => flat.push(item),
        }
    }
    match flat.len() {
        0 => None,
        1 => flat.pop(),
        _ if all => Some(TextExpr::And(flat)),
        _ => Some(TextExpr::Or(flat)),
    }
}

/// Recursive descent, lenient: an operator with nothing to apply to is
/// dropped, and an open bracket closes at the end of the text.
struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn or(&mut self) -> Option<TextExpr> {
        let mut items: Vec<TextExpr> = self.and().into_iter().collect();
        while self.peek() == Some(&Token::Or) {
            self.pos += 1;
            items.extend(self.and());
        }
        group(items, false)
    }

    fn and(&mut self) -> Option<TextExpr> {
        let mut items = Vec::new();
        loop {
            match self.peek() {
                None | Some(Token::Close) | Some(Token::Or) => break,
                Some(Token::And) => self.pos += 1,
                Some(_) => items.extend(self.unary()),
            }
        }
        group(items, true)
    }

    fn unary(&mut self) -> Option<TextExpr> {
        if self.peek() == Some(&Token::Not) {
            self.pos += 1;
            return self.unary().map(|inner| TextExpr::Not(Box::new(inner)));
        }
        let token = self.tokens.get(self.pos).cloned();
        self.pos += 1;
        match token? {
            Token::Word(word) => Some(TextExpr::Term(word)),
            Token::Phrase(phrase) => Some(TextExpr::Phrase(phrase)),
            Token::Open => {
                let inner = self.or();
                if self.peek() == Some(&Token::Close) {
                    self.pos += 1;
                }
                inner
            }
            // `NOT` before `)`, `OR` or the end: nothing to negate.
            Token::Close | Token::Or | Token::And | Token::Not => {
                self.pos -= 1;
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fts(text: &str) -> String {
        match text_filter(text) {
            Some(TextFilter::Match(query)) => query,
            other => panic!("{text:?} should be one MATCH, got {other:?}"),
        }
    }

    fn condition(text: &str) -> (String, Vec<String>) {
        let Some(TextFilter::Condition(expr)) = text_filter(text) else {
            panic!("{text:?} should be a condition");
        };
        let (mut idx, mut binds) = (3, Vec::new());
        let sql = expr.sql("e2", &mut idx, &mut binds);
        assert_eq!(idx, 3 + binds.len(), "one placeholder per MATCH value");
        (sql, binds)
    }

    #[test]
    fn words_side_by_side_must_all_match_as_prefixes() {
        assert_eq!(fts("invoice march"), r#"("invoice"* AND "march"*)"#);
        assert_eq!(fts("invoice"), r#""invoice"*"#);
    }

    #[test]
    fn a_quoted_phrase_matches_exactly() {
        assert_eq!(
            fts(r#""quarterly report" budget"#),
            r#"("quarterly report" AND "budget"*)"#
        );
    }

    #[test]
    fn or_binds_looser_than_and() {
        assert_eq!(
            fts("invoice OR receipt march"),
            r#"("invoice"* OR ("receipt"* AND "march"*))"#
        );
        assert_eq!(
            fts("(invoice OR receipt) march"),
            r#"(("invoice"* OR "receipt"*) AND "march"*)"#
        );
    }

    #[test]
    fn explicit_and_is_the_same_as_words_side_by_side() {
        assert_eq!(fts("invoice AND march"), fts("invoice march"));
    }

    #[test]
    fn not_and_a_leading_minus_take_mail_away() {
        assert_eq!(fts("invoice -draft"), r#"("invoice"* NOT "draft"*)"#);
        assert_eq!(fts("invoice NOT draft"), fts("invoice -draft"));
        assert_eq!(
            fts(r#"invoice -"old quote" -(draft OR test)"#),
            r#"(("invoice"* NOT "old quote") NOT ("draft"* OR "test"*))"#
        );
    }

    /// Lowercase `or` / `and` / `not` are ordinary words: "terms and
    /// conditions" is a search for three words.
    #[test]
    fn only_uppercase_operators_are_operators() {
        assert_eq!(fts("cats or dogs"), r#"("cats"* AND "or"* AND "dogs"*)"#);
    }

    #[test]
    fn a_hyphen_inside_a_word_is_part_of_it() {
        assert_eq!(fts("e-mail"), r#""e-mail"*"#);
    }

    #[test]
    fn a_negation_with_nothing_to_take_away_from_is_a_condition() {
        let (sql, binds) = condition("-draft");
        assert_eq!(
            sql,
            "NOT (e2.id IN (SELECT email_id FROM emails_fts WHERE emails_fts MATCH ?3))"
        );
        assert_eq!(binds, vec![r#""draft"*"#]);

        let (sql, binds) = condition("invoice OR -draft");
        assert_eq!(
            sql,
            "(e2.id IN (SELECT email_id FROM emails_fts WHERE emails_fts MATCH ?3) \
             OR NOT (e2.id IN (SELECT email_id FROM emails_fts WHERE emails_fts MATCH ?4)))"
        );
        assert_eq!(binds, vec![r#""invoice"*"#, r#""draft"*"#]);
    }

    /// Whatever is typed, the query runs: a stray operator or bracket is
    /// dropped, an open bracket closes at the end.
    #[test]
    fn malformed_queries_are_read_leniently() {
        assert_eq!(fts("OR invoice"), r#""invoice"*"#);
        assert_eq!(fts("invoice OR"), r#""invoice"*"#);
        assert_eq!(fts("invoice OR OR receipt"), r#"("invoice"* OR "receipt"*)"#);
        assert_eq!(fts("(invoice OR receipt"), r#"("invoice"* OR "receipt"*)"#);
        assert_eq!(fts("invoice) march"), r#"("invoice"* AND "march"*)"#);
        assert_eq!(fts(r#""quarterly report"#), r#""quarterly report""#);
        assert_eq!(fts("invoice NOT"), r#""invoice"*"#);
    }

    /// Symbols cannot match; the rest of the query still applies, as before.
    #[test]
    fn symbols_are_dropped_from_words() {
        assert_eq!(fts("invoice!! #42"), r#"("invoice"* AND "42"*)"#);
        assert_eq!(fts(r#"say "hi" there"#), r#"("say"* AND "hi" AND "there"*)"#);
    }

    #[test]
    fn a_query_of_only_symbols_or_operators_matches_nothing() {
        for text in ["!!!", "OR", "( )", r#""" "#, "NOT", "- -"] {
            assert_eq!(text_filter(text), None, "{text:?}");
        }
    }
}
