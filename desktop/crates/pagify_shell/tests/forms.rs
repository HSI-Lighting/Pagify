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

#[test]
fn fields_set_read_back_and_reach_the_file() {
    let session = Session::open(fixture("form-fields.pdf")).expect("open");

    assert!(session.set_form_text("name", "Grace Hopper").expect("set text"));
    assert!(session.set_form_checkbox("agree", false).expect("uncheck"));
    assert!(!session.set_form_text("no-such-field", "x").expect("missing field"));
    assert!(
        !session.set_form_radio("speed", "fast").expect("radio"),
        "this build must answer `false` for radios rather than write a wrong state"
    );

    let fields = session.form_fields().expect("reread");
    assert_eq!(fields[0].value.as_deref(), Some("Grace Hopper"));
    assert_eq!(fields[2].value, None, "the checkbox did not uncheck");
    assert_eq!(fields[3].value.as_deref(), Some("slow"), "a refused radio change must not touch the file");
    assert_eq!(fields[4].value, None);

    // Written out and read back: the values are in the file, not just in
    // this process's copy of it.
    let path = std::env::temp_dir().join(format!("pagify-form-test-{}.pdf", std::process::id()));
    session.save_to(&path, false).expect("save");
    let reopened = Session::open(&path).expect("reopen");
    let again = reopened.form_fields().expect("reread");
    assert_eq!(again[0].value.as_deref(), Some("Grace Hopper"));
    assert_eq!(again[2].value, None);
    assert_eq!(again[3].value.as_deref(), Some("slow"), "the radio must survive the save unchanged");
    std::fs::remove_file(&path).ok();
}

#[test]
fn the_json_carries_every_field_and_applies_back() {
    let session = Session::open(fixture("form-fields.pdf")).expect("open");

    let json = session.form_data_json().expect("json");
    assert!(json.contains("Ada Lovelace") && json.contains("Green"), "{json}");
    assert!(json.contains("\"kind\":\"radio\""), "kinds are not in the JSON: {json}");

    let (applied, not_applied) = session
        .apply_form_data_json(
            r#"[
                {"name":"name","kind":"text","value":"Alan Turing"},
                {"name":"agree","kind":"checkbox","value":null},
                {"name":"colour","kind":"combo","value":"Blue"}
            ]"#,
        )
        .expect("apply");

    assert_eq!(
        (applied, not_applied),
        (2, 1),
        "combos are not settable in this build and must count as not applied"
    );
    let fields = session.form_fields().expect("reread");
    assert_eq!(fields[0].value.as_deref(), Some("Alan Turing"));
    assert_eq!(fields[2].value, None, "the checkbox did not clear from JSON");
    assert_eq!(fields[5].value.as_deref(), Some("Green"), "the combo must be untouched");
}

#[test]
fn the_csv_carries_a_header_and_quotes_what_it_must() {
    let session = Session::open(fixture("form-fields.pdf")).expect("open");

    let csv = session.form_data_csv().expect("csv");
    assert!(csv.starts_with("name,kind,value\n"), "{csv}");
    assert!(csv.contains("name,text,Ada Lovelace"), "{csv}");

    // A value with a comma and a quote must come back quoted, doubled.
    assert!(session.set_form_text("name", "Hopper, Grace \"Amazing\"").expect("set"));
    let csv = session.form_data_csv().expect("csv again");
    assert!(
        csv.contains("\"Hopper, Grace \"\"Amazing\"\"\""),
        "the awkward value was not quoted per RFC 4180: {csv}"
    );
}

#[test]
fn tooltips_set_read_back_and_reach_the_file() {
    let session = Session::open(fixture("form-fields.pdf")).expect("open");

    assert!(session.set_form_tooltip("email", "Your address").expect("set tooltip"));
    assert!(!session.set_form_tooltip("no-such-field", "x").expect("missing field"));

    let tooltips = session.form_field_tooltips().expect("tooltips");
    let email = tooltips.iter().find(|(name, _)| name == "email").expect("email row");
    assert_eq!(email.1.as_deref(), Some("Your address"));

    let path = std::env::temp_dir().join(format!("pagify-tooltip-test-{}.pdf", std::process::id()));
    session.save_to(&path, false).expect("save");
    let reopened = Session::open(&path).expect("reopen");
    let tooltips = reopened.form_field_tooltips().expect("tooltips again");
    let email = tooltips.iter().find(|(name, _)| name == "email").expect("email row");
    assert_eq!(email.1.as_deref(), Some("Your address"), "the tooltip is not in the file");
    std::fs::remove_file(&path).ok();
}

#[test]
fn clearing_empties_what_this_build_can_set_and_leaves_the_rest() {
    let session = Session::open(fixture("form-fields.pdf")).expect("open");

    let cleared = session.clear_form_fields().expect("clear");

    // One text field with a value, one empty text field, one checked box:
    // the two text fields and the box are clearable; the radios and the
    // combo are not.
    assert_eq!(cleared, 3, "two texts and a checkbox should clear");
    let fields = session.form_fields().expect("reread");
    assert_eq!(fields[0].value, None, "the text did not clear");
    assert_eq!(fields[2].value, None, "the checkbox did not clear");
    assert_eq!(fields[3].value.as_deref(), Some("slow"), "the radio must be untouched");
    assert_eq!(fields[5].value.as_deref(), Some("Green"), "the combo must be untouched");
}
