//! The accessibility checks this build can actually make — behind
//! `accesscheck` and `accessreport` (the Accessibility tab's Full Check and
//! Accessibility Report).
//!
//! **A short list, deliberately.** The checks a full accessibility audit
//! wants — alt text, tags, reading order, language, contrast — are not things
//! this engine can see, and a check that cannot see a thing must not report
//! it as passed. What is here is what page classification already knows,
//! said in words that name the verb which fixes it.

use crate::Session;
use pdf_core::document::PageTextKind;

/// One line of the check, and whether it is a problem.
pub struct Finding {
    /// Which page it is about, one-based in [`Finding::line`].
    pub page: usize,
    pub problem: bool,
    pub line: String,
}

/// Every finding the engine can make: what each page's text layer is like,
/// for the pages that are not simply readable.
///
/// A `Native` or `Hybrid` page is fine and contributes nothing. An `Empty`
/// page is fine too — a blank sheet has nothing to fail to read.
pub fn findings(session: &Session) -> pdf_core::Result<Vec<Finding>> {
    let mut out = Vec::new();
    for page in 0..session.page_count()? {
        let kind = session.classify(page)?.kind;
        let line = match kind {
            PageTextKind::Native | PageTextKind::Hybrid | PageTextKind::Empty => continue,
            PageTextKind::Unmappable => Finding {
                page,
                problem: true,
                line: format!(
                    "page {}: the text is there but its characters do not map — selection \
                     and copying come out wrong. `extracttext` can read the page again.",
                    page + 1
                ),
            },
            PageTextKind::Scanned => Finding {
                page,
                problem: true,
                line: format!(
                    "page {} is a scan with no text: a screen reader finds nothing on it. \
                     `extracttext` reads it into a searchable layer.",
                    page + 1
                ),
            },
            PageTextKind::Outlined => Finding {
                page,
                problem: true,
                line: format!(
                    "page {} draws its letters as outlines: there is no text to select or \
                     read. `outlinedfont` learns the face from it, then `extracttext` can \
                     read the page.",
                    page + 1
                ),
            },
        };
        out.push(line);
    }
    Ok(out)
}

/// The findings as one HTML file — what `accessreport` writes.
///
/// The scope sentence stays in, in the document itself: a report that listed
/// only what it checked would read as a clean bill of health for everything
/// it did not.
pub fn report_html(session: &Session, title: &str) -> pdf_core::Result<String> {
    let findings = findings(session)?;
    let problems = findings.iter().filter(|f| f.problem).count();
    let mut out = String::new();
    out.push_str("<!doctype html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n");
    out.push_str(&format!("<title>{} — accessibility check</title>\n", crate::export::escape(title)));
    out.push_str("<style>li.fail { color: #B00020; }</style>\n</head>\n<body>\n");
    out.push_str(&format!("<h1>{} — accessibility check</h1>\n", crate::export::escape(title)));
    out.push_str(&format!(
        "<p>{} problem(s) found by the checks this build makes — what each page's text \
         layer is like. Alt text, tags, reading order, language and contrast are not \
         visible to this engine and are not reported on.</p>\n",
        problems
    ));
    if findings.is_empty() {
        out.push_str("<p>No problems.</p>\n");
    } else {
        out.push_str("<ul>\n");
        for finding in &findings {
            out.push_str(&format!(
                "<li class=\"{}\">{}</li>\n",
                if finding.problem { "fail" } else { "note" },
                crate::export::escape(&finding.line)
            ));
        }
        out.push_str("</ul>\n");
    }
    out.push_str("</body>\n</html>\n");
    Ok(out)
}
