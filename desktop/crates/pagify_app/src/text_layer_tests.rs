use super::*;

fn fixture(name: &str) -> String {
    format!(
        "{}/../../../rust/pdf_core/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn said(app: &PagifyApp) -> String {
    app.cmd.history().iter().map(|e| e.text.as_str()).collect::<Vec<_>>().join("\n")
}

#[test]
fn a_page_with_no_text_says_so_on_opening() {
    // The whole of phase 1: a silent, confusing failure becomes an
    // explained one. `single-page.pdf` has no content stream at all.
    let mut app = PagifyApp::new(Some(&fixture("single-page.pdf")));
    assert!(app.tab_mut().doc.is_some());
    assert!(
        said(&app).contains("nothing on it to select"),
        "the reader was left to guess why selection does nothing:\n{}",
        said(&app)
    );
    let _ = &mut app;
}

#[test]
fn a_page_with_text_does_not_nag() {
    // Said only when something is wrong. A note on every page turn would
    // be noise, and noise is how real warnings get ignored.
    let app = PagifyApp::new(Some(&fixture("text-lines.pdf")));
    assert!(
        !said(&app).contains("no text layer"),
        "a perfectly good page was warned about:\n{}",
        said(&app)
    );
}

#[test]
fn textlayer_answers_either_way_and_shows_its_working() {
    let mut app = PagifyApp::new(Some(&fixture("text-lines.pdf")));
    app.submit("textlayer");
    let output = said(&app);

    assert!(output.contains("selectable text"), "no verdict:\n{output}");
    assert!(
        output.contains("characters"),
        "the counts behind the verdict were not shown — a threshold nobody \
         can see is one nobody can argue with:\n{output}"
    );
}
