//! `Session::render_page_hiding`: the page drawn without some of its objects, for the
//! picture Edit Text puts under a paragraph being retyped. The engine's own checks are
//! `rust/pdf_core/tests/render_hiding.rs`; this holds the session's wrapper to them.

use pagify_shell::session::PageRaster;
use pagify_shell::Session;
use pdf_core::document::Rect;
use pdf_core::Rotation;

const SCALE: f32 = 2.0;

fn fixture(name: &str) -> String {
    format!("{}/../../../rust/pdf_core/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))
}

/// How many pixels of `raster` inside `rect` (page points) are dark.
fn inked_in(raster: &PageRaster, rect: &Rect) -> usize {
    let mut inked = 0;
    for y in 0..raster.height {
        for x in 0..raster.width {
            let (px, py) = (x as f32 / SCALE, y as f32 / SCALE);
            if px >= rect.left && px <= rect.right && py >= rect.top && py <= rect.bottom {
                let at = (y as usize * raster.width as usize + x as usize) * 4;
                if raster.pixels[at].min(raster.pixels[at + 1]).min(raster.pixels[at + 2]) < 128 {
                    inked += 1;
                }
            }
        }
    }
    inked
}

#[test]
fn the_session_draws_a_page_without_a_word_it_is_told_to_hide() {
    let session = Session::open(fixture("two-column.pdf")).expect("open");
    let run = session.text_runs(0).expect("runs").into_iter().find(|r| r.text.trim().len() > 3).expect("a word");
    let plain = session.render_page(0, SCALE).expect("plain");
    let hidden = session.render_page_hiding(0, SCALE, Rotation::None, &[run.object]).expect("hidden");

    assert_eq!((hidden.width, hidden.height), (plain.width, plain.height));
    let before = inked_in(&plain, &run.rect);
    assert!(before > 20, "the word must show in the plain render ({before} inked pixels), or this proves nothing");
    // Neighbouring letters may touch the edge of the word's box; the word itself is gone.
    assert!(inked_in(&hidden, &run.rect) * 4 <= before, "the hidden word is still drawn");
    assert!(!hidden.from_cache, "a hiding render is never a cached picture");
}

#[test]
fn an_object_that_is_not_there_is_an_error_not_a_blank_page() {
    let session = Session::open(fixture("two-column.pdf")).expect("open");
    assert!(session.render_page_hiding(0, SCALE, Rotation::None, &[99_999]).is_err());
}
