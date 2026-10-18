//! The internal links a document already has — the Edit tab's Cross
//! Reference.
//!
//! **A cross-reference here is a link that stays inside the file**: a link
//! annotation whose destination is another page of the same document, rather
//! than a web address, a file, or a script. This reports the ones the engine
//! can already follow — `Session::internal_links`, the same read that makes a
//! link clickable on the page — as one line per link: where it can be clicked,
//! and the page it goes to.
//!
//! **Read-only, and it stays that way.** Creating an internal link needs a
//! destination-writing primitive PDFium's public API here does not have: it can
//! set a web address on a link (`FPDFAnnot_SetURI`) but not a page destination,
//! so `weblinks` links words to an address and this only reports. The answer
//! says so rather than leaving "Cross Reference" looking like an unbuilt
//! creation tool.

use pdf_core::document::InternalLink;

use crate::Session;

/// Report every internal link in the document, page by page.
pub fn report(session: &Session) -> pdf_core::Result<Vec<String>> {
    let pages = session.page_count()?;
    let mut found = Vec::new();
    for page in 0..pages {
        for link in session.internal_links(page)? {
            found.push(link_line(page, &link, pages));
        }
    }
    Ok(answer(pages, found))
}

/// One line for one link, in the page's own top-left space — the space every
/// annotation's rectangle is reported in, and the space the page is drawn in.
fn link_line(page: usize, link: &InternalLink, pages: usize) -> String {
    // PDFium hands back a destination's page index as it stored it. A link to a
    // page the file does not have is still a link annotation worth naming, even
    // where nothing can follow it.
    let target = if link.page < pages {
        format!("page {}", link.page + 1)
    } else {
        "a page the file does not have".to_string()
    };
    format!(
        "page {}: link at ({:.0}, {:.0}) → {target}",
        page + 1,
        link.rect.left,
        link.rect.top
    )
}

/// The whole answer: the links found, or the sentence that there are none.
///
/// Kept apart from [`report`] so the wording — including what is *not* built —
/// is testable without a window or a document.
fn answer(pages: usize, found: Vec<String>) -> Vec<String> {
    let mut lines = found;
    if lines.is_empty() {
        lines.push(format!(
            "no cross-references — none of the {pages} page(s) has a link to another page \
             of this document. Creating one is not built: PDFium here can put a web address \
             on a link, not a page destination; `weblinks` makes an address link."
        ));
    } else {
        lines.push(format!(
            "{} cross-reference(s) over {pages} page(s). Read-only — creating an internal \
             link needs a destination writer this build does not have.",
            lines.len()
        ));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdf_core::document::Rect;

    fn link(left: f32, top: f32, page: usize) -> InternalLink {
        InternalLink {
            rect: Rect { left, top, right: left + 50.0, bottom: top + 40.0 },
            page,
        }
    }

    /// The line the plan promised, in the page's top-left space, one-based on
    /// both sides — page 2's link to page 5 reads as page 2 and page 5.
    #[test]
    fn a_link_reads_as_page_point_and_target() {
        assert_eq!(
            link_line(1, &link(10.4, 29.6, 4), 6),
            "page 2: link at (10, 30) → page 5"
        );
    }

    /// A destination past the end of the document is still reported, and named
    /// as what it is rather than as a page number that does not exist.
    #[test]
    fn a_link_off_the_end_says_so() {
        let line = link_line(0, &link(1.0, 2.0, 99), 3);
        assert!(line.contains("a page the file does not have"), "{line}");
        assert!(!line.contains("page 100"), "{line}");
    }

    /// Nothing found says so in words, and says what is not built — the two
    /// things a reader reaching for Cross Reference needs.
    #[test]
    fn an_empty_document_answers_with_the_none_sentence() {
        let lines = answer(3, Vec::new());
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].contains("no cross-references"), "{lines:?}");
        assert!(lines[0].contains("Creating one is not built"), "{lines:?}");
        assert!(lines[0].contains("3 page(s)"), "{lines:?}");
    }

    /// With links found the lines come first and a count closes the answer.
    #[test]
    fn found_links_are_listed_then_counted() {
        let lines = answer(2, vec!["page 1: link at (10, 20) → page 2".into()]);
        assert_eq!(lines[0], "page 1: link at (10, 20) → page 2");
        assert!(lines[1].contains("1 cross-reference(s) over 2 page(s)"), "{lines:?}");
        assert!(lines[1].contains("Read-only"), "{lines:?}");
    }
}
