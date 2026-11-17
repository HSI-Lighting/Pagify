//! **A moved, then turned, placed picture** — through the shell the app uses,
//! in both renderers, and put back by undo.
//!
//! **Checked against the picture, not against the code under test.** The
//! engine's own marks are read back too, but the pixel checks are what say the
//! turn is actually drawn: PDFium shows a placed annotation, hayro shows what
//! a preview and any other reader would bake into or draw from the save.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test --release -p pagify_shell --test placed_picture_turn
//! ```

use pagify_shell::Session;
use pdf_core::command::Command;
use pdf_core::document::{Annotation, Rect};

fn fixture(name: &str) -> String {
    format!("{}/../../../rust/pdf_core/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))
}

/// An 8 x 4 opaque black picture — no pixel ambiguous with paper.
fn black() -> Vec<u8> {
    let mut rgba = vec![0u8; 8 * 4 * 4];
    for pixel in rgba.chunks_exact_mut(4) {
        pixel[3] = 255;
    }
    rgba
}

fn sample(raster: &pagify_shell::session::PageRaster, x: f32, y: f32) -> u8 {
    let (px, _, _) = raster.crop(x as u32, y as u32, x as u32, y as u32);
    px[0]
}

#[test]
fn a_moved_picture_turns_in_both_renderers_and_undo_puts_it_back() {
    let session = Session::open(fixture("two-column.pdf")).expect("open");
    let frame = Rect { left: 300.0, top: 300.0, right: 500.0, bottom: 400.0 };
    let moved = Rect { left: 300.0, top: 400.0, right: 500.0, bottom: 500.0 };

    session
        .execute(Command::AddAnnotation {
            page_index: 0,
            annotation: Annotation::Image { rect: frame, rgba: black(), width: 8, height: 4 },
        })
        .expect("place");
    let idx = session.placed_image_marks(0).expect("marks")[0].index;

    session.set_image_signature_rect(0, idx, moved, frame).expect("move");
    session.rotate_image_signature(0, idx, 90.0, 0.0).expect("turn");

    // PDFium: the placed annotation as the app's own renderer draws it.
    let pdfium = session.render_page(0, 1.0).expect("render");
    // hayro: what a preview draws from the current bytes.
    let bytes = session.current_bytes().expect("bytes");
    let preview = pagify_shell::preview::Preview::new(bytes).expect("preview");
    let hayro = preview.render_scaled(0, 1.0).expect("preview render");

    for (name, raster) in [("PDFium", &pdfium), ("hayro", &hayro)] {
        for (x, y) in [(400.0, 370.0), (400.0, 450.0), (400.0, 530.0)] {
            assert!(
                sample(raster, x, y) < 60,
                "{name}: the turned picture is missing at ({x}, {y}): {}",
                sample(raster, x, y)
            );
        }
        for (x, y) in [(310.0, 450.0), (490.0, 450.0)] {
            assert!(
                sample(raster, x, y) > 200,
                "{name}: there is ink outside the turned picture at ({x}, {y}): {}",
                sample(raster, x, y)
            );
        }
    }

    let mark = &session.placed_image_marks(0).expect("marks")[0];
    assert_eq!(mark.rect, moved, "the read-back rect should be where it was moved");
    assert!((mark.rotation - 90.0).abs() < 0.01, "rotation was not read back: {}", mark.rotation);

    session.undo().expect("undo the turn");
    session.undo().expect("undo the move");

    let mark = &session.placed_image_marks(0).expect("marks")[0];
    assert_eq!(mark.rect, frame, "undo did not put the picture back");
    assert_eq!(mark.rotation, 0.0, "undo did not put the angle back");
    let back = session.render_page(0, 1.0).expect("render");
    assert!(sample(&back, 400.0, 530.0) > 200, "the un-turned picture is still drawn where the turn put it");
}
