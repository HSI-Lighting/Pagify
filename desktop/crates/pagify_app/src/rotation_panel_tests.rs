//! **Rotation in the Properties panel** (`rotation_panel.rs`): an angle box, Left 90°, Right 90° and
//! Reset, for placed pictures, signature pictures, and pictures that are part of the page.
//!
//! **The angle is read back from the engine, not from the number the panel kept.** A placed picture's
//! turn is checked in `placed_image_marks`, a signature's in `image_signature_marks`, and a page
//! picture's in the matrix `images_on` reports — the same things the page draws from.

use super::*;
use crate::rotation_panel::{normalised, shortest_turn, shown_angle, Turnable};
use crate::ui_tests::{fixture, harness_from};
use egui_kittest::kittest::Queryable as _;
use pdf_core::command::Command;
use pdf_core::document::{Annotation, Rect};

fn open_app(name: &str) -> PagifyApp {
    let app = PagifyApp::new(Some(&fixture(name)));
    assert!(app.tab().doc.is_some(), "{name} did not open");
    app
}

/// A placed picture on page 1, selected the way a click selects it.
fn with_a_placed_picture() -> PagifyApp {
    let mut app = open_app("two-column.pdf");
    let rect = Rect { left: 100.0, top: 200.0, right: 160.0, bottom: 240.0 };
    app.tab()
        .doc
        .as_ref()
        .expect("open")
        .session
        .execute(Command::AddAnnotation {
            page_index: 0,
            annotation: Annotation::Image { rect, rgba: [200u8, 30, 30, 255].repeat(4), width: 2, height: 2 },
        })
        .expect("place the picture");
    let mark = app
        .tab()
        .doc
        .as_ref()
        .expect("open")
        .session
        .placed_image_marks(0)
        .expect("marks")
        .remove(0);
    app.tab_mut().selection.placed_image_selected =
        Some(PlacedImageSelected { page: 0, index: mark.index, rect: mark.rect, rotation: mark.rotation });
    app
}

fn placed_rotation(app: &PagifyApp) -> f32 {
    app.tab()
        .doc
        .as_ref()
        .expect("open")
        .session
        .placed_image_marks(0)
        .expect("marks")[0]
        .rotation
}

fn signature_rotation(app: &PagifyApp) -> f32 {
    app.tab()
        .doc
        .as_ref()
        .expect("open")
        .session
        .image_signature_marks(0)
        .expect("marks")[0]
        .rotation
}

/// A picture marked as a signature, selected the way a click selects it.
fn with_a_signature() -> PagifyApp {
    let mut app = open_app("two-column.pdf");
    let rect = Rect { left: 100.0, top: 300.0, right: 160.0, bottom: 340.0 };
    let session = &app.tab().doc.as_ref().expect("open").session;
    session
        .place_image_signature(0, rect, [20u8, 40, 60, 255].repeat(4), 2, 2, "Test")
        .expect("place the signature");
    let mark = session.image_signature_marks(0).expect("marks").remove(0);
    app.tab_mut().selection.signature_selected =
        Some(SignatureSelected { page: 0, index: mark.index, rect: mark.rect, rotation: mark.rotation });
    app
}

#[test]
fn the_pure_maths_of_the_angle() {
    assert_eq!(normalised(-90.0), 270.0);
    assert_eq!(normalised(360.0), 0.0);
    assert_eq!(normalised(725.0), 5.0);
    assert!(normalised(-0.0).is_sign_positive(), "-0.0 should come back as 0.0: {}", normalised(-0.0));

    assert_eq!(shortest_turn(350.0, 10.0), 20.0);
    assert_eq!(shortest_turn(10.0, 350.0), -20.0);
    assert_eq!(shortest_turn(0.0, 180.0), 180.0);
    assert_eq!(shortest_turn(90.0, 90.0), 0.0);

    assert_eq!(shown_angle([100.0, 0.0, 0.0, 50.0, 10.0, 20.0], 0), 0.0);
    assert_eq!(shown_angle([0.0, -100.0, 50.0, 0.0, 0.0, 0.0], 0), 90.0);
    assert_eq!(shown_angle([0.0, 100.0, -50.0, 0.0, 0.0, 0.0], 0), 270.0);
    assert_eq!(shown_angle([100.0, 0.0, 0.0, 50.0, 0.0, 0.0], 1), 90.0);
}

/// **The angle box turns a placed picture, and one undo turns it back.**
#[test]
fn the_angle_box_turns_a_placed_picture_and_one_undo_turns_it_back() {
    let mut app = with_a_placed_picture();
    assert_eq!(placed_rotation(&app), 0.0);

    app.turn_selected_to(45.0);
    assert!((placed_rotation(&app) - 45.0).abs() < 0.01, "the picture is at {}", placed_rotation(&app));
    match app.rotation_subject() {
        Some(Turnable::Placed(sel)) => assert!((sel.rotation - 45.0).abs() < 0.01, "{sel:?}"),
        other => panic!("the panel does not report the placed picture: {other:?}"),
    }

    app.submit("undo");
    assert!((placed_rotation(&app)).abs() < 0.01, "one undo left it at {}", placed_rotation(&app));

    // The same for a picture marked as a signature.
    let mut app = with_a_signature();
    assert_eq!(signature_rotation(&app), 0.0);
    app.turn_selected_to(45.0);
    assert!((signature_rotation(&app) - 45.0).abs() < 0.01, "the signature is at {}", signature_rotation(&app));
    match app.rotation_subject() {
        Some(Turnable::Signature(sel)) => assert!((sel.rotation - 45.0).abs() < 0.01, "{sel:?}"),
        other => panic!("the panel does not report the signature: {other:?}"),
    }
    app.submit("undo");
    assert!((signature_rotation(&app)).abs() < 0.01, "one undo left the signature at {}", signature_rotation(&app));
}

/// **Left 90°, Right 90° and Reset.**
#[test]
fn the_ninety_buttons_and_reset() {
    let mut app = with_a_placed_picture();
    app.turn_selected_by(90.0);
    app.turn_selected_by(90.0);
    assert!((placed_rotation(&app) - 180.0).abs() < 0.01, "{}", placed_rotation(&app));

    app.turn_selected_to(0.0);
    assert!(placed_rotation(&app).abs() < 0.01, "Reset did not return to 0: {}", placed_rotation(&app));

    app.turn_selected_by(-90.0);
    assert!((placed_rotation(&app) - 270.0).abs() < 0.01, "{}", placed_rotation(&app));

    app.turn_selected_to(0.0);
    assert!(placed_rotation(&app).abs() < 0.01, "{}", placed_rotation(&app));
}

/// **A page picture shows the angle its matrix gives, and turns to a typed one** — by the shortest
/// difference, about its centre, one `RotateObject` and so one undo.
#[test]
fn a_page_picture_shows_its_angle_and_turns_to_a_typed_one() {
    let mut app = open_app("pictures.pdf");
    app.submit("editobject");
    let image = app
        .tab_mut()
        .doc
        .as_ref()
        .expect("open")
        .session
        .images_on(0)
        .expect("images")
        .remove(0);
    let quarter_turns = app.tab().doc.as_ref().expect("open").session.page_rotation(0).expect("rotation");
    let shown = shown_angle(image.matrix, quarter_turns);
    app.tab_mut().selection.selected =
        Some(Selected { page: 0, object: image.object, rect: image.rect, what: "the picture" });

    match app.rotation_subject() {
        Some(Turnable::PagePicture { angle, .. }) => {
            assert!((angle - shown).abs() < 0.01, "the panel says {angle}, the matrix {shown}")
        }
        other => panic!("the page picture was not the rotation subject: {other:?}"),
    }

    let wanted = normalised(shown + 30.0);
    app.turn_selected_to(wanted);
    let after = app
        .tab()
        .doc
        .as_ref()
        .expect("open")
        .session
        .images_on(0)
        .expect("images")
        .into_iter()
        .find(|i| i.object == image.object)
        .expect("the picture is still on the page");
    let now = shown_angle(after.matrix, quarter_turns);
    assert!(
        (shortest_turn(now, wanted)).abs() < 0.5,
        "asked for {wanted}, the matrix now says {now} (shortest difference {})",
        shortest_turn(now, wanted)
    );

    app.submit("undo");
    let back = app
        .tab()
        .doc
        .as_ref()
        .expect("open")
        .session
        .images_on(0)
        .expect("images")
        .into_iter()
        .find(|i| i.object == image.object)
        .expect("the picture came back");
    let restored = shown_angle(back.matrix, quarter_turns);
    assert!(
        (shortest_turn(restored, shown)).abs() < 0.5,
        "one undo should put it back at {shown}, it is at {restored}"
    );
}

/// **Words and shapes get no rotation section**, and neither do groups or paragraphs.
#[test]
fn words_and_shapes_get_no_rotation_section() {
    let mut app = open_app("pictures.pdf");
    app.submit("editobject");
    let pictures = app.tab().doc.as_ref().expect("open").session.images_on(0).expect("images");
    app.tab_mut().selection.group = pictures
        .iter()
        .map(|p| Selected { page: 0, object: p.object, rect: p.rect, what: "the picture" })
        .collect();
    assert!(app.rotation_subject().is_none(), "a group was offered rotation: {:?}", app.rotation_subject());

    // A drawn shape, on a page built for it: 50 pt square whose top-left shows at (100, 642).
    let mut app = super::wrap_hyphen_tests::open_page("rotation-shape", &[], "1 0 0 rg 100 100 50 50 re f\n");
    app.submit("editobject");
    assert!(
        app.select_thing_at_drilling(0, AppPoint { x: 125.0, y: 667.0 }, false),
        "the square was not picked"
    );
    assert_eq!(app.tab().selection.selected.as_ref().map(|s| s.what), Some("the shape"));
    assert!(app.rotation_subject().is_none(), "a shape was offered rotation: {:?}", app.rotation_subject());

    let mut app = open_app("text-lines.pdf");
    app.submit("editobject");
    let run = app
        .tab()
        .doc
        .as_ref()
        .expect("open")
        .session
        .text_runs(0)
        .expect("runs")
        .remove(0);
    app.tab_mut().selection.selected = Some(Selected { page: 0, object: run.object, rect: run.rect, what: "the word" });
    assert!(app.rotation_subject().is_none(), "words were offered rotation: {:?}", app.rotation_subject());
}

/// **The panel itself**: the label and all three buttons, Right 90° turns once, and typing 45 into
/// the angle box is one further undo step and is not also sent to the page.
#[test]
fn the_panel_shows_rotation_and_its_controls_turn_a_placed_picture() {
    let app = with_a_placed_picture();
    let mut h = harness_from(app);
    h.run_steps(2);

    assert!(h.query_by_label_contains("Rotation").is_some(), "the Rotation section is not in the panel");
    for label in ["Left 90°", "Right 90°", "Reset"] {
        assert!(
            h.query_all_by_role_and_label(egui::accesskit::Role::Button, label).next().is_some(),
            "the {label} button is not in the panel"
        );
    }

    h.get_by_role_and_label(egui::accesskit::Role::Button, "Right 90°").click();
    h.run_steps(2);
    assert!((placed_rotation(h.state()) - 90.0).abs() < 0.01, "Right 90° left it at {}", placed_rotation(h.state()));

    // Type 45 into the angle box. Clicking a DragValue starts editing; clear what is there first.
    h.get_all_by_role(egui::accesskit::Role::SpinButton).next().expect("the angle box").click();
    h.run_steps(1);
    for _ in 0..3 {
        h.key_press(egui::Key::Backspace);
        h.run_steps(1);
    }
    for ch in "45".chars() {
        h.event(egui::Event::Text(ch.to_string()));
        h.run_steps(1);
    }
    h.key_press(egui::Key::Enter);
    h.run_steps(2);

    assert!((placed_rotation(h.state()) - 45.0).abs() < 0.01, "typing 45 left it at {}", placed_rotation(h.state()));
    assert!(
        h.state().tab().edit.editing_run.is_none(),
        "the Enter opened the page's own text editor — the field did not keep it"
    );

    // One change, one undo: back to 90 rather than to 0.
    h.state_mut().submit("undo");
    h.run_steps(1);
    assert!(
        (placed_rotation(h.state()) - 90.0).abs() < 0.01,
        "typing 45 should be one undo step back to 90; it is at {}",
        placed_rotation(h.state())
    );
}
