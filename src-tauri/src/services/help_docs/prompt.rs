//! The "EMAILOPS HELP" block a chat turn carries when the guides matched.
//!
//! Rides in the final user message (never the system prompt: it varies per
//! turn and would invalidate the llama.cpp KV prefix), next to the mailbox
//! Sources block. Pure, so the wording the model sees is pinned by tests.

use super::retrieval::HelpSource;

/// Render the block, or `None` when there is nothing to say.
pub fn render_help_block(sources: &[HelpSource]) -> Option<String> {
    if sources.is_empty() {
        return None;
    }
    let mut s = String::with_capacity(sources.len() * 1400 + 700);
    s.push_str(
        "EMAILOPS HELP — documentation about the EmailOps app itself (its settings, features, setup), \
NOT the user's mail. Use it ONLY if the question is about how EmailOps works or how to do something in \
the app. For a question about the user's emails, contacts, calendar or files, IGNORE this block completely \
and use the mailbox sources and tools as usual. When you do answer from it: stay within what the guide \
says, answer in the user's language, do not call any tool, and end your answer with a link to the section \
you used, written exactly as [section title](help-link) using the help:// link shown next to it.\n",
    );
    for (i, src) in sources.iter().enumerate() {
        s.push_str(&format!(
            "[H{}] {}  ({})\n    {}\n\n",
            i + 1,
            src.title(),
            src.link,
            indent(&src.content)
        ));
    }
    Some(s.trim_end().to_string())
}

fn indent(text: &str) -> String {
    text.lines().collect::<Vec<_>>().join("\n    ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::HelpChunk;

    fn source(lang: &str, heading: &str, content: &str) -> HelpSource {
        HelpSource::from_chunk(
            HelpChunk {
                chunk_id: format!("{lang}/ai-features#1.0"),
                lang: lang.into(),
                page: "ai-features".into(),
                section_index: 1,
                part: 0,
                anchor: "choosing-a-backend".into(),
                page_title: "AI features".into(),
                heading: heading.into(),
                content: content.into(),
                nav_target: Some("settings/ai".into()),
            },
            0.8,
        )
    }

    #[test]
    fn empty_sources_render_nothing() {
        assert_eq!(render_help_block(&[]), None);
    }

    #[test]
    fn block_names_the_scope_and_the_link_contract() {
        let block = render_help_block(&[source("en", "Choosing a backend", "Line one.\nLine two.")]).unwrap();
        assert!(block.starts_with("EMAILOPS HELP"));
        assert!(block.contains("NOT the user's mail"));
        assert!(block.contains("[H1] AI features › Choosing a backend  (help://en/ai-features#choosing-a-backend)"));
        assert!(block.contains("    Line one.\n    Line two."), "{block}");
        assert!(block.contains("[section title](help-link)"));
    }

    #[test]
    fn sources_are_numbered_in_order() {
        let block = render_help_block(&[source("es", "A", "a"), source("es", "B", "b")]).unwrap();
        let a = block.find("[H1] AI features › A").unwrap();
        let b = block.find("[H2] AI features › B").unwrap();
        assert!(a < b);
    }
}
