//! Does a run's rectangle sit on the ink it claims to describe?
//!
//! The pick tests a click against a run's box. If the box and the drawn glyphs
//! disagree, everything inside the program is consistent and a person clicking
//! what they can see misses every time.
use pdf_core::document::Document;

fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let page: usize = std::env::args().nth(2).and_then(|p| p.parse().ok()).unwrap_or(0);

    let doc = pdf_core::document::pdfium_doc::PdfiumDocument::open_path(&path, None).expect("open");
    let size = doc.page_size(page).expect("size");
    let scale = 3.0f32;
    let (w, h) = ((size.width_pt * scale) as u32, (size.height_pt * scale) as u32);
    let mut pixels = vec![0u8; (w * h * 4) as usize];
    let mut target = pdf_core::render::RenderTarget {
        width: w,
        height: h,
        stride: (w * 4) as usize,
        order: pdf_core::render::PixelOrder::Rgba,
        pixels: &mut pixels,
    };
    doc.page(page)
        .expect("page")
        .render_into(
            &pdf_core::document::RenderRequest { scale, ..Default::default() },
            &mut target,
        )
        .expect("render");

    let inked = |l: f32, t: f32, r: f32, b: f32| -> f32 {
        let (x0, y0) = ((l * scale).max(0.0) as u32, (t * scale).max(0.0) as u32);
        let (x1, y1) = (((r * scale) as u32).min(w), ((b * scale) as u32).min(h));
        let (mut dark, mut seen) = (0usize, 0usize);
        for y in y0..y1 {
            for x in x0..x1 {
                let at = ((y * w + x) * 4) as usize;
                seen += 1;
                if pixels[at] < 200 || pixels[at + 1] < 200 || pixels[at + 2] < 200 {
                    dark += 1;
                }
            }
        }
        100.0 * dark as f32 / seen.max(1) as f32
    };

    println!("page {} at {scale}x — {w}x{h} px\n", page + 1);
    println!("{:>4}  {:>7}  {:>7} {:>7} {:>7}   {}", "obj", "in box", "above", "below", "shifted", "text");
    for run in doc.text_runs(page).expect("runs").iter().take(10) {
        let r = run.rect;
        let height = (r.bottom - r.top).abs().max(1.0);
        let here = inked(r.left, r.top, r.right, r.bottom);
        let above = inked(r.left, r.top - height, r.right, r.top);
        let below = inked(r.left, r.bottom, r.right, r.bottom + height);
        // Where the ink actually is, if it is not in the box.
        let verdict = if here > 2.0 {
            "on it"
        } else if above > here && above > 2.0 {
            "ABOVE the box"
        } else if below > here && below > 2.0 {
            "BELOW the box"
        } else {
            "no ink anywhere near"
        };
        println!(
            "{:>4}  {here:>6.1}%  {above:>6.1}% {below:>6.1}%  {verdict:<20} {:?}",
            run.object,
            run.text.chars().take(22).collect::<String>()
        );
    }
}
