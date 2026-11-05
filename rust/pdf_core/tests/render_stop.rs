//! A render that can be given up part way (`Page::render_into_until`): with
//! nothing stopping it, it draws exactly what `render_into` draws; told to stop,
//! it stops, and the page draws whole again after.

mod harness;
use harness::{helvetica_page, open_fixture, serial, skip_without_pdfium};

use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Document, Page, RenderRequest, Rotation};
use pdf_core::render::{PixelOrder, RenderTarget};

/// Page `page` at 1.5 px/pt, turned by `rotation`: its pixels, and whether it
/// was drawn to the end.
fn draw(doc: &dyn Document, page: usize, rotation: Rotation, stop: Option<&mut dyn FnMut() -> bool>) -> (Vec<u8>, bool) {
    draw_on(&*doc.page(page).expect("page"), rotation, stop)
}

fn draw_on(page: &dyn Page, rotation: Rotation, stop: Option<&mut dyn FnMut() -> bool>) -> (Vec<u8>, bool) {
    let size = page.size();
    let (w, h) = ((size.width_pt * 1.5).round() as u32, (size.height_pt * 1.5).round() as u32);
    let (w, h) = if matches!(rotation, Rotation::Clockwise90 | Rotation::Clockwise270) { (h, w) } else { (w, h) };
    let request = RenderRequest { scale: 1.5, rotation, ..Default::default() };
    let mut pixels = vec![0u8; w as usize * h as usize * 4];
    let finished = {
        let mut target = RenderTarget { width: w, height: h, stride: w as usize * 4, order: PixelOrder::Rgba, pixels: &mut pixels };
        match stop {
            None => page.render_into(&request, &mut target).map(|_| true),
            Some(stop) => page.render_into_until(&request, &mut target, stop),
        }
        .expect("render")
    };
    (pixels, finished)
}

/// A page of `lines` short strokes: thousands of objects, as a CAD page has
/// hundreds of thousands, for PDFium to pause between.
fn lines(count: usize) -> PdfiumDocument {
    let mut content = Vec::new();
    for i in 0..count {
        let (x, y) = ((i % 100) as f32 * 5.0 + 20.0, (i / 100) as f32 * 7.0 + 20.0);
        content.extend_from_slice(format!("{x} {y} m {} {} l S\n", x + 4.0, y + 5.0).as_bytes());
    }
    PdfiumDocument::open_bytes(helvetica_page(&content), None).expect("open")
}

#[test]
fn with_nothing_stopping_it_a_page_draws_what_it_always_drew() {
    let Some(()) = skip_without_pdfium() else { return };
    let _serial = serial();
    let rotations = [Rotation::None, Rotation::Clockwise90, Rotation::Clockwise180, Rotation::Clockwise270];
    for name in ["forms.pdf", "pictures.pdf", "hidden-things.pdf", "pages-ladder.pdf"] {
        let doc = open_fixture(&(), name);
        for page in 0..doc.page_count() {
            for rotation in rotations {
                let (before, _) = draw(&*doc, page, rotation, None);
                let (after, finished) = draw(&*doc, page, rotation, Some(&mut || false));
                assert!(finished, "{name} page {page} {rotation:?}: stopped with nothing to stop it");
                assert!(before == after, "{name} page {page} {rotation:?}: the pixels differ");
            }
        }
    }
    let busy = lines(3000);
    let (before, _) = draw(&busy, 0, Rotation::None, None);
    let mut asked = 0;
    let (after, finished) = draw(&busy, 0, Rotation::None, Some(&mut || {
        asked += 1;
        false
    }));
    assert!(finished && before == after, "the busy page drew differently");
    assert!(asked > 1, "PDFium asked whether to stop {asked} times over 3,000 objects");
}

#[test]
fn told_to_stop_a_render_stops_and_the_page_still_draws_after() {
    let Some(()) = skip_without_pdfium() else { return };
    let _serial = serial();
    let busy = lines(3000);
    let (whole, _) = draw(&busy, 0, Rotation::None, None);

    // One load of the page for both: the render given up must leave it drawable.
    let page = busy.page(0).expect("page");
    let mut asked = 0;
    let (_, finished) = draw_on(&*page, Rotation::None, Some(&mut || {
        asked += 1;
        asked >= 2
    }));
    assert!(!finished, "the render went on after it was told to stop");
    assert_eq!(asked, 2, "it was asked again after it said stop");

    let (again, finished) = draw_on(&*page, Rotation::None, None);
    assert!(finished && again == whole, "the page no longer draws whole after a render was given up");
}
