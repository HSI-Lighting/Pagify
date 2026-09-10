//! Adversarial: does Nastaliq survive SAVE + REOPEN + RENDER after the
//! subsetter has removed GSUB/GPOS/GDEF/cmap? One object per glyph, dx/dy
//! applied exactly as the clients do.
use pdf_core::document::pdfium_doc::pdfium;
use pdfium_render::prelude::*;
use rustybuzz::{Direction, Face, UnicodeBuffer};
use std::os::raw::c_uint;

fn main() {
    let font_path = std::env::args().nth(1).expect("font");
    let scratch = std::env::args().nth(2).expect("scratch");
    let pdfium = pdfium().expect("pdfium");
    let bindings = pdfium.bindings();
    let full = std::fs::read(&font_path).expect("read font");
    let face = Face::from_slice(&full, 0).expect("parse");
    let upem = face.units_per_em() as f32;

    for (word_label, word) in [("zwnj", "می\u{200c}روم"), ("plain", "میروم")] {
    for use_subset in [true, false] {
        let label = format!("{word_label}_{}", if use_subset {"subset"} else {"fullfont"});
        let mut buf = UnicodeBuffer::new();
        buf.push_str(word);
        buf.set_direction(Direction::RightToLeft);
        buf.set_script(rustybuzz::script::ARABIC);
        let shaped = rustybuzz::shape(&face, &[], buf);
        let ids: Vec<u32> = shaped.glyph_infos().iter().map(|i| i.glyph_id).collect();
        let pos = shaped.glyph_positions().to_vec();

        let mut remapper = subsetter::GlyphRemapper::new();
        let sub_ids: Vec<u32> = ids.iter().map(|&i| remapper.remap(i as u16) as u32).collect();
        let sub_bytes = subsetter::subset(&full, 0, &remapper).expect("subset");
        let (sub, sub_ids, count) = if use_subset {
            (sub_bytes.clone(), sub_ids.clone(), remapper.num_gids())
        } else {
            (full.clone(), ids.clone(), face.number_of_glyphs())
        };
        let cid: Vec<u8> = (0..count as usize).flat_map(|g| [(g >> 8) as u8, g as u8]).collect();

        let mut tu = String::from("/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> def\n/CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n");
        tu.push_str(&format!("{} beginbfchar\n", sub_ids.len()));
        for (g, ch) in sub_ids.iter().zip(word.chars()) {
            tu.push_str(&format!("<{:04X}> <{:04X}>\n", g, ch as u32));
        }
        tu.push_str("endbfchar\nendcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");

        let doc = pdfium.create_new_pdf().expect("new");
        let h = doc.handle();
        let size = 72.0f32;
        unsafe {
            let font = bindings.FPDFText_LoadCidType2Font(
                h, sub.as_ptr(), sub.len() as c_uint, &tu, cid.as_ptr(), cid.len() as c_uint);
            assert!(!font.is_null(), "{label}: font refused");
            let page = bindings.FPDFPage_New(h, 0, 420.0, 220.0);
            // RTL: pen starts at the right edge and walks left, exactly as a
            // client laying out a right-to-left run must.
            let mut pen_x = 360.0f32;
            let pen_y = 90.0f32;
            for (i, &gid) in sub_ids.iter().enumerate() {
                let adv = pos[i].x_advance as f32 / upem * size;
                let dx = pos[i].x_offset as f32 / upem * size;
                let dy = pos[i].y_offset as f32 / upem * size;
                pen_x -= adv;
                let obj = bindings.FPDFPageObj_CreateTextObj(h, font, size);
                let code = [gid];
                assert!(bindings.FPDFText_SetCharcodes(obj, code.as_ptr(), 1) != 0);
                bindings.FPDFPageObj_Transform(obj, 1.0, 0.0, 0.0, 1.0,
                    (pen_x + dx) as f64, (pen_y + dy) as f64);
                bindings.FPDFPage_InsertObject(page, obj);
            }
            assert!(bindings.FPDFPage_GenerateContent(page) != 0);
            bindings.FPDF_ClosePage(page);
        }
        let out = format!("{scratch}/adv_{label}.pdf");
        doc.save_to_file(&out).expect("save");
        drop(doc);

        // REOPEN the saved file and render with PDFium's own rasteriser.
        let re = pdfium.load_pdf_from_file(&out, None).expect("reopen");
        let page = re.pages().get(0).expect("page");
        let cfg = PdfRenderConfig::new().scale_page_by_factor(3.0);
        let bmp = page.render_with_config(&cfg).expect("render");
        let (w, hgt) = (bmp.width() as u32, bmp.height() as u32);
        let raw = bmp.as_rgba_bytes();
        let ink = raw.chunks(4).filter(|p| p[0] < 200 && p[1] < 200 && p[2] < 200).count();
        let img: image::RgbaImage = image::ImageBuffer::from_raw(w, hgt, raw.clone()).expect("buf");
        img.save(format!("{scratch}/adv_{label}.png")).expect("png");
        // Vertical spread of ink: a Nastaliq cascade descends, a flat run does not.
        let rows: Vec<u32> = (0..hgt).filter(|&y| (0..w).any(|x| {
            let i = ((y * w + x) * 4) as usize; raw[i] < 200 && raw[i+1] < 200 && raw[i+2] < 200
        })).collect();
        let spread = rows.last().copied().unwrap_or(0) as i64 - rows.first().copied().unwrap_or(0) as i64;
        // Extracted text, to prove the words survive as words.
        let text = page.text().expect("text").all();
        println!("{label}: subset {} bytes, {} glyphs, ink px={ink}, ink-rows-span={spread}px @3x, extracted={:?} (source={:?})",
            sub.len(), sub_ids.len(), text, word);
    }
    }
}
