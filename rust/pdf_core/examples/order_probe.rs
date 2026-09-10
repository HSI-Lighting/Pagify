//! Does the order of a page's runs match the order of its text operators?
//!
//! If it does, a run the distance test cannot place can still be placed — by
//! counting. This checks the claim against the runs the distance test *can*
//! place, which is the only evidence that would justify trusting it.
fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let page: usize = std::env::args().nth(2).and_then(|p| p.parse().ok()).unwrap_or(0);

    let doc = pdf_core::document::pdfium_doc::PdfiumDocument::open_path(&path, None).expect("open");
    let found = doc.run_distances_indexed(page).expect("distances");
    let placed = found.first().map(|f| f.5).unwrap_or(0);

    println!("{} run(s), {placed} placed operator(s)\n", found.len());
    println!("{:>4} {:>5} {:>9} {:>8}   {}", "run", "obj", "distance", "nearest", "text");

    let (mut agree, mut disagree, mut unplaced) = (0usize, 0usize, 0usize);
    for (run_index, (object, text, by_box, _, nearest, _)) in found.iter().enumerate() {
        let placeable = *by_box <= 4.0;
        if !placeable {
            unplaced += 1;
        } else if *nearest == run_index {
            agree += 1;
        } else {
            disagree += 1;
        }
        println!(
            "{run_index:>4} {object:>5} {by_box:>9.2} {nearest:>8}   {:?}",
            text.chars().take(28).collect::<String>()
        );
    }
    println!("\nplaced and in the same position : {agree}");
    println!("placed but somewhere else       : {disagree}");
    println!("not placeable by distance       : {unplaced}");
}
