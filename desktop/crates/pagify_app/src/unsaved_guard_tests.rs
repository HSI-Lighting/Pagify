use super::*;

fn fixture(name: &str) -> String {
    format!(
        "{}/../../../rust/pdf_core/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    )
}

/// A **copy** of a fixture, in a scratch directory.
///
/// These tests call `save`, and `save` with no destination writes back to
/// the file that is open — which is the whole point of it. Pointed at a
/// fixture, that truncates the fixture: `single-page.pdf` was reduced to
/// zero bytes by this suite before the copy was introduced, and every test
/// in the workspace that touched it started failing at once.
///
/// A test that writes must never be given the original.
fn scratch_copy(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("pagify-app-tests");
    std::fs::create_dir_all(&dir).expect("scratch dir");

    // Named per test thread so two running at once cannot share a file.
    let unique = format!("{:?}-{name}", std::thread::current().id())
        .replace(['(', ')', ' '], "");
    let path = dir.join(unique);
    std::fs::copy(fixture(name), &path).expect("stage a copy of the fixture");
    path
}

fn app() -> PagifyApp {
    let path = scratch_copy("single-page.pdf");
    let app = PagifyApp::new(Some(&path.to_string_lossy()));
    assert!(
        app.tab().doc.is_some(),
        "fixture did not open: {path:?}\nhistory:\n{}",
        app.cmd.history().iter().map(|e| e.text.as_str()).collect::<Vec<_>>().join("\n")
    );
    app
}

#[test]
fn a_clean_document_closes_without_argument() {
    let mut app = app();
    app.submit("close");
    assert!(app.tab_mut().doc.is_none(), "an unmarked document refused to close");
}

/// The only data-loss path in the program.
#[test]
fn closing_a_marked_up_document_stops_to_ask() {
    let mut app = app();
    app.submit("l 30,30 200,200");
    assert_eq!(app.tab_mut().markup.existing(0).map(|l| l.len()), Some(1));

    app.submit("close");
    assert!(app.tab_mut().doc.is_some(), "the document closed and took the marks with it");
    assert_eq!(
        app.tab_mut().closing,
        Some(Closing::Document),
        "it neither closed nor asked, which leaves the user with nothing to act on"
    );
}

#[test]
/// What the dialog is built from has to be true, because it is the only
/// thing the user is shown before deciding.
fn the_question_can_say_exactly_what_would_be_lost() {
    let mut app = app();
    app.submit("l 0,0 10,10");
    app.submit("l 0,20 10,30");
    app.submit("close");

    assert_eq!(app.tab_mut().closing, Some(Closing::Document));
    assert_eq!(
        app.unsaved(),
        Some((2, 1)),
        "the count behind the question is wrong, so the question would be a lie"
    );
}

#[test]
fn the_bang_form_discards_deliberately() {
    let mut app = app();
    app.submit("l 30,30 200,200");
    app.submit("close!");
    assert!(app.tab_mut().doc.is_none(), "`close!` did not discard");
}

#[test]
fn opening_another_document_lands_in_its_own_tab() {
    // Opening no longer replaces the current document — it can't drop
    // its markup, because it never touches it.
    let mut app = app();
    app.submit("l 30,30 200,200");
    app.submit(&format!("open \"{}\"", scratch_copy("text-lines.pdf").display()));

    assert_eq!(app.tabs.len(), 2, "opening another file should have made a second tab");
    assert!(
        app.tab().doc.as_ref().unwrap().session.path().to_string_lossy().contains("text-lines"),
        "the new tab should be showing the file that was just opened"
    );
    // The newest tab is the leftmost, so the first one is now the second.
    assert_eq!(
        app.tabs[1].markup.existing(0).map(|l| l.len()),
        Some(1),
        "the first tab's mark should still be there, untouched"
    );
}

#[test]
fn a_moved_mark_still_counts_as_unsaved() {
    // The case a count of objects would miss: the drawing changed, the
    // number of things in it did not.
    let mut app = app();
    app.submit("l 30,30 200,200");
    app.submit("save");
    assert!(
        app.unsaved().is_none(),
        "a saved document still reports unsaved work. history:\n{}",
        app.cmd.history().iter().map(|e| e.text.as_str()).collect::<Vec<_>>().join("\n")
    );

    app.submit("all");
    app.submit("move");
    app.submit("pick 0,0");
    app.submit("pick 0,50");

    assert!(app.unsaved().is_some(), "moving a mark did not register as unsaved");
}

#[test]
fn saving_clears_the_debt() {
    let mut app = app();
    app.submit("l 30,30 200,200");
    assert!(app.unsaved().is_some());

    app.submit("save");
    assert!(app.unsaved().is_none(), "a save did not clear the unsaved state");

    app.submit("close");
    assert!(app.tab_mut().doc.is_none(), "a saved document still refused to close");
}
