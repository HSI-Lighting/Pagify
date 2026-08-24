//! Will a CJK font go into a PDF, and come back out as the words that went in?
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=/tmp/pdfw/bin/pdfium.dll \
//!   cargo run --example cjk_probe -- <font-dir> <scratch-dir>
//! ```
//!
//! Asked separately from the Arabic probe because CJK differs in three ways that
//! each threaten the same design, and none of them shows up on a 250 kB Naskh:
//!
//!   1. **Size.** These are ten to eighteen megabytes apiece. PDFium embeds a
//!      font whole, so every caption in Chinese puts the entire font in the file.
//!      Worth knowing the real number before shipping four of them.
//!   2. **Variable fonts.** The ones Google ships are variable — one file holding
//!      a whole weight axis. A reader that does not understand that draws the
//!      default instance, which is fine; one that mishandles it draws nothing.
//!   3. **Glyph count.** Sixty-odd thousand glyphs means a CID-to-glyph table of
//!      a hundred and thirty thousand bytes, built for every write.
//!
//! Chinese needs no shaping to speak of — one character, one glyph — so what is
//! actually being tested here is embedding and the round trip, not joining.

use pdf_core::document::pdfium_doc::pdfium;
use pdfium_render::prelude::*;
use rustybuzz::{Direction, Face, UnicodeBuffer};
use std::os::raw::c_uint;

/// One script, its font, and something written in it.
const CASES: &[(&str, &str, &str)] = &[
    ("Simplified Chinese", "NotoSansSC.ttf", "中文测试"),
    ("Traditional Chinese", "NotoSansTC.ttf", "繁體中文"),
    ("Japanese", "NotoSansJP.ttf", "日本語のテスト"),
    ("Korean", "NotoSansKR.ttf", "한국어 시험"),
];

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = args.get(1).expect("usage: cjk_probe <font-dir> <scratch-dir>");
    let scratch = args.get(2).expect("usage: cjk_probe <font-dir> <scratch-dir>");
    std::env::var("PAGIFY_PDFIUM_LIB").expect("set PAGIFY_PDFIUM_LIB");

    let pdfium = pdfium().expect("pdfium");
    let bindings = pdfium.bindings();

    for (script, file, words) in CASES {
        let path = format!("{dir}/{file}");
        let font_data = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) => {
                println!("{script}: no {file} ({e})");
                continue;
            }
        };

        let face = Face::from_slice(&font_data, 0).expect("parse the font");
        let glyph_count = face.number_of_glyphs();
        let variable = face.tables().fvar.is_some();

        let mut buffer = UnicodeBuffer::new();
        buffer.push_str(words);
        buffer.set_direction(Direction::LeftToRight);
        let shaped = rustybuzz::shape(&face, &[], buffer);
        let ids: Vec<u32> = shaped.glyph_infos().iter().map(|i| i.glyph_id).collect();

        println!(
            "\n{script}: {} MB, {glyph_count} glyphs, variable={variable}, \
             {} chars -> {} glyphs",
            font_data.len() / 1_048_576,
            words.chars().count(),
            ids.len(),
        );

        // Nothing may come back as notdef. A file full of empty boxes is what
        // "the font embedded fine" looks like when it did not.
        assert!(
            ids.iter().all(|&id| id != 0),
            "{script}: some characters have no glyph in {file}",
        );

        // Subset before embedding. PDFium embeds whatever it is given, whole:
        // a four-character caption was putting a ten-megabyte font in the file.
        // The remapper renumbers the glyphs that survive, so every id below has
        // to be the *new* one.
        let mut remapper = subsetter::GlyphRemapper::new();
        let subset_ids: Vec<u32> = ids
            .iter()
            .map(|&id| remapper.remap(id as u16) as u32)
            .collect();
        let subset = subsetter::subset(&font_data, 0, &remapper).expect("subset");
        let subset_glyphs = remapper.num_gids();
        println!(
            "   subset to {} kB, {} glyphs",
            subset.len() / 1024,
            subset_glyphs,
        );
        let font_data = subset;
        let ids = subset_ids;
        let glyph_count = subset_glyphs;

        let cid_to_gid: Vec<u8> = (0..glyph_count as usize)
            .flat_map(|gid| [(gid >> 8) as u8, gid as u8])
            .collect();
        let to_unicode = cmap_for(words, &shaped, &ids);

        let document = pdfium.create_new_pdf().expect("new document");
        let handle = document.handle();

        // Safety: handles are checked; the page is closed before the save.
        unsafe {
            let font = bindings.FPDFText_LoadCidType2Font(
                handle,
                font_data.as_ptr(),
                font_data.len() as c_uint,
                &to_unicode,
                cid_to_gid.as_ptr(),
                cid_to_gid.len() as c_uint,
            );
            assert!(!font.is_null(), "{script}: PDFium would not embed {file}");

            let page = bindings.FPDFPage_New(handle, 0, 400.0, 160.0);
            assert!(!page.is_null());

            let object = bindings.FPDFPageObj_CreateTextObj(handle, font, 36.0);
            assert!(!object.is_null());
            assert!(
                bindings.FPDFText_SetCharcodes(object, ids.as_ptr(), ids.len()) != 0,
                "{script}: the glyph ids were refused",
            );
            bindings.FPDFPageObj_SetFillColor(object, 20, 20, 20, 255);
            bindings.FPDFPageObj_Transform(object, 1.0, 0.0, 0.0, 1.0, 30.0, 60.0);
            bindings.FPDFPage_InsertObject(page, object);

            assert!(bindings.FPDFPage_GenerateContent(page) != 0);
            bindings.FPDF_ClosePage(page);
        }

        let out = format!("{scratch}/cjk-{file}.pdf");
        document.save_to_file(&out).expect("save");
        let written = std::fs::metadata(&out).expect("stat").len();

        let reopened = pdfium.load_pdf_from_file(&out, None).expect("reopen");
        let extracted = reopened
            .pages()
            .get(0)
            .expect("page 0")
            .text()
            .expect("text page")
            .all();

        println!(
            "   file {} MB, extracted {extracted:?}",
            written / 1_048_576,
        );
        assert!(
            extracted.contains(words),
            "{script}: the words cannot be searched or copied out again",
        );
    }

    println!("\nVERDICT: CJK embeds, draws, and stays text");
}

/// The ToUnicode CMap, from the shaper's clusters.
fn cmap_for(text: &str, shaped: &rustybuzz::GlyphBuffer, ids: &[u32]) -> String {
    let infos = shaped.glyph_infos();
    let mut boundaries: Vec<usize> = infos.iter().map(|i| i.cluster as usize).collect();
    boundaries.sort_unstable();
    boundaries.dedup();

    let mut entries: Vec<(u32, String)> = infos
        .iter()
        .zip(ids)
        .filter_map(|(info, &id)| {
            let start = info.cluster as usize;
            let end = boundaries
                .iter()
                .find(|&&b| b > start)
                .copied()
                .unwrap_or(text.len());
            text.get(start..end).map(|s| (id, s.to_string()))
        })
        .collect();
    entries.sort_by_key(|(id, _)| *id);
    entries.dedup_by_key(|(id, _)| *id);

    let mut out = String::from(
        "/CIDInit /ProcSet findresource begin\n\
         12 dict begin\n\
         begincmap\n\
         /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> def\n\
         /CMapName /Adobe-Identity-UCS def\n\
         /CMapType 2 def\n\
         1 begincodespacerange\n\
         <0000> <FFFF>\n\
         endcodespacerange\n",
    );
    for chunk in entries.chunks(100) {
        out.push_str(&format!("{} beginbfchar\n", chunk.len()));
        for (id, text) in chunk {
            let utf16: String = text.encode_utf16().map(|u| format!("{u:04X}")).collect();
            out.push_str(&format!("<{id:04X}> <{utf16}>\n"));
        }
        out.push_str("endbfchar\n");
    }
    out.push_str(
        "endcmap\n\
         CMapName currentdict /CMap defineresource pop\n\
         end\n\
         end\n",
    );
    out
}
