//! Two questions the image-signature design rested on, answered by running
//! it rather than by reading PDFium's headers — kept as a probe because the
//! answers are the kind a future PDFium upgrade could quietly change.
//!
//! **1. Does a Stamp's appended image object survive save → reopen?** Ink
//! survives a round trip because `/InkList` is a plain dictionary entry
//! PDFium re-reads from the file; an object appended via
//! `FPDFAnnot_AppendObject` might have been only a live, in-session thing,
//! reconstructed into an appearance stream on save and never parsed back out.
//! **Yes, it survives** — `fill_image_annotation`'s picture, its rect and its
//! opaque pixels all read back correctly after a real save and reopen.
//!
//! **2. Does the picture's alpha channel survive at all?** **No** — and not
//! only across a save: `FPDFImageObj_SetBitmap` drops it even in the same
//! session, before a single byte reaches disk. `FPDFImageObj_GetBitmap`
//! reads every alpha byte back as 255 regardless of what went in. This is
//! why `Annotation::Image` places every picture opaque; see its doc comment
//! and `tests/image_signature.rs`, which assert both findings so a future
//! PDFium that starts honouring alpha is noticed rather than silently
//! trusted.
use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Annotation, Document, DocumentMut, Rect};

fn main() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut doc =
        PdfiumDocument::open_path(root.join("fixtures/two-column.pdf").to_str().unwrap(), None)
            .unwrap();

    // A small solid-red test image, 4x3, fully opaque.
    let (w, h) = (4u32, 3u32);
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    for px in rgba.chunks_exact_mut(4) {
        px.copy_from_slice(&[220, 40, 40, 255]);
    }

    let index = doc
        .add_annotation(
            0,
            &Annotation::Image {
                rect: Rect { left: 100.0, top: 100.0, right: 180.0, bottom: 130.0 },
                rgba: rgba.clone(),
                width: w,
                height: h,
            },
        )
        .expect("add image annotation");
    doc.mark_as_signature(0, index, "Test Picture").expect("mark as signature");

    println!("== same session, before any save ==");
    let marks = doc.image_signature_marks(0).expect("image_signature_marks");
    println!("image_signature_marks in-session: {}", marks.len());
    for m in &marks {
        println!(
            "  name={:?} rect={:?} {}x{} bytes={}",
            m.name,
            m.rect,
            m.width,
            m.height,
            m.rgba.len()
        );
    }

    println!("== after a full-copy save and reopen ==");
    let mut saved = Vec::new();
    doc.save_full_copy(&mut saved).expect("save");
    println!("saved {} bytes", saved.len());
    let reopened = PdfiumDocument::open_bytes(saved, None).expect("reopen");
    let marks = reopened.image_signature_marks(0).expect("image_signature_marks after reopen");
    println!("image_signature_marks after reopen: {}", marks.len());
    for m in &marks {
        println!(
            "  name={:?} rect={:?} {}x{} bytes={}",
            m.name,
            m.rect,
            m.width,
            m.height,
            m.rgba.len()
        );
        println!("  first pixel: {:?} (expected [220, 40, 40, 255])", &m.rgba[0..4]);
        println!(
            "  pixels match exactly: {}",
            m.rgba.chunks_exact(4).all(|p| p == [220, 40, 40, 255])
        );
    }

    println!("== a semi-transparent picture, PNG-style ==");
    let (w2, h2) = (3u32, 3u32);
    let mut soft = vec![0u8; (w2 * h2 * 4) as usize];
    for (i, px) in soft.chunks_exact_mut(4).enumerate() {
        px.copy_from_slice(&[10, 200, 30, (i as u8) * 25]);
    }
    let mut doc2 =
        PdfiumDocument::open_path(root.join("fixtures/two-column.pdf").to_str().unwrap(), None)
            .unwrap();
    let idx2 = doc2
        .add_annotation(
            0,
            &Annotation::Image {
                rect: Rect { left: 50.0, top: 50.0, right: 80.0, bottom: 80.0 },
                rgba: soft.clone(),
                width: w2,
                height: h2,
            },
        )
        .unwrap();
    doc2.mark_as_signature(0, idx2, "Soft").unwrap();
    let in_session = doc2.image_signature_marks(0).unwrap();
    if let Some(m) = in_session.first() {
        println!(
            "in-session alpha (before any save): {:?}",
            m.rgba.chunks_exact(4).map(|p| p[3]).collect::<Vec<_>>()
        );
    }
    let mut saved2 = Vec::new();
    doc2.save_full_copy(&mut saved2).unwrap();
    let reopened2 = PdfiumDocument::open_bytes(saved2, None).unwrap();
    let marks2 = reopened2.image_signature_marks(0).unwrap();
    println!("count: {}", marks2.len());
    if let Some(m) = marks2.first() {
        println!("alpha values: {:?}", m.rgba.chunks_exact(4).map(|p| p[3]).collect::<Vec<_>>());
        println!("expected     : {:?}", soft.chunks_exact(4).map(|p| p[3]).collect::<Vec<_>>());
    }
}
