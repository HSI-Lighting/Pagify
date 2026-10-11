//! `accesscheck`'s findings and `accessreport`'s HTML, against fixtures
//! chosen to classify as different things — the check's whole input is page
//! classification.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test -p pagify_shell --test accessibility
//! ```

use pagify_shell::{accessibility, Session};

fn fixture(name: &str) -> String {
    format!(
        "{}/../../../rust/pdf_core/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    )
}

#[test]
fn a_native_page_has_nothing_to_report() {
    let session = Session::open(fixture("two-column.pdf")).expect("open");

    let findings = accessibility::findings(&session).expect("check");

    assert!(
        findings.is_empty(),
        "well-authored text was reported: {:?}",
        findings.iter().map(|f| &f.line).collect::<Vec<_>>()
    );
}

#[test]
fn a_scanned_page_is_reported_with_the_verb_that_fixes_it() {
    let session = Session::open(fixture("scan-300dpi.pdf")).expect("open");

    let findings = accessibility::findings(&session).expect("check");

    let line = &findings.first().expect("a scan is a finding").line;
    assert!(line.contains("scan with no text"), "{line}");
    assert!(line.contains("extracttext"), "the fix is not named: {line}");
}

#[test]
fn outlined_text_is_reported_with_outlinedfont_first() {
    let session = Session::open(fixture("outlined.pdf")).expect("open");

    let findings = accessibility::findings(&session).expect("check");

    assert!(
        findings.iter().any(|f| f.line.contains("outlinedfont")),
        "no finding names the outlined-font route: {:?}",
        findings.iter().map(|f| &f.line).collect::<Vec<_>>()
    );
}

#[test]
fn the_report_carries_the_findings_and_its_own_scope() {
    let session = Session::open(fixture("scan-300dpi.pdf")).expect("open");

    let html = accessibility::report_html(&session, "scan").expect("html");

    assert!(html.contains("accessibility check"), "no title: {html}");
    assert!(html.contains("scan with no text"), "no finding in the report: {html}");
    assert!(
        html.contains("not reported on"),
        "the scope sentence is missing — the report would read as a clean bill: {html}"
    );
}
