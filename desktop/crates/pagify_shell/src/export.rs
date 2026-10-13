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

/// Hard-wrap `line` at `width` characters, on character boundaries.
///
/// `fromclipboard`'s own wrap, and the reason it is a character count rather
/// than a measurement: this build writes lines it cannot ask the width of
/// (see the `pagemarks` parse arm), so a wrap point can only be a count. It
/// is a formatting choice, not a typographic claim — anything that needs
/// better goes through the editor.
pub fn wrap_by_chars(line: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![line.to_string()];
    }
    let chars: Vec<char> = line.chars().collect();
    chars.chunks(width).map(|chunk| chunk.iter().collect()).collect()
}

/// The document's text in one of the plain formats — the Conversion tab's
/// To Other: `.txt`, `.rtf` or `.xml`, chosen by the extension the caller
/// asks for. `.html` is [`document_html`]'s.
pub fn document_other(session: &Session, extension: &str) -> pdf_core::Result<String> {
    let pages: Vec<Vec<String>> = (0..session.page_count()?)
        .map(|page| {
            session.page_text_snapshot(page).map(|snapshot| {
                snapshot
                    .runs
                    .iter()
                    .map(|run| run.text.trim().to_string())
                    .filter(|text| !text.is_empty())
                    .collect()
            })
        })
        .collect::<pdf_core::Result<Vec<_>>>()?;

    let text = match extension.to_ascii_lowercase().as_str() {
        "txt" => pages
            .iter()
            .map(|lines| lines.join("\n"))
            .collect::<Vec<_>>()
            .join("\n\n"),
        "rtf" => {
            let mut out = String::from("{\\rtf1\\ansi\n");
            for (page, lines) in pages.iter().enumerate() {
                if page > 0 {
                    out.push_str("\\page\n");
                }
                for line in lines {
                    out.push_str(&escape_rtf(line));
                    out.push_str("\\par\n");
                }
            }
            out.push('}');
            out
        }
        "xml" => {
            let mut out = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<document>\n");
            for (page, lines) in pages.iter().enumerate() {
                out.push_str(&format!("  <page n=\"{}\">\n", page + 1));
                for line in lines {
                    out.push_str(&format!("    <line>{}</line>\n", escape_xml(line)));
                }
                out.push_str("  </page>\n");
            }
            out.push_str("</document>\n");
            out
        }
        other => {
            return Err(pdf_core::PdfError::InvalidArgument(format!(
                "toother writes .txt, .rtf and .xml — not `.{other}`; .html is `tohtml`"
            )))
        }
    };
    Ok(text)
}

/// RTF: backslash, braces and anything above ASCII, the way RTF spells them.
pub(crate) fn escape_rtf(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '{' => out.push_str("\\{"),
            '}' => out.push_str("\\}"),
            c if (c as u32) < 128 => out.push(c),
            c => out.push_str(&format!("\\u{}?", c as u32)),
        }
    }
    out
}

/// XML: the four markup characters, nothing more.
pub(crate) fn escape_xml(text: &str) -> String {
    let mut escaped = text.replace('&', "&amp;");
    escaped = escaped.replace('<', "&lt;").replace('>', "&gt;");
    escaped.replace('"', "&quot;")
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

#[cfg(test)]
mod other_format_tests {
    use super::*;

    #[test]
    fn the_rtf_escape_spells_markup_and_unicode() {
        assert_eq!(escape_rtf("a\\b{c}"), "a\\\\b\\{c\\}");
        assert_eq!(escape_rtf("café"), "caf\\u233?");
    }

    #[test]
    fn the_xml_escape_covers_the_four_characters() {
        assert_eq!(escape_xml("a<b>&\"c\""), "a&lt;b&gt;&amp;&quot;c&quot;");
    }
}
