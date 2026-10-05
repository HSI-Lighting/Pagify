//! The session's door to `Document::run_styles`.
//!
//! The measuring itself is tested against PDFium, on a synthetic page and on the
//! real datasheet, in `pdf_core`'s own `run_styles` test. This only checks that
//! a `Session` reaches it: the same page, a style for every run that
//! `run_font_names` names, and nothing for any other.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test -p pagify_shell --release --test run_styles -- --nocapture
//! ```

use pagify_shell::Session;

#[test]
fn a_session_hands_back_a_style_for_every_named_run() {
    if std::env::var("PAGIFY_PDFIUM_LIB").is_err() {
        eprintln!("skipped: set PAGIFY_PDFIUM_LIB to a desktop PDFium to run this");
        return;
    }
    let fixture = format!(
        "{}/../../../rust/pdf_core/fixtures/two-column.pdf",
        env!("CARGO_MANIFEST_DIR")
    );
    let session = Session::open(fixture).expect("open");

    let mut named: Vec<usize> = session.run_font_names(0).expect("names").into_keys().collect();
    let styles = session.run_styles(0).expect("styles");
    let mut styled: Vec<usize> = styles.keys().copied().collect();
    named.sort_unstable();
    styled.sort_unstable();
    println!("{} runs named, {} styled", named.len(), styled.len());

    assert!(!styled.is_empty(), "the page has text, so some run must have a style");
    assert_eq!(styled, named, "a style for every run that has a font name, and no other");
    // One Helvetica page, drawn upright: one font id, level text — and no stem,
    // because the page's Helvetica is not embedded: PDFium would measure a
    // stand-in from this computer (Arial's 95), which says nothing about the file.
    assert!(
        styles.values().all(|s| s.font == 0 && s.stem_milli_em.is_none() && s.axis == (1.0, 0.0)),
        "unexpected style on a one-font, upright page: {:?}",
        styles.values().find(|s| s.font != 0 || s.stem_milli_em.is_some() || s.axis != (1.0, 0.0))
    );
}
