//! Preflight's findings, against fixtures with known-shaped content: text
//! pages whose fonts may or may not be in the file, pictures with pixels of
//! their own, and a page of pure vector colour that can hold no finding at
//! all.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test -p pagify_shell --test preflight
//! ```

use pagify_shell::{preflight, Session};

fn fixture(name: &str) -> String {
    format!(
        "{}/../../../rust/pdf_core/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    )
}

#[test]
fn a_vector_page_has_nothing_to_report() {
    // Four solid colours: no fonts, no images, nothing to be wrong.
    let session = Session::open(fixture("quadrants.pdf")).expect("open");

    let findings = preflight::findings(&session).expect("check");

    assert!(findings.is_empty(), "quadrants reported: {:?}", findings.iter().map(|f| &f.line).collect::<Vec<_>>());
}

#[test]
fn the_font_finding_agrees_with_the_engine() {
    let session = Session::open(fixture("two-column.pdf")).expect("open");

    // What the engine itself says about each face on the page.
    let mut missing: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for (object, name) in session.run_font_names(0).expect("names") {
        if !session.run_font_is_embedded(0, object).expect("embedded") {
            missing.insert(name);
        }
    }

    let findings = preflight::findings(&session).expect("check");
    let font_lines = findings.iter().filter(|f| f.line.contains("not embedded")).count();

    assert_eq!(
        font_lines,
        missing.len(),
        "one line per missing face, no more: engine says {missing:?}, findings {:?}",
        findings.iter().map(|f| &f.line).collect::<Vec<_>>()
    );
}

#[test]
fn a_low_resolution_picture_over_its_box_is_reported() {
    let session = Session::open(fixture("pictures.pdf")).expect("open");

    // What the engine says, computed the same way the check computes it.
    let expected: Vec<String> = session
        .images_on(0)
        .expect("images")
        .into_iter()
        .filter_map(|image| {
            let width_pt = (image.rect.right - image.rect.left).abs();
            (width_pt > 0.0).then_some(image.pixel_width as f32 * 72.0 / width_pt)
        })
        .filter(|dpi| *dpi < 150.0)
        .map(|_| "soft".to_string())
        .collect();

    let findings = preflight::findings(&session).expect("check");
    let picture_lines = findings.iter().filter(|f| f.line.contains("dpi over the box")).count();

    assert_eq!(
        picture_lines,
        expected.len(),
        "the picture check disagrees with the engine's own numbers: {:?}",
        findings.iter().map(|f| &f.line).collect::<Vec<_>>()
    );
}
