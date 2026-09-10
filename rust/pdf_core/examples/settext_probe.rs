//! Does `FPDFText_SetText` actually write what it was given?
//!
//! It reports success on a font that cannot draw the new characters. A
//! subsetted font carries only the glyphs the document already used, and asking
//! it for others gets you nothing, or something else.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo run --example settext_probe -- <pdf> <page>
//! ```

use pdfium_render::prelude::*;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let index: i32 = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let lib = std::env::var("PAGIFY_PDFIUM_LIB").expect("PAGIFY_PDFIUM_LIB");
    let pdfium = Pdfium::new(Pdfium::bind_to_library(&lib).expect("bind"));
    let doc = pdfium.load_pdf_from_file(&a[0], None).expect("open");

    let (mut took, mut refused, mut lied) = (0, 0, 0);
    for run in 0..40usize {
        let mut page = doc.pages().get(index).expect("page");
        let mut seen = 0usize;
        let mut before = None;
        for mut object in page.objects_mut().iter() {
            let Some(t) = object.as_text_object_mut() else { continue };
            if t.text().trim().is_empty() {
                continue;
            }
            if seen == run {
                let was = t.text();
                match t.set_text("PAGIFY") {
                    Ok(()) => before = Some((was, t.text())),
                    Err(_) => refused += 1,
                }
                break;
            }
            seen += 1;
        }
        let Some((was, now)) = before else { continue };
        if now.trim() == "PAGIFY" {
            took += 1;
        } else {
            lied += 1;
            println!(
                "  run {run:<3} asked for \"PAGIFY\", got {:?}   (was {:?})",
                now.chars().take(20).collect::<String>(),
                was.chars().take(20).collect::<String>()
            );
        }
    }
    println!("\n{took} took, {refused} refused, {lied} reported success and did not write it");
}
