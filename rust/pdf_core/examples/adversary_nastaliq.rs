//! Adversarial re-test of the "pdfium-survival" claim, using the PRODUCTION
//! functions: pdf_core::text::{shape_with, subset, identity_table,
//! to_unicode_from_glyphs} and the same PDFium calls write_text() makes.
use pdf_core::document::pdfium_doc::pdfium;
use pdf_core::document::Glyph;
use pdfium_render::prelude::*;
use rustybuzz::Face;
use std::collections::HashMap;
use std::os::raw::c_uint;

fn main() {
    let font_path = std::env::args().nth(1).expect("font");
    let out = std::env::args().nth(2).expect("outdir");
    let text = std::env::args().nth(3).expect("text");

    let data = std::fs::read(&font_path).expect("read font");
    pdf_core::text::register("Probe", data.clone()).expect("register");
    let face = Face::from_slice(&data, 0).expect("parse");
    let upem = face.units_per_em() as f32;
    println!("FONT  {} glyphs={} upem={} outlines={}",
        font_path, face.number_of_glyphs(), upem,
        if face.tables().glyf.is_some() { "glyf" } else { "CFF" });

    // Production shaper.
    let shaped = pdf_core::text::shape_with(&face, &text);
    println!("SHAPED {} glyphs, rtl={}", shaped.glyphs.len(), shaped.right_to_left);

    // Emulate the platform layer (TextLayout.kt): pen walk + offsetX/offsetY.
    let size = 36.0f32;
    let mut glyphs: Vec<Glyph> = Vec::new();
    let mut pen = 0.0f32;
    let mut nonzero_dy = 0;
    for g in &shaped.glyphs {
        if g.offset_y != 0.0 { nonzero_dy += 1; }
        glyphs.push(Glyph {
            ch: String::new(),
            id: g.id,
            x: (pen + g.offset_x) * size,
            y: (-g.offset_y) * size,
            radians: 0.0,
        });
        pen += g.advance;
    }
    println!("GPOS  glyphs with non-zero y-offset: {}/{}", nonzero_dy, shaped.glyphs.len());

    // --- ToUnicode honesty check: how many DISTINCT texts share one glyph id? ---
    let mut per_id: HashMap<u32, Vec<String>> = HashMap::new();
    let mut bounds: Vec<usize> = shaped.glyphs.iter().map(|g| g.cluster as usize).collect();
    bounds.sort_unstable(); bounds.dedup();
    for g in &shaped.glyphs {
        let s = g.cluster as usize;
        let e = bounds.iter().find(|&&b| b > s).copied().unwrap_or(text.len());
        if let Some(sl) = text.get(s..e) {
            per_id.entry(g.id).or_default().push(sl.to_string());
        }
    }
    let mut collisions = 0;
    for (id, v) in &per_id {
        let mut u: Vec<&String> = v.iter().collect(); u.sort(); u.dedup();
        if u.len() > 1 { collisions += 1;
            if collisions <= 6 { println!("  TOUNICODE COLLISION gid {} means {:?}", id, u); } }
    }
    println!("TOUNICODE {} distinct gids, {} of them stand for >1 different string",
        per_id.len(), collisions);

    // Fill ch from cluster spans, as the platform layer does.
    for (i, g) in shaped.glyphs.iter().enumerate() {
        let st = g.cluster as usize;
        let en = bounds.iter().find(|&&b| b > st).copied().unwrap_or(text.len());
        glyphs[i].ch = text.get(st..en).unwrap_or("").to_string();
    }

    // --- Production subsetter ---
    let wanted: Vec<u32> = glyphs.iter().map(|g| g.id).collect();
    let sub = pdf_core::text::subset("Probe", &wanted).expect("subset");
    let cid = pdf_core::text::identity_table(sub.glyph_count);
    println!("SUBSET bytes={} declared glyph_count={} cid_to_gid_entries={}",
        sub.data.len(), sub.glyph_count, cid.len() / 2);
    let sface = Face::from_slice(&sub.data, 0).expect("parse subset");
    println!("SUBSET actual numGlyphs in file = {}", sface.number_of_glyphs());
    if sface.number_of_glyphs() != sub.glyph_count {
        println!("  !! MISMATCH: CIDToGIDMap covers {} of {} glyphs",
            sub.glyph_count, sface.number_of_glyphs());
    }

    let renumbered: Vec<Glyph> = glyphs.iter().zip(&sub.ids)
        .map(|(g, &id)| Glyph { id, ..g.clone() }).collect();
    let to_unicode = pdf_core::text::to_unicode_from_glyphs(&renumbered);

    // --- Two documents: A = subset (production), B = full font (control) ---
    let pdfium = pdfium().expect("pdfium");
    let b = pdfium.bindings();
    for (label, fdata, ids, count) in [
        ("A_subset", sub.data.clone(), sub.ids.clone(), sub.glyph_count),
        ("B_full", data.clone(), wanted.clone(), face.number_of_glyphs()),
    ] {
        let cidmap: Vec<u8> = (0..count as usize).flat_map(|g| [(g >> 8) as u8, g as u8]).collect();
        let doc = pdfium.create_new_pdf().expect("new");
        let h = doc.handle();
        unsafe {
            let font = b.FPDFText_LoadCidType2Font(h, fdata.as_ptr(), fdata.len() as c_uint,
                &to_unicode, cidmap.as_ptr(), cidmap.len() as c_uint);
            if font.is_null() { println!("{label}: FONT REFUSED"); continue; }
            let page = b.FPDFPage_New(h, 0, 900.0, 300.0);
            // Exactly write_text(): one text object per glyph, absolute transform.
            for (i, g) in glyphs.iter().enumerate() {
                let obj = b.FPDFPageObj_CreateTextObj(h, font, size);
                let code = [ids.get(i).copied().unwrap_or(0)];
                assert!(b.FPDFText_SetCharcodes(obj, code.as_ptr(), 1) != 0, "{label} gid refused");
                b.FPDFPageObj_SetFillColor(obj, 0, 0, 0, 255);
                // RTL: pen runs right-to-left from the right margin.
                let x = 850.0 - g.x as f64;
                b.FPDFPageObj_Transform(obj, 1.0, 0.0, 0.0, 1.0, x, 150.0 + g.y as f64);
                b.FPDFPage_InsertObject(page, obj);
            }
            assert!(b.FPDFPage_GenerateContent(page) != 0);
            b.FPDF_ClosePage(page);
        }
        let path = format!("{out}/{label}.pdf");
        doc.save_to_file(&path).expect("save");
        println!("{label}: wrote {} ({} bytes)", path, std::fs::metadata(&path).unwrap().len());
    }
}
