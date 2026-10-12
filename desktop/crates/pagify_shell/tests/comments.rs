//! `managecomments`' listing, against a document with a mark added by the
//! test itself: the round trip that matters is that the index the list hands
//! out is the index the removal takes.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test -p pagify_shell --test comments
//! ```

use pagify_shell::{comments, Session};

fn fixture(name: &str) -> String {
    format!(
        "{}/../../../rust/pdf_core/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    )
}

#[test]
fn a_note_lands_in_the_list_and_can_be_removed_by_its_listed_index() {
    let session = Session::open(fixture("two-column.pdf")).expect("open");
    assert!(
        comments::comments(&session).expect("list").is_empty(),
        "the fixture starts with no annotations"
    );

    session
        .execute(pdf_core::command::Command::AddAnnotation {
            page_index: 0,
            annotation: pdf_core::document::Annotation::Note {
                rect: pdf_core::document::Rect {
                    left: 40.0,
                    top: 40.0,
                    right: 60.0,
                    bottom: 60.0,
                },
                contents: "check this figure".into(),
                color: pdf_core::document::Color { r: 255, g: 220, b: 0, a: 255 },
            },
        })
        .expect("add a note");

    let listed = comments::comments(&session).expect("list");
    assert_eq!(listed.len(), 1, "one note was added: {listed:?}");
    let row = &listed[0];
    assert_eq!(row.page, 0);
    assert_eq!(row.kind, "note");
    assert!(row.detail.contains("check this figure"), "no contents: {row:?}");

    session
        .execute(pdf_core::command::Command::RemoveAnnotation {
            page_index: row.page,
            index: row.index,
        })
        .expect("remove by the listed index");

    assert!(
        comments::comments(&session).expect("list").is_empty(),
        "the removal did not take the listed annotation"
    );
}

#[test]
fn a_picture_reads_as_its_pixel_size() {
    let session = Session::open(fixture("two-column.pdf")).expect("open");
    session
        .execute(pdf_core::command::Command::AddAnnotation {
            page_index: 0,
            annotation: pdf_core::document::Annotation::Image {
                rect: pdf_core::document::Rect {
                    left: 100.0,
                    top: 100.0,
                    right: 140.0,
                    bottom: 130.0,
                },
                rgba: vec![0u8; 4 * 8 * 6],
                width: 8,
                height: 6,
            },
        })
        .expect("add a picture");

    let listed = comments::comments(&session).expect("list");
    let row = listed.first().expect("one row");
    assert_eq!(row.kind, "picture");
    assert_eq!(row.detail, "8×6");
}
