//! Copying drawings as lines: lifting a page's own operators into a clip and
//! writing them into the target page as lines of its own (`Pagify-Vector-Copy-Plan.md`).
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> PAGIFY_QPDF=<qpdf> cargo test --release --test lift -- --nocapture
//! ```
//!
//! Reported: the lens drawing in VIVO mini, copied with Edit Object and
//! pasted, came back visibly softer, because a copy was a picture of the page.

mod harness;
use harness::{hand_built_pdf, hand_built_pdf_with, serial, skip_without_pdfium, HandPage};

use pdf_core::command::{Command, CommandHistory};
use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Document, DocumentMut, DrawnKind, DrawnObject, Lifted, Point, Rect, RegionRequest};
use pdf_core::error::PdfError;
use pdf_core::pdf::{File, Object};
use pdf_core::render::{Bitmap, PixelOrder};

fn open(bytes: Vec<u8>) -> PdfiumDocument {
    PdfiumDocument::open_bytes(bytes, None).expect("open")
}

/// `(x, y)` through a PDF matrix.
fn apply(m: [f32; 6], x: f32, y: f32) -> (f32, f32) {
    (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5])
}

#[test]
fn page_space_maps_to_pdf_space_on_every_rotation_and_offset_box() {
    let Some(()) = skip_without_pdfium() else { return };
    let _lock = serial();
    let page = |extra| HandPage { content: "0 0 1 rg 100 200 50 30 re f", resources: "<< >>", extra };
    let doc = open(hand_built_pdf(
        &[
            page("/MediaBox [10 20 410 620] /CropBox [30 40 390 600]"),
            page("/MediaBox [10 20 410 620] /CropBox [30 40 390 600] /Rotate 90"),
            page("/MediaBox [10 20 410 620] /CropBox [30 40 390 600] /Rotate 180"),
            page("/MediaBox [10 20 410 620] /CropBox [30 40 390 600] /Rotate 270"),
        ],
        &[],
    ));
    for index in 0..4 {
        let drawn = doc.drawn_objects(index).expect("drawn");
        assert_eq!(drawn.len(), 1, "page {index}");
        let r = drawn[0].rect;
        let (m, turns) = doc.page_to_user_for_probe(index).expect("matrix");
        assert_eq!(turns as usize, index, "quarter turns of page {index}");
        let (x0, y0) = apply(m, r.left, r.bottom);
        let (x1, y1) = apply(m, r.right, r.top);
        let (lo_x, hi_x, lo_y, hi_y) = (x0.min(x1), x0.max(x1), y0.min(y1), y0.max(y1));
        for (got, want) in [(lo_x, 100.0), (lo_y, 200.0), (hi_x, 150.0), (hi_y, 230.0)] {
            assert!((got - want).abs() < 0.01, "page {index}: {got} for {want}, rect {r:?}, matrix {m:?}");
        }
    }
}

// ------------------------------------------------------------------ helpers --

/// What the page itself draws (not what its groups draw inside them).
fn top_level(doc: &PdfiumDocument, page: usize) -> Vec<DrawnObject> {
    doc.drawn_objects(page).expect("drawn").into_iter().filter(|d| d.depth == 0).collect()
}

/// `crop` of the page rendered at `scale`, cut out of a whole-page render: a
/// sub-page crop through `render_region` came back blank on these pages
/// (2026-10-13, see the plan ledger), so the crop is taken here.
fn region(doc: &PdfiumDocument, page: usize, crop: Rect, scale: f32) -> Bitmap {
    let size = doc.page_size(page).expect("size");
    let whole_page = Rect { left: 0.0, top: 0.0, right: size.width_pt, bottom: size.height_pt };
    let mut whole = doc
        .page(page)
        .expect("page")
        .render_region(&RegionRequest { crop: whole_page, scale, render_annotations: false, render_form_data: false })
        .expect("render");
    whole.convert_to(PixelOrder::Rgba);
    let (x0, y0) = ((crop.left * scale).round() as usize, (crop.top * scale).round() as usize);
    let w = ((crop.right - crop.left) * scale).round() as u32;
    let h = ((crop.bottom - crop.top) * scale).round() as u32;
    let mut out = Bitmap::new(w, h, PixelOrder::Rgba).expect("bitmap");
    for y in 0..h as usize {
        let from = (y0 + y) * whole.stride + x0 * 4;
        out.data[y * out.stride..y * out.stride + w as usize * 4].copy_from_slice(&whole.data[from..from + w as usize * 4]);
    }
    out
}

/// Mean absolute difference per colour channel, 0..255.
fn mean_difference(a: &Bitmap, b: &Bitmap) -> f32 {
    assert_eq!((a.width, a.height), (b.width, b.height), "renders of different sizes");
    let mut sum = 0u64;
    for y in 0..a.height as usize {
        for x in 0..a.width as usize {
            for c in 0..3 {
                let (p, q) = (a.data[y * a.stride + x * 4 + c], b.data[y * b.stride + x * 4 + c]);
                sum += p.abs_diff(q) as u64;
            }
        }
    }
    sum as f32 / (a.width as f32 * a.height as f32 * 3.0)
}

fn centre_colour(doc: &PdfiumDocument, page: usize, r: Rect) -> [u8; 3] {
    let b = region(doc, page, r, 1.0);
    let (x, y) = (b.width as usize / 2, b.height as usize / 2);
    let at = y * b.stride + x * 4;
    [b.data[at], b.data[at + 1], b.data[at + 2]]
}

fn near(a: Rect, b: Rect, within: f32) -> bool {
    (a.left - b.left).abs() <= within
        && (a.top - b.top).abs() <= within
        && (a.right - b.right).abs() <= within
        && (a.bottom - b.bottom).abs() <= within
}

/// A page's own dictionary in a file, walked from the catalogue (one level of `/Kids`).
fn page_dict(file: &File<'_>, index: usize) -> pdf_core::pdf::Dict {
    let root = file.resolve(file.trailer().get(b"Root").unwrap()).unwrap();
    let pages = file.resolve(root.as_dict().unwrap().get(b"Pages").unwrap()).unwrap();
    let Object::Array(kids) = file.resolve(pages.as_dict().unwrap().get(b"Kids").unwrap()).unwrap() else {
        panic!("no kids")
    };
    file.resolve(&kids[index]).unwrap().as_dict().unwrap().clone()
}

/// What a refusal must leave alone: every page's stream and pictures (the count is the vec's length).
fn fingerprint(doc: &PdfiumDocument) -> Vec<(Vec<u8>, Vec<(Vec<u8>, u32)>)> {
    let bytes = doc.readable_bytes_for_probe().expect("bytes");
    let file = File::parse(&bytes).expect("parse");
    (0..doc.page_count())
        .map(|p| (doc.page_stream(p).expect("stream"), doc.xobject_names_for_probe(&file, p).expect("names")))
        .collect()
}

const GREY_IMAGE: &str =
    "<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceGray /Length 1 >>\nstream\nx\nendstream";

// --------------------------------------------------------------------- lift --

#[test]
fn lifting_shapes_gives_a_page_that_draws_exactly_them() {
    let Some(()) = skip_without_pdfium() else { return };
    let _lock = serial();
    let doc = open(hand_built_pdf(
        &[HandPage {
            content: "1 0 0 rg 10 600 50 50 re f 0 1 0 rg 100 600 50 50 re f 0 0 1 rg 200 600 50 50 re f",
            resources: "<< >>",
            extra: "",
        }],
        &[],
    ));
    let source = top_level(&doc, 0);
    assert_eq!(source.len(), 3);

    let lifted: Lifted = doc.lift_objects(0, &[source[0].object, source[2].object]).expect("lift");
    assert_eq!(lifted.objects, 2);
    let clip = open(lifted.pdf.clone());
    let drawn = top_level(&clip, 0);
    assert_eq!(drawn.len(), 2, "{drawn:?}");
    for (got, want) in drawn.iter().zip([&source[0], &source[2]]) {
        assert_eq!(got.kind, DrawnKind::Shape);
        assert!(near(got.rect, want.rect, 0.5), "{:?} for {:?}", got.rect, want.rect);
    }
    assert_eq!(centre_colour(&clip, 0, drawn[0].rect), [255, 0, 0]);
    assert_eq!(centre_colour(&clip, 0, drawn[1].rect), [0, 0, 255]);
    let union = Rect {
        left: source[0].rect.left,
        top: source[0].rect.top,
        right: source[2].rect.right,
        bottom: source[2].rect.bottom,
    };
    assert!(near(lifted.rect, union, 0.5), "{:?}", lifted.rect);
}

#[test]
fn a_picture_is_lifted_at_its_own_resolution() {
    let Some(()) = skip_without_pdfium() else { return };
    let _lock = serial();
    let doc = PdfiumDocument::open_path(harness::fixture_path("pictures.pdf").to_str().unwrap(), None).expect("open");
    let picture = doc.images_on(0).expect("images")[0].clone();

    let lifted = doc.lift_objects(0, &[picture.object]).expect("lift");
    let clip = open(lifted.pdf.clone());
    let pictures = clip.images_on(0).expect("clip images");
    assert_eq!(pictures.len(), 1);
    assert_eq!((pictures[0].pixel_width, pictures[0].pixel_height), (picture.pixel_width, picture.pixel_height));
    let file = File::parse(&lifted.pdf).expect("parse clip");
    assert_eq!(clip.xobject_names_for_probe(&file, 0).expect("names").len(), 1, "the clip holds the other picture too");
}

#[test]
fn a_shape_inside_a_clip_is_lifted_with_its_clip() {
    let Some(()) = skip_without_pdfium() else { return };
    let _lock = serial();
    // A circle seen through a square: rounded corners only if the clip travels.
    let doc = open(hand_built_pdf(
        &[HandPage {
            content: "q 120 620 50 50 re W n 0 0 1 rg 190 650 m 190 672.09 172.09 690 150 690 c \
                      127.91 690 110 672.09 110 650 c 110 627.91 127.91 610 150 610 c \
                      172.09 610 190 627.91 190 650 c f Q",
            resources: "<< >>",
            extra: "",
        }],
        &[],
    ));
    let shape = top_level(&doc, 0);
    assert_eq!(shape.len(), 1);
    let lifted = doc.lift_objects(0, &[shape[0].object]).expect("lift");
    let clip = open(lifted.pdf);
    let area = Rect { left: 100.0, top: 92.0, right: 200.0, bottom: 192.0 };
    let difference = mean_difference(&region(&doc, 0, area, 2.0), &region(&clip, 0, area, 2.0));
    assert!(difference < 1.0, "the lifted shape draws differently: {difference}");
}

#[test]
fn a_framed_picture_brings_its_frame_and_still_verifies() {
    let Some(()) = skip_without_pdfium() else { return };
    let _lock = serial();
    let doc = PdfiumDocument::open_path(harness::fixture_path("framed.pdf").to_str().unwrap(), None).expect("open");
    let picture = doc.images_on(0).expect("images")[0].object;
    let lifted = doc.lift_objects(0, &[picture]).expect("lift");
    assert!(lifted.objects > 1, "only the picture came: {}", lifted.objects);
}

#[test]
fn words_are_not_lifted() {
    let Some(()) = skip_without_pdfium() else { return };
    let _lock = serial();
    let doc = open(hand_built_pdf(
        &[HandPage {
            content: "BT /F1 12 Tf 72 700 Td (Words) Tj ET",
            resources: "<< /Font << /F1 5 0 R >> >>",
            extra: "",
        }],
        &["<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"],
    ));
    let run = doc.text_runs(0).expect("runs")[0].object;
    assert!(matches!(doc.lift_objects(0, &[run]), Err(PdfError::Unsupported(_))));
}

#[test]
fn a_picture_that_cannot_be_followed_refuses_and_changes_nothing() {
    let Some(()) = skip_without_pdfium() else { return };
    let _lock = serial();
    let doc = open(hand_built_pdf(
        &[HandPage {
            content: "q 20 0 0 20 72 600 cm /Im0 Do Q q 20 0 0 20 200 600 cm BI /W 1 /H 1 /BPC 8 /CS /G ID x EI Q",
            resources: "<< /XObject << /Im0 5 0 R >> >>",
            extra: "",
        }],
        &[GREY_IMAGE],
    ));
    let pictures = doc.images_on(0).expect("images");
    assert_eq!(pictures.len(), 2, "PDFium lists the inline image too");
    let before = fingerprint(&doc);
    assert!(matches!(doc.lift_objects(0, &[pictures[0].object]), Err(PdfError::Unsupported(_))));
    assert_eq!(fingerprint(&doc), before);
}

#[test]
fn the_clip_has_no_annotations_and_only_the_resources_it_uses() {
    let Some(()) = skip_without_pdfium() else { return };
    let _lock = serial();
    // Words drawn first leave their font in force for the shape after them.
    let doc = open(hand_built_pdf(
        &[HandPage {
            content: "BT /F1 12 Tf 72 700 Td (One) Tj ET BT /F2 10 Tf 72 680 Td (Two) Tj ET 1 0 0 rg 100 500 50 50 re f",
            resources: "<< /Font << /F1 5 0 R /F2 6 0 R >> >>",
            extra: "/Annots [7 0 R]",
        }],
        &[
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
            "<< /Type /Font /Subtype /Type1 /BaseFont /Courier >>",
            "<< /Type /Annot /Subtype /Link /Rect [100 500 150 550] /Border [0 0 0] /A << /S /URI /URI (https://example.com) >> >>",
        ],
    ));
    let shape = top_level(&doc, 0).into_iter().find(|d| d.kind == DrawnKind::Shape).expect("the square").object;
    let lifted = doc.lift_objects(0, &[shape]).expect("lift");
    let file = File::parse(&lifted.pdf).expect("parse clip");
    let page = page_dict(&file, 0);
    assert!(page.get(b"Annots").is_none(), "annotations travelled");
    let resources = file.resolve(page.get(b"Resources").expect("resources")).unwrap();
    assert!(resources.as_dict().unwrap().get(b"Font").is_none(), "a font travelled with a square");
}

// -------------------------------------------------------------------- paste --

fn shifted(r: Rect, dx: f32, dy: f32) -> Rect {
    Rect { left: r.left + dx, top: r.top + dy, right: r.right + dx, bottom: r.bottom + dy }
}

fn centre(r: Rect) -> Point {
    Point { x: (r.left + r.right) / 2.0, y: (r.top + r.bottom) / 2.0 }
}

fn grown(r: Rect, by: f32) -> Rect {
    Rect { left: r.left - by, top: r.top - by, right: r.right + by, bottom: r.bottom + by }
}

/// The first picture of `pictures.pdf`, lifted.
fn a_picture() -> Lifted {
    let doc = PdfiumDocument::open_path(harness::fixture_path("pictures.pdf").to_str().unwrap(), None).expect("open");
    let picture = doc.images_on(0).expect("images")[0].object;
    doc.lift_objects(0, &[picture]).expect("lift the picture")
}

fn xobject_names(doc: &PdfiumDocument, page: usize) -> Vec<Vec<u8>> {
    let bytes = doc.readable_bytes_for_probe().unwrap();
    let file = File::parse(&bytes).unwrap();
    doc.xobject_names_for_probe(&file, page).unwrap().into_iter().map(|(n, _)| n).collect()
}

/// A page with a drawing at the top: two coloured squares and a stroked circle.
fn drawing_page() -> PdfiumDocument {
    open(hand_built_pdf(
        &[HandPage {
            content: concat!(
                "1 0 0 rg 100 600 50 50 re f 0 0.6 0 rg 160 610 30 40 re f ",
                "0 0 1 RG 3 w 300 625 m 300 638.8 288.8 650 275 650 c 261.2 650 250 638.8 250 625 c ",
                "250 611.2 261.2 600 275 600 c 288.8 600 300 611.2 300 625 c S"
            ),
            resources: "<< >>",
            extra: "",
        }],
        &[],
    ))
}

/// A red square lifted from a document of its own.
fn red_square() -> Lifted {
    let doc = open(hand_built_pdf(
        &[HandPage { content: "1 0 0 rg 100 600 50 50 re f", resources: "<< >>", extra: "" }],
        &[],
    ));
    doc.lift_objects(0, &[0]).expect("lift the square")
}

fn luminance(b: &Bitmap, x: usize, y: usize) -> f32 {
    let at = y * b.stride + x * 4;
    0.299 * b.data[at] as f32 + 0.587 * b.data[at + 1] as f32 + 0.114 * b.data[at + 2] as f32
}

/// The bounding box of everything not white, in pixels: left, top, right, bottom inclusive.
fn ink_box(b: &Bitmap) -> Option<(usize, usize, usize, usize)> {
    let mut found: Option<(usize, usize, usize, usize)> = None;
    for y in 0..b.height as usize {
        for x in 0..b.width as usize {
            if luminance(b, x, y) < 250.0 {
                found = Some(match found {
                    None => (x, y, x, y),
                    Some((l, t, r, bt)) => (l.min(x), t.min(y), r.max(x), bt.max(y)),
                });
            }
        }
    }
    found
}

fn whole(doc: &PdfiumDocument, page: usize, scale: f32) -> Bitmap {
    let size = doc.page_size(page).expect("size");
    region(doc, page, Rect { left: 0.0, top: 0.0, right: size.width_pt, bottom: size.height_pt }, scale)
}

fn region_px(b: &Bitmap, l: usize, t: usize, w: usize, h: usize) -> Bitmap {
    let mut out = Bitmap::new(w as u32, h as u32, PixelOrder::Rgba).expect("bitmap");
    for y in 0..h {
        let from = (t + y) * b.stride + l * 4;
        out.data[y * out.stride..y * out.stride + w * 4].copy_from_slice(&b.data[from..from + w * 4]);
    }
    out
}

/// Pixels changed by more than 30 (channel sum) outside `area` + 4 pt, as
/// redact's harm check counts them, at scale 1. Written here rather than
/// calling the engine's, so the check does not grade itself.
fn changed_outside(before: &Bitmap, after: &Bitmap, area: Rect) -> usize {
    let mut changed = 0;
    for y in 0..before.height as usize {
        for x in 0..before.width as usize {
            let (px, py) = (x as f32, y as f32);
            if px >= area.left - 4.0 && px <= area.right + 4.0 && py >= area.top - 4.0 && py <= area.bottom + 4.0 {
                continue;
            }
            let at = y * before.stride + x * 4;
            let d: i32 = (0..3).map(|c| (before.data[at + c] as i32 - after.data[at + c] as i32).abs()).sum();
            changed += usize::from(d > 30);
        }
    }
    changed
}

#[test]
fn a_lifted_drawing_pasted_on_its_own_page_lands_where_asked_and_draws_the_same() {
    let Some(()) = skip_without_pdfium() else { return };
    let _lock = serial();
    let mut doc = drawing_page();
    let before = top_level(&doc, 0);
    let all: Vec<usize> = before.iter().map(|d| d.object).collect();
    let lifted = doc.lift_objects(0, &all).expect("lift");
    let area = grown(lifted.rect, 4.0);
    let source = [region(&doc, 0, area, 1.5), region(&doc, 0, area, 4.0)];

    // 300 pt straight down: a whole number of pixels at 1.5 and 4.
    let at = centre(shifted(lifted.rect, 0.0, 300.0));
    doc.paste_lifted_mut(0, &lifted.pdf, at).expect("paste");

    // Each line is a shape of the page's own, where its original is, 300 pt down.
    let after = top_level(&doc, 0);
    assert_eq!(after.len(), before.len() + lifted.objects, "{after:?}");
    for (pasted, original) in after[before.len()..].iter().zip(&before) {
        assert_eq!(pasted.kind, original.kind);
        assert!(near(pasted.rect, shifted(original.rect, 0.0, 300.0), 1.0), "{:?} for {:?}", pasted.rect, original.rect);
    }
    for (scale, was) in [1.5f32, 4.0].into_iter().zip(source) {
        let now = region(&doc, 0, shifted(area, 0.0, 300.0), scale);
        let difference = mean_difference(&was, &now);
        assert!(difference < 1.0, "at scale {scale} the paste draws differently: {difference}");
    }
}

#[test]
fn everything_already_on_the_page_is_untouched() {
    let Some(()) = skip_without_pdfium() else { return };
    let _lock = serial();
    let mut doc = open(hand_built_pdf(
        &[HandPage {
            content: concat!(
                "BT /F1 14 Tf 72 720 Td (Untouched words) Tj ET ",
                "0 0 1 rg 300 100 80 80 re f 1 0 0 rg 100 600 50 50 re f"
            ),
            resources: "<< /Font << /F1 5 0 R >> >>",
            extra: "",
        }],
        &["<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"],
    ));
    let before_objects = top_level(&doc, 0);
    let before_runs = doc.text_runs(0).expect("runs");
    let before_pixels = whole(&doc, 0, 1.0);
    let square = before_objects.last().unwrap().object;
    let lifted = doc.lift_objects(0, &[square]).expect("lift");

    doc.paste_lifted_mut(0, &lifted.pdf, Point { x: 450.0, y: 500.0 }).expect("paste");

    let after = top_level(&doc, 0);
    assert_eq!(after.len(), before_objects.len() + 1);
    for (now, was) in after.iter().zip(&before_objects) {
        assert_eq!(now.kind, was.kind);
        assert!(near(now.rect, was.rect, 0.5));
    }
    assert_eq!(doc.text_runs(0).expect("runs"), before_runs);
    let pasted = Rect { left: 425.0, top: 475.0, right: 475.0, bottom: 525.0 };
    assert_eq!(changed_outside(&before_pixels, &whole(&doc, 0, 1.0), pasted), 0);
}

#[test]
fn pasted_into_another_document_and_saved_both_ways_the_file_is_well_formed() {
    let Some(()) = skip_without_pdfium() else { return };
    let _lock = serial();
    let clip = red_square();
    let mut target =
        PdfiumDocument::open_path(harness::fixture_path("pages-ladder.pdf").to_str().unwrap(), None).expect("open");
    let streams: Vec<Vec<u8>> = (0..target.page_count()).map(|p| target.page_stream(p).unwrap_or_default()).collect();
    let size = target.page_size(2).expect("size");
    let drawn_before = top_level(&target, 2).len();
    target
        .paste_lifted_mut(2, &clip.pdf, Point { x: size.width_pt / 2.0, y: size.height_pt / 2.0 })
        .expect("paste");

    let mut boxed: Box<dyn Document> = Box::new(target);
    let widths = harness::page_widths(&boxed);
    harness::check_both_saves(&mut boxed, "lift-paste");
    let mut full = Vec::new();
    boxed.as_document_mut().unwrap().save_full_copy(&mut full).unwrap();
    let reopened = open(full);
    assert_eq!((0..reopened.page_count()).map(|p| reopened.page_size(p).unwrap().width_pt.round() as i32).collect::<Vec<_>>(), widths);
    assert_eq!(top_level(&reopened, 2).len(), drawn_before + 1, "the paste was not saved");
    for (page, was) in streams.iter().enumerate().filter(|(p, _)| *p != 2) {
        assert_eq!(&reopened.page_stream(page).unwrap_or_default(), was, "page {page} changed");
    }
}

#[test]
fn one_undo_takes_the_paste_back_and_redo_puts_it_back() {
    let Some(()) = skip_without_pdfium() else { return };
    let _lock = serial();
    let clip = red_square();
    let mut doc = drawing_page();
    let before: Vec<(DrawnKind, Rect)> = top_level(&doc, 0).into_iter().map(|d| (d.kind, d.rect)).collect();
    let mut history = CommandHistory::new(8);
    let paste = Command::PasteLifted { page_index: 0, lifted: clip.pdf.clone(), at: Point { x: 300.0, y: 500.0 } };
    history.execute(paste, &mut doc).expect("paste");
    assert_eq!(top_level(&doc, 0).len(), before.len() + 1);
    history.undo(&mut doc).expect("undo").expect("a step to undo");
    let undone: Vec<(DrawnKind, Rect)> = top_level(&doc, 0).into_iter().map(|d| (d.kind, d.rect)).collect();
    assert_eq!(undone, before);
    history.redo(&mut doc).expect("redo").expect("a step to redo");
    assert_eq!(top_level(&doc, 0).len(), before.len() + 1);
}

/// **Pasted lines are lines.** Reported from use on 0.1.70: the pasted lens
/// could be selected only as one group, which could not be deleted or
/// resized. Each pasted line is now a shape of the page's own, edited like
/// the original: moved alone, removed alone.
#[test]
fn pasted_lines_are_separate_shapes_that_can_be_moved_and_removed() {
    let Some(()) = skip_without_pdfium() else { return };
    let _lock = serial();
    let mut doc = drawing_page();
    let before = top_level(&doc, 0);
    let all: Vec<usize> = before.iter().map(|d| d.object).collect();
    let lifted = doc.lift_objects(0, &all).expect("lift");
    doc.paste_lifted_mut(0, &lifted.pdf, centre(shifted(lifted.rect, 0.0, 300.0))).expect("paste");
    let pasted: Vec<DrawnObject> = top_level(&doc, 0).split_off(before.len());
    assert_eq!(pasted.len(), 3, "{pasted:?}");
    assert!(pasted.iter().all(|d| d.kind == DrawnKind::Shape), "{pasted:?}");

    let mut history = CommandHistory::new(8);
    history
        .execute(Command::MoveObject { page_index: 0, object: pasted[0].object, by: Point { x: 20.0, y: 10.0 } }, &mut doc)
        .expect("move one pasted line");
    let moved = top_level(&doc, 0)[pasted[0].object].rect;
    assert!(near(moved, shifted(pasted[0].rect, 20.0, 10.0), 0.5), "{moved:?} from {:?}", pasted[0].rect);

    history
        .execute(Command::RemoveObjects { page_index: 0, objects: vec![pasted[2].object] }, &mut doc)
        .expect("remove one pasted line");
    let left = top_level(&doc, 0);
    assert_eq!(left.len(), before.len() + 2, "{left:?}");
    assert!(near(left[before.len() + 1].rect, pasted[1].rect, 0.5), "the other pasted line moved");
}

#[test]
fn a_page_left_inside_a_transform_and_a_clip_still_gets_the_paste_where_asked() {
    let Some(()) = skip_without_pdfium() else { return };
    let _lock = serial();
    let clip = red_square();
    // The stream never closes its `q`: a doubling transform and a clip stay in force.
    let mut doc = open(hand_built_pdf(
        &[HandPage {
            content: "q 2 0 0 2 30 30 cm 0 0 100 100 re W n 0 0 1 rg 10 10 20 20 re f",
            resources: "<< >>",
            extra: "",
        }],
        &[],
    ));
    doc.paste_lifted_mut(0, &clip.pdf, Point { x: 450.0, y: 300.0 }).expect("paste");
    let drawn = top_level(&doc, 0);
    assert_eq!(drawn.len(), 2, "{drawn:?}");
    let want = Rect { left: 425.0, top: 275.0, right: 475.0, bottom: 325.0 };
    assert!(near(drawn[1].rect, want, 1.0), "{:?}", drawn[1].rect);
    assert_eq!(centre_colour(&doc, 0, want), [255, 0, 0], "the paste was clipped away or moved");
}

#[test]
fn a_rotated_source_pasted_onto_an_upright_page_looks_the_same() {
    let Some(()) = skip_without_pdfium() else { return };
    let _lock = serial();
    // An L of two colours, so a quarter turn the wrong way shows.
    let source = open(hand_built_pdf(
        &[HandPage {
            content: "1 0 0 rg 100 500 20 100 re f 0 0 1 rg 120 500 60 20 re f",
            resources: "<< >>",
            extra: "/Rotate 90",
        }],
        &[],
    ));
    let lifted = source.lift_objects(0, &[0, 1]).expect("lift");
    let mut target = open(hand_built_pdf(&[HandPage { content: "", resources: "<< >>", extra: "" }], &[]));
    target.paste_lifted_mut(0, &lifted.pdf, Point { x: 306.0, y: 396.0 }).expect("paste");

    let (was, now) = (whole(&source, 0, 2.0), whole(&target, 0, 2.0));
    let (a, b) = (ink_box(&was).expect("ink"), ink_box(&now).expect("pasted ink"));
    let (w, h) = ((a.2 - a.0 + 1).min(b.2 - b.0 + 1), (a.3 - a.1 + 1).min(b.3 - b.1 + 1));
    assert!(
        (b.2 - b.0).abs_diff(a.2 - a.0) <= 1 && (b.3 - b.1).abs_diff(a.3 - a.1) <= 1,
        "the paste is not the shape the source shows: {a:?} vs {b:?}"
    );
    let difference = mean_difference(&region_px(&was, a.0, a.1, w, h), &region_px(&now, b.0, b.1, w, h));
    assert!(difference < 2.0, "the paste does not look as it did: {difference}");
}

/// The second paste of a picture finds its name taken by the first, and is
/// given another: two names, two pictures.
#[test]
fn pasting_a_picture_twice_gives_two_names_and_two_pictures() {
    let Some(()) = skip_without_pdfium() else { return };
    let _lock = serial();
    let clip = a_picture();
    let mut doc = drawing_page();
    doc.paste_lifted_mut(0, &clip.pdf, Point { x: 200.0, y: 400.0 }).expect("first");
    doc.paste_lifted_mut(0, &clip.pdf, Point { x: 400.0, y: 400.0 }).expect("second");
    assert_eq!(doc.images_on(0).expect("images").len(), 2);
    let names = xobject_names(&doc, 0);
    assert_eq!(names.len(), 2, "{names:?}");
    assert_ne!(names[0], names[1]);
}

/// Two pages that share their resources, one way or the other, each drawing `/Im0`.
fn siblings(pages_extra: &str, resources: &'static str, objects: &[&str]) -> PdfiumDocument {
    let page = || HandPage { content: "q 40 0 0 40 72 600 cm /Im0 Do Q", resources, extra: "" };
    open(hand_built_pdf_with(&[page(), page()], objects, pages_extra))
}

/// A pasted picture also called `/Im0` goes into page 1's own resources under
/// another name; page 2, which shares or inherits them, is not touched.
fn sibling_is_untouched(mut doc: PdfiumDocument) {
    let clip = a_picture();
    let before = fingerprint(&doc);
    doc.paste_lifted_mut(0, &clip.pdf, Point { x: 300.0, y: 400.0 }).expect("paste");
    let after = fingerprint(&doc);
    assert_eq!(after[1], before[1], "the other page changed");
    let names: Vec<&[u8]> = after[0].1.iter().map(|(n, _)| n.as_slice()).collect();
    assert!(names.len() == 2 && names.contains(&&b"Im0"[..]), "{names:?}");
    assert_eq!(doc.images_on(0).expect("images").len(), 2, "page 1 should show its picture and the pasted one");
    assert_eq!(doc.images_on(1).expect("images").len(), 1, "page 2 lost its picture or gained one");
}

#[test]
fn a_page_that_inherits_its_resources_does_not_change_its_siblings() {
    let Some(()) = skip_without_pdfium() else { return };
    let _lock = serial();
    sibling_is_untouched(siblings("/Resources << /XObject << /Im0 7 0 R >> >>", "", &[GREY_IMAGE]));
}

#[test]
fn a_page_sharing_one_resources_object_does_not_change_its_siblings() {
    let Some(()) = skip_without_pdfium() else { return };
    let _lock = serial();
    sibling_is_untouched(siblings("", "7 0 R", &["<< /XObject 8 0 R >>", "<< /Im0 9 0 R >>", GREY_IMAGE]));
}

#[test]
fn a_signed_document_keeps_its_signature_when_pasted_on() {
    let Some(()) = skip_without_pdfium() else { return };
    let _lock = serial();
    let clip = red_square();
    let path = harness::fixture_path("rsa-signed.pdf");
    let original = std::fs::read(&path).expect("fixture");
    let mut doc = PdfiumDocument::open_path(path.to_str().unwrap(), None).expect("open");
    let size = doc.page_size(0).expect("size");
    let drawn_before = top_level(&doc, 0).len();
    doc.paste_lifted_mut(0, &clip.pdf, Point { x: size.width_pt / 2.0, y: size.height_pt / 2.0 }).expect("paste");
    let mut saved = Vec::new();
    doc.save_incremental(&mut saved).expect("save");
    assert!(saved.starts_with(&original), "the signed bytes moved");
    let reopened = open(saved);
    assert_eq!(reopened.signature_count(), 1);
    assert_eq!(top_level(&reopened, 0).len(), drawn_before + 1, "the paste was not kept");
}

#[test]
fn a_clip_missing_a_resource_is_refused_and_the_document_is_unchanged() {
    let Some(()) = skip_without_pdfium() else { return };
    let _lock = serial();
    // A clip whose content names a picture its resources do not hold.
    let broken = hand_built_pdf(
        &[HandPage { content: "q 40 0 0 40 72 600 cm /Im0 Do Q", resources: "<< >>", extra: "" }],
        &[],
    );
    let mut doc = drawing_page();
    let before = fingerprint(&doc);
    assert!(matches!(
        doc.paste_lifted_mut(0, &broken, Point { x: 300.0, y: 400.0 }),
        Err(PdfError::Unsupported(_))
    ));
    assert_eq!(fingerprint(&doc), before);
}

#[test]
fn transparency_travels() {
    let Some(()) = skip_without_pdfium() else { return };
    let _lock = serial();
    let mut doc = open(hand_built_pdf(
        &[HandPage {
            content: "/GS0 gs 1 0 0 rg 100 600 50 50 re f",
            resources: "<< /ExtGState << /GS0 << /Type /ExtGState /ca 0.5 >> >> >>",
            extra: "",
        }],
        &[],
    ));
    let square = top_level(&doc, 0)[0].rect;
    let was = centre_colour(&doc, 0, square);
    let lifted = doc.lift_objects(0, &[0]).expect("lift");
    doc.paste_lifted_mut(0, &lifted.pdf, centre(shifted(square, 0.0, 300.0))).expect("paste");
    let now = centre_colour(&doc, 0, shifted(square, 0.0, 300.0));
    assert!(was.iter().zip(now).all(|(a, b)| a.abs_diff(b) <= 2), "{was:?} became {now:?}");
    assert!(was[1] > 100, "the source itself was not see-through: {was:?}");
}

#[test]
fn an_upright_source_pasted_onto_a_rotated_page_looks_the_same() {
    let Some(()) = skip_without_pdfium() else { return };
    let _lock = serial();
    let source = open(hand_built_pdf(
        &[HandPage { content: "1 0 0 rg 100 500 20 100 re f 0 0 1 rg 120 500 60 20 re f", resources: "<< >>", extra: "" }],
        &[],
    ));
    let lifted = source.lift_objects(0, &[0, 1]).expect("lift");
    // A page the way Pagify turns one: /Rotate on the page, the box inherited.
    let mut target = open(hand_built_pdf(&[HandPage { content: "", resources: "<< >>", extra: "/Rotate 90" }], &[]));
    target.paste_lifted_mut(0, &lifted.pdf, Point { x: 306.0, y: 396.0 }).expect("paste");

    let (was, now) = (whole(&source, 0, 2.0), whole(&target, 0, 2.0));
    let (a, b) = (ink_box(&was).expect("ink"), ink_box(&now).expect("pasted ink"));
    assert!(
        (b.2 - b.0).abs_diff(a.2 - a.0) <= 1 && (b.3 - b.1).abs_diff(a.3 - a.1) <= 1,
        "the paste is not the shape the source shows: {a:?} vs {b:?}"
    );
    let (w, h) = ((a.2 - a.0 + 1).min(b.2 - b.0 + 1), (a.3 - a.1 + 1).min(b.3 - b.1 + 1));
    let difference = mean_difference(&region_px(&was, a.0, a.1, w, h), &region_px(&now, b.0, b.1, w, h));
    assert!(difference < 2.0, "the paste does not look as it did: {difference}");
}

#[test]
fn a_page_that_leaves_white_fill_and_faint_opacity_in_force_still_gets_the_paste_as_copied() {
    let Some(()) = skip_without_pdfium() else { return };
    let _lock = serial();
    // The clip's square never sets a colour: it is black by default.
    let source = open(hand_built_pdf(&[HandPage { content: "100 600 50 50 re f", resources: "<< >>", extra: "" }], &[]));
    let lifted = source.lift_objects(0, &[0]).expect("lift");
    let mut target = open(hand_built_pdf(
        &[HandPage {
            content: "1 1 1 rg 1 1 1 RG /Faint gs 10 10 5 5 re f",
            resources: "<< /ExtGState << /Faint << /Type /ExtGState /ca 0.2 /CA 0.2 >> >> >>",
            extra: "",
        }],
        &[],
    ));
    target.paste_lifted_mut(0, &lifted.pdf, Point { x: 300.0, y: 400.0 }).expect("paste");
    let square = Rect { left: 275.0, top: 375.0, right: 325.0, bottom: 425.0 };
    assert_eq!(centre_colour(&target, 0, square), [0, 0, 0], "the paste took the page's leftover state");
}

#[test]
fn a_page_whose_clip_cannot_be_closed_refuses_the_paste_and_changes_nothing() {
    let Some(()) = skip_without_pdfium() else { return };
    let _lock = serial();
    let clip = red_square();
    // A clip set at the top level, outside any `q`: nothing can lift it, and a
    // paste outside it would be drawn invisibly.
    let mut doc = open(hand_built_pdf(
        &[HandPage { content: "0 0 100 100 re W n 0 0 1 rg 10 10 20 20 re f", resources: "<< >>", extra: "" }],
        &[],
    ));
    let before = fingerprint(&doc);
    assert!(matches!(doc.paste_lifted_mut(0, &clip.pdf, Point { x: 400.0, y: 300.0 }), Err(PdfError::Unsupported(_))));
    assert_eq!(fingerprint(&doc), before);
}

#[test]
fn a_paste_undone_and_saved_leaves_nothing_of_it_in_the_file() {
    let Some(()) = skip_without_pdfium() else { return };
    let _lock = serial();
    // A 20 KB picture, so a copy left behind would show in the file's size.
    // Noise, written as hex: PDFium compresses a stream with no filter when it
    // saves, and a run of one byte would shrink to nothing.
    let mut seed = 12345u32;
    let noise: String = (0..20000)
        .map(|_| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            format!("{:02x}", (seed >> 16) as u8)
        })
        .collect();
    let picture = format!(
        "<< /Type /XObject /Subtype /Image /Width 100 /Height 200 /BitsPerComponent 8 /ColorSpace /DeviceGray /Filter /ASCIIHexDecode /Length {} >>\nstream\n{noise}>\nendstream",
        noise.len() + 1
    );
    let source = open(hand_built_pdf(
        &[HandPage { content: "q 100 0 0 200 50 500 cm /Im0 Do Q", resources: "<< /XObject << /Im0 5 0 R >> >>", extra: "" }],
        &[&picture],
    ));
    let lifted = source.lift_objects(0, &[0]).expect("lift");
    let mut doc = drawing_page();
    let saved = |doc: &mut PdfiumDocument| {
        let mut out = Vec::new();
        doc.save_full_copy(&mut out).expect("save");
        out.len()
    };
    let before = saved(&mut doc);
    let mut history = CommandHistory::new(8);
    history
        .execute(Command::PasteLifted { page_index: 0, lifted: lifted.pdf, at: Point { x: 300.0, y: 400.0 } }, &mut doc)
        .expect("paste");
    let pasted = saved(&mut doc);
    history.undo(&mut doc).expect("undo").expect("a step to undo");
    let undone = saved(&mut doc);
    eprintln!("saved sizes: before {before}, pasted {pasted}, undone {undone}");
    assert!(pasted > before + 15_000, "the picture was not in the pasted file");
    assert!(undone < before + 2_000, "the undone paste is still in the file: {undone} vs {before}");
}

/// The page has an `/Im0` (grey) and so does the pasted drawing (black): the
/// paste must draw its own, under a new name, not the page's.
#[test]
fn a_pasted_resource_named_like_one_on_the_page_is_renamed_not_confused() {
    let Some(()) = skip_without_pdfium() else { return };
    let _lock = serial();
    const BLACK: &str = "<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceGray /Length 1 >>\nstream\n\u{0}\nendstream";
    let source = open(hand_built_pdf(
        &[HandPage { content: "q 40 0 0 40 72 600 cm /Im0 Do Q", resources: "<< /XObject << /Im0 5 0 R >> >>", extra: "" }],
        &[BLACK],
    ));
    let lifted = source.lift_objects(0, &[0]).expect("lift the black picture");
    let mut target = open(hand_built_pdf(
        &[HandPage { content: "q 40 0 0 40 72 600 cm /Im0 Do Q", resources: "<< /XObject << /Im0 5 0 R >> >>", extra: "" }],
        &[GREY_IMAGE],
    ));
    target.paste_lifted_mut(0, &lifted.pdf, Point { x: 300.0, y: 400.0 }).expect("paste");
    let pasted = Rect { left: 280.0, top: 380.0, right: 320.0, bottom: 420.0 };
    assert_eq!(centre_colour(&target, 0, pasted), [0, 0, 0], "the paste drew the page's own /Im0");
    let names = xobject_names(&target, 0);
    assert_eq!(names.len(), 2, "{names:?}");
}
