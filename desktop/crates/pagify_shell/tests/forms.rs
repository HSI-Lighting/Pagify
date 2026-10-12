//! Reading the form fixture back: names, kinds and values widget by widget.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test -p pagify_shell --test forms
//! ```

use pagify_shell::Session;
use pdf_core::document::{FormFieldInfo, FormFieldKind};

fn fixture(name: &str) -> String {
    format!(
        "{}/../../../rust/pdf_core/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    )
}

#[test]
fn the_form_fixture_reads_back_widget_for_widget() {
    let session = Session::open(fixture("form-fields.pdf")).expect("open");

    let fields: Vec<FormFieldInfo> = session.form_fields().expect("fields");

    let summary: Vec<(String, FormFieldKind, Option<String>)> = fields
        .iter()
        .map(|f| (f.name.clone(), f.kind, f.value.clone()))
        .collect();
    assert_eq!(
        summary,
        vec![
            ("name".into(), FormFieldKind::Text, Some("Ada Lovelace".into())),
            ("email".into(), FormFieldKind::Text, None),
            ("agree".into(), FormFieldKind::Checkbox, Some("Yes".into())),
            ("speed".into(), FormFieldKind::Radio, Some("slow".into())),
            ("speed".into(), FormFieldKind::Radio, None),
            ("colour".into(), FormFieldKind::Combo, Some("Green".into())),
        ]
    );
}

#[test]
fn a_document_without_a_form_reads_no_fields() {
    let session = Session::open(fixture("two-column.pdf")).expect("open");

    let fields = session.form_fields().expect("fields");

    assert!(fields.is_empty(), "a plain document reported fields: {fields:?}");
}
