//! Does rewriting one run of text damage the rest of the page?
//!
//! Renders the page, rewrites a single run, renders again, and compares. Pixels
//! that changed **outside** the run that was edited are damage: colour shifts
//! from a content stream regenerated in the wrong colour space, glyphs redrawn
//! in a substituted font, whatever PDFium did not model faithfully the first
//! time.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo run --example edit_damage_probe -- <pdf> [page] [run]
//! ```

use pdfium_render::prelude::*;

const SCALE: f32 = 2.0;

fn render(page: &PdfPage) -> (u32, u32, Vec<u8>) {
    let config = PdfRenderConfig::new().scale_page_by_factor(SCALE);
    let bitmap = page.render_with_config(&config).expect("render");
    (
        bitmap.width() as u32,
        bitmap.height() as u32,
        bitmap.as_raw_bytes().to_vec(),
    )
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let index: i32 = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let which: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
    let lib = std::env::var("PAGIFY_PDFIUM_LIB").expect("PAGIFY_PDFIUM_LIB");
    let pdfium = Pdfium::new(Pdfium::bind_to_library(&lib).expect("bind"));
    let doc = pdfium.load_pdf_from_file(&a[0], None).expect("open");

    let mut page = doc.pages().get(index).expect("page");
    let (w, h, before) = render(&page);

    // The run to rewrite, and where it sits, so its own pixels can be excluded.
    let mut target = None;
    let mut seen = 0usize;
    for object in page.objects().iter() {
        if let Some(t) = object.as_text_object() {
            if t.text().trim().is_empty() {
                continue;
            }
            if seen == which {
                let b = object.bounds().expect("bounds");
                target = Some((t.text(), b));
                break;
            }
            seen += 1;
        }
    }
    let Some((words, bounds)) = target else {
        println!("no run {which} on this page");
        return;
    };
    println!("rewriting run {which}: {:?}", words.chars().take(40).collect::<String>());

    let mut wrote = false;
    for mut object in page.objects_mut().iter() {
        if let Some(t) = object.as_text_object_mut() {
            if t.text() == words {
                wrote = t.set_text("PAGIFY").is_ok();
                break;
            }
        }
    }
    if !wrote {
        println!("  the run refused to be rewritten (a subsetted font, most likely)");
        return;
    }
    page.regenerate_content().expect("regenerate");

    let (w2, h2, after) = render(&page);
    if (w, h) != (w2, h2) {
        println!("  *** the page changed size: {w}x{h} -> {w2}x{h2} ***");
        return;
    }

    // The run's own box, in pixels, with a generous margin — a replacement of a
    // different length legitimately paints outside the original.
    let page_h = page.height().value;
    let to_px = |v: f32| (v * SCALE) as i64;
    let (x0, x1) = (to_px(bounds.left().value) - 40, to_px(bounds.right().value) + 400);
    let (y0, y1) = (
        to_px(page_h - bounds.top().value) - 40,
        to_px(page_h - bounds.bottom().value) + 40,
    );

    let (mut inside, mut outside) = (0u64, 0u64);
    let mut first: Option<(i64, i64)> = None;
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            let i = ((y as usize * w as usize) + x as usize) * 4;
            if before[i..i + 4] == after[i..i + 4] {
                continue;
            }
            if x >= x0 && x <= x1 && y >= y0 && y <= y1 {
                inside += 1;
            } else {
                outside += 1;
                first.get_or_insert((x, y));
            }
        }
    }

    let total = (w as u64) * (h as u64);
    println!("  changed inside the run's area : {inside}");
    println!("  changed elsewhere on the page : {outside}  ({:.3}% of the page)",
        outside as f64 * 100.0 / total as f64);
    // Where the changes actually are. A compact cluster is the edit itself
    // (possibly outside a box this probe placed wrongly); changes scattered
    // across the page are damage.
    let (mut lx, mut ly, mut hx, mut hy) = (i64::MAX, i64::MAX, i64::MIN, i64::MIN);
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            let i = ((y as usize * w as usize) + x as usize) * 4;
            if before[i..i + 4] != after[i..i + 4] {
                lx = lx.min(x); ly = ly.min(y); hx = hx.max(x); hy = hy.max(y);
            }
        }
    }
    if lx <= hx {
        println!("  everything that changed fits in {}x{} at {lx},{ly}", hx - lx + 1, hy - ly + 1);
    }
    let _ = first;

    if let Some(out) = a.get(3) {
        let save = |name: &str, data: &[u8]| {
            image::save_buffer(name, data, w, h, image::ColorType::Rgba8).expect("write");
        };
        save(&format!("{out}-before.png"), &before);
        save(&format!("{out}-after.png"), &after);
        println!("  wrote {out}-before.png and {out}-after.png");
    }
}
