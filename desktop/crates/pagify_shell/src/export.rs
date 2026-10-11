//! Whole-document export: the pages' own text as one HTML file — the
//! Conversion tab's `tohtml`.
//!
//! A shell function over `Session`, not app code: it needs no window, and
//! here it can be tested against the committed fixtures. The PNG half of the
//! same tab (`toimage`) is `Session::export_page_png`.

use crate::Session;

/// The document's text as a plain HTML file: a heading per page and the
/// page's runs in the file's own order, escaped.
///
/// **The file's own order, not a reading order.** `two-column.pdf` is
/// authored left column then right, so it reads well; `shredded.pdf` is the
/// same words in paint order and does not. Reconstruction is `reflow`'s job,
/// done on the page itself — this walks what the page stores, which is also
/// what selecting all and copying would give.
pub fn document_html(session: &Session, title: &str) -> pdf_core::Result<String> {
    let mut out = String::new();
    out.push_str("<!doctype html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n");
    out.push_str(&format!("<title>{}</title>\n", escape(title)));
    // Runs are laid out, not paragraphs: keeping their line breaks and not
    // inventing structure between them is the honest rendering of that.
    out.push_str("<style>p { white-space: pre-wrap; }</style>\n</head>\n<body>\n");
    out.push_str(&format!("<h1>{}</h1>\n", escape(title)));
    for page in 0..session.page_count()? {
        out.push_str(&format!("<h2>Page {}</h2>\n", page + 1));
        let snapshot = session.page_text_snapshot(page)?;
        let runs: Vec<&str> = snapshot
            .runs
            .iter()
            .map(|run| run.text.trim())
            .filter(|text| !text.is_empty())
            .collect();
        if runs.is_empty() {
            out.push_str("<p><em>no text on this page</em></p>\n");
        } else {
            out.push_str(&format!("<p>{}</p>\n", escape(&runs.join("\n"))));
        }
    }
    out.push_str("</body>\n</html>\n");
    Ok(out)
}

/// Four characters stand between text and markup; everything else passes
/// through as it is. Shared with the accessibility report, which writes
/// markup too.
pub(crate) fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
    out
}
