//! Each page's transition, as the file on disk stores it — the View tab's Page
//! Transitions.
//!
//! **What a page transition is.** A page may carry a `/Trans` dictionary saying
//! how a viewer should move *to* it — dissolve, a wipe, a box, a fly — with the
//! effect's duration and direction, and a `/Dur` saying how long the page stays
//! on screen before an automatic advance. It is presentation, not page content:
//! it never prints, and a viewer that ignores it simply shows the next page.
//!
//! **Read from the file, not from the open document.** The engine has no
//! transition read or write, so the honest source is the bytes: this parses
//! the page tree with `pdf_core::pdf::File` and reports exactly what the file
//! says. An edit in this session that has not been saved is not in the answer,
//! and the answer says so.
//!
//! **Writing is not built.** A transition is a raw dictionary entry, not
//! something PDFium's annotation API makes, and this reads only.

use std::collections::BTreeSet;

use pdf_core::pdf::{Dict, File, Object};
use pdf_core::{PdfError, Result};

use crate::Session;

/// Report every page that carries page timing, one line each.
///
/// The bytes come from [`Session::path`] — the file the document was opened
/// from — because the open document's in-memory copy keeps nothing this reader
/// can see through. See the module note.
pub fn report(session: &Session) -> Result<Vec<String>> {
    let bytes = std::fs::read(session.path())?;
    report_bytes(&bytes)
}

/// [`report`] over a file's bytes — the whole of the work, and what the tests
/// use without opening a document.
pub fn report_bytes(bytes: &[u8]) -> Result<Vec<String>> {
    let file = File::parse(bytes)?;
    let pages = page_dicts(&file)?;

    let mut lines = Vec::new();
    for (index, page) in pages.iter().enumerate() {
        let Some(raw) = page.get(b"Trans") else {
            // A display time with no effect is page timing too — the Page
            // Transitions tool is where both live — so it is reported rather
            // than silently dropped.
            if let Some(stay) = number(&file, page, b"Dur") {
                lines.push(format!("page {}: no transition, displayed for {} s", index + 1, words(stay)));
            }
            continue;
        };

        let trans = match file.resolve(raw) {
            Ok(trans) => trans,
            Err(_) => {
                lines.push(format!("page {}: a /Trans entry this build cannot follow", index + 1));
                continue;
            }
        };
        let Some(trans) = trans.as_dict() else {
            lines.push(format!("page {}: a /Trans entry that is not a dictionary", index + 1));
            continue;
        };

        let mut parts: Vec<String> = Vec::new();
        match name(&file, trans, b"S") {
            Some(kind) => parts.push(transition_word(&kind)),
            None => parts.push("an unrecognised transition".into()),
        }
        if let Some(seconds) = number(&file, trans, b"D") {
            parts.push(format!("{} s", words(seconds)));
        }
        if let Some(dimension) = name(&file, trans, b"Dm") {
            parts.push(
                match dimension.as_str() {
                    "h" => "horizontal",
                    "v" => "vertical",
                    other => other,
                }
                .to_string(),
            );
        }
        if let Some(motion) = name(&file, trans, b"M") {
            parts.push(
                match motion.as_str() {
                    "i" => "inward",
                    "o" => "outward",
                    other => other,
                }
                .to_string(),
            );
        }
        if let Some(degrees) = number(&file, trans, b"Di") {
            parts.push(format!("{}°", words(degrees)));
        }
        if let Some(stay) = number(&file, page, b"Dur") {
            parts.push(format!("displayed for {} s", words(stay)));
        }
        lines.push(format!("page {}: {}", index + 1, parts.join(", ")));
    }

    Ok(answer(pages.len(), lines))
}

/// The whole answer: the lines found, or the sentence that there are none.
fn answer(pages: usize, found: Vec<String>) -> Vec<String> {
    let mut lines = found;
    if lines.is_empty() {
        lines.push(format!(
            "no page transitions — none of the {pages} page(s) carries a /Trans effect or a \
             /Dur display time. Read from the file on disk, so unsaved edits are not in it; \
             writing a transition is not built."
        ));
    } else {
        lines.push(format!(
            "{} page(s) with page timing over {pages} page(s), read from the file on disk — \
             unsaved edits are not in it. Writing page transitions is not built.",
            lines.len()
        ));
    }
    lines
}

/// The page dictionaries in page-tree order — the order a reader shows them.
///
/// The page tree is `/Pages` nodes whose `/Kids` hold further nodes or the
/// pages themselves. A cycle or a pathologically deep tree is refused rather
/// than walked forever: the file is somebody else's bytes.
fn page_dicts(file: &File<'_>) -> Result<Vec<Dict>> {
    let root = file
        .trailer()
        .get(b"Root")
        .and_then(Object::as_reference)
        .map(|(number, _)| number)
        .ok_or_else(|| PdfError::InvalidArgument("the trailer names no document catalogue".into()))?;
    let catalog = file.object(root)?;
    let Some(catalog) = catalog.as_dict() else {
        return Err(PdfError::InvalidArgument("the document catalogue is not a dictionary".into()));
    };
    let pages = catalog
        .get(b"Pages")
        .ok_or_else(|| PdfError::InvalidArgument("the catalogue names no page tree".into()))?;

    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    collect_pages(file, pages, &mut out, &mut seen, 0)?;
    Ok(out)
}

/// Depth cap: deeper than any real page tree, far shallower than the stack.
const MAX_TREE_DEPTH: usize = 64;

fn collect_pages(
    file: &File<'_>,
    node: &Object,
    out: &mut Vec<Dict>,
    seen: &mut BTreeSet<u32>,
    depth: usize,
) -> Result<()> {
    if depth > MAX_TREE_DEPTH {
        return Err(PdfError::InvalidArgument(
            "the page tree nests deeper than this reader follows".into(),
        ));
    }

    let dict = match node {
        Object::Dict(dict) | Object::Stream(dict, _) => dict,
        Object::Reference(number, _) => {
            if !seen.insert(*number) {
                return Err(PdfError::InvalidArgument(format!(
                    "the page tree reaches object {number} twice"
                )));
            }
            return collect_pages(file, &file.object(*number)?, out, seen, depth);
        }
        _ => return Ok(()),
    };

    // A page is a leaf; a `/Pages` node is anything with children.
    let is_page = dict.get(b"Type").and_then(Object::as_name) == Some(b"Page".as_slice());
    let kids = dict.get(b"Kids");
    if is_page || kids.is_none() {
        out.push(dict.clone());
        return Ok(());
    }

    let kids = file.resolve(kids.expect("just checked"))?;
    let Object::Array(kids) = kids else {
        return Err(PdfError::InvalidArgument("a page-tree node's /Kids is not an array".into()));
    };
    for kid in &kids {
        collect_pages(file, kid, out, seen, depth + 1)?;
    }
    Ok(())
}

/// A name value of a dictionary, lowercased — `/Split` reads as `split`.
fn name(file: &File<'_>, dict: &Dict, key: &[u8]) -> Option<String> {
    let value = file.resolve(dict.get(key)?).ok()?;
    value.as_name().map(|name| String::from_utf8_lossy(name).to_ascii_lowercase())
}

/// A number value of a dictionary; `None` when the key is absent or not a
/// number.
fn number(file: &File<'_>, dict: &Dict, key: &[u8]) -> Option<f64> {
    let value = file.resolve(dict.get(key)?).ok()?;
    value.as_f64()
}

/// The transition kinds the PDF specification names, as the words a person
/// would use. An unnamed kind is passed through rather than dropped.
fn transition_word(name: &str) -> String {
    match name {
        "split" => "split",
        "blinds" => "blinds",
        "box" => "box",
        "wipe" => "wipe",
        "dissolve" => "dissolve",
        "glitter" => "glitter",
        "r" => "replace",
        "fly" => "fly",
        "push" => "push",
        "cover" => "cover",
        "uncover" => "uncover",
        "fade" => "fade",
        other => other,
    }
    .to_string()
}

/// A number as a person writes it: `1.5`, `2`, not `2.000000`.
fn words(value: f64) -> String {
    let rounded = (value * 1000.0).round() / 1000.0;
    if rounded.fract().abs() < f64::EPSILON {
        format!("{}", rounded as i64)
    } else {
        format!("{rounded}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small, valid PDF with the objects given, written with a plain xref
    /// table — the shape `pdf_core::pdf::File` reads.
    fn pdf(objects: &[&str]) -> Vec<u8> {
        let mut out = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (index, body) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", index + 1).as_bytes());
        }
        let xref_at = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
        for offset in &offsets {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        out
    }

    /// A two-page document whose first page has a transition and a display
    /// time, and whose second has neither.
    fn split_page() -> Vec<u8> {
        pdf(&[
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] \
             /Trans << /S /Split /D 1.5 /Dm /H /M /I /Di 90 >> /Dur 3 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] >>",
        ])
    }

    /// The plan's line: type, duration and direction, from the file's own
    /// `/Trans` dictionary — and the page with nothing stays out of it.
    #[test]
    fn a_transition_reads_type_duration_and_direction() {
        let lines = report_bytes(&split_page()).expect("read");
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert_eq!(
            lines[0],
            "page 1: split, 1.5 s, horizontal, inward, 90°, displayed for 3 s"
        );
        assert!(
            lines[1].contains("1 page(s) with page timing over 2 page(s)"),
            "{lines:?}"
        );
        assert!(lines[1].contains("read from the file on disk"), "{lines:?}");
        assert!(lines[1].contains("Writing page transitions is not built"), "{lines:?}");
    }

    /// A document with no timing answers in words, and says where the answer
    /// came from and what is not built.
    #[test]
    fn a_document_without_transitions_answers_none() {
        let bytes = pdf(&[
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] >>",
        ]);
        let lines = report_bytes(&bytes).expect("read");
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].contains("no page transitions"), "{lines:?}");
        assert!(lines[0].contains("1 page(s)"), "{lines:?}");
        assert!(lines[0].contains("Read from the file on disk"), "{lines:?}");
    }

    /// A display time without an effect is timing too, and is reported as the
    /// page-timing fact it is rather than dropped.
    #[test]
    fn a_display_time_without_an_effect_is_still_reported() {
        let bytes = pdf(&[
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] /Dur 4.25 >>",
        ]);
        let lines = report_bytes(&bytes).expect("read");
        assert_eq!(lines[0], "page 1: no transition, displayed for 4.25 s");
    }

    /// The remaining kinds and dimensions read as words: a Wipe is a wipe, and
    /// a vertical one is vertical.
    #[test]
    fn other_kinds_read_as_their_words() {
        let bytes = pdf(&[
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] \
             /Trans << /S /Wipe /D 2 /Dm /V /M /O >> >>",
        ]);
        let lines = report_bytes(&bytes).expect("read");
        assert_eq!(lines[0], "page 1: wipe, 2 s, vertical, outward");
    }

    /// The page tree is walked, not assumed flat: a nested `/Pages` node still
    /// yields every page in order, so the page numbers in the answer are the
    /// page numbers a reader sees.
    #[test]
    fn a_nested_page_tree_yields_pages_in_order() {
        let bytes = pdf(&[
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 2 >>",
            "<< /Type /Pages /Parent 2 0 R /Kids [4 0 R 5 0 R] /Count 2 >>",
            "<< /Type /Page /Parent 3 0 R /MediaBox [0 0 200 300] /Trans << /S /Fade >> >>",
            "<< /Type /Page /Parent 3 0 R /MediaBox [0 0 200 300] /Trans << /S /Push >> >>",
        ]);
        let lines = report_bytes(&bytes).expect("read");
        assert_eq!(lines[0], "page 1: fade");
        assert_eq!(lines[1], "page 2: push");
    }

    /// A file whose page tree points at itself is refused rather than walked
    /// forever — the bytes are somebody else's.
    #[test]
    fn a_cyclic_page_tree_is_refused() {
        let bytes = pdf(&[
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [2 0 R] /Count 1 >>",
        ]);
        let problem = report_bytes(&bytes).expect_err("a cycle must be refused");
        assert!(problem.to_string().contains("twice"), "{problem}");
    }

    /// A cross-reference stream is a legal file this build's raw reader does
    /// not read; the report says so rather than answering "none".
    #[test]
    fn a_file_the_raw_reader_cannot_read_is_an_error_not_a_none() {
        let problem = report_bytes(b"%PDF-1.7\nnot really a pdf").expect_err("must be refused");
        assert!(!problem.to_string().is_empty(), "{problem}");
    }
}
