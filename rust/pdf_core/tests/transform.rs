//! Resizing and fading one thing on a page, without touching anything else.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test --test transform
//! ```

mod harness;
use harness::{serial, skip_without_pdfium};

use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Document, DocumentMut, DrawnKind, Point};

fn open(name: &str) -> PdfiumDocument {
    PdfiumDocument::open_path(harness::fixture_path(name).to_str().expect("path"), None)
        .expect("open")
}

/// Every pixel of a render, with its position, for asking where a colour is.
fn render(doc: &dyn Document) -> (u32, Vec<u8>) {
    let size = doc.page_size(0).expect("size");
    let width = 306u32;
    let scale = width as f32 / size.width_pt;
    let height = (size.height_pt * scale).max(1.0) as u32;
    let mut pixels = vec![0u8; (width * height * 4) as usize];
    let mut target = pdf_core::render::RenderTarget {
        width,
        height,
        stride: (width * 4) as usize,
        order: pdf_core::render::PixelOrder::Rgba,
        pixels: &mut pixels,
    };
    doc.page(0)
        .expect("page")
        .render_into(
            &pdf_core::document::RenderRequest { scale, ..Default::default() },
            &mut target,
        )
        .expect("render");
    (width, pixels)
}

/// Where the fixture's red picture is drawn: its pixel count and its box.
fn red_box(doc: &dyn Document) -> (usize, (u32, u32, u32, u32)) {
    let (width, pixels) = render(doc);
    let (mut n, mut l, mut t, mut r, mut b) = (0usize, u32::MAX, u32::MAX, 0u32, 0u32);
    for (i, p) in pixels.chunks_exact(4).enumerate() {
        if p[0] > 180 && p[1] < 80 && p[2] < 80 {
            let (x, y) = ((i as u32) % width, (i as u32) / width);
            n += 1;
            l = l.min(x);
            t = t.min(y);
            r = r.max(x);
            b = b.max(y);
        }
    }
    (n, (l, t, r, b))
}

/// **A picture resized about a corner keeps that corner and shrinks the
/// rest** — frame, placeholder and all.
#[test]
fn a_picture_resizes_about_the_anchor_and_keeps_its_frame() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let doc = open("framed.pdf");
    let picture = doc
        .drawn_objects(0)
        .expect("objects")
        .into_iter()
        .find(|d| d.kind == DrawnKind::Picture)
        .expect("the picture");
    let (was, (l0, t0, r0, b0)) = red_box(&doc);
    assert!(was > 500, "the fixture should draw the picture");
    drop(doc);

    // Half the size, holding the top-left corner still.
    let mut doc = open("framed.pdf");
    doc.scale_object(
        0,
        picture.object,
        Point { x: picture.rect.left, y: picture.rect.top },
        0.5,
        0.5,
    )
    .expect("resize");

    let (now, (l1, t1, r1, b1)) = red_box(&doc);
    assert!(
        (now as f32) > (was as f32) * 0.2 && (now as f32) < (was as f32) * 0.3,
        "half the size each way should be a quarter of the pixels: {now} of {was}"
    );
    // The anchored corner stayed; the far corner came in by half.
    assert!((l1 as i32 - l0 as i32).abs() <= 1 && (t1 as i32 - t0 as i32).abs() <= 1, "the anchor moved");
    let (w0, h0, w1, h1) = (r0 - l0, b0 - t0, r1 - l1, b1 - t1);
    assert!((w1 as f32 - w0 as f32 / 2.0).abs() <= 2.0, "width {w0} should have halved, is {w1}");
    assert!((h1 as f32 - h0 as f32 / 2.0).abs() <= 2.0, "height {h0} should have halved, is {h1}");

    // The picture is still the picture: it did not get cut by a frame that
    // stayed the old size.
    let listed = doc.drawn_objects(0).expect("objects");
    let still = listed.iter().find(|d| d.kind == DrawnKind::Picture).expect("still there");
    assert!((still.rect.right - still.rect.left - (picture.rect.right - picture.rect.left) / 2.0).abs() < 1.0);
}

/// **A shape resizes too**, and the words around it do not.
#[test]
fn a_shape_resizes_and_the_words_do_not() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let doc = open("covered.pdf");
    let panel = doc
        .drawn_objects(0)
        .expect("objects")
        .into_iter()
        .find(|d| d.kind == DrawnKind::Shape)
        .expect("the panel");
    let words_before: Vec<_> = doc.text_runs(0).expect("runs").into_iter().map(|r| r.rect).collect();
    drop(doc);

    let mut doc = open("covered.pdf");
    doc.scale_object(
        0,
        panel.object,
        Point { x: panel.rect.right, y: panel.rect.bottom },
        1.5,
        1.5,
    )
    .expect("resize the panel");

    let now = doc
        .drawn_objects(0)
        .expect("objects")
        .into_iter()
        .find(|d| d.kind == DrawnKind::Shape)
        .expect("the panel");
    assert!(((now.rect.right - now.rect.left) / (panel.rect.right - panel.rect.left) - 1.5).abs() < 0.02);
    assert!((now.rect.right - panel.rect.right).abs() < 0.5 && (now.rect.bottom - panel.rect.bottom).abs() < 0.5, "the anchor corner moved");

    let words_after: Vec<_> = doc.text_runs(0).expect("runs").into_iter().map(|r| r.rect).collect();
    assert_eq!(words_before.len(), words_after.len());
    for (a, b) in words_before.iter().zip(&words_after) {
        assert!((a.left - b.left).abs() < 0.5 && (a.top - b.top).abs() < 0.5, "resizing the panel moved the words");
    }
}

/// **Opacity is absolute.** Set twice, it is set twice — not multiplied.
#[test]
fn opacity_fades_one_thing_and_setting_it_again_does_not_compound() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    fn centre_pixel(doc: &dyn Document, at: pdf_core::document::Rect) -> (u8, u8, u8) {
        let (width, pixels) = render(doc);
        let scale = 306.0 / doc.page_size(0).expect("size").width_pt;
        let x = (((at.left + at.right) / 2.0) * scale) as u32;
        let y = (((at.top + at.bottom) / 2.0) * scale) as u32;
        let i = ((y * width + x) * 4) as usize;
        (pixels[i], pixels[i + 1], pixels[i + 2])
    }

    let mut doc = open("framed.pdf");
    let picture = doc
        .drawn_objects(0)
        .expect("objects")
        .into_iter()
        .find(|d| d.kind == DrawnKind::Picture)
        .expect("the picture");
    let solid = centre_pixel(&doc, picture.rect);
    assert!(solid.0 > 180 && solid.1 < 80, "the picture should start solid red: {solid:?}");

    doc.set_opacity(0, picture.object, 0.5).expect("fade");
    let faded = centre_pixel(&doc, picture.rect);
    // Half red over the page: the green and blue that red was hiding come
    // through. (The placeholder fades with it — it is part of the picture —
    // so what shows through is the white page, and red itself barely drops.)
    assert!(faded.1 > solid.1 + 40 && faded.2 > solid.2 + 40, "it did not fade: {solid:?} then {faded:?}");
    let listed = doc.drawn_objects(0).expect("objects");
    let reported = listed.iter().find(|d| d.kind == DrawnKind::Picture).expect("still there").opacity;
    assert!((reported - 0.5).abs() < 0.02, "the list should report the new opacity, got {reported}");

    // Again at the same value: identical, not a quarter.
    let object = listed.iter().find(|d| d.kind == DrawnKind::Picture).expect("it").object;
    doc.set_opacity(0, object, 0.5).expect("fade again");
    let again = centre_pixel(&doc, picture.rect);
    assert!((again.0 as i32 - faded.0 as i32).abs() <= 2, "opacity compounded: {faded:?} then {again:?}");

    // And back to solid.
    let object = doc.drawn_objects(0).expect("objects").into_iter().find(|d| d.kind == DrawnKind::Picture).expect("it").object;
    doc.set_opacity(0, object, 1.0).expect("solid");
    let back = centre_pixel(&doc, picture.rect);
    assert!((back.0 as i32 - solid.0 as i32).abs() <= 2, "it did not come back solid: {back:?}");
}
