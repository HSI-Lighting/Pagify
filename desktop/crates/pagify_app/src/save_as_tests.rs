//! Save As makes the new file the open one.
use super::*;

/// A scratch folder holding a copy of the fixture, so a save that goes to the
/// wrong place cannot touch the repository's own file.
fn scratch(test: &str) -> (std::path::PathBuf, PagifyApp) {
    let dir = std::env::temp_dir().join(format!("pagify-save-as-{test}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch folder");
    let original = dir.join("original.pdf");
    let fixture = format!("{}/../../../rust/pdf_core/fixtures/text-lines.pdf", env!("CARGO_MANIFEST_DIR"));
    std::fs::copy(fixture, &original).expect("copy the fixture");
    let app = PagifyApp::new(Some(original.to_str().expect("path")));
    assert!(app.tab().doc.is_some(), "the copy did not open");
    (dir, app)
}

fn open_path(app: &PagifyApp) -> std::path::PathBuf {
    app.tab().doc.as_ref().expect("open").session.path()
}

#[test]
fn after_save_as_the_new_file_is_the_open_one_and_save_goes_to_it() {
    let (dir, mut app) = scratch("opens");
    let original = dir.join("original.pdf");
    let renamed = dir.join("renamed.pdf");
    let before = std::fs::read(&original).expect("read");

    assert_eq!(app.save_as(renamed.clone()), SaveOutcome::Done);
    assert!(renamed.exists(), "Save As wrote nothing");
    assert_eq!(open_path(&app), renamed, "the old file is still the open one");
    assert_eq!(app.cmd.prompt_mut().document.as_deref(), Some("renamed.pdf"));

    // The next plain Save belongs to the new file, not the old one.
    std::fs::remove_file(&renamed).expect("remove the new file");
    assert_eq!(app.save(None), SaveOutcome::Done);
    assert!(renamed.exists(), "Save went somewhere else:\n{}", app.cmd.history().iter().map(|e| e.text.as_str()).collect::<Vec<_>>().join("\n"));
    assert_eq!(std::fs::read(&original).expect("read"), before, "Save wrote over the original");
}

#[test]
fn a_save_as_that_fails_leaves_the_open_file_alone() {
    let (dir, mut app) = scratch("fails");
    let nowhere = dir.join("no such folder").join("x.pdf");

    assert_eq!(app.save_as(nowhere), SaveOutcome::Failed);
    assert_eq!(open_path(&app), dir.join("original.pdf"));
}

/// "Save a secured copy" writes another path too, but there the original is
/// meant to stay open — which is why the move lives in `save_as`, not `save`.
#[test]
fn a_plain_copy_does_not_move_the_open_file() {
    let (dir, mut app) = scratch("copy");

    assert_eq!(app.save(Some(dir.join("copy.pdf"))), SaveOutcome::Done);
    assert_eq!(open_path(&app), dir.join("original.pdf"));
}
