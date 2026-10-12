//! The Conversion tab's two exports, at the shell level: `toimage`'s PNG
//! bytes and `tohtml`'s document.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test -p pagify_shell --test document_export
//! ```

use pagify_shell::{export, Session};

fn fixture(name: &str) -> String {
    format!(
        "{}/../../../rust/pdf_core/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    )
}

#[test]
fn a_page_encodes_as_a_png() {
    let session = Session::open(fixture("quadrants.pdf")).expect("open");

    let bytes = session.export_page_png(0, 2.0).expect("png");

    assert!(
        bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]),
        "not a PNG signature: {:?}",
        &bytes[..8.min(bytes.len())]
    );
    assert!(
        bytes.len() > 1000,
        "a 400pt page at 2x is not {} bytes — the render or encode came back empty",
        bytes.len()
    );
}

#[test]
fn the_html_carries_the_page_text_and_escapes_its_title() {
    let session = Session::open(fixture("two-column.pdf")).expect("open");

    let html = export::document_html(&session, "a<b&c").expect("html");

    assert!(html.contains("<title>a&lt;b&amp;c</title>"), "title not escaped: {html}");
    assert!(html.contains("<h2>Page 1</h2>"), "no page heading: {html}");
    assert!(
        html.contains("<p>") && !html.contains("no text on this page"),
        "page 1 has text and the export did not carry it: {html}"
    );
    assert!(html.trim_end().ends_with("</html>"), "the document is not closed: {html}");
}

#[test]
fn a_page_with_no_text_says_so_rather_than_going_missing() {
    // quadrants is four solid colours — a page with no text at all.
    let session = Session::open(fixture("quadrants.pdf")).expect("open");

    let html = export::document_html(&session, "quadrants").expect("html");

    assert!(
        html.contains("no text on this page"),
        "an empty page vanished from the export: {html}"
    );
}

#[test]
fn an_embedded_picture_exports_as_a_png_of_its_own_pixels() {
    let session = Session::open(fixture("pictures.pdf")).expect("open");
    let images = session.images_on(0).expect("images");
    let first = images.first().expect("pictures.pdf carries pictures");

    let png = session.export_image_png(0, first.object).expect("png");

    assert!(png.starts_with(&[0x89, b'P', b'N', b'G']), "not a PNG");
    // PNG's IHDR: width and height are the first two 4-byte fields after the
    // 8-byte signature and the 8-byte chunk header.
    let width = u32::from_be_bytes([png[16], png[17], png[18], png[19]]);
    let height = u32::from_be_bytes([png[20], png[21], png[22], png[23]]);
    assert_eq!(
        (width, height),
        (first.pixel_width, first.pixel_height),
        "the PNG's dimensions must be the image's own pixels, not its size on the page"
    );
}

#[test]
fn the_character_wrap_is_a_count_and_keeps_the_characters() {
    use pagify_shell::export::wrap_by_chars;

    let wrapped = wrap_by_chars("abcdefghij", 4);
    assert_eq!(wrapped, ["abcd", "efgh", "ij"]);
    assert_eq!(wrapped.concat(), "abcdefghij", "the wrap lost characters");

    // A line short of the width is one piece, not zero.
    assert_eq!(wrap_by_chars("abc", 4), ["abc"]);
}
