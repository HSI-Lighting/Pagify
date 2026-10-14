//! `sharecopy`: a copy of the open document for sending, its placed pictures
//! baked into the pages.
//!
//! Reported from use: a picture placed in Pagify showed as the grey box under
//! it in WhatsApp's preview, and only appeared once the file was downloaded.

use std::path::PathBuf;

use super::*;
use crate::ui_tests::fixture;
use pdf_core::command::Command;
use pdf_core::document::{Annotation, Color, DrawnKind, Rect};

/// A directory of this test's own holding a copy of a fixture, so that the file
/// the app opens is one whose bytes the test can compare before and after.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> (Self, PathBuf) {
        let dir = std::env::temp_dir().join(format!("pagify-share-copy-app-{name}"));
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

fn open_with_a_picture(path: &PathBuf) -> PagifyApp {
    let app = PagifyApp::new(Some(&path.to_string_lossy()));
    app.tab()
        .doc
        .as_ref()
        .expect("the document opened")
        .session
        .execute(Command::AddAnnotation {
            page_index: 0,
            annotation: Annotation::Image { rect: WHERE, rgba: [255u8, 0, 0, 255].repeat(16), width: 4, height: 4 },
        })
        .expect("place the picture");
    app
}

fn said(app: &PagifyApp) -> String {
    app.cmd.history().iter().map(|e| e.text.as_str()).collect::<Vec<_>>().join("\n")
}

fn errors(app: &PagifyApp) -> Vec<String> {
    app.cmd.history().iter().filter(|e| matches!(e.kind, Kind::Error)).map(|e| e.text.clone()).collect()
}

/// **The acceptance test.** The command writes a file; opened afresh its page
/// draws the picture itself; and the document still open holds it as a placed
/// picture, so it can be dragged by its handles as before.
#[test]
fn sharecopy_writes_a_copy_whose_page_holds_the_picture_and_leaves_the_open_document_alone() {
    let (scratch, original) = Scratch::new("acceptance");
    let mut app = open_with_a_picture(&original);
    let dest = scratch.0.join("for sharing.pdf");

    app.submit(&format!("sharecopy {}", dest.display()));

    assert!(dest.exists(), "no copy was written; said {}", said(&app));
    let copy = pagify_shell::Session::open(&dest).expect("the copy opens on its own");
    assert!(
        copy.drawn_objects(0).expect("drawn").iter().any(|o| o.kind == DrawnKind::Picture && o.depth == 0),
        "the copy's page draws no picture of its own"
    );
    let session = &app.tab().doc.as_ref().expect("still open").session;
    assert_eq!(session.placed_image_marks(0).expect("marks").len(), 1, "the open document lost its placed picture");
    assert!(said(&app).contains("1 picture"), "the person was not told what went in: {}", said(&app));
    assert!(errors(&app).is_empty(), "{:?}", errors(&app));
}

/// **The file that started this.** The VIVO datasheet is an Illustrator export,
/// and its picture, as a form, could neither be removed nor moved once anything
/// else on the page had been — measured with a probe on 2026-10-14. Made as the
/// copy writes it, the picture is a picture: it moves after the grey box under it
/// has been moved (the order in the user's own log), and it removes. Skipped when
/// the file is not on this machine, or has no picture placed in it.
#[test]
fn the_vivo_datasheets_copy_has_a_picture_that_moves_after_the_grey_box_and_removes() {
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
        eprintln!("skipped: the datasheet has no placed picture to bake in");
        return;
    }

    let dest = scratch.0.join("for sharing.pdf");
    app.submit(&format!("sharecopy {}", dest.display()));
    assert!(dest.exists(), "no copy was written; said {}", said(&app));

    // **Under the registry lock, the whole of it.** These are engine documents of
    // the test's own, outside any session, and PDFium is not thread-safe: left to
    // overlap the other tests' sessions they killed the whole test process now and
    // then with a native abort (`STATUS_STACK_BUFFER_OVERRUN`, no message) — see
    // `pdf_core::registry::exclusive`.
    pdf_core::registry::exclusive(|| {
        let open = || PdfiumDocument::open_path(&dest.to_string_lossy(), None).expect("the copy opens");
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

/// **The break:** the feature existing only for somebody who knows to type
/// `sharecopy`. Save As is on the File tab; this sits beside it.
#[test]
fn the_file_tab_has_a_button_that_asks_for_a_copy_to_send() {
    use crate::ribbon::{Command, Tab};
    let ask = Tab::File.buttons().iter().find(|(_, _, command)| matches!(command, Command::Verb("sharecopy")));
    assert!(ask.is_some(), "no File tab button runs `sharecopy`");
}

/// **The break this catches:** `sharecopy` aimed at the document's own file —
/// the one way to flatten the original, which could then no longer be dragged by
/// its handles. Nothing is written, the person is told — and the same document
/// makes a copy at any other path straight after, so this is a refusal of that
/// path and not of the command.
#[test]
fn sharecopy_over_the_open_documents_own_file_writes_nothing_and_says_so() {
    let (scratch, original) = Scratch::new("own-file");
    let mut app = open_with_a_picture(&original);
    let before = std::fs::read(&original).expect("read");

    app.submit(&format!("sharecopy {}", original.display()));
    assert_eq!(std::fs::read(&original).expect("read"), before, "the original file was written to");
    assert!(!errors(&app).is_empty(), "nothing said why; said {}", said(&app));

    let elsewhere = scratch.0.join("for sharing.pdf");
    app.submit(&format!("sharecopy {}", elsewhere.display()));
    assert!(elsewhere.exists(), "the refusal was of the command, not of the path; said {}", said(&app));
}

/// **The break:** a highlight in the document that the copy cannot bake — it is
/// a colour and some coordinates, with no appearance — going unmentioned, so
/// the person finds out from the recipient. It is said.
#[test]
fn sharecopy_says_how_many_marks_a_preview_will_still_not_show() {
    let (scratch, original) = Scratch::new("bare");
    let mut app = open_with_a_picture(&original);
    app.tab()
        .doc
        .as_ref()
        .expect("open")
        .session
        .execute(Command::AddAnnotation {
            page_index: 0,
            annotation: Annotation::Highlight {
                rects: vec![Rect { left: 72.0, top: 100.0, right: 200.0, bottom: 114.0 }],
                color: Color { r: 255, g: 235, b: 59, a: 255 },
            },
        })
        .expect("highlight");

    app.submit(&format!("sharecopy {}", scratch.0.join("for sharing.pdf").display()));

    assert!(said(&app).contains("1 mark"), "the highlight was not mentioned: {}", said(&app));
}
