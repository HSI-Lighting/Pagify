//! Save As makes the new file the open one, and writes what Save writes.
use super::*;

use pdf_core::document::{Annotation, Document, DrawnKind, Rect};

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

/// **The approved behaviour change.** A picture placed in Pagify is an
/// annotation while it is open; Save As writes it into the page, and reopened it
/// is an ordinary page picture. The document in hand keeps its handles.
#[test]
fn save_as_writes_a_placed_picture_into_the_page() {
    let (dir, mut app) = scratch("picture");
    let session = app.tab().doc.as_ref().expect("open").session.clone();
    let rect = Rect { left: 100.0, top: 200.0, right: 160.0, bottom: 240.0 };
    session
        .execute(
            pdf_core::command::Command::AddAnnotation {
                page_index: 0,
                annotation: Annotation::Image {
                    rect,
                    rgba: [255u8, 0, 0, 255].repeat(16),
                    width: 4,
                    height: 4,
                },
            },
        )
        .expect("place the picture");
    let annotations = session.annotations(0).expect("annotations").len();

    let out = dir.join("with-picture.pdf");
    assert_eq!(app.save_as(out.clone()), SaveOutcome::Done);

    let opened = pdf_core::registry::exclusive(|| {
        pdf_core::document::pdfium_doc::PdfiumDocument::open_path(out.to_str().expect("path"), None)
    })
    .expect("the saved file opens");
    assert_eq!(
        opened.annotations(0).expect("annotations").len(),
        annotations - 1,
        "the picture is still only an annotation in the saved file"
    );
    assert!(
        opened.drawn_objects(0).expect("drawn").iter().any(|o| o.kind == DrawnKind::Picture && o.depth == 0),
        "the page draws no picture of its own"
    );
    assert_eq!(session.placed_image_marks(0).expect("marks").len(), 1, "the open document lost its handles");
}

/// The line is the owner's, not an error, and it names what was not stored.
#[test]
fn the_missing_face_line_says_what_another_device_will_do() {
    assert_eq!(
        missing_face_message(&["Montserrat-Thin".to_string()]),
        "Saved. Montserrat-Thin is not on this PC, so it is not stored in the file — other devices \
         will draw those words in a font of their own."
    );
    assert_eq!(
        missing_face_message(&["Montserrat-Thin".to_string(), "PTSans-Regular".to_string()]),
        "Saved. Montserrat-Thin and PTSans-Regular are not on this PC, so they are not stored in the \
         file — other devices will draw those words in a font of their own."
    );
}

/// "Save a secured copy" writes another path too, but there the original is
/// meant to stay open — which is why the move lives in `save_as`, not `save`.
#[test]
fn a_plain_copy_does_not_move_the_open_file() {
    let (dir, mut app) = scratch("copy");

    assert_eq!(app.save(Some(dir.join("copy.pdf"))), SaveOutcome::Done);
    assert_eq!(open_path(&app), dir.join("original.pdf"));
}
