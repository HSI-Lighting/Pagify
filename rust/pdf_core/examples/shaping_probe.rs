//! Can we write Persian into a PDF and get Persian back out?
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=/tmp/pdfw/bin/pdfium.dll \
//!   cargo run --example shaping_probe -- <font.ttf> <scratch-dir>
//! ```
//!
//! The app writes text one character at a time, each as its own positioned object,
//! at widths taken from the standard-14 metric tables. For English that is exactly
//! right. For Arabic it produces what the user reported and the screenshot shows:
//! every letter in its isolated form, unjoined, and laid out left to right so the
//! word reads backwards.
//!
//! Fixing it needs three things that are each a separate gamble:
//!
//!   1. a shaper that turns a string into the *glyphs* a font actually draws —
//!      joined forms, ligatures, marks placed over their letters,
//!   2. PDFium accepting a font file of ours, embedding it, and letting us write
//!      by glyph id rather than by character,
//!   3. the text still being *text* afterwards — selectable, searchable, and
//!      extractable as the Persian that went in, not as a list of glyph numbers.
//!
//! Any one of them failing changes the design, so all three are asked here before
//! anything is built. The third is the one most likely to fail quietly: a file
//! that looks perfect and whose words cannot be found or copied.

use pdf_core::document::pdfium_doc::pdfium;
use pdfium_render::prelude::*;
use rustybuzz::{Direction, Face, UnicodeBuffer};
use std::os::raw::c_uint;

/// Persian for "Arabic" — the word from the report, and a good test: it joins,
/// it has a final form, and it ends in a taa marbuta.
static mut SAMPLE: String = String::new();
fn sample() -> &'static str {
    // Safety: set once, before anything reads it.
    unsafe { &*std::ptr::addr_of!(SAMPLE) }
}

/// English through the same path, because a fix that breaks Latin is not a fix.
const ENGLISH: &str = "Hello";

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let font_path = args.get(1).expect("usage: shaping_probe <font.ttf> <scratch>");
    let scratch = args.get(2).expect("usage: shaping_probe <font.ttf> <scratch>");
    let written = format!("{scratch}/shaped.pdf");

    // Safety: written once, before the first read.
    unsafe {
        SAMPLE = args.get(3).cloned().unwrap_or_else(|| "العربية".to_string());
    }
    let font_data = std::fs::read(font_path).expect("read the font");
    println!("font: {} bytes", font_data.len());

    // ------------------------------------------------------------- shaping --
    let face = Face::from_slice(&font_data, 0).expect("parse the font");
    println!("units per em: {}", face.units_per_em());

    let persian = shape(&face, sample(), Direction::RightToLeft);
    let english = shape(&face, ENGLISH, Direction::LeftToRight);

    println!(
        "{:?}: {} chars -> {} glyphs",
        sample(),
        sample().chars().count(),
        persian.len(),
    );
    for g in &persian {
        println!("   glyph {:>5}  advance {:>6}  cluster {}", g.id, g.advance, g.cluster);
    }

    // Did joining actually happen? The isolated glyph for a letter is what the
    // old path drew; if shaping returns the same ids, the shaper ran but did
    // nothing and the output would look exactly as broken as before.
    let ain = 'ع';
    let isolated = face
        .glyph_index(ain)
        .expect("the font has no ain at all")
        .0;
    let joined_ain = persian.iter().any(|g| g.id != isolated as u32);
    println!("isolated ain is glyph {isolated}; shaping produced different forms: {joined_ain}");
    assert!(
        persian.len() >= 4 && joined_ain,
        "the shaper returned isolated forms — no joining happened",
    );

    // Right-to-left means the glyphs come back in visual order already: the
    // first glyph of the run is the *rightmost* on the page. Laying them out
    // left to right from here is then correct, which is the whole point of
    // asking the shaper rather than reversing the string ourselves.

    // ------------------------------------------------------------ the file --
    let pdfium = pdfium().expect("pdfium");
    let bindings = pdfium.bindings();
    let document = pdfium.create_new_pdf().expect("new document");
    let handle = document.handle();

    // Safety: every handle is checked, the page is closed before the save, and
    // the font data outlives the call that copies it.
    unsafe {
        // A CID font, because a glyph id does not fit in a byte and a simple
        // font cannot address more than 256 of them. This is also what makes
        // the file valid for a script whose font has thousands of glyphs.
        //
        // LoadCidType2Font rather than LoadFont because of the ToUnicode: the
        // simpler call builds one by running the font's cmap backwards, and a
        // joined form has no character to run back to. The words drew perfectly
        // and came out of the file as "اϨʹ۰ՍЪة" — unsearchable, uncopyable, and
        // silent about it. This one takes a CMap we build from what the shaper
        // told us each glyph came from.
        // Two bytes per CID, big-endian, mapping each to itself.
        let glyph_count = face.number_of_glyphs() as usize;
        let cid_to_gid: Vec<u8> = (0..glyph_count)
            .flat_map(|gid| [(gid >> 8) as u8, gid as u8])
            .collect();
        println!("CIDToGIDMap: {} glyphs, {} bytes", glyph_count, cid_to_gid.len());

        let to_unicode = to_unicode_cmap(&[(sample(), &persian), (ENGLISH, &english)]);
        println!("ToUnicode CMap: {} bytes", to_unicode.len());
        let font = bindings.FPDFText_LoadCidType2Font(
            handle,
            font_data.as_ptr(),
            font_data.len() as c_uint,
            &to_unicode,
            // An explicit identity table. Passing none was refused outright —
            // the call returned null with nothing said about why.
            cid_to_gid.as_ptr(),
            cid_to_gid.len() as c_uint,
        );
        assert!(!font.is_null(), "PDFium would not take the font");
        println!("font embedded");

        let page = bindings.FPDFPage_New(handle, 0, 595.0, 842.0);
        assert!(!page.is_null(), "page was not made");

        write(bindings, handle, page, font, &persian, 60.0, 700.0, 28.0);
        write(bindings, handle, page, font, &english, 60.0, 640.0, 28.0);

        assert!(
            bindings.FPDFPage_GenerateContent(page) != 0,
            "the page's content was not written",
        );
        bindings.FPDF_ClosePage(page);
    }

    document.save_to_file(&written).expect("save");
    println!("wrote {written}");

    // ---------------------------------------------------------- read back --
    let reopened = pdfium.load_pdf_from_file(&written, None).expect("reopen");
    let page = reopened.pages().get(0).expect("page 0");
    let extracted = page.text().expect("text page").all();
    println!("extracted: {extracted:?}");

    let persian_survives = extracted.contains(sample());
    let english_survives = extracted.contains(ENGLISH);
    println!("the Persian comes back as Persian: {persian_survives}");
    println!("the English comes back as English: {english_survives}");

    // And is anything actually on the page? Text that extracts perfectly and
    // draws nothing is the other way this fails.
    let objects = page.objects().len();
    println!("objects on the reopened page: {objects}");
    assert!(objects >= 2, "the words did not survive as page objects");

    assert!(english_survives, "the Latin text did not survive the round trip");
    assert!(
        persian_survives,
        "the words are drawn but cannot be searched or copied: no usable ToUnicode",
    );
    render_to_png(&written, &format!("{scratch}/shaped.png"));
    println!("VERDICT: shaped, embedded, drawn, and still text afterwards");
}

/// The ToUnicode CMap: which characters each glyph stands for.
///
/// Built from the shaper's clusters. A cluster is the byte offset in the source
/// string that a glyph came from, so the characters a glyph represents run from
/// its own cluster to the next one along — which is how one glyph can honestly
/// claim a two-character ligature, and how three marks stacked on a letter can
/// all claim the same one.
fn to_unicode_cmap(runs: &[(&str, &[Shaped])]) -> String {
    let mut entries: Vec<(u32, String)> = Vec::new();

    for (text, glyphs) in runs {
        // The cluster boundaries present in this run, in ascending order. The
        // shaper emits them descending for right-to-left, and "the next boundary
        // along" is a fact about the string, not about the drawing order.
        let mut boundaries: Vec<usize> = glyphs.iter().map(|g| g.cluster as usize).collect();
        boundaries.sort_unstable();
        boundaries.dedup();

        for glyph in glyphs.iter() {
            let start = glyph.cluster as usize;
            let end = boundaries
                .iter()
                .find(|&&b| b > start)
                .copied()
                .unwrap_or(text.len());
            if let Some(slice) = text.get(start..end) {
                entries.push((glyph.id, slice.to_string()));
            }
        }
    }

    // One entry per glyph id. A glyph used twice must map the same way both
    // times, and a duplicate entry in a CMap is undefined rather than harmless.
    entries.sort_by_key(|(id, _)| *id);
    entries.dedup_by_key(|(id, _)| *id);

    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin
         12 dict begin
         begincmap
         /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> def
         /CMapName /Adobe-Identity-UCS def
         /CMapType 2 def
         1 begincodespacerange
         <0000> <FFFF>
         endcodespacerange
",
    );

    // In chunks, because a CMap may not declare more than 100 in one block.
    for chunk in entries.chunks(100) {
        cmap.push_str(&format!("{} beginbfchar
", chunk.len()));
        for (id, text) in chunk {
            let utf16: String = text
                .encode_utf16()
                .map(|unit| format!("{unit:04X}"))
                .collect();
            cmap.push_str(&format!("<{id:04X}> <{utf16}>
"));
        }
        cmap.push_str("endbfchar
");
    }

    cmap.push_str(
        "endcmap
         CMapName currentdict /CMap defineresource pop
         end
         end
",
    );
    cmap
}

/// One shaped glyph: what to draw and how far it moves the pen.
struct Shaped {
    id: u32,
    advance: i32,
    cluster: u32,
}

fn shape(face: &Face, text: &str, direction: Direction) -> Vec<Shaped> {
    let mut buffer = UnicodeBuffer::new();
    buffer.push_str(text);
    buffer.set_direction(direction);
    let output = rustybuzz::shape(face, &[], buffer);

    output
        .glyph_infos()
        .iter()
        .zip(output.glyph_positions())
        .map(|(info, position)| Shaped {
            id: info.glyph_id,
            advance: position.x_advance,
            cluster: info.cluster,
        })
        .collect()
}

/// Write one shaped run as a single text object.
///
/// Safety: `page` and `font` must be live.
unsafe fn write(
    bindings: &dyn PdfiumLibraryBindings,
    document: FPDF_DOCUMENT,
    page: FPDF_PAGE,
    font: FPDF_FONT,
    glyphs: &[Shaped],
    x: f64,
    y: f64,
    size: f32,
) {
    let object = bindings.FPDFPageObj_CreateTextObj(document, font, size);
    assert!(!object.is_null(), "text object was not created");

    // Charcodes, not characters: with a CID font these are glyph ids, which is
    // the only way to ask for a joined form — it has no character of its own.
    let codes: Vec<u32> = glyphs.iter().map(|g| g.id).collect();
    let set = bindings.FPDFText_SetCharcodes(object, codes.as_ptr(), codes.len());
    assert!(set != 0, "the glyph ids were refused");

    bindings.FPDFPageObj_SetFillColor(object, 20, 20, 20, 255);
    bindings.FPDFPageObj_Transform(object, 1.0, 0.0, 0.0, 1.0, x, y);
    bindings.FPDFPage_InsertObject(page, object);
}


/// Render page 0 to a PNG, so the words can be looked at rather than described.
///
/// The file saying "العربية" and the page *showing* Persian are two different
/// claims, and the second is the one that was reported broken.
fn render_to_png(path: &str, out: &str) {
    use pdf_core::document::{Document, RenderRequest};
    use pdf_core::render::{PixelOrder, RenderTarget};

    let doc = pdf_core::document::pdfium_doc::PdfiumDocument::open_path(path, None)
        .expect("reopen for rendering");
    let page = doc.page(0).expect("page 0");
    let request = RenderRequest { scale: 2.0, ..Default::default() };
    let (width, height) = page.size().pixel_size(request.scale);

    let mut pixels = vec![0u8; (width * height * 4) as usize];
    let mut target = RenderTarget::new(width, height, (width * 4) as usize, PixelOrder::Rgba, &mut pixels)
        .expect("target");
    page.render_into(&request, &mut target).expect("render");

    image::RgbaImage::from_raw(width, height, pixels)
        .expect("image")
        .save(out)
        .expect("write the png");
    println!("rendered {out}");
}
