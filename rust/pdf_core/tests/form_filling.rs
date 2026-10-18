//! Filling an AcroForm has to **show** — and to reach the file.
//!
//! Measured while building this: a value written behind PDFium's back (a
//! plain `/V` string write) reads back from every later open and moves no
//! pixel of any render, because the widget keeps drawing the appearance
//! stream its author made. The setters drive the *form environment* the way
//! a reader's own typing and clicking do, and these tests hold them to the
//! rendered page, the read-back, and the saved file.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test --test form_filling
//! ```

mod harness;
use harness::{serial, skip_without_pdfium};

use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{
    Document, DocumentMut, FormFieldKind, Rect, RenderRequest, Rotation,
};

fn open() -> PdfiumDocument {
    let doc = PdfiumDocument::open_bytes(
        harness::fixture_bytes("form-fields.pdf"),
        None,
    )
    .expect("open");
    // Reading the fields is also what creates the form environment the
    // render path draws widgets with.
    doc.form_fields().expect("fields");
    doc
}

fn render(doc: &PdfiumDocument) -> Vec<u8> {
    let request = RenderRequest {
        scale: 2.0,
        rotation: Rotation::None,
        render_annotations: true,
        render_form_data: true,
    };
    doc.render_page_to_bitmap(0, &request).expect("render").data
}

fn changed(a: &[u8], b: &[u8]) -> usize {
    let mut changed = 0;
    for i in (0..a.len().min(b.len())).step_by(4) {
        if a[i..i + 4] != b[i..i + 4] {
            changed += 1;
        }
    }
    changed
}

/// The widgets are on the page: rendering without form data loses them.
#[test]
fn a_form_page_renders_its_widgets() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    let doc = open();

    let with = render(&doc);
    let without_request = RenderRequest {
        scale: 2.0,
        rotation: Rotation::None,
        render_annotations: false,
        render_form_data: false,
    };
    let without = doc.render_page_to_bitmap(0, &without_request).expect("render").data;

    assert!(
        changed(&with, &without) > 1000,
        "the widgets did not render: only {} pixels differ",
        changed(&with, &without)
    );
}

/// Every widget carries the page it is drawn on and where it sits — what a
/// filling UI hit-tests a click against.
#[test]
fn the_fields_carry_their_page_their_place_and_their_options() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    let doc = open();

    let fields = doc.form_fields().expect("fields");
    assert_eq!(fields.len(), 6);
    assert!(fields.iter().all(|f| f.page == 0), "every widget is on page 1");

    let name = fields.iter().find(|f| f.name == "name").expect("the name field");
    assert_eq!(name.kind, FormFieldKind::Text);
    // /Rect [140 664 380 688] on a 792-point page, top-left points.
    assert!((name.rect.left - 140.0).abs() < 0.5 && (name.rect.right - 380.0).abs() < 0.5);
    assert!((name.rect.top - 104.0).abs() < 0.5 && (name.rect.bottom - 128.0).abs() < 0.5);

    let colour = fields.iter().find(|f| f.name == "colour").expect("the combo");
    assert_eq!(colour.kind, FormFieldKind::Combo);
    assert_eq!(colour.options, vec!["Red", "Green", "Blue"]);
    assert!(
        fields.iter().all(|f| f.options.is_empty() || f.name == "colour"),
        "only a choice field carries options"
    );
}

/// A text field: the words go in, the render moves, the file keeps them.
#[test]
fn filling_a_text_field_shows_at_once_and_survives_the_save() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    let mut doc = open();
    let before = render(&doc);

    assert!(doc.set_form_text("name", "Grace Hopper").expect("set text"));
    let after = render(&doc);
    assert!(
        changed(&before, &after) > 200,
        "the typed value did not reach the render: {} pixels",
        changed(&before, &after)
    );

    let mut bytes = Vec::new();
    doc.save_full_copy(&mut bytes).expect("save");
    let reopened = PdfiumDocument::open_bytes(bytes, None).expect("reopen");
    let fields = reopened.form_fields().expect("fields");
    assert_eq!(
        fields.iter().find(|f| f.name == "name").and_then(|f| f.value.as_deref()),
        Some("Grace Hopper")
    );
    // And the reopened page *looks* the same as the filled one did: the
    // appearance came through the save, not just the value.
    assert_eq!(
        changed(&after, &render(&reopened)),
        0,
        "the saved file does not render like the filled page did"
    );
}

/// A checkbox: the click route is the one that shows.
#[test]
fn filling_a_checkbox_shows_at_once_and_survives_the_save() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    let mut doc = open();
    let before = render(&doc);

    assert!(doc.set_form_checkbox("agree", false).expect("uncheck"));
    let after = render(&doc);
    assert!(changed(&before, &after) > 50, "the tick did not come off the render");

    let fields = doc.form_fields().expect("reread");
    assert_eq!(fields.iter().find(|f| f.name == "agree").and_then(|f| f.value.clone()), None);

    let mut bytes = Vec::new();
    doc.save_full_copy(&mut bytes).expect("save");
    let reopened = PdfiumDocument::open_bytes(bytes, None).expect("reopen");
    let fields = reopened.form_fields().expect("fields");
    assert!(fields.iter().any(|f| f.name == "agree" && f.value.is_none()));
}

/// A radio: set by clicking its widget — the route that used to be refused
/// as impossible with these bindings, and the only one that shows.
#[test]
fn filling_a_radio_clicks_it_into_place_and_shows() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    let mut doc = open();
    let before = render(&doc);

    assert!(
        doc.set_form_radio("speed", "fast").expect("radio"),
        "the radio could not be set by clicking its widgets"
    );
    let after = render(&doc);
    assert!(changed(&before, &after) > 50, "the radio did not move the render");

    let fields = doc.form_fields().expect("reread");
    let speeds: Vec<_> = fields
        .iter()
        .filter(|f| f.name == "speed")
        .map(|f| f.value.clone())
        .collect();
    assert_eq!(speeds, vec![None, Some("fast".into())], "the group did not switch");
}

/// **A state no widget carries must not re-answer the group.** The search
/// has to click widgets to learn what they carry, so a refused value has to
/// put the prior selection back — otherwise a form import with a stale value
/// reports it "not applied" while silently changing the answer to whatever
/// widget was clicked last.
#[test]
fn a_refused_radio_state_leaves_the_group_as_it_was() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    let mut doc = open();
    let group = |doc: &PdfiumDocument| -> Vec<Option<String>> {
        doc.form_fields()
            .expect("reread")
            .iter()
            .filter(|f| f.name == "speed")
            .map(|f| f.value.clone())
            .collect()
    };

    let before = group(&doc);
    assert_eq!(before, vec![Some("slow".into()), None], "the fixture's starting answer");
    assert!(
        !doc.set_form_radio("speed", "no-such-state").expect("radio"),
        "a state no widget carries must not report success"
    );
    assert_eq!(group(&doc), before, "a refused radio state changed the group");
}

/// Asking for the answer the group already has is a success and does not
/// dirty the document by clicking through the widgets.
#[test]
fn asking_for_the_radio_state_it_already_has_is_a_no_op_success() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    let mut doc = open();
    let before = render(&doc);
    assert!(
        doc.set_form_radio("speed", "slow").expect("radio"),
        "the group's own current state should be a success"
    );
    assert_eq!(render(&doc), before, "an already-answered radio was clicked through");
}

/// A choice: the option route is the one a non-editable combo takes, and it
/// shows.
#[test]
fn filling_a_choice_selects_the_option_and_shows() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    let mut doc = open();
    let before = render(&doc);

    assert!(
        doc.set_form_choice("colour", "Red").expect("choice"),
        "the combo would not take its own option"
    );
    let after = render(&doc);
    assert!(changed(&before, &after) > 200, "the chosen option did not reach the render");

    let fields = doc.form_fields().expect("reread");
    assert_eq!(
        fields.iter().find(|f| f.name == "colour").and_then(|f| f.value.as_deref()),
        Some("Red")
    );
}

/// **A value the combo does not carry is a refusal, not a silent no-change
/// success.** `FORM_ReplaceSelection` is void and PDFium ignores it for a
/// non-editable combo; the setter now reads the field back before saying it
/// worked, so a form import with a stale value is counted as not applied.
#[test]
fn a_choice_the_combo_does_not_carry_is_refused() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    let mut doc = open();
    let colour = |doc: &PdfiumDocument| -> Option<String> {
        doc.form_fields()
            .expect("reread")
            .into_iter()
            .find(|f| f.name == "colour")
            .and_then(|f| f.value)
    };
    let before = colour(&doc);
    assert!(
        !doc.set_form_choice("colour", "Not An Option").expect("choice"),
        "a value the combo does not carry must not report success"
    );
    assert_eq!(colour(&doc), before, "a refused choice changed the field");
}

/// A click on a widget, through the engine door the app uses — page points,
/// top-left — ticks the box it lands on.
#[test]
fn a_click_on_a_widget_fills_it() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    let mut doc = open();

    // The checkbox's own rectangle, centre — /Rect [140 584 154 598] on a
    // 792-point page: (147, 201) in top-left points.
    assert!(doc.click_form_field(0, (147.0, 201.0)).expect("click"));
    let fields = doc.form_fields().expect("reread");
    assert_eq!(
        fields.iter().find(|f| f.name == "agree").and_then(|f| f.value.clone()),
        None,
        "the click did not toggle the box"
    );

    // Bare paper is not a widget.
    assert!(!doc.click_form_field(0, (500.0, 500.0)).expect("click"));
}

/// **A cropping must not move the widgets out from under the pointer.**
/// `croppages` is this app's own command, and what `form_fields` reports is in
/// the crop's top-left space; the click route has to convert through the same
/// crop box or a click on a widget's own reported centre misses it (it used to
/// use the page height alone, so the crop's left/top were lost).
#[test]
fn a_click_on_a_widget_still_lands_after_the_page_is_cropped() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    let mut doc = open();

    let agree = doc
        .form_fields()
        .expect("fields")
        .into_iter()
        .find(|f| f.name == "agree")
        .expect("the agree checkbox");
    let centre = |rect: &Rect| ((rect.left + rect.right) / 2.0, (rect.top + rect.bottom) / 2.0);
    let (before_x, before_y) = centre(&agree.rect);
    assert!(doc.click_form_field(0, (before_x, before_y)).expect("before crop"));

    // Inset the page on all four sides, the way the Organize tab's crop does.
    doc.set_page_crop(0, Rect { left: 50.0, top: 100.0, right: 500.0, bottom: 600.0 })
        .expect("crop");

    let agree = doc
        .form_fields()
        .expect("fields after the crop")
        .into_iter()
        .find(|f| f.name == "agree")
        .expect("the agree checkbox after the crop");
    let (x, y) = centre(&agree.rect);
    assert!(
        doc.click_form_field(0, (x, y)).expect("after crop"),
        "after a crop, a click on the widget's own reported centre missed it"
    );
}
