//! How long a glyph catalogue takes to build, and how big it is.
//!
//! The outlined-text matcher compares a page's contours against every glyph of
//! every font offered. Both halves of that cost scale with the fonts, so a
//! change to what is bundled is a change to how long reading a page takes.
fn main() {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    let mut bytes = Vec::new();
    for path in &paths {
        match std::fs::read(path) {
            Ok(font) => {
                if let Ok(face) = ttf_parser::Face::parse(&font, 0) {
                    println!(
                        "{}: {} bytes, {} glyphs",
                        path.rsplit('/').next().unwrap_or(path),
                        font.len(),
                        face.number_of_glyphs()
                    );
                }
                bytes.push(font);
            }
            Err(e) => println!("{path}: {e}"),
        }
    }

    let started = std::time::Instant::now();
    let mut catalogue = pdf_core::document::glyphs::Catalogue::default();
    for font in &bytes {
        catalogue.extend_from_font_common(font);
    }
    let elapsed = started.elapsed();
    println!(
        "\ncatalogue: {} entries, built in {:.2}s",
        catalogue.len(),
        elapsed.as_secs_f32()
    );
}
