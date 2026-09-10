//! Where a page of drawn words is lost: the shape filter, or the match.
fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let page: usize = std::env::args().nth(2).and_then(|p| p.parse().ok()).unwrap_or(0);
    let fonts: Vec<Vec<u8>> =
        std::env::args().skip(3).filter_map(|p| std::fs::read(p).ok()).collect();

    let mut catalogue = pdf_core::document::glyphs::Catalogue::default();
    for font in &fonts {
        catalogue.extend_from_font_common(font);
    }
    let doc = pdf_core::document::pdfium_doc::PdfiumDocument::open_path(&path, None).expect("open");
    let (objects, accepted, matched, clusters) = doc.outlined_report(page, &catalogue).expect("report");

    println!(
        "page {}: {} path object(s), {accepted} look like type, {clusters} cluster(s), {matched} matched ({:.0}%)",
        page + 1,
        objects.len(),
        100.0 * matched as f32 / clusters.max(1) as f32
    );
    println!("\nthe biggest paths, and whether the shape filter took them:");
    let mut sorted = objects.clone();
    sorted.sort_by(|a, b| (b.0 * b.1).total_cmp(&(a.0 * a.1)));
    for (w, h, segs, ok) in sorted.iter().take(14) {
        println!("   {w:>7.1} x {h:>6.1}  {segs:>6} segment(s)  {}", if *ok { "type" } else { "REJECTED" });
    }
    let rejected: Vec<_> = objects.iter().filter(|o| !o.3).collect();
    println!("\n{} rejected; of those, {} are wide-and-many-segmented (a whole line?)",
        rejected.len(),
        rejected.iter().filter(|o| o.2 >= 100).count());
}
