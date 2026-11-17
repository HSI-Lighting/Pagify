//! `flatten`: every visible mark made part of its page, as Foxit's Flatten
//! Pages does.
//!
//! Reported from use: a picture placed in Pagify showed as the grey box under
//! it in WhatsApp's preview, and only appeared once the file was downloaded.
//! Save for sharing answered that with a second file; on 2026-11-17 the owner
//! chose Foxit's way instead — Save keeps marks as marks (with their
//! appearance, pictures in the page), and Flatten, on the open document and
//! undoable until it is saved, is for a file whose marks must show where marks
//! are not drawn at all.

use std::path::PathBuf;

use super::*;
use crate::ui_tests::fixture;
use pdf_core::command::Command;
use pdf_core::document::{Annotation, Color, DrawnKind, Rect};

/// A directory of this test's own holding a copy of a fixture.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> (Self, PathBuf) {
        let dir = std::env::temp_dir().join(format!("pagify-flatten-app-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        let original = dir.join("original.pdf");
        std::fs::copy(fixture("two-column.pdf"), &original).expect("copy the fixture");
        (Scratch(dir), original)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const WHERE: Rect = Rect { left: 100.0, top: 200.0, right: 180.0, bottom: 240.0 };

fn add(app: &PagifyApp, annotation: Annotation) {
    app.tab()
        .doc
        .as_ref()
        .expect("the document opened")
        .session
        .execute(Command::AddAnnotation { page_index: 0, annotation })
        .expect("place the mark");
}

/// The document open with a placed picture and a highlight on page 1.
fn open_with_marks(path: &PathBuf) -> PagifyApp {
    let app = PagifyApp::new(Some(&path.to_string_lossy()));
    add(&app, Annotation::Image { rect: WHERE, rgba: [255u8, 0, 0, 255].repeat(16), width: 4, height: 4 });
    add(
        &app,
        Annotation::Highlight {
            rects: vec![Rect { left: 72.0, top: 100.0, right: 200.0, bottom: 114.0 }],
            color: Color { r: 255, g: 235, b: 59, a: 255 },
        },
    );
    app
}

fn said(app: &PagifyApp) -> String {
    app.cmd.history().iter().map(|e| e.text.as_str()).collect::<Vec<_>>().join("\n")
}

fn errors(app: &PagifyApp) -> Vec<String> {
    app.cmd.history().iter().filter(|e| matches!(e.kind, Kind::Error)).map(|e| e.text.clone()).collect()
}

fn marks(app: &PagifyApp) -> usize {
    app.tab().doc.as_ref().expect("open").session.annotations(0).expect("marks").len()
}

/// **The acceptance test.** `flatten` makes the placed picture and the
/// highlight part of the page — the page now draws a picture of its own — and
/// says how many; `undo` puts both back as marks, the picture with its handles.
#[test]
fn flatten_makes_marks_part_of_the_page_and_undo_puts_them_back() {
    let (_scratch, original) = Scratch::new("acceptance");
    let mut app = open_with_marks(&original);
    let before = marks(&app);

    app.submit("flatten");

    let session = app.tab().doc.as_ref().expect("still open").session.clone();
    assert_eq!(marks(&app), before - 2, "the marks are still marks; said {}", said(&app));
    assert!(
        session.drawn_objects(0).expect("drawn").iter().any(|o| o.kind == DrawnKind::Picture && o.depth == 0),
        "the page draws no picture of its own"
    );
    assert!(said(&app).contains("2 marks made part of the pages"), "the person was not told: {}", said(&app));
    assert!(errors(&app).is_empty(), "{:?}", errors(&app));

    app.submit("undo");
    assert_eq!(marks(&app), before, "undo did not put the marks back; said {}", said(&app));
    assert_eq!(session.placed_image_marks(0).expect("placed").len(), 1, "the picture came back without its handles");
}

/// **Save writes what Flatten made.** Flattened, then saved as another file:
/// opened afresh, its page draws the picture and holds neither mark.
#[test]
fn a_flattened_document_saved_holds_the_marks_in_its_pages() {
    let (scratch, original) = Scratch::new("saved");
    let mut app = open_with_marks(&original);
    let before = marks(&app);
    app.submit("flatten");
    let dest = scratch.0.join("flattened.pdf");
    app.submit(&format!("saveas {}", dest.display()));

    let saved = pagify_shell::Session::open(&dest).expect("the saved file opens");
    assert_eq!(saved.annotations(0).expect("marks").len(), before - 2, "a mark was saved as a mark");
    assert!(
        saved.drawn_objects(0).expect("drawn").iter().any(|o| o.kind == DrawnKind::Picture && o.depth == 0),
        "the saved page draws no picture of its own"
    );
}

/// A document with no marks: told so, and nothing changes.
#[test]
fn flatten_with_no_marks_says_so() {
    let (_scratch, original) = Scratch::new("none");
    let mut app = PagifyApp::new(Some(&original.to_string_lossy()));
    let before = marks(&app);
    if before > 0 {
        eprintln!("skipped: the fixture already has marks");
        return;
    }
    app.submit("flatten");
    assert!(said(&app).contains("nothing to flatten"), "{}", said(&app));
    assert!(errors(&app).is_empty(), "{:?}", errors(&app));
}

/// **The break:** the feature existing only for somebody who knows to type
/// `flatten`. It is on the Organize tab where Foxit has it, and Save for
/// sharing is gone from the File tab.
#[test]
fn the_organize_tab_runs_flatten_and_the_file_tab_has_no_save_for_sharing() {
    use crate::ribbon::{Command, Tab};
    let runs = |tab: Tab, verb: &str| tab.buttons().iter().any(|(_, _, command)| matches!(command, Command::Verb(v) if *v == verb));
    assert!(runs(Tab::Organize, "flatten"), "no Organize tab button runs `flatten`");
    assert!(!runs(Tab::File, "sharecopy"), "the File tab still runs `sharecopy`");
}

/// **The file that started this.** The VIVO datasheet is an Illustrator export,
/// and its picture, as a form, could neither be removed nor moved once anything
/// else on the page had been — measured with a probe on 2026-10-14. Flattened,
/// the picture is a picture: it moves after the grey box under it has been
/// moved (the order in the user's own log), and it removes. Skipped when the
/// file is not on this machine, or has no picture placed in it.
#[test]
fn the_vivo_datasheet_flattened_has_a_picture_that_moves_after_the_grey_box_and_removes() {
    use pdf_core::document::pdfium_doc::PdfiumDocument;
    use pdf_core::document::{Document, Point};

    let vivo = std::path::Path::new(r"C:\Users\hsili\Desktop\VIVO mini.pdf");
    if !vivo.exists() {
        eprintln!("skipped: VIVO mini.pdf is not at {}", vivo.display());
        return;
    }
    let (scratch, original) = Scratch::new("vivo");
    std::fs::copy(vivo, &original).expect("copy the datasheet");
    let mut app = PagifyApp::new(Some(&original.to_string_lossy()));
    let stamps = app.tab().doc.as_ref().expect("open").session.placed_image_marks(0).expect("marks").len();
    if stamps == 0 {
        eprintln!("skipped: the datasheet has no placed picture to flatten");
        return;
    }

    app.submit("flatten");
    let dest = scratch.0.join("flattened.pdf");
    app.submit(&format!("saveas {}", dest.display()));
    assert!(dest.exists(), "nothing was saved; said {}", said(&app));

    // **Under the registry lock, the whole of it.** These are engine documents of
    // the test's own, outside any session, and PDFium is not thread-safe: left to
    // overlap the other tests' sessions they killed the whole test process now and
    // then with a native abort (`STATUS_STACK_BUFFER_OVERRUN`, no message) — see
    // `pdf_core::registry::exclusive`.
    pdf_core::registry::exclusive(|| {
        let open = || PdfiumDocument::open_path(&dest.to_string_lossy(), None).expect("the file opens");
        let drawn = open().drawn_objects(0).expect("drawn");
        let near = |o: &pdf_core::document::DrawnObject, l: f32, r: f32, t: f32| {
            (o.rect.left - l).abs() < 1.5 && (o.rect.right - r).abs() < 1.5 && (o.rect.top - t).abs() < 1.5
        };
        let picture = drawn
            .iter()
            .find(|o| o.kind == DrawnKind::Picture && o.depth == 0 && near(o, 33.29, 157.92, 74.78))
            .unwrap_or_else(|| panic!("no picture of its own where the stamp was"));
        let grey = drawn.iter().find(|o| o.kind == DrawnKind::Shape && near(o, 26.78, 168.01, 56.87)).expect("the grey box");

        let mut after_the_box = open();
        after_the_box.move_object(0, grey.object, Point { x: 50.7, y: 25.8 }).expect("the grey box moves");
        after_the_box.move_object(0, grey.object, Point { x: 56.7, y: 56.5 }).expect("the grey box moves again");
        after_the_box.move_object(0, picture.object, Point { x: 38.9, y: 24.8 }).expect("the picture moves after the box");
        open().remove_object(0, picture.object).expect("the picture is removed");
    });
}
