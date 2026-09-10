//! How well a given face (or faces) recognises `make_text_fixtures.rs`'s
//! outlined-type fixture, built from that same face.
//!
//! Not specific to any one font — pass any number of candidate `.ttf`/`.otf`
//! files, merged into one catalogue exactly as
//! `Catalogue::extend_from_font_common` does at the app layer, and this
//! reports the similarity score the real tests use rather than eyeballing the
//! recovered text.
//!
//! Built to answer one question honestly before bundling a font as a default
//! candidate: does it actually work, on a fixture built from that same font —
//! see `examples/make_text_fixtures.rs`'s `PAGIFY_OUTLINED_FONT` override —
//! rather than assumed from a generic system font's score. The ground truth
//! below is that generator's own fixed text, unrelated to which font drew it.
//!
//! ```text
//! PAGIFY_OUTLINED_FONT=<path> cargo run --example make_text_fixtures -- /tmp/fixtures
//! cargo run --example outlined_recognition_quality -- /tmp/fixtures/outlined.pdf <path> [<more fonts>...]
//! ```

use pdf_core::document::glyphs::Catalogue;
use pdf_core::document::Document;

/// The exact sixteen lines `outlined()` in `make_text_fixtures.rs` draws —
/// the same ground truth `tests/outlined_type.rs` checks against, since this
/// tool exists to measure the identical thing that test does, just for an
/// arbitrary font instead of a fixed one.
const EXPECTED: &str = "The luminaire housing is formed from
extruded aluminium with a powder
coated finish. Ingress protection is
rated to IP65 throughout the range,
and the diffuser is opal polycarbonate
with a nominal transmission of eighty
two percent measured at the centre
of the emitting surface.
Control gear is supplied loose or
integral depending on the variant
ordered. DALI-2 dimming is available
across every output, and emergency
versions carry a three hour battery
tested to the relevant standard for
self contained luminaires used in
commercial installations.";

fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i];
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            cur.push((prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost));
        }
        prev = cur;
    }
    prev[b.len()]
}
fn similarity(text: &str, expected: &str) -> f32 {
    let max_len = text.chars().count().max(expected.chars().count()).max(1);
    1.0 - edit_distance(text, expected) as f32 / max_len as f32
}

fn main() {
    let pdf_path = std::env::args().nth(1).expect("outlined.pdf path");
    let font_paths: Vec<String> = std::env::args().skip(2).collect();
    assert!(!font_paths.is_empty(), "need at least one font path");

    let doc = pdf_core::document::pdfium_doc::PdfiumDocument::open_path(&pdf_path, None).expect("open");
    let page = doc.page(0).expect("page 0");

    let mut catalogue = Catalogue::default();
    for font_path in &font_paths {
        let data = std::fs::read(font_path).expect("read font");
        catalogue.extend_from_font_common(&data);
    }
    println!("catalogue size (merged from {} font(s)): {}", font_paths.len(), catalogue.len());

    let result = page.recognise_outlined(&catalogue).expect("recognise");
    let text = result.plain();
    let score = similarity(&text, EXPECTED);

    println!("=== RECOVERED (score {score:.3}) ===\n{text}\n");
    println!("fonts: {font_paths:?}");
}
