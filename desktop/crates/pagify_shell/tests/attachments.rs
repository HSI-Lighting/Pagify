//! Files riding with a document: `attach` embeds one in the document,
//! `attachcomment` pins one to a page — and `flatten` bakes annotations and
//! form fields into content, which `forms.rs` also leans on.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test -p pagify_shell --test attachments
//! ```

use pagify_shell::Session;

fn fixture(name: &str) -> String {
    format!(
        "{}/../../../rust/pdf_core/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    )
}

#[test]
fn an_attached_file_lists_and_reaches_the_saved_file() {
    let session = Session::open(fixture("two-column.pdf")).expect("open");
    assert!(session.attachments().expect("list").is_empty(), "nothing attached yet");

    let bytes = b"Pagify attachment fixture bytes.";
    assert!(session.add_attachment("notes.txt", bytes).expect("attach"));
    assert!(
        !session.add_attachment("notes.txt", bytes).expect("attach twice"),
        "a used name must be refused"
    );
    assert_eq!(session.attachments().expect("list"), ["notes.txt"]);

    let path =
        std::env::temp_dir().join(format!("pagify-attach-test-{}.pdf", std::process::id()));
    session.save_to(&path, false).expect("save");
    let reopened = Session::open(&path).expect("reopen");
    assert_eq!(
        reopened.attachments().expect("list again"),
        ["notes.txt"],
        "the attachment is not in the file"
    );
    std::fs::remove_file(&path).ok();
}

#[test]
fn a_comment_attachment_pins_to_its_page() {
    let session = Session::open(fixture("two-column.pdf")).expect("open");

    assert!(session.add_annot_attachment(0, "receipt.txt", b"receipt bytes").expect("pin"));

    assert_eq!(session.page_attachments(0).expect("read page 0"), ["receipt.txt"]);

    let path =
        std::env::temp_dir().join(format!("pagify-pin-test-{}.pdf", std::process::id()));
    session.save_to(&path, false).expect("save");
    let reopened = Session::open(&path).expect("reopen");
    assert_eq!(reopened.page_attachments(0).expect("pins again"), ["receipt.txt"]);
    std::fs::remove_file(&path).ok();
}

#[test]
fn flatten_bakes_annotations_and_form_fields_into_content() {
    let session = Session::open(fixture("form-fields.pdf")).expect("open");
    assert_eq!(session.form_fields().expect("fields").len(), 6, "the fixture's fields");

    session.flatten_page(0).expect("flatten");

    assert!(
        session.form_fields().expect("fields again").is_empty(),
        "the fields were not baked into content"
    );
    // And the page's own words are still there: flattening bakes, it does
    // not blank.
    let text = session.page_text_snapshot(0).expect("text").runs.iter().map(|r| r.text.clone()).collect::<String>();
    assert!(text.contains("Pagify form fixture"), "content was lost: {text:?}");
}
