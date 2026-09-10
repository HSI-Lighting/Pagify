//! Lock some words and a picture with one passcode, and look at the page.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo run --release --example one_passcode_demo -- <file.pdf> <out-dir>
//! ```

use pdf_core::document::{Document, DocumentMut, Rect, Redaction};

type Doc = pdf_core::document::pdfium_doc::PdfiumDocument;

fn render(doc: &dyn Document, page: usize, to: &str) {
    let size = doc.page_size(page).expect("size");
    let width = 700u32;
    let scale = width as f32 / size.width_pt;
    let height = (size.height_pt * scale).max(1.0) as u32;
    let mut pixels = vec![0u8; (width * height * 4) as usize];
    let mut target = pdf_core::render::RenderTarget {
        width, height, stride: (width * 4) as usize,
        order: pdf_core::render::PixelOrder::Rgba, pixels: &mut pixels,
    };
    doc.page(page).expect("page")
        .render_into(&pdf_core::document::RenderRequest { scale, ..Default::default() }, &mut target)
        .expect("render");

    // A PNG, written by hand — no image crate in this dependency tree.
    let mut raw = Vec::with_capacity((width * height * 4 + height) as usize);
    for row in 0..height {
        raw.push(0u8);
        let at = (row * width * 4) as usize;
        raw.extend_from_slice(&pixels[at..at + (width * 4) as usize]);
    }
    std::fs::write(to, png(&raw, width, height)).expect("write");
}

fn png(raw: &[u8], width: u32, height: u32) -> Vec<u8> {
    fn chunk(kind: &[u8], body: &[u8]) -> Vec<u8> {
        let mut out = (body.len() as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(body);
        let mut crc = 0xFFFF_FFFFu32;
        for byte in kind.iter().chain(body) {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
            }
        }
        out.extend_from_slice(&(!crc).to_be_bytes());
        out
    }
    let mut header = width.to_be_bytes().to_vec();
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 0]);

    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    out.extend_from_slice(&chunk(b"IHDR", &header));
    out.extend_from_slice(&chunk(b"IDAT", &pdf_core::pdf::content::encode(raw).expect("deflate")));
    out.extend_from_slice(&chunk(b"IEND", &[]));
    out
}

fn main() {
    const BOTH: &[u8] = b"Correct-Horse-99-Battery";
    let path = std::env::args().nth(1).expect("a pdf path");
    let dir = std::env::args().nth(2).expect("an output directory");

    let mut doc = Doc::open_path(&path, None).expect("open");
    let page = (0..doc.page_count().min(60))
        .find(|p| {
            doc.images_on(*p).map(|i| !i.is_empty()).unwrap_or(false)
                && doc.page(*p).and_then(|q| q.characters()).map(|c| c.text.chars().count()).unwrap_or(0) > 40
        })
        .expect("a page with text and an image");
    println!("page {}", page + 1);
    render(&doc, page, &format!("{dir}/1-before.png"));

    let before = doc.page(page).expect("page").characters().expect("characters");
    let text = before.text.clone();
    let words: Vec<&str> = text.split_whitespace().collect();
    let phrase = words.windows(3).map(|w| w.join(" "))
        .find(|p| text.matches(p.as_str()).count() == 1)
        .expect("a phrase");
    let at = text[..text.find(&phrase).expect("at")].chars().count();

    let mut area = Rect { left: f32::MAX, top: f32::MAX, right: f32::MIN, bottom: f32::MIN };
    for i in at..at + phrase.chars().count() {
        let b = &before.boxes[i * 4..i * 4 + 4];
        area.left = area.left.min(b[0]); area.top = area.top.min(b[1]);
        area.right = area.right.max(b[2]); area.bottom = area.bottom.max(b[3]);
    }

    doc.lock_area(&Redaction { require_complete: false, ..Redaction::new(page, area) }, BOTH, None)
        .expect("lock the words");
    println!("locked the words  : {phrase:?}");

    let image = doc.images_on(page).expect("images")[0].object;
    doc.lock_image(page, image, BOTH).expect("lock the image with the same passcode");
    println!("locked the picture: with the very same passcode");
    render(&doc, page, &format!("{dir}/2-locked.png"));

    // Through the file, then back with the one passcode.
    let mut bytes = Vec::new();
    doc.save_full_copy(&mut bytes).expect("save");
    let mut again = Doc::open_bytes(bytes, None).expect("reopen");
    for (index, pdf) in again.open_lock(BOTH).expect("one passcode opens both") {
        again.replace_page(index, &pdf).expect("restore");
    }
    println!("one passcode brought both back");
    render(&again, page, &format!("{dir}/3-unlocked.png"));
}
