//! **Marks drawn into the page come back as marks, and are never drawn twice.**
//!
//! Save writes Pagify's own ink into the page under `/PagifyMarks` so every
//! viewer shows it; the geometry stays in the hidden block. This is the other
//! half: the drawing is taken back out when the page's marks are next committed,
//! and a reopen with no edit leaves both the drawing and the geometry alone.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test --release -p pagify_shell --test marks_into_the_page
//! ```

use cad_kernel::{Geom, PolyVertex, Polyline, Vec2};
use pagify_shell::commit;
use pagify_shell::markup::Layer;
use pagify_shell::Session;
use pdf_core::document::{Annotation, Color};

fn fixture(name: &str) -> String {
    format!("{}/../../../rust/pdf_core/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))
}

fn scratch(name: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("pagify-marks-{name}-{}.pdf", std::process::id()));
    let _ = std::fs::remove_file(&path);
    path
}

/// One closed rectangle, the way the markup tool draws it.
fn rectangle_layer(page_height_pt: f64) -> Layer {
    let mut layer = Layer::new(page_height_pt);
    layer.add(Geom::Polyline(Polyline {
        vertices: vec![
            PolyVertex { pos: Vec2::new(100.0, 200.0), bulge: 0.0 },
            PolyVertex { pos: Vec2::new(300.0, 200.0), bulge: 0.0 },
            PolyVertex { pos: Vec2::new(300.0, 400.0), bulge: 0.0 },
            PolyVertex { pos: Vec2::new(100.0, 400.0), bulge: 0.0 },
            PolyVertex { pos: Vec2::new(100.0, 200.0), bulge: 0.0 },
        ],
        closed: true,
        widths: Vec::new(),
    }));
    layer
}

fn pink() -> Color {
    Color { r: 255, g: 92, b: 138, a: 255 }
}

fn ink_annotations(session: &Session) -> usize {
    session
        .annotations(0)
        .expect("annotations")
        .iter()
        .filter(|a| matches!(a.annotation, Annotation::Ink { .. }))
        .count()
}

/// Whether the file draws anything under the tag — decoded, because PDFium
/// compresses the page's content when it saves.
fn holds_the_tag(bytes: &[u8]) -> bool {
    pdf_core::pdf::content::may_name(bytes, b"/PagifyMarks")
}

/// **The whole round trip.** A rectangle is drawn, saved, reopened, edited and
/// written again: one Ink annotation, no tagged drawing left, and a page that
/// looks exactly as it did before the save.
#[test]
fn a_rectangle_saved_reopened_and_edited_is_drawn_once() {
    let path = scratch("once");
    let session = Session::open(fixture("two-column.pdf")).expect("open");
    let height = session.page_size(0).expect("size").height_pt as f64;
    let layer = rectangle_layer(height);

    session.commit_markup(0, &layer, pink(), 1.0).expect("commit");
    assert_eq!(ink_annotations(&session), 1, "setup: the rectangle was not written as ink");
    let before = session.render_page(0, 1.0).expect("render before");

    session.save_to_with_fixups(&path, false, &|_: &str| None).expect("save with fix-ups");
    drop(session);
    let reopened = Session::open(&path).expect("reopen the saved bytes");

    let stored = reopened
        .with_engine(|s| commit::restore_page(&*s.document, 0).map_err(pdf_core::PdfError::InvalidArgument))
        .expect("read the hidden block")
        .expect("the layer came back");
    assert_eq!(stored.version, 2, "the marks were written in a version that does not say the drawing may be in the page");
    assert_eq!(stored.objects.len(), 1, "the rectangle did not come back");
    assert_eq!(ink_annotations(&reopened), 0, "the reopened page still carries the rectangle as a mark");
    assert!(
        holds_the_tag(&reopened.current_bytes().expect("bytes")),
        "the saved page does not draw the rectangle as tagged page content"
    );

    let again = commit::to_layer(&stored);
    reopened.commit_markup(0, &again, pink(), 1.0).expect("commit the restored layer");
    assert_eq!(ink_annotations(&reopened), 1, "the rectangle was not written back as exactly one mark");
    assert!(
        !holds_the_tag(&reopened.current_bytes().expect("bytes")),
        "the drawing is still in the page, so the next save would draw it twice"
    );

    let after = reopened.render_page(0, 1.0).expect("render after");
    assert_eq!(before.pixels, after.pixels, "the page looks different after the edit");
    let _ = std::fs::remove_file(&path);
}

/// **Opening and closing changes nothing.** The drawing is still in the page,
/// the marks are still only in the hidden block, and they still come back.
#[test]
fn reopening_without_editing_leaves_the_drawing_and_restores_the_marks() {
    let path = scratch("reopen");
    let session = Session::open(fixture("two-column.pdf")).expect("open");
    let height = session.page_size(0).expect("size").height_pt as f64;
    session.commit_markup(0, &rectangle_layer(height), pink(), 1.0).expect("commit");
    session.save_to_with_fixups(&path, false, &|_: &str| None).expect("save with fix-ups");
    drop(session);

    let reopened = Session::open(&path).expect("reopen the saved bytes");
    assert_eq!(ink_annotations(&reopened), 0, "the rectangle came back as an annotation");
    assert!(holds_the_tag(&reopened.current_bytes().expect("bytes")), "the drawing is not in the page");

    let stored = reopened
        .with_engine(|s| commit::restore_page(&*s.document, 0).map_err(pdf_core::PdfError::InvalidArgument))
        .expect("read the hidden block")
        .expect("the layer came back");
    assert_eq!(stored.version, 2);
    assert_eq!(stored.objects.len(), 1, "the rectangle did not come back");

    // The drawing is still what the page shows.
    let raster = reopened.render_page(0, 1.0).expect("render");
    let pink = raster
        .pixels
        .chunks_exact(4)
        .filter(|p| p[0] > 200 && (60..140).contains(&p[1]) && (100..180).contains(&p[2]))
        .count();
    assert!(pink > 0, "the page does not draw the rectangle at all");
    let _ = std::fs::remove_file(&path);
}
