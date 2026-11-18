//! What the desktop's ordinary Save writes, over and above the document.
//!
//! Two faults from the 2026-11-16 report are fixed on the bytes a full save
//! writes and nowhere else: a placed picture is baked into the page, so a
//! preview and a print job draw it, and a font the file names but does not carry
//! is embedded when the app can find it. The open document is never changed, an
//! incremental save is untouched, and the engine's own default save keeps the
//! meaning it had for Android.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test --release --test save_fixups
//! ```

mod harness;
use harness::{serial, skip_without_pdfium};

use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{
    Annotation, Color, Document, DocumentMut, Point, Rect, RenderRequest, SaveFixups,
};
use pdf_core::pdf::encrypt::Permissions;
use pdf_core::pdf::{File, Object};
use pdf_core::render::bitmap::PixelOrder;

fn open(name: &str) -> PdfiumDocument {
    PdfiumDocument::open_path(harness::fixture_path(name).to_str().expect("path"), None).expect("open")
}

fn red(width: u32, height: u32) -> Vec<u8> {
    [255u8, 0, 0, 255].repeat((width * height) as usize)
}

fn place_picture(doc: &mut PdfiumDocument, rect: Rect) {
    doc.add_annotation(0, &Annotation::Image { rect, rgba: red(4, 4), width: 4, height: 4 })
        .expect("place the picture");
}

/// Whether the middle of `rect` is drawn red — the picture's colour.
fn middle_is_red(doc: &PdfiumDocument, rect: Rect, annotations: bool) -> bool {
    let bitmap = doc
        .render_page_to_bitmap(0, &RenderRequest { scale: 1.0, render_annotations: annotations, ..Default::default() })
        .expect("render");
    let (x, y) = (((rect.left + rect.right) / 2.0) as usize, ((rect.top + rect.bottom) / 2.0) as usize);
    let at = y * bitmap.stride + x * 4;
    let p = &bitmap.data[at..at + 4];
    let (r, g, b) = match bitmap.order {
        PixelOrder::Bgra => (p[2], p[1], p[0]),
        PixelOrder::Rgba => (p[0], p[1], p[2]),
    };
    r > 200 && g < 60 && b < 60
}

/// One page in Helvetica as Pagify's own fallback writes it: WinAnsi and no
/// program (`FPDFText_LoadStandardFont` — all 29 such fonts in the 2026-11-16
/// files). A base-14 font in another encoding is left alone by the save.
fn pagify_helvetica_page(content: &'static str) -> Vec<u8> {
    harness::hand_built_pdf(
        &[harness::HandPage { content, resources: "<< /Font << /F1 5 0 R >> >>", extra: "" }],
        &["<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"],
    )
}

fn no_face(_: &str) -> Option<Vec<u8>> {
    None
}

/// **The reported fault.** A preview draws the page and not its annotations, so
/// the picture vanished. After the full save it is part of the page — and the
/// document in hand still holds it as a placed picture.
#[test]
fn a_full_save_bakes_a_placed_picture_and_leaves_the_open_document_as_it_was() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    let rect = Rect { left: 100.0, top: 200.0, right: 180.0, bottom: 240.0 };
    place_picture(&mut doc, rect);
    assert!(middle_is_red(&doc, rect, true), "setup: annotations are not drawn");
    assert!(!middle_is_red(&doc, rect, false), "setup: the picture is already page content");
    let annotations = doc.annotation_count(0).expect("count");

    let finder = no_face;
    let mut saved = Vec::new();
    let report = doc
        .save_full_copy_with_fixups(&mut saved, &SaveFixups { find_face: &finder })
        .expect("the full save");
    assert_eq!(report.pictures, 1, "the picture was not baked");

    assert_eq!(doc.annotation_count(0).expect("count"), annotations, "the open document changed");
    assert_eq!(doc.placed_image_marks(0).expect("marks").len(), 1, "the picture lost its handles");

    let baked = PdfiumDocument::open_bytes(saved, None).expect("the saved file opens");
    assert!(middle_is_red(&baked, rect, false), "the saved page does not draw the picture");
}

/// **The break:** fixing up an incremental save, which is what a signed
/// document gets. The appended revision must be the document as it was.
#[test]
fn an_incremental_save_does_not_bake_the_picture() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    let rect = Rect { left: 100.0, top: 200.0, right: 180.0, bottom: 240.0 };
    place_picture(&mut doc, rect);

    let mut saved = Vec::new();
    doc.save_incremental(&mut saved).expect("save incrementally");
    let opened = PdfiumDocument::open_bytes(saved, None).expect("reopen");
    assert!(!middle_is_red(&opened, rect, false), "an incremental save baked the picture");
}

/// **The break:** refusing a document with a password, the way the copy for sharing
/// does. The fix-ups go on the plaintext bytes and the password on after.
#[test]
fn a_document_with_a_password_is_fixed_up_too() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    let rect = Rect { left: 100.0, top: 200.0, right: 180.0, bottom: 240.0 };
    place_picture(&mut doc, rect);
    doc.secure_document(b"correct horse", None, Permissions::all()).expect("a password is set");

    let finder = no_face;
    let mut saved = Vec::new();
    let report = doc
        .save_full_copy_with_fixups(&mut saved, &SaveFixups { find_face: &finder })
        .expect("a password-protected document is not refused");
    assert_eq!(report.pictures, 1);

    assert!(PdfiumDocument::open_bytes(saved.clone(), None).is_err(), "the file has no password");
    let opened = PdfiumDocument::open_bytes(saved, Some("correct horse")).expect("it opens with its password");
    assert!(middle_is_red(&opened, rect, false), "the password-protected copy does not draw the picture");
}

/// Every annotation Pagify writes carries the Print flag; a link keeps none.
/// Read from the bytes, which is what another reader sees.
#[test]
fn every_annotation_pagify_writes_prints_and_a_link_does_not() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = PdfiumDocument::open_bytes(
        harness::helvetica_page(b"BT /F1 12 Tf 72 700 Td (Note) Tj ET"),
        None,
    )
    .expect("open");
    doc.add_annotation(
        0,
        &Annotation::Highlight {
            rects: vec![Rect { left: 70.0, top: 100.0, right: 180.0, bottom: 114.0 }],
            color: Color { r: 255, g: 235, b: 59, a: 255 },
        },
    )
    .expect("highlight");
    doc.add_annotation(
        0,
        &Annotation::Link { rect: Rect { left: 70.0, top: 120.0, right: 180.0, bottom: 134.0 }, uri: "https://example.com".into() },
    )
    .expect("link");

    let mut saved = Vec::new();
    doc.save_full_copy(&mut saved).expect("save");
    let file = File::parse(&saved).expect("a readable file");
    let page = page(&file);
    let Some(Object::Array(annots)) = page.get(b"Annots") else { panic!("no annotations") };
    let mut seen = Vec::new();
    for item in annots {
        let Ok(Object::Dict(annot)) = file.resolve(item) else { continue };
        let Some(Object::Name(kind)) = annot.get(b"Subtype") else { continue };
        let flags = annot.get(b"F").and_then(Object::as_i64).unwrap_or(0);
        seen.push(String::from_utf8_lossy(kind).to_string());
        if kind.as_slice() == b"Link" {
            assert_eq!(flags & 4, 0, "a link was given the Print flag");
        } else {
            assert_eq!(flags & 4, 4, "{} has no Print flag (F={flags})", String::from_utf8_lossy(kind));
        }
    }
    assert!(seen.iter().any(|k| k == "Highlight"), "the highlight was not written: {seen:?}");
    assert!(seen.iter().any(|k| k == "Link"), "the link was not written: {seen:?}");
}

/// Every pixel of page 1 as RGB, drawn with its marks.
fn pixels(doc: &PdfiumDocument) -> Vec<[u8; 3]> {
    let bitmap = doc
        .render_page_to_bitmap(0, &RenderRequest { scale: 1.0, render_annotations: true, ..Default::default() })
        .expect("render");
    let mut out = Vec::new();
    for y in 0..bitmap.height as usize {
        for x in 0..bitmap.width as usize {
            let p = &bitmap.data[y * bitmap.stride + x * 4..y * bitmap.stride + x * 4 + 4];
            out.push(match bitmap.order {
                PixelOrder::Bgra => [p[2], p[1], p[0]],
                PixelOrder::Rgba => [p[0], p[1], p[2]],
            });
        }
    }
    out
}

/// **Foxit's way.** A highlight and an ink stroke saved by Pagify stay marks —
/// still listed, still removable — and now carry an appearance of their own,
/// which draws them as Pagify drew them before the save.
#[test]
fn highlights_and_ink_stay_marks_and_carry_their_appearance() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    let yellow = Color { r: 255, g: 230, b: 0, a: 255 };
    let blue = Color { r: 0, g: 0, b: 255, a: 255 };
    doc.add_annotation(0, &Annotation::Highlight {
        rects: vec![Rect { left: 100.0, top: 200.0, right: 300.0, bottom: 220.0 }],
        color: yellow,
    })
    .expect("highlight");
    doc.add_annotation(0, &Annotation::Ink {
        strokes: vec![vec![Point { x: 100.0, y: 300.0 }, Point { x: 200.0, y: 350.0 }, Point { x: 300.0, y: 300.0 }]],
        color: blue,
        width: 3.0,
    })
    .expect("ink");

    let finder = no_face;
    let mut saved = Vec::new();
    let report = doc
        .save_full_copy_with_fixups(&mut saved, &SaveFixups { find_face: &finder })
        .expect("save");
    assert_eq!(report.marks_drawn, 2, "the marks were not given an appearance");

    let file = File::parse(&saved).expect("a readable file");
    let Some(Ok(Object::Array(annots))) = page(&file).get(b"Annots").map(|a| file.resolve(a)) else {
        panic!("the marks are gone")
    };
    for annot in &annots {
        let annot = file.resolve(annot).expect("a mark");
        assert!(annot.as_dict().expect("a dict").get(b"AP").is_some(), "a mark has no appearance: {annot:?}");
    }

    let reopened = PdfiumDocument::open_bytes(saved, None).expect("reopen");
    let marks = reopened.annotations(0).expect("marks");
    assert!(marks.iter().any(|m| matches!(m.annotation, Annotation::Highlight { .. })), "no highlight: {marks:?}");
    assert!(marks.iter().any(|m| matches!(m.annotation, Annotation::Ink { .. })), "no ink: {marks:?}");

    let (before, after) = (pixels(&doc), pixels(&reopened));
    let differ = before.iter().zip(&after).filter(|(a, b)| a.iter().zip(b.iter()).any(|(x, y)| x.abs_diff(*y) > 48)).count();
    assert!(differ * 1000 < before.len(), "{differ} of {} pixels look different after the save", before.len());
}

/// **The reported fault.** A font the file does not carry gets a program from
/// the app's finder on the ordinary full save.
#[test]
fn a_font_the_file_does_not_carry_is_embedded_from_the_finder() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = PdfiumDocument::open_bytes(
        pagify_helvetica_page("BT /F1 24 Tf 72 700 Td (HS90LC-1201F) Tj ET"),
        None,
    )
    .expect("open");
    let program = include_bytes!("../../../desktop/third_party/fonts/Montserrat-Regular.ttf").to_vec();
    let finder = |name: &str| (name == "ArialMT").then(|| program.clone());

    let mut saved = Vec::new();
    let report = doc
        .save_full_copy_with_fixups(&mut saved, &SaveFixups { find_face: &finder })
        .expect("save");
    assert_eq!(report.fonts_stored.len(), 1, "missing {:?}", report.fonts_missing);
    // The finder answers ArialMT with this program; the report names the face
    // it actually stored, which is the program's own PostScript name.
    assert_eq!(report.fonts_stored[0].0, "Montserrat-Regular");

    let file = File::parse(&saved).expect("a readable file");
    let (font, descriptor) = font_and_descriptor(&file);
    assert_eq!(font.get(b"Subtype"), Some(&Object::Name(b"TrueType".to_vec())));
    assert_eq!(font.get(b"Encoding"), Some(&Object::Name(b"WinAnsiEncoding".to_vec())));
    assert!(descriptor.get(b"FontFile2").is_some(), "the face was not written into the file");
}

/// A font the finder does not have is reported by name and left alone.
#[test]
fn a_font_the_finder_does_not_have_is_reported_and_the_file_left_alone() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = PdfiumDocument::open_bytes(
        pagify_helvetica_page("BT /F1 24 Tf 72 700 Td (Hello) Tj ET"),
        None,
    )
    .expect("open");
    let finder = no_face;
    let mut saved = Vec::new();
    let report = doc
        .save_full_copy_with_fixups(&mut saved, &SaveFixups { find_face: &finder })
        .expect("save");
    assert_eq!(report.fonts_missing, vec!["Helvetica".to_string()]);
    assert!(report.fonts_stored.is_empty());
}

fn page(file: &File<'_>) -> pdf_core::pdf::Dict {
    fn walk(file: &File<'_>, node: &Object, out: &mut Vec<Object>) {
        let Some(dict) = node.as_dict() else { return };
        if dict.get(b"Type").and_then(Object::as_name) == Some(b"Page") {
            out.push(node.clone());
            return;
        }
        if let Some(Ok(Object::Array(kids))) = dict.get(b"Kids").map(|k| file.resolve(k)) {
            for kid in kids {
                if let Ok(kid) = file.resolve(&kid) {
                    walk(file, &kid, out);
                }
            }
        }
    }
    let root = file.resolve(file.trailer().get(b"Root").expect("root")).expect("catalogue");
    let pages = file.resolve(root.as_dict().and_then(|d| d.get(b"Pages")).expect("pages")).expect("tree");
    let mut all = Vec::new();
    walk(file, &pages, &mut all);
    all[0].as_dict().expect("a page").clone()
}

/// The first font the page names, and its descriptor.
fn font_and_descriptor(file: &File<'_>) -> (pdf_core::pdf::Dict, pdf_core::pdf::Dict) {
    let page = page(file);
    let resources = page.get(b"Resources").expect("resources");
    let resources = file.resolve(resources).expect("resources");
    let fonts = resources.as_dict().and_then(|d| d.get(b"Font")).expect("fonts");
    let fonts = file.resolve(fonts).expect("font dict");
    let (_, value) = fonts.as_dict().and_then(|d| d.0.first().cloned()).expect("a font");
    let font = file.resolve(&value).expect("font");
    let font = font.as_dict().expect("a font dict").clone();
    let descriptor = font.get(b"FontDescriptor").expect("descriptor");
    let descriptor = file.resolve(descriptor).expect("descriptor");
    (font, descriptor.as_dict().expect("a descriptor").clone())
}

/// **A turned picture is baked turned by a full save.** The annotation is
/// gone from the saved file and the page itself draws the turn — checked by
/// where the red is, not by reading a key back.
#[test]
fn a_turned_picture_is_baked_turned_by_a_full_save() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    let frame = Rect { left: 300.0, top: 400.0, right: 500.0, bottom: 500.0 };
    place_picture(&mut doc, frame);
    doc.rotate_image_signature(0, 0, 90.0).expect("rotate");

    let finder = no_face;
    let mut saved = Vec::new();
    let report = doc
        .save_full_copy_with_fixups(&mut saved, &SaveFixups { find_face: &finder })
        .expect("the full save");
    assert_eq!(report.pictures, 1, "the turned picture was not baked");

    let baked = PdfiumDocument::open_bytes(saved, None).expect("the saved file opens");
    assert!(
        middle_is_red(&baked, Rect { left: 395.0, top: 365.0, right: 405.0, bottom: 375.0 }, false),
        "the saved page does not draw the picture where the turn puts it"
    );
    assert!(
        !middle_is_red(&baked, Rect { left: 305.0, top: 445.0, right: 315.0, bottom: 455.0 }, false),
        "the saved page still draws the picture in the un-turned part of its old frame"
    );
}

/// **A picture turned and then moved is baked where it was moved to.** The move is a shift of `/Rect`
/// now, not a rebuild, so this guards that the shifted appearance still bakes the turn in the right
/// place.
#[test]
fn a_turned_then_moved_picture_is_baked_where_it_was_moved_to() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    place_picture(&mut doc, Rect { left: 300.0, top: 300.0, right: 500.0, bottom: 400.0 });
    doc.rotate_image_signature(0, 0, 90.0).expect("rotate");
    doc.set_image_signature_rect(0, 0, Rect { left: 300.0, top: 400.0, right: 500.0, bottom: 500.0 })
        .expect("move");

    let finder = no_face;
    let mut saved = Vec::new();
    let report = doc
        .save_full_copy_with_fixups(&mut saved, &SaveFixups { find_face: &finder })
        .expect("the full save");
    assert_eq!(report.pictures, 1, "the turned, moved picture was not baked");

    let baked = PdfiumDocument::open_bytes(saved, None).expect("the saved file opens");
    assert!(
        middle_is_red(&baked, Rect { left: 395.0, top: 365.0, right: 405.0, bottom: 375.0 }, false),
        "the saved page does not draw the picture where the turn and the move put it"
    );
    assert!(
        !middle_is_red(&baked, Rect { left: 305.0, top: 445.0, right: 315.0, bottom: 455.0 }, false),
        "the saved page still draws the picture in the un-turned part of its old frame"
    );
}

/// **A turn must not leave a copy of the picture behind.** Every turn rebuilds the appearance and creates
/// a new image object, and a full save used to write every object made in the session: ten turns of a
/// 300 x 200 picture saved about 1.5 MB bigger than one. A save now drops what nothing refers to.
#[test]
fn turning_a_picture_ten_times_saves_no_bigger_than_turning_it_once() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let (w, h) = (300u32, 200u32);
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            rgba[i..i + 4].copy_from_slice(&[
                (x * 7 % 251) as u8,
                (y * 13 % 241) as u8,
                ((x ^ y) % 239) as u8,
                255,
            ]);
        }
    }
    let rect = Rect { left: 60.0, top: 60.0, right: 210.0, bottom: 160.0 };
    let finder = no_face;

    let saved_size = |turns: u32| -> usize {
        let mut doc = open("two-column.pdf");
        let index = doc
            .add_annotation(0, &Annotation::Image { rect, rgba: rgba.clone(), width: w, height: h })
            .expect("place the picture");
        for turn in 1..=turns {
            doc.rotate_image_signature(0, index, 10.0 * turn as f32).expect("turn");
        }
        let mut saved = Vec::new();
        let report = doc
            .save_full_copy_with_fixups(&mut saved, &SaveFixups { find_face: &finder })
            .expect("the full save");
        assert_eq!(report.pictures, 1, "the picture was not baked");
        saved.len()
    };

    let once = saved_size(1);
    let ten = saved_size(10);
    eprintln!("one turn {once} bytes, ten turns {ten} bytes");
    assert!(
        ten <= once + 4096,
        "ten turns grew the saved file from {once} to {ten} bytes (+{}): every turn left a copy",
        ten - once
    );
}

/// **A save keeps every bookmark of a long list.** Outlines are a `/First` then `/Next` chain, and the
/// old reachability walk stopped at depth 96 — 95 siblings. The walk is iterative now, with no depth cap.
#[test]
fn a_full_save_keeps_every_bookmark_of_a_long_list() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = PdfiumDocument::open_bytes(harness::bookmarks_pdf(300), None).expect("open the bookmarks");
    // A placed picture, so the save really is a rewrite.
    doc.add_annotation(
        0,
        &Annotation::Image { rect: Rect { left: 60.0, top: 60.0, right: 80.0, bottom: 80.0 }, rgba: red(2, 2), width: 2, height: 2 },
    )
    .expect("place a picture");

    let finder = no_face;
    let mut saved = Vec::new();
    doc.save_full_copy_with_fixups(&mut saved, &SaveFixups { find_face: &finder })
        .expect("the full save");
    assert_eq!(harness::bookmark_count(&saved), 300, "the save lost part of the bookmark list");
}

/// **Dropping what nothing refers to changes no page, on the owner's own files.** Each file is opened,
/// given one placed picture and full-saved; the dropped copy must render identically, page for page,
/// and pass qpdf. The files are skipped when they are not on this machine.
#[test]
fn on_the_owners_files_dropping_unreachable_objects_changes_no_page() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let files = [
        r"C:\Users\hsili\Desktop\TECHNICAL DATASHEET.pdf",
        r"C:\Users\hsili\Downloads\TECHNICAL DATASHEET - Q2447.pdf",
        r"C:\Users\hsili\Desktop\Pagify_EditText_Evidence\LUX REPORT -REV.pdf",
    ];
    for (index, path) in files.into_iter().enumerate() {
        if !std::path::Path::new(path).is_file() {
            eprintln!("skipping: {path} is not on this machine");
            continue;
        }
        let mut doc = PdfiumDocument::open_path(path, None).expect("open the owner's file");
        let (w, h) = (300u32, 200u32);
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                rgba[i..i + 4].copy_from_slice(&[
                    (x * 7 % 251) as u8,
                    (y * 13 % 241) as u8,
                    ((x ^ y) % 239) as u8,
                    255,
                ]);
            }
        }
        doc.add_annotation(
            0,
            &Annotation::Image { rect: Rect { left: 40.0, top: 40.0, right: 190.0, bottom: 140.0 }, rgba, width: w, height: h },
        )
        .expect("place a picture");

        let mut plain = Vec::new();
        doc.save_full_copy(&mut plain).expect("PDFium's full save");
        // **Baked first, as the ordinary Save does.** A plain PDFium full save still refers to the
        // placed stamp, so nothing is unreachable yet; `flatten_pictures` puts the picture in the page
        // and leaves the annotation and its appearance behind — exactly the 1–2 objects the prototype
        // measured. (The plan said plain `save_full_copy` alone reaches that state; measured, it does
        // not, and the ordinary Save's own cleanup would remove the orphans before this test saw them.)
        let before = pdf_core::pdf::flatten::flatten_pictures(&plain).expect("bake the picture").bytes;
        let (after, dropped) = pdf_core::pdf::hidden::drop_unreachable(&before).expect("drop unreachable");
        eprintln!("{path}: dropped {dropped} objects, {} -> {} bytes", before.len(), after.len());
        assert!(dropped >= 1, "{path}: nothing was dropped");
        assert!(after.len() < before.len(), "{path}: the cleaned file is not smaller");

        harness::check_with_qpdf(&after, &format!("owner-{index}"));

        let a = PdfiumDocument::open_bytes(before, None).expect("open the full save");
        let b = PdfiumDocument::open_bytes(after, None).expect("open the cleaned save");
        let pages = a.page_count().min(b.page_count()).min(40);
        assert_eq!(a.page_count(), b.page_count(), "{path}: a page went");
        for page in 0..pages {
            let one = a
                .render_page_to_bitmap(page, &RenderRequest { scale: 0.3, ..Default::default() })
                .expect("render before");
            let two = b
                .render_page_to_bitmap(page, &RenderRequest { scale: 0.3, ..Default::default() })
                .expect("render after");
            assert_eq!(
                (one.width, one.height, one.stride),
                (two.width, two.height, two.stride),
                "{path}: page {} changed size",
                page + 1
            );
            assert!(one.data == two.data, "{path}: page {} draws differently after the drop", page + 1);
        }
    }
}
