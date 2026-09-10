//! Why a run cannot be found in the content stream.
//!
//! The edit path matches a run against the operators parsed out of the page by
//! comparing origins, and refuses when the nearest is more than a few points
//! away. This reports the distance two ways — from the run's bounding box, and
//! from the matrix translation the run is actually drawn from — so which is the
//! right question can be seen rather than assumed.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo run --release --example locate_probe -- <file.pdf> [page]
//! ```

fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let page: usize = std::env::args().nth(2).and_then(|p| p.parse().ok()).unwrap_or(0);

    let doc = pdf_core::document::pdfium_doc::PdfiumDocument::open_path(&path, None).expect("open");
    let found = doc.run_distances(page).expect("distances");

    println!("{:>5}  {:>9}  {:>9}   {}", "obj", "from box", "from org", "text");
    let (mut by_box, mut by_origin, mut total) = (0usize, 0usize, 0usize);
    for (object, text, box_distance, origin_distance) in &found {
        if text.trim().is_empty() {
            continue;
        }
        total += 1;
        if *box_distance <= 4.0 {
            by_box += 1;
        }
        if *origin_distance <= 4.0 {
            by_origin += 1;
        }
        if *box_distance > 4.0 || *origin_distance > 4.0 {
            println!(
                "{object:>5}  {box_distance:>9.2}  {origin_distance:>9.2}   {:?}",
                text.chars().take(34).collect::<String>()
            );
        }
    }
    println!("\nfound from the box    : {by_box} of {total}");
    println!("found from the origin : {by_origin} of {total}");
}
