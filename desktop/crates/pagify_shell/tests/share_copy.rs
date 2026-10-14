//! A copy of the open document written for sending: its placed pictures in the
//! page itself, so a preview that draws only the page shows them.
//!
//! Like `round_trip`, these write a real file, open it again from scratch and
//! assert on what is actually there — none of the writing code in the way.

use std::path::PathBuf;

use pagify_shell::Session;
use pdf_core::command::Command;
use pdf_core::document::{Annotation, DrawnKind, Rect};

fn fixture(name: &str) -> String {
    format!("{}/../../../rust/pdf_core/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))
}

/// A directory of this test's own, so two running at once cannot collide.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("pagify-share-copy-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        Scratch(dir)
    }
    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const WHERE: Rect = Rect { left: 100.0, top: 200.0, right: 180.0, bottom: 240.0 };

fn with_a_placed_picture() -> Session {
    let session = Session::open(fixture("two-column.pdf")).expect("open");
    session
        .execute(Command::AddAnnotation {
            page_index: 0,
            annotation: Annotation::Image { rect: WHERE, rgba: [255u8, 0, 0, 255].repeat(16), width: 4, height: 4 },
        })
        .expect("place the picture");
    session
}

/// **The acceptance test.** The copy is a real file on disk, and opened afresh
/// the picture is part of the page — a picture the engine lists as drawn by the
/// page itself — while the open document still holds it as a placed picture.
#[test]
fn the_copy_is_a_file_whose_page_draws_the_picture_and_the_open_document_is_unchanged() {
    let scratch = Scratch::new("acceptance");
    let session = with_a_placed_picture();
    let dest = scratch.file("for-sharing.pdf");

    let report = session.save_flattened_to(&dest).expect("the copy is written");
    assert_eq!((report.pictures, report.forms, report.bare), (1, 0, 0));
    assert!(dest.exists(), "no file was written");

    let reopened = Session::open(&dest).expect("the copy opens on its own");
    let drawn = reopened.drawn_objects(0).expect("drawn");
    assert!(
        drawn.iter().any(|o| o.kind == DrawnKind::Picture && o.depth == 0),
        "the page of the copy draws no picture of its own: {drawn:?}"
    );
    assert!(reopened.placed_image_marks(0).expect("marks").is_empty(), "it is still a placed picture in the copy");

    assert_eq!(session.placed_image_marks(0).expect("marks").len(), 1, "the open document lost its placed picture");
}

/// **The break:** writing the copy over the document being edited — which the
/// person would then find flattened — or over a file already at the path
/// without that file being replaced whole.
#[test]
fn a_copy_replaces_a_file_at_its_path_whole_and_never_touches_the_original() {
    let scratch = Scratch::new("replace");
    let original = scratch.file("original.pdf");
    std::fs::copy(fixture("two-column.pdf"), &original).expect("copy fixture");
    let before = std::fs::read(&original).expect("read");

    let session = Session::open(&original).expect("open");
    session
        .execute(Command::AddAnnotation {
            page_index: 0,
            annotation: Annotation::Image { rect: WHERE, rgba: [255u8, 0, 0, 255].repeat(16), width: 4, height: 4 },
        })
        .expect("place");
    let dest = scratch.file("for-sharing.pdf");
    std::fs::write(&dest, b"an older copy that must not survive").expect("plant");

    session.save_flattened_to(&dest).expect("written over the older copy");
    assert_ne!(std::fs::read(&dest).expect("read"), b"an older copy that must not survive");
    assert!(Session::open(&dest).is_ok(), "what is at the path is not a document");
    assert_eq!(std::fs::read(&original).expect("read"), before, "the original file was written to");
}
