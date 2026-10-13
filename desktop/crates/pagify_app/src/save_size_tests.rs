//! **What a save writes.**
//!
//! Reported from use: "save saves a copy instead of saving the changes", and "the
//! size of the file multiplies". Every save was incremental, and PDFium's
//! incremental save appends every object it has *loaded* — showing a page loads
//! nearly all of them — so each save wrote most of the document again on the end
//! of the file. Measured on the user's own files: a save with nothing changed
//! added 713 KB to a 2.7 MB datasheet; one pasted picture added 465 of a page's
//! 484 objects. Incremental saving is kept for what it is for, a signature.

use super::*;

/// A copy of `source` in a folder of its own, so a save can be measured.
fn copy_to_scratch(source: &std::path::Path, name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("pagify-save-size-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch folder");
    let path = dir.join(name);
    std::fs::copy(source, &path).expect("copy");
    path
}

fn size(path: &std::path::Path) -> u64 {
    std::fs::metadata(path).expect("saved file").len()
}

fn fixture(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(crate::ui_tests::fixture(name))
}

/// Opened, looked at, saved three times: the file is the size of what is in it,
/// once — not a copy more for every save.
#[test]
fn saving_writes_the_document_once_however_often_it_is_saved() {
    let path = copy_to_scratch(&fixture("pictures.pdf"), "pictures.pdf");
    let original = size(&path);
    let mut app = PagifyApp::new(Some(path.to_str().expect("path")));
    // Looking at the page is what loads its objects — and what an incremental
    // save then wrote out again.
    app.tab().doc.as_ref().expect("open").session.render_page(0, 1.0).expect("render");

    assert!(matches!(app.save(None), SaveOutcome::Done));
    let once = size(&path);
    assert!(once <= original + original / 10, "one save took {original} bytes to {once}");
    for _ in 0..2 {
        assert!(matches!(app.save(None), SaveOutcome::Done));
    }
    assert_eq!(size(&path), once, "saving again with nothing changed changed the file");
}

/// A signed document is still saved by appending, so the signature's bytes stay
/// where it says they are.
#[test]
fn a_signed_document_is_still_saved_by_appending() {
    let source = fixture("rsa-signed.pdf");
    let original = std::fs::read(&source).expect("fixture");
    let path = copy_to_scratch(&source, "rsa-signed.pdf");
    let mut app = PagifyApp::new(Some(path.to_str().expect("path")));
    let session = app.tab().doc.as_ref().expect("open").session.clone();
    assert!(session.signature_count() > 0, "the fixture should be signed");
    session
        .execute(pdf_core::command::Command::SetPageRotation { index: 0, quarter_turns: 1 })
        .expect("an edit");

    assert!(matches!(app.save(None), SaveOutcome::Done));
    let saved = std::fs::read(&path).expect("saved");
    assert!(saved.len() > original.len() && saved[..original.len()] == original[..], "the signed bytes were rewritten");
}
