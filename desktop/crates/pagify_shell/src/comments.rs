//! Every annotation in the document as one list — behind `managecomments`.
//!
//! A shell function over `Session`, so it needs no window and can be tested
//! against fixtures. The list is read once when the panel opens and re-read
//! after every removal; it is not rebuilt every frame.

use crate::Session;
use pdf_core::document::Annotation;

/// One annotation, as the comments panel needs it.
#[derive(Debug)]
pub struct Comment {
    /// Which page it is on, zero-based.
    pub page: usize,
    /// PDFium's own index for it — what `RemoveAnnotation` takes.
    pub index: usize,
    /// The kind, as a word.
    pub kind: &'static str,
    /// A short reading of what it says, where it says anything.
    pub detail: String,
}

/// Every annotation in the document, page by page, in page order.
pub fn comments(session: &Session) -> pdf_core::Result<Vec<Comment>> {
    let mut out = Vec::new();
    for page in 0..session.page_count()? {
        for mark in session.annotations(page)? {
            let (kind, detail) = match &mark.annotation {
                Annotation::Highlight { .. } => ("highlight", String::new()),
                Annotation::Underline { .. } => ("underline", String::new()),
                Annotation::StrikeOut { .. } => ("strikethrough", String::new()),
                Annotation::Squiggly { .. } => ("squiggle", String::new()),
                Annotation::Ink { strokes, .. } => ("ink", format!("{} stroke(s)", strokes.len())),
                Annotation::Note { contents, .. } => ("note", snippet(contents)),
                Annotation::Text { text, .. } => ("text", snippet(text)),
                Annotation::Image { width, height, .. } => {
                    ("picture", format!("{width}×{height}"))
                }
                Annotation::Fill { .. } => ("shape", String::new()),
                Annotation::Link { uri, .. } => ("link", snippet(uri)),
            };
            out.push(Comment { page, index: mark.index, kind, detail });
        }
    }
    Ok(out)
}

/// The first few words of what a mark says, one line with an ellipsis — the
/// panel is a list, not a reader.
fn snippet(text: &str) -> String {
    let text = text.trim().replace(['\n', '\r'], " ");
    if text.chars().count() > 60 {
        text.chars().take(57).collect::<String>() + "…"
    } else {
        text
    }
}
