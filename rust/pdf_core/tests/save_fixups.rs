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

/// **Foxit's way, for the marks that stay marks.** A highlight saved by Pagify
/// stays a highlight — still listed, still removable — and now carries an
/// appearance of its own, which draws it as Pagify drew it before the save.
///
/// **Ink is not checked here any more.** The owner's decision of 2026-11-18 is
/// that Pagify's own ink goes into the page at Save, which
/// `a_rectangle_drawn_in_pagify_is_part_of_the_page_after_save` proves.
#[test]
fn a_highlight_stays_a_mark_and_carries_its_appearance() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    let yellow = Color { r: 255, g: 230, b: 0, a: 255 };
    doc.add_annotation(0, &Annotation::Highlight {
        rects: vec![Rect { left: 100.0, top: 200.0, right: 300.0, bottom: 220.0 }],
        color: yellow,
    })
    .expect("highlight");

    let finder = no_face;
    let mut saved = Vec::new();
    let report = doc
        .save_full_copy_with_fixups(&mut saved, &SaveFixups { find_face: &finder })
        .expect("save");
    assert_eq!(report.marks_drawn, 1, "the highlight was not given an appearance");

    let file = File::parse(&saved).expect("a readable file");
    let Some(Ok(Object::Array(annots))) = page(&file).get(b"Annots").map(|a| file.resolve(a)) else {
        panic!("the mark is gone")
    };
    for annot in &annots {
        let annot = file.resolve(annot).expect("a mark");
        assert!(annot.as_dict().expect("a dict").get(b"AP").is_some(), "a mark has no appearance: {annot:?}");
    }

    let reopened = PdfiumDocument::open_bytes(saved, None).expect("reopen");
    let marks = reopened.annotations(0).expect("marks");
    assert!(marks.iter().any(|m| matches!(m.annotation, Annotation::Highlight { .. })), "no highlight: {marks:?}");

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

// -- Task 8: a page's unused images and fonts ---------------------------------

/// A PDF stream object body with its exact `/Length`.
fn stream_object(dict: &str, data: &str) -> String {
    format!("<< {dict} /Length {} >>\nstream\n{data}\nendstream", data.len())
}

/// The first page's dict in a classic or rewritten file.
fn page_dict(file: &File<'_>, index: usize) -> pdf_core::pdf::Dict {
    let root = file
        .trailer()
        .get(b"Root")
        .and_then(|o| file.resolve(o).ok())
        .and_then(|o| o.as_dict().cloned())
        .expect("a catalogue");
    let pages = root
        .get(b"Pages")
        .and_then(|o| file.resolve(o).ok())
        .and_then(|o| o.as_dict().cloned())
        .expect("a page tree");
    let kids = pages
        .get(b"Kids")
        .and_then(|o| file.resolve(o).ok())
        .and_then(|o| match o {
            Object::Array(items) => Some(items),
            _ => None,
        })
        .expect("kids");
    let Object::Reference(number, _) = kids[index] else { panic!("a page is not a reference") };
    file.object(number).ok().and_then(|o| o.as_dict().cloned()).expect("the first page")
}

/// The names a page's `category` table (`/Font`, `/XObject`, ...) holds.
fn resource_keys(file: &File<'_>, page: &pdf_core::pdf::Dict, category: &[u8]) -> Vec<Vec<u8>> {
    let Some(resources) = page
        .get(b"Resources")
        .and_then(|o| file.resolve(o).ok())
        .and_then(|o| o.as_dict().cloned())
    else {
        return Vec::new();
    };
    resources
        .get(category)
        .and_then(|o| file.resolve(o).ok())
        .and_then(|o| o.as_dict().cloned())
        .map(|table| table.0.iter().map(|(name, _)| name.clone()).collect())
        .unwrap_or_default()
}

fn rendered_page(doc: &PdfiumDocument, page: usize, scale: f32) -> (Vec<u8>, u32, u32, usize) {
    let bitmap = doc
        .render_page_to_bitmap(page, &RenderRequest { scale, ..Default::default() })
        .expect("render");
    (bitmap.data, bitmap.width, bitmap.height, bitmap.stride)
}

/// **A removed page picture leaves the file.** Edit Object's delete cuts the drawing operator out
/// of the page's content but leaves the image in the page's `/Resources`; the page still refers to
/// it, so `drop_unreachable` rightly keeps it and the bytes stay. This is what left PRISMA at 796 KB
/// after the screenshot was removed.
#[test]
fn removing_a_page_picture_takes_its_bytes_out_of_the_saved_file() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    let mut plain = Vec::new();
    doc.save_full_copy(&mut plain).expect("the plain full save");

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
        &Annotation::Image {
            rect: Rect { left: 60.0, top: 60.0, right: 210.0, bottom: 160.0 },
            rgba,
            width: w,
            height: h,
        },
    )
    .expect("place the picture");
    let finder = no_face;
    let mut baked = Vec::new();
    doc.save_full_copy_with_fixups(&mut baked, &SaveFixups { find_face: &finder }).expect("bake it");

    let mut reopened = PdfiumDocument::open_bytes(baked, None).expect("reopen the baked file");
    let picture = reopened
        .images_on(0)
        .expect("images")
        .into_iter()
        .find(|i| i.rect.left >= 55.0 && i.rect.right <= 215.0 && i.rect.top >= 55.0 && i.rect.bottom <= 165.0)
        .expect("the baked picture is on the page");
    reopened.remove_object_mut(0, picture.object).expect("remove it, as Edit Object does");

    let mut saved = Vec::new();
    reopened.save_full_copy_with_fixups(&mut saved, &SaveFixups { find_face: &finder }).expect("save again");
    eprintln!("plain {} bytes, after removal {} bytes", plain.len(), saved.len());
    assert!(
        saved.len() <= plain.len() + 4096,
        "removing the picture left {} bytes behind (plain {}, after removal {})",
        saved.len().saturating_sub(plain.len()),
        plain.len(),
        saved.len()
    );
}

/// **A form with no `/Resources` draws with the page's names** — removing them would erase its text.
#[test]
fn a_form_without_resources_keeps_the_page_names_it_draws_with() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let form = "BT /F1 24 Tf 72 700 Td (Kept) Tj ET";
    let objects = [
        stream_object("/Type /XObject /Subtype /Form /BBox [0 0 612 792]", form),
        // **Courier, not Helvetica.** If the name is taken off the list, PDFium's fallback is
        // Helvetica, which draws "Kept" at a different width — so the pixels say whether the font
        // is still named. A Helvetica font here would make the fallback pixel-identical.
        "<< /Type /Font /Subtype /Type1 /BaseFont /Courier >>".to_string(),
    ];
    let refs: Vec<&str> = objects.iter().map(String::as_str).collect();
    let input = harness::hand_built_pdf(
        &[harness::HandPage {
            content: "q /Fm0 Do Q",
            resources: "<< /XObject << /Fm0 5 0 R >> /Font << /F1 6 0 R >> >>",
            extra: "",
        }],
        &refs,
    );

    let mut doc = PdfiumDocument::open_bytes(input.clone(), None).expect("open the hand-built page");
    doc.add_annotation(
        0,
        &Annotation::Image {
            rect: Rect { left: 400.0, top: 100.0, right: 420.0, bottom: 120.0 },
            rgba: red(2, 2),
            width: 2,
            height: 2,
        },
    )
    .expect("an edit, so the save is a rewrite");
    let finder = no_face;
    let mut saved = Vec::new();
    doc.save_full_copy_with_fixups(&mut saved, &SaveFixups { find_face: &finder }).expect("full save");

    let file = File::parse(&saved).expect("the saved file parses");
    let page = page_dict(&file, 0);
    assert!(
        resource_keys(&file, &page, b"Font").iter().any(|name| name == b"F1"),
        "the form's font was taken off the page's list: {:?}",
        resource_keys(&file, &page, b"Font")
    );

    // **Every pixel**, skipping only the picture that forced the rewrite: the word "Kept" is near the
    // top of the bitmap (rows 75–96 at scale 1), not the bottom the old check looked at.
    let before = PdfiumDocument::open_bytes(input, None).expect("reopen the input");
    let after = PdfiumDocument::open_bytes(saved, None).expect("reopen the saved");
    let (a, aw, ah, a_stride) = rendered_page(&before, 0, 1.0);
    let (b, bw, bh, bs) = rendered_page(&after, 0, 1.0);
    assert_eq!((aw, ah), (bw, bh), "the page changed size");
    for y in 0..ah {
        for x in 0..aw {
            if (395..425).contains(&x) && (95..125).contains(&y) {
                continue;
            }
            let at = y as usize * bs + x as usize * 4;
            let old = y as usize * a_stride + x as usize * 4;
            assert_eq!(a[old..old + 4], b[at..at + 4], "the form's text changed at ({x}, {y})");
        }
    }
}

/// **An annotation's appearance with no `/Resources` draws with the page's names too.**
#[test]
fn an_appearance_without_resources_keeps_the_page_names_it_uses() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let appearance = "BT /F1 12 Tf 5 5 Td (x) Tj ET";
    let objects = [
        "<< /Type /Annot /Subtype /Stamp /Rect [10 10 110 110] /AP << /N 6 0 R >> >>".to_string(),
        stream_object("/Type /XObject /Subtype /Form /BBox [0 0 100 100]", appearance),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
    ];
    let refs: Vec<&str> = objects.iter().map(String::as_str).collect();
    let input = harness::hand_built_pdf(
        &[harness::HandPage {
            content: "",
            resources: "<< /Font << /F1 7 0 R >> >>",
            extra: "/Annots [5 0 R]",
        }],
        &refs,
    );

    let mut doc = PdfiumDocument::open_bytes(input, None).expect("open the hand-built page");
    doc.add_annotation(
        0,
        &Annotation::Image {
            rect: Rect { left: 400.0, top: 100.0, right: 420.0, bottom: 120.0 },
            rgba: red(2, 2),
            width: 2,
            height: 2,
        },
    )
    .expect("an edit, so the save is a rewrite");
    let finder = no_face;
    let mut saved = Vec::new();
    doc.save_full_copy_with_fixups(&mut saved, &SaveFixups { find_face: &finder }).expect("full save");

    let file = File::parse(&saved).expect("the saved file parses");
    let page = page_dict(&file, 0);
    assert!(
        resource_keys(&file, &page, b"Font").iter().any(|name| name == b"F1"),
        "the appearance's font was taken off the page's list: {:?}",
        resource_keys(&file, &page, b"Font")
    );
}

/// **Pages sharing one resources dict keep what any of them draws**, and only the unused name goes.
#[test]
fn pages_sharing_resources_keep_what_any_of_them_uses() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let image = || {
        stream_object(
            "/Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8",
            "A",
        )
    };
    let objects = [
        image(),
        image(),
        image(),
        "<< /XObject << /Im1 9 0 R /Im2 10 0 R /Im3 11 0 R >> >>".to_string(),
    ];
    let refs: Vec<&str> = objects.iter().map(String::as_str).collect();
    let input = harness::hand_built_pdf(
        &[
            harness::HandPage {
                content: "q 10 0 0 10 10 10 cm /Im1 Do Q",
                resources: "12 0 R",
                extra: "",
            },
            harness::HandPage {
                content: "q 10 0 0 10 10 10 cm /Im2 Do Q",
                resources: "12 0 R",
                extra: "",
            },
            harness::HandPage { content: "", resources: "<< >>", extra: "" },
        ],
        &refs,
    );

    let mut doc = PdfiumDocument::open_bytes(input.clone(), None).expect("open the three pages");
    // **On the third page**, whose resources are its own: the two sharing pages must stay sharing.
    doc.add_annotation(
        2,
        &Annotation::Image {
            rect: Rect { left: 400.0, top: 100.0, right: 420.0, bottom: 120.0 },
            rgba: red(2, 2),
            width: 2,
            height: 2,
        },
    )
    .expect("an edit, so the save is a rewrite");
    let finder = no_face;
    let mut saved = Vec::new();
    doc.save_full_copy_with_fixups(&mut saved, &SaveFixups { find_face: &finder }).expect("full save");

    let file = File::parse(&saved).expect("the saved file parses");
    let one_page = page_dict(&file, 0);
    let two_page = page_dict(&file, 1);
    let one = resource_keys(&file, &one_page, b"XObject");
    let two = resource_keys(&file, &two_page, b"XObject");
    for (page, kept, names) in [(1, b"Im1".as_slice(), &one), (2, b"Im2".as_slice(), &two)] {
        assert!(names.iter().any(|name| name.as_slice() == kept), "page {page} lost {kept:?}: {names:?}");
        assert!(!names.iter().any(|name| name.as_slice() == b"Im3"), "page {page} kept unused Im3: {names:?}");
    }
    // **One resources object, still**, or the union was never what was tested.
    let resources_ref = |page: &pdf_core::pdf::Dict| match page.get(b"Resources") {
        Some(Object::Reference(number, _)) => Some(*number),
        _ => None,
    };
    let (one_ref, two_ref) = (resources_ref(&one_page), resources_ref(&two_page));
    assert_eq!(one_ref, two_ref, "pages 1 and 2 no longer share one indirect /Resources");
    assert!(one_ref.is_some(), "the shared resources are no longer indirect: {one_ref:?}");

    let before = PdfiumDocument::open_bytes(input, None).expect("reopen the input");
    let after = PdfiumDocument::open_bytes(saved, None).expect("reopen the saved");
    for page in 0..3 {
        let a = rendered_page(&before, page, 1.0);
        let b = rendered_page(&after, page, 1.0);
        assert_eq!((a.1, a.2, a.3), (b.1, b.2, b.3), "page {} changed size", page + 1);
        for y in 0..a.2 {
            for x in 0..a.1 {
                // Page 3 gained the small picture that forced the rewrite; everything else is the same.
                if page == 2 && (395..425).contains(&x) && (95..125).contains(&y) {
                    continue;
                }
                let at = y as usize * a.3 + x as usize * 4;
                assert!(a.0[at..at + 4] == b.0[at..at + 4], "page {} draws differently at ({x}, {y})", page + 1);
            }
        }
    }
}

/// **On the owner's files, pruning changes no page**, and PRISMA is back under 650 KB.
#[test]
fn on_the_owners_files_pruning_changes_no_page() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let files = [
        (r"C:\Users\hsili\Desktop\Pagify_EditText_Evidence\PRISMA-after-removal.pdf", true),
        (r"C:\Users\hsili\Desktop\TECHNICAL DATASHEET.pdf", false),
        (r"C:\Users\hsili\Downloads\TECHNICAL DATASHEET - Q2447.pdf", false),
        (r"C:\Users\hsili\Desktop\Pagify_EditText_Evidence\LUX REPORT -REV.pdf", false),
    ];
    for (index, (path, must_be_under_650k)) in files.into_iter().enumerate() {
        if !std::path::Path::new(path).is_file() {
            eprintln!("skipping: {path} is not on this machine");
            continue;
        }
        let mut doc = PdfiumDocument::open_path(path, None).expect("open the owner's file");
        let mut a = Vec::new();
        doc.save_full_copy(&mut a).expect("PDFium's full save");
        let (pruned, entries) = pdf_core::pdf::hidden::drop_unused_resources(&a).expect("prune");
        let (b, objects) = pdf_core::pdf::hidden::drop_unreachable(&pruned).expect("drop unreachable");
        eprintln!("{path}: {} -> {} bytes, {entries} entries off, {objects} objects dropped", a.len(), b.len());
        if must_be_under_650k {
            assert!(b.len() < 650_000, "PRISMA is still {} bytes", b.len());
        }

        harness::check_with_qpdf(&b, &format!("pruned-owner-{index}"));

        let before = PdfiumDocument::open_bytes(a, None).expect("open the full save");
        let after = PdfiumDocument::open_bytes(b, None).expect("open the pruned save");
        assert_eq!(before.page_count(), after.page_count(), "{path}: a page went");
        for page in 0..before.page_count() {
            let one = rendered_page(&before, page, 0.3);
            let two = rendered_page(&after, page, 0.3);
            assert_eq!((one.1, one.2, one.3), (two.1, two.2, two.3), "{path}: page {} changed size", page + 1);
            assert!(one.0 == two.0, "{path}: page {} draws differently after pruning", page + 1);
        }
    }
}

/// **A looping appearance must not stop the save.** `/AP`'s states are `<< /On 7 0 R >>` and object 7
/// is that same dictionary: the old walk followed it forever and overflowed the stack, which no
/// `unwrap_or_else` can catch. `/AP` is one level deep per ISO 32000-1 §12.5.5.
#[test]
fn a_looping_appearance_does_not_stop_the_save() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let objects = [
        "<< /Type /Annot /Subtype /Stamp /Rect [10 10 50 50] /AP << /N 7 0 R >> >>".to_string(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        "<< /On 7 0 R >>".to_string(),
    ];
    let refs: Vec<&str> = objects.iter().map(String::as_str).collect();
    let input = harness::hand_built_pdf(
        &[harness::HandPage { content: "", resources: "<< >>", extra: "/Annots [5 0 R]" }],
        &refs,
    );

    let mut doc = PdfiumDocument::open_bytes(input, None).expect("open the looping page");
    doc.add_annotation(
        0,
        &Annotation::Image {
            rect: Rect { left: 400.0, top: 100.0, right: 420.0, bottom: 120.0 },
            rgba: red(2, 2),
            width: 2,
            height: 2,
        },
    )
    .expect("an edit, so the save is a rewrite");
    let finder = no_face;
    let mut saved = Vec::new();
    doc.save_full_copy_with_fixups(&mut saved, &SaveFixups { find_face: &finder })
        .expect("a looping appearance must not stop the save");
}

// -- Task 11: a font that draws nothing but spaces ---------------------------

/// **A simple font that draws only a space needs no program.** It shows
/// nothing, so every reader's own face draws it; storing one only costs bytes.
/// It is not reported as missing either — nothing is missing.
#[test]
fn a_font_used_only_for_a_space_is_not_stored() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = PdfiumDocument::open_bytes(
        pagify_helvetica_page("BT /F1 1 Tf ( ) Tj ET"),
        None,
    )
    .expect("open");
    let program = include_bytes!("../../../desktop/third_party/fonts/Montserrat-Regular.ttf").to_vec();
    let finder = |name: &str| (name == "ArialMT").then(|| program.clone());

    let mut saved = Vec::new();
    let report = doc
        .save_full_copy_with_fixups(&mut saved, &SaveFixups { find_face: &finder })
        .expect("save");
    assert!(report.fonts_stored.is_empty(), "a face was stored for a space: {:?}", report.fonts_stored);
    assert!(report.fonts_missing.is_empty(), "a font that draws a space was reported missing: {:?}", report.fonts_missing);

    let file = File::parse(&saved).expect("a readable file");
    let page = page_dict(&file, 0);
    let resources = page
        .get(b"Resources")
        .and_then(|o| file.resolve(o).ok())
        .and_then(|o| o.as_dict().cloned())
        .expect("resources");
    let fonts = resources
        .get(b"Font")
        .and_then(|o| file.resolve(o).ok())
        .and_then(|o| o.as_dict().cloned())
        .expect("fonts");
    let (_, value) = fonts.0.first().expect("a font");
    let font = file.resolve(value).expect("the font").as_dict().cloned().expect("a font dict");
    assert_eq!(font.get(b"Subtype"), Some(&Object::Name(b"Type1".to_vec())), "the font was rewritten");
    assert!(font.get(b"FontDescriptor").is_none(), "a descriptor and a program were written for a space");
}

/// **The 75 KB the owner saw.** A page's hidden block is a fully transparent
/// space in Helvetica; storing a face for it put 75 KB of Arial into PRISMA on
/// every save that had marks on it.
#[test]
fn the_marks_hidden_block_adds_no_font_to_prisma() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let path = r"C:\Users\hsili\Desktop\Pagify_EditText_Evidence\PRISMA-after-removal.pdf";
    if !std::path::Path::new(path).is_file() {
        eprintln!("skipping: {path} is not on this machine");
        return;
    }
    let arial = r"C:\Windows\Fonts\arial.ttf";
    if !std::path::Path::new(arial).is_file() {
        eprintln!("skipping: {arial} is not on this machine");
        return;
    }
    let arial = std::fs::read(arial).expect("read arial");

    let mut doc = PdfiumDocument::open_path(path, None).expect("open the owner's file");
    // The hidden block exactly as `commit_page` writes it: one fully transparent
    // space in Helvetica, carrying the markup blob under the carrier id.
    doc.add_annotation(0, &Annotation::Text {
        text: " ".into(),
        font: "Helvetica".into(),
        font_asset: None,
        size: 1.0,
        color: Color { r: 0, g: 0, b: 0, a: 0 },
        glyphs: vec![pdf_core::document::Glyph { ch: " ".into(), id: 0, x: 0.0, y: 0.0, radians: 0.0 }],
        id: 0x7061_67_01,
        restore: "{\"magic\":\"pagify.markup\",\"version\":2,\"page_height_pt\":792.0,\"objects\":[]}".into(),
        frame: Vec::new(),
        frame_width: 0.0,
        scale_x: 1.0,
        more: Vec::new(),
        rules: Vec::new(),
    })
    .expect("the hidden block");
    doc.add_annotation(0, &ink_rectangle(100.0, 200.0, 300.0, 400.0)).expect("the rectangle");

    let finder = |name: &str| (name == "ArialMT").then(|| arial.clone());
    let mut saved = Vec::new();
    let report = doc
        .save_full_copy_with_fixups(&mut saved, &SaveFixups { find_face: &finder })
        .expect("the full save");
    eprintln!("PRISMA with the hidden block: {} bytes, fonts stored {:?}", saved.len(), report.fonts_stored);
    assert!(report.fonts_stored.is_empty(), "a face was stored for the hidden block: {:?}", report.fonts_stored);
    assert!(saved.len() < 650_000, "the hidden block brought {} bytes; PRISMA is {} bytes", saved.len().saturating_sub(650_000), saved.len());
    harness::check_with_qpdf(&saved, "prisma-hidden-block");
}

// -- Task 9: Pagify's own marks go into the page ------------------------------

/// The drawing a page's content streams hold, decoded, one after another.
fn page_drawing(file: &File<'_>, page: &pdf_core::pdf::Dict) -> String {
    let Some(contents) = page.get(b"Contents") else { return String::new() };
    let items = match file.resolve(contents).expect("contents") {
        Object::Array(items) => items,
        Object::Stream(..) => vec![contents.clone()],
        _ => return String::new(),
    };
    let mut out = String::new();
    for item in items {
        if let Object::Stream(dict, range) = file.resolve(&item).expect("a content stream") {
            out.push_str(&String::from_utf8_lossy(&pdf_core::pdf::content::decode(&dict, &file.bytes()[range]).expect("decodes")));
        }
    }
    out
}

/// The `/Subtype` of every annotation the page still lists.
fn annotation_subtypes(file: &File<'_>, page: &pdf_core::pdf::Dict) -> Vec<String> {
    let Some(annots) = page.get(b"Annots") else { return Vec::new() };
    let Object::Array(items) = file.resolve(annots).expect("annots") else { return Vec::new() };
    items
        .iter()
        .filter_map(|item| file.resolve(item).ok())
        .filter_map(|object| object.as_dict().cloned())
        .filter_map(|annot| annot.get(b"Subtype").and_then(Object::as_name).map(|name| String::from_utf8_lossy(name).to_string()))
        .collect()
}

/// Every page's dict, in page order.
fn all_pages(file: &File<'_>) -> Vec<pdf_core::pdf::Dict> {
    fn walk(file: &File<'_>, node: &Object, out: &mut Vec<pdf_core::pdf::Dict>) {
        let Some(dict) = node.as_dict() else { return };
        if dict.get(b"Type").and_then(Object::as_name) == Some(b"Page") {
            out.push(dict.clone());
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
    all
}

/// Every page's annotations of one subtype, across the file.
fn annotations_of(file: &File<'_>, wanted: &[u8]) -> usize {
    all_pages(file)
        .iter()
        .map(|page| annotation_subtypes(file, page).iter().filter(|kind| kind.as_bytes() == wanted).count())
        .sum()
}

/// The pink Pagify writes a markup rectangle in.
fn pink() -> Color {
    Color { r: 255, g: 92, b: 138, a: 255 }
}

/// The closed five-point stroke `commit_page` writes for a rectangle: pink, 1 pt.
fn ink_rectangle(left: f32, top: f32, right: f32, bottom: f32) -> Annotation {
    Annotation::Ink {
        strokes: vec![vec![
            Point { x: left, y: top },
            Point { x: right, y: top },
            Point { x: right, y: bottom },
            Point { x: left, y: bottom },
            Point { x: left, y: top },
        ]],
        color: pink(),
        width: 1.0,
    }
}

/// How many pixels are the rectangle's pink — the stroke itself, within a
/// rendering's own antialiasing.
fn pink_pixels(bitmap: &pdf_core::render::bitmap::Bitmap) -> usize {
    let mut count = 0;
    for y in 0..bitmap.height as usize {
        for x in 0..bitmap.width as usize {
            let p = &bitmap.data[y * bitmap.stride + x * 4..y * bitmap.stride + x * 4 + 4];
            let (r, g, b) = match bitmap.order {
                PixelOrder::Bgra => (p[2], p[1], p[0]),
                PixelOrder::Rgba => (p[0], p[1], p[2]),
            };
            if r > 200 && (60..140).contains(&g) && (100..180).contains(&b) {
                count += 1;
            }
        }
    }
    count
}

fn render(doc: &PdfiumDocument, scale: f32, annotations: bool) -> pdf_core::render::bitmap::Bitmap {
    doc.render_page_to_bitmap(0, &RenderRequest { scale, render_annotations: annotations, ..Default::default() })
        .expect("render")
}

/// The largest difference on any channel between two renders of the same page,
/// and how many pixels differ at all.
fn render_difference(a: &pdf_core::render::bitmap::Bitmap, b: &pdf_core::render::bitmap::Bitmap) -> (usize, u8) {
    assert_eq!((a.width, a.height, a.stride), (b.width, b.height, b.stride), "the page changed size");
    let mut differing = 0;
    let mut largest = 0u8;
    for y in 0..a.height as usize {
        for x in 0..a.width as usize {
            let pa = &a.data[y * a.stride + x * 4..y * a.stride + x * 4 + 4];
            let pb = &b.data[y * b.stride + x * 4..y * b.stride + x * 4 + 4];
            let worst = (0..4).map(|c| pa[c].abs_diff(pb[c])).max().unwrap_or(0);
            if worst > 0 {
                differing += 1;
                largest = largest.max(worst);
            }
        }
    }
    (differing, largest)
}

/// **The owner's decision, 2026-11-18.** A rectangle drawn in Pagify looks the
/// same in every viewer after Save: it is drawn into the page, under a
/// `/PagifyMarks` tag, and leaves the page's `/Annots`.
#[test]
fn a_rectangle_drawn_in_pagify_is_part_of_the_page_after_save() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    doc.add_annotation(0, &ink_rectangle(100.0, 200.0, 300.0, 400.0)).expect("the rectangle");
    let annotations = doc.annotation_count(0).expect("count");
    let before = render(&doc, 1.0, true);
    assert!(pink_pixels(&before) > 0, "setup: the rectangle is not drawn at all");

    let finder = no_face;
    let mut saved = Vec::new();
    let report = doc
        .save_full_copy_with_fixups(&mut saved, &SaveFixups { find_face: &finder })
        .expect("the full save");
    assert_eq!(report.marks_baked, 1, "the rectangle was not baked into the page");
    assert_eq!(doc.annotation_count(0).expect("count"), annotations, "the open document changed");

    let file = File::parse(&saved).expect("a readable file");
    let page = page_dict(&file, 0);
    assert!(annotation_subtypes(&file, &page).iter().all(|kind| kind != "Ink"), "the rectangle is still an annotation");
    let drawing = page_drawing(&file, &page);
    assert!(drawing.contains("/PagifyMarks BMC"), "the drawing is not tagged:\n{drawing}");

    let baked = PdfiumDocument::open_bytes(saved, None).expect("the saved file opens");
    let as_page_content = render(&baked, 1.0, false);
    assert!(pink_pixels(&as_page_content) > 0, "a renderer that draws only the page does not show the rectangle");
    let after = render(&baked, 1.0, true);
    let (differing, largest) = render_difference(&before, &after);
    eprintln!("Task 9 rectangle: {differing} of {} pixels differ, largest channel difference {largest}", before.data.len() / 4);
    assert_eq!((differing, largest), (0, 0), "the drawing looks different after the save");
}

/// A filled shape and its arrowheads are `Annotation::Fill`: a Stamp whose
/// appearance is a form, not a picture. They go into the page with the ink.
#[test]
fn a_filled_shape_and_its_arrowheads_go_into_the_page_too() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    let ring = vec![
        Point { x: 100.0, y: 500.0 },
        Point { x: 200.0, y: 500.0 },
        Point { x: 200.0, y: 560.0 },
        Point { x: 100.0, y: 560.0 },
        Point { x: 100.0, y: 500.0 },
    ];
    doc.add_annotation(0, &Annotation::Fill { outline: ring, fill_color: pink(), stroke_color: pink(), width: 1.0 })
        .expect("the filled shape");
    let annotations = doc.annotation_count(0).expect("count");
    let before = render(&doc, 1.0, true);

    let finder = no_face;
    let mut saved = Vec::new();
    let report = doc
        .save_full_copy_with_fixups(&mut saved, &SaveFixups { find_face: &finder })
        .expect("the full save");
    assert_eq!(report.marks_baked, 1, "the filled shape was not baked into the page");
    assert_eq!(doc.annotation_count(0).expect("count"), annotations, "the open document changed");

    let file = File::parse(&saved).expect("a readable file");
    let page = page_dict(&file, 0);
    assert!(annotation_subtypes(&file, &page).iter().all(|kind| kind != "Stamp"), "the fill is still an annotation");
    assert!(page_drawing(&file, &page).contains("/PagifyMarks BMC"), "the drawing is not tagged");

    let baked = PdfiumDocument::open_bytes(saved, None).expect("the saved file opens");
    assert!(pink_pixels(&render(&baked, 1.0, false)) > 0, "a renderer that draws only the page does not show the fill");
    let (differing, largest) = render_difference(&before, &render(&baked, 1.0, true));
    eprintln!("Task 9 fill: {differing} of {} pixels differ, largest channel difference {largest}", before.data.len() / 4);
    assert_eq!((differing, largest), (0, 0), "the filled shape looks different after the save");
}

/// **Not ours to fix in place.** Highlights, notes and links stay annotations,
/// and so does another app's ink — `/PagifyColor` is what says a mark is
/// Pagify's own.
#[test]
fn highlights_notes_links_and_other_apps_ink_stay_marks() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let foreign = "<< /Type /Annot /Subtype /Ink /Rect [400 100 500 150] /InkList [[420 110 480 140]] /C [0 0 1] /F 4 >>";
    let input = harness::hand_built_pdf(
        &[harness::HandPage { content: "", resources: "<< >>", extra: "/Annots [5 0 R]" }],
        &[foreign],
    );
    let mut doc = PdfiumDocument::open_bytes(input, None).expect("open");
    doc.add_annotation(0, &Annotation::Highlight {
        rects: vec![Rect { left: 60.0, top: 100.0, right: 200.0, bottom: 120.0 }],
        color: Color { r: 255, g: 235, b: 59, a: 255 },
    })
    .expect("highlight");
    doc.add_annotation(0, &Annotation::Note {
        rect: Rect { left: 60.0, top: 140.0, right: 80.0, bottom: 160.0 },
        contents: "look here".into(),
        color: Color { r: 255, g: 200, b: 0, a: 255 },
    })
    .expect("note");
    doc.add_annotation(0, &Annotation::Link {
        rect: Rect { left: 60.0, top: 180.0, right: 200.0, bottom: 200.0 },
        uri: "https://example.com".into(),
    })
    .expect("link");
    let annotations = doc.annotation_count(0).expect("count");

    let finder = no_face;
    let mut saved = Vec::new();
    let report = doc
        .save_full_copy_with_fixups(&mut saved, &SaveFixups { find_face: &finder })
        .expect("the full save");
    assert_eq!(report.marks_baked, 0, "a mark that is not ours was baked into the page");

    let file = File::parse(&saved).expect("a readable file");
    let page = page_dict(&file, 0);
    let kinds = annotation_subtypes(&file, &page);
    assert_eq!(doc.annotation_count(0).expect("count"), annotations, "the open document changed");
    for wanted in ["Highlight", "Text", "Link", "Ink"] {
        assert!(kinds.iter().any(|kind| kind == wanted), "the {wanted} is gone: {kinds:?}");
    }
    assert!(annotations_of(&file, b"Ink") == 1, "another app's ink was baked in");
    assert!(!String::from_utf8_lossy(&saved).contains("/PagifyMarks"), "something was tagged");
}

/// A placed picture is baked as it was, and **not tagged**: it is a page
/// picture on purpose.
#[test]
fn a_placed_picture_is_baked_as_before_and_untagged() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    place_picture(&mut doc, Rect { left: 100.0, top: 200.0, right: 180.0, bottom: 240.0 });

    let finder = no_face;
    let mut saved = Vec::new();
    let report = doc
        .save_full_copy_with_fixups(&mut saved, &SaveFixups { find_face: &finder })
        .expect("the full save");
    assert_eq!(report.pictures, 1, "the picture was not baked");
    assert_eq!(report.marks_baked, 0, "a picture was counted as a mark");
    assert!(!String::from_utf8_lossy(&saved).contains("/PagifyMarks"), "a picture was tagged as a mark");
}

/// **The owner's own three rectangles, through the save path itself.** Every
/// viewer sees them once they are part of the page; a reader that draws only
/// the page draws them as the file does with its marks.
#[test]
fn on_the_owners_rectangles_every_viewer_sees_them() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let path = r"C:\Users\hsili\Desktop\Pagify_EditText_Evidence\PRISMA-3-rectangles.pdf";
    if !std::path::Path::new(path).is_file() {
        eprintln!("skipping: {path} is not on this machine");
        return;
    }
    let input = std::fs::read(path).expect("read the owner's file");
    let out = pdf_core::pdf::flatten::flatten_for_save(&input).expect("bake the marks");
    assert_eq!(out.marks, 3, "the three rectangles were not all baked");
    assert_eq!(annotations_of(&File::parse(&out.bytes).expect("a readable file"), b"Ink"), 0, "an Ink annotation is left");

    harness::check_with_qpdf(&out.bytes, "prisma-marks");

    let before = PdfiumDocument::open_bytes(input, None).expect("open the input");
    let after = PdfiumDocument::open_bytes(out.bytes, None).expect("open the baked file");
    for page in 0..before.page_count() {
        let drawn_with_marks = before
            .render_page_to_bitmap(page, &RenderRequest { scale: 0.3, render_annotations: true, ..Default::default() })
            .expect("render before");
        let drawn_as_page = after
            .render_page_to_bitmap(page, &RenderRequest { scale: 0.3, render_annotations: false, ..Default::default() })
            .expect("render after");
        let (differing, largest) = render_difference(&drawn_with_marks, &drawn_as_page);
        eprintln!("PRISMA page {}: {differing} of {} pixels differ, largest channel difference {largest}", page + 1, drawn_with_marks.data.len() / 4);
        assert!(largest <= 8, "page {} draws differently after baking: {differing} pixels, largest {largest}", page + 1);
    }
}

/// **A still-encrypted copy is not pruned.** The save can reach the fix-ups before a rewrite, with
/// `/Encrypt` in the trailer and ciphertext in the streams; pruning that would read ciphertext and a
/// stream that happened to parse would take every font and image off its page.
#[test]
fn a_file_still_encrypted_is_not_pruned() {
    let image = stream_object(
        "/Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8",
        "A",
    );
    let refs: Vec<&str> = [image.as_str()].into_iter().collect();
    let input = harness::hand_built_pdf(
        &[harness::HandPage {
            content: "q Q",
            resources: "<< /XObject << /Im3 5 0 R >> >>",
            extra: "",
        }],
        &refs,
    );
    let text = String::from_utf8(input).expect("the hand-built file is text");
    let marked = text.replacen("trailer\n<< ", "trailer\n<< /Encrypt << /Filter /Standard /V 4 >> ", 1);
    assert_ne!(marked, text, "the trailer was not found");

    let (out, dropped) = pdf_core::pdf::hidden::drop_unused_resources(marked.as_bytes()).expect("prune");
    assert_eq!(dropped, 0, "an encrypted file's unused names were pruned");
    assert_eq!(out, marked.as_bytes(), "the encrypted bytes were changed");
}
