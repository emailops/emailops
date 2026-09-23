//! Reference tables in the published docs, generated from the code they describe.
//!
//! `docs/site/<lang>/*.md` brackets each generated table between
//! `<!-- generated:<name> -->` and `<!-- /generated:<name> -->`. A test per
//! table renders it from the real constants and compares: a model resized in
//! the catalog, or an exit code added, fails the test until the docs are
//! regenerated — `make docs-gen` (which sets `UPDATE_DOCS=1`) rewrites every
//! region in all four languages, and the diff is what gets reviewed.
//!
//! Only the rows are generated. The prose around a table — and the translated
//! headers and labels the generators take as input — stays hand-written.

use std::path::PathBuf;

pub const LANGS: [&str; 4] = ["en", "es", "fr", "de"];

fn page_path(lang: &str, page: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../docs/site")
        .join(lang)
        .join(page)
}

/// The current contents of the `name` region, with line endings normalised to
/// `\n` (Windows checks the docs out with CRLF; the generators render LF).
pub fn region(text: &str, name: &str) -> Result<String, String> {
    let text = text.replace("\r\n", "\n");
    let open = format!("<!-- generated:{name} -->\n");
    let close = format!("<!-- /generated:{name} -->");
    let start = text.find(&open).ok_or_else(|| format!("no `{}` marker", open.trim()))? + open.len();
    let end = text[start..]
        .find(&close)
        .ok_or_else(|| format!("`generated:{name}` is opened but never closed with `{close}`"))?
        + start;
    Ok(text[start..end].to_string())
}

/// Make the `name` region of `lang/page` read exactly `body`.
///
/// → `Ok(())` when it already does, or when `UPDATE_DOCS` is set and it has
/// just been rewritten; `Err` with what to do otherwise.
pub fn ensure(lang: &str, page: &str, name: &str, body: &str) -> Result<(), String> {
    let path = page_path(lang, page);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let current = region(&text, name).map_err(|e| format!("{lang}/{page}: {e}"))?;
    let want = format!("{}\n", body.trim_end());
    if current == want {
        return Ok(());
    }
    if std::env::var_os("UPDATE_DOCS").is_some() {
        // Rewrite against the file as it sits on disk, line endings and all.
        let open = format!("<!-- generated:{name} -->\n");
        let close = format!("<!-- /generated:{name} -->");
        let start = text
            .find(&open)
            .ok_or_else(|| format!("{lang}/{page}: no `{}` marker", open.trim()))?
            + open.len();
        let end = text[start..]
            .find(&close)
            .ok_or_else(|| format!("{lang}/{page}: `{name}` never closed"))?
            + start;
        let updated = format!("{}{want}{}", &text[..start], &text[end..]);
        return std::fs::write(&path, updated).map_err(|e| format!("cannot write {}: {e}", path.display()));
    }
    Err(format!(
        "{lang}/{page}: the `{name}` table is out of date with the code. Run `make docs-gen` and \
         review the diff. Expected:\n{want}"
    ))
}

/// Run `ensure` for every language and fail once, listing all of them.
pub fn ensure_all(page: &str, name: &str, render: impl Fn(&str) -> String) {
    let errors: Vec<String> = LANGS
        .iter()
        .filter_map(|lang| ensure(lang, page, name, &render(lang)).err())
        .collect();
    assert!(errors.is_empty(), "{}", errors.join("\n\n"));
}

/// A number as each language writes it: decimal comma outside English.
pub fn localized_decimal(lang: &str, s: &str) -> String {
    if lang == "en" {
        s.to_string()
    } else {
        s.replace('.', ",")
    }
}

/// Unit spelling per language (French writes Go / Mo).
pub fn unit(lang: &str, en: &str) -> String {
    match (lang, en) {
        ("fr", "GB") => "Go".into(),
        ("fr", "MB") => "Mo".into(),
        _ => en.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_region_matches_whatever_line_endings_the_checkout_has() {
        // Windows checks out the docs with CRLF; the generator renders LF.
        // Comparing them byte for byte failed the whole suite on Windows only.
        let crlf = "intro\r\n<!-- generated:t -->\r\n| a | b |\r\n<!-- /generated:t -->\r\nrest\r\n";
        assert_eq!(region(crlf, "t"), Ok("| a | b |\n".to_string()));
    }

    #[test]
    fn a_missing_region_says_which_marker_is_absent() {
        assert!(region("no markers here", "t").unwrap_err().contains("generated:t"));
    }

    #[test]
    fn numbers_follow_the_reader_s_language() {
        assert_eq!(localized_decimal("en", "3.0"), "3.0");
        assert_eq!(localized_decimal("es", "3.0"), "3,0");
        assert_eq!(unit("fr", "GB"), "Go");
        assert_eq!(unit("de", "GB"), "GB");
    }
}
