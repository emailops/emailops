//! Plain text of a Word (.docx) file, for the chat's open-document context.
//!
//! Pure: bytes in, text out, like `pdf_text`. Fetching the bytes and putting
//! the text in a prompt live elsewhere (`services::chat::open_document`).

use std::io::{Cursor, Read};

use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::Event;
use quick_xml::name::{Namespace, ResolveResult};
use quick_xml::NsReader;

/// The part that holds the body of every .docx.
const DOCUMENT_PART: &str = "word/document.xml";
/// The document part is read up to this size: tens of megabytes of XML is a
/// very long book, and a small zip can inflate far past its own size.
const MAX_DOCUMENT_XML_BYTES: u64 = 64 * 1024 * 1024;
const W_NS: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const MC_NS: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";

/// Why a Word file yielded no text the chat can use.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DocxTextError {
    /// Not a .docx the reader can open: not a zip, no document part, broken
    /// XML, or a document part too large to read.
    #[error("the Word file could not be read: {0}")]
    Unreadable(String),
    /// The document opened but has no text — only images or other objects.
    #[error("the Word file has no text")]
    NoText,
}

/// The text of `bytes`, one line per paragraph and per table row (cells
/// joined by ` | `), whitespace tidied as for a PDF.
pub fn extract_docx_text(bytes: &[u8]) -> Result<String, DocxTextError> {
    let xml = read_document_xml(bytes, MAX_DOCUMENT_XML_BYTES)?;
    let text = crate::services::pdf_text::tidy_whitespace(&body_text(&xml)?);
    if text.is_empty() {
        return Err(DocxTextError::NoText);
    }
    Ok(text)
}

/// The document part of the .docx in `bytes`, refused past `limit` bytes.
fn read_document_xml(bytes: &[u8], limit: u64) -> Result<String, DocxTextError> {
    let unreadable = |e: &dyn std::fmt::Display| DocxTextError::Unreadable(e.to_string());
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| unreadable(&e))?;
    let part = archive
        .by_name(DOCUMENT_PART)
        .map_err(|e| unreadable(&format!("{DOCUMENT_PART}: {e}")))?;
    let mut xml = String::new();
    part.take(limit + 1)
        .read_to_string(&mut xml)
        .map_err(|e| unreadable(&format!("{DOCUMENT_PART}: {e}")))?;
    if xml.len() as u64 > limit {
        return Err(unreadable(&format!("{DOCUMENT_PART} is larger than {limit} bytes")));
    }
    Ok(xml)
}

/// What a WordprocessingML element means for the text.
enum Element {
    /// `w:t`: the only element whose content is document text.
    Text,
    Tab,
    Break,
    Paragraph,
    Cell,
    Row,
    /// A subtree that holds no text of its own: paragraph properties (whose
    /// `w:tabs` are tab stops, not tabs) and the old-reader copy of a text
    /// box (`mc:Fallback`), which would repeat it.
    Skipped,
    Other,
}

fn element(ns: &ResolveResult, local: &str) -> Element {
    match ns {
        ResolveResult::Bound(Namespace(ns)) if *ns == W_NS => match local {
            "t" => Element::Text,
            "tab" => Element::Tab,
            "br" | "cr" => Element::Break,
            "p" => Element::Paragraph,
            "tc" => Element::Cell,
            "tr" => Element::Row,
            "pPr" => Element::Skipped,
            _ => Element::Other,
        },
        ResolveResult::Bound(Namespace(ns)) if *ns == MC_NS && local == "Fallback" => Element::Skipped,
        _ => Element::Other,
    }
}

/// The text of a document part, before whitespace is tidied.
fn body_text(xml: &str) -> Result<String, DocxTextError> {
    let unreadable = |e: quick_xml::Error| DocxTextError::Unreadable(e.to_string());
    let mut reader = NsReader::from_str(xml);
    let mut out = String::new();
    let mut in_text = false;
    let mut skipped = 0usize;
    let mut cells = 0usize;
    loop {
        let (ns, event) = reader.read_resolved_event().map_err(unreadable)?;
        match event {
            Event::Eof => break,
            Event::Start(e) => match element(&ns, e.local_name().as_ref()) {
                Element::Skipped => skipped += 1,
                _ if skipped > 0 => {}
                Element::Text => in_text = true,
                Element::Cell => cells += 1,
                _ => {}
            },
            Event::End(e) => match element(&ns, e.local_name().as_ref()) {
                Element::Skipped => skipped = skipped.saturating_sub(1),
                _ if skipped > 0 => {}
                Element::Text => in_text = false,
                Element::Paragraph => end_paragraph(&mut out, cells),
                Element::Cell => {
                    cells = cells.saturating_sub(1);
                    out.truncate(out.trim_end().len());
                    out.push_str(" | ");
                }
                Element::Row => {
                    if let Some(row) = out.strip_suffix(" | ") {
                        out.truncate(row.len());
                    }
                    out.push(if cells > 0 { ' ' } else { '\n' });
                }
                _ => {}
            },
            Event::Empty(e) if skipped == 0 => match element(&ns, e.local_name().as_ref()) {
                Element::Tab => out.push('\t'),
                Element::Break => out.push(if cells > 0 { ' ' } else { '\n' }),
                Element::Paragraph => end_paragraph(&mut out, cells),
                _ => {}
            },
            Event::Text(t) if in_text && skipped == 0 => out.push_str(&t.xml10_content()),
            Event::GeneralRef(r) if in_text && skipped == 0 => {
                if let Some(c) = r.resolve_char_ref().map_err(unreadable)? {
                    out.push(c);
                } else if let Some(entity) = resolve_predefined_entity(&r.xml10_content()) {
                    out.push_str(entity);
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

/// A paragraph ends a line, except inside a table cell, whose paragraphs
/// stay on the row's line.
fn end_paragraph(out: &mut String, cells: usize) {
    out.push(if cells > 0 { ' ' } else { '\n' });
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Write;

    const W_NS: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
    const MC_NS: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";

    /// A zip holding `parts`, each `(name, content)`.
    fn zip_of(parts: &[(&str, &str)]) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, content) in parts {
            zip.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(content.as_bytes()).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    /// A .docx whose `<w:body>` holds `body`.
    pub(crate) fn docx(body: &str) -> Vec<u8> {
        let document = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><w:document xmlns:w="{W_NS}" xmlns:mc="{MC_NS}"><w:body>{body}</w:body></w:document>"#
        );
        zip_of(&[("[Content_Types].xml", "<Types/>"), ("word/document.xml", &document)])
    }

    pub(crate) fn p(text: &str) -> String {
        format!(r#"<w:p><w:r><w:t xml:space="preserve">{text}</w:t></w:r></w:p>"#)
    }

    #[test]
    fn paragraphs_come_out_one_per_line() {
        let text = extract_docx_text(&docx(&(p("Kickoff minutes") + &p("Go-live: 14 April 2026.")))).unwrap();
        assert_eq!(text, "Kickoff minutes\nGo-live: 14 April 2026.");
    }

    #[test]
    fn runs_of_one_paragraph_join_without_a_break() {
        let body = r#"<w:p><w:r><w:t xml:space="preserve">Budget: </w:t></w:r><w:r><w:rPr><w:b/></w:rPr><w:t>EUR 8,400</w:t></w:r></w:p>"#;
        assert_eq!(extract_docx_text(&docx(body)).unwrap(), "Budget: EUR 8,400");
    }

    #[test]
    fn a_table_row_stays_on_one_line_with_its_cells_separated() {
        let row = |cells: &[&str]| {
            let cells: String = cells.iter().map(|c| format!("<w:tc>{}</w:tc>", p(c))).collect();
            format!("<w:tr>{cells}</w:tr>")
        };
        let body = format!(
            "{}<w:tbl>{}{}</w:tbl>{}",
            p("Action items"),
            row(&["Item", "Owner", "Due"]),
            row(&["Prototype", "Ulises", "17 March 2026"]),
            p("Next meeting on Friday.")
        );
        assert_eq!(
            extract_docx_text(&docx(&body)).unwrap(),
            "Action items\nItem | Owner | Due\nPrototype | Ulises | 17 March 2026\nNext meeting on Friday."
        );
    }

    #[test]
    fn a_cell_with_two_paragraphs_stays_in_its_row() {
        let body = format!(
            "<w:tbl><w:tr><w:tc>{}{}</w:tc><w:tc>{}</w:tc></w:tr></w:tbl>",
            p("Line one"),
            p("line two"),
            p("B")
        );
        assert_eq!(extract_docx_text(&docx(&body)).unwrap(), "Line one line two | B");
    }

    #[test]
    fn entities_and_character_references_are_decoded() {
        let text = extract_docx_text(&docx(&p("R&amp;D &lt;draft&gt; caf&#233; &#x2014; ok"))).unwrap();
        assert_eq!(text, "R&D <draft> café — ok");
    }

    #[test]
    fn tabs_and_line_breaks_keep_their_meaning() {
        let body = r#"<w:p><w:r><w:t>Name</w:t><w:tab/><w:t>Ulises</w:t><w:br/><w:t>Second line</w:t></w:r></w:p>"#;
        assert_eq!(extract_docx_text(&docx(body)).unwrap(), "Name\tUlises\nSecond line");
    }

    #[test]
    fn deleted_text_and_field_codes_are_left_out() {
        // A tracked deletion is not what the document says; a field code
        // (PAGE, a hyperlink target) is markup, not prose.
        let body = r#"<w:p><w:r><w:t xml:space="preserve">Fee: </w:t></w:r><w:del><w:r><w:delText>EUR 9,000</w:delText></w:r></w:del><w:r><w:instrText> PAGE </w:instrText></w:r><w:r><w:t>EUR 8,400</w:t></w:r></w:p>"#;
        assert_eq!(extract_docx_text(&docx(body)).unwrap(), "Fee: EUR 8,400");
    }

    #[test]
    fn a_text_box_is_read_once_not_again_from_its_fallback() {
        // Word stores a text box twice: the modern shape and a VML fallback
        // for old readers, each with the same paragraphs.
        let box_body = format!("<w:txbxContent>{}</w:txbxContent>", p("Boxed note"));
        let body = format!(
            "<w:p><w:r><mc:AlternateContent><mc:Choice Requires=\"wps\">{box_body}</mc:Choice><mc:Fallback>{box_body}</mc:Fallback></mc:AlternateContent></w:r></w:p>{}",
            p("After the box")
        );
        let text = extract_docx_text(&docx(&body)).unwrap();
        assert_eq!(text.matches("Boxed note").count(), 1, "{text:?}");
        assert!(text.ends_with("After the box"), "{text:?}");
    }

    #[test]
    fn blank_paragraphs_fold_into_one_blank_line() {
        let body = format!("{}<w:p/><w:p/><w:p/>{}", p("Top"), p("Bottom"));
        assert_eq!(extract_docx_text(&docx(&body)).unwrap(), "Top\n\nBottom");
    }

    #[test]
    fn a_document_without_text_has_no_text() {
        assert_eq!(
            extract_docx_text(&docx("<w:p/><w:p><w:r><w:t>  </w:t></w:r></w:p>")),
            Err(DocxTextError::NoText)
        );
    }

    #[test]
    fn bytes_that_are_not_a_zip_are_unreadable() {
        assert!(matches!(
            extract_docx_text(b"%PDF-1.4 not a docx"),
            Err(DocxTextError::Unreadable(_))
        ));
    }

    #[test]
    fn a_zip_without_a_document_part_is_unreadable() {
        let bytes = zip_of(&[("[Content_Types].xml", "<Types/>")]);
        assert!(matches!(extract_docx_text(&bytes), Err(DocxTextError::Unreadable(_))));
    }

    #[test]
    fn broken_xml_is_unreadable() {
        let bytes = zip_of(&[("word/document.xml", "<w:document><w:body><w:p></w:body>")]);
        assert!(matches!(extract_docx_text(&bytes), Err(DocxTextError::Unreadable(_))));
    }

    #[test]
    fn a_document_part_over_the_size_limit_is_unreadable() {
        // A small zip can inflate to gigabytes; the reader stops at the limit.
        let bytes = docx(&p(&"x".repeat(2_000)));
        assert!(matches!(
            read_document_xml(&bytes, 1_000),
            Err(DocxTextError::Unreadable(_))
        ));
        assert!(read_document_xml(&bytes, 10_000).is_ok());
    }
}
