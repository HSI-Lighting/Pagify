use super::*;

fn fixture(name: &str) -> String {
    format!(
        "{}/../../../rust/pdf_core/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn app() -> PagifyApp {
    let app = PagifyApp::new(Some(&fixture("single-page.pdf")));
    assert!(app.tab().doc.is_some());
    app
}

fn said(app: &PagifyApp) -> String {
    app.cmd.history().iter().map(|e| e.text.as_str()).collect::<Vec<_>>().join("\n")
}

fn marks(app: &PagifyApp) -> usize {
    app.tab().markup.existing(app.tab().page).map(|l| l.len()).unwrap_or(0)
}

/// The defect this fixes: the app answered "nothing to undo" while three
/// freshly drawn lines sat on the page.
#[test]
fn undo_takes_back_a_drawn_mark() {
    let mut app = app();
    app.submit("l 10,10 100,100");
    app.submit("l 10,50 100,150");
    assert_eq!(marks(&app), 2);

    app.submit("undo");
    assert_eq!(marks(&app), 1, "undo did nothing:\n{}", said(&app));
    app.submit("undo");
    assert_eq!(marks(&app), 0);
}

#[test]
fn redo_puts_it_back() {
    let mut app = app();
    app.submit("l 10,10 100,100");
    app.submit("undo");
    assert_eq!(marks(&app), 0);

    app.submit("redo");
    assert_eq!(marks(&app), 1, "redo did nothing:\n{}", said(&app));
}

#[test]
fn a_fillet_undoes_in_one_step() {
    let mut app = app();
    app.submit("l 30,250 170,250");
    app.submit("l 170,250 170,350");
    let before = marks(&app);

    app.submit("fillet 25");
    app.submit("pick 100,250");
    app.submit("pick 170,320");
    assert!(marks(&app) > before, "the fillet did not run:\n{}", said(&app));

    app.submit("undo");
    assert_eq!(marks(&app), before, "the fillet came back in pieces");
}

#[test]
fn a_move_undoes_to_where_it_was() {
    let mut app = app();
    app.submit("l 0,100 100,100");
    app.submit("all");
    app.submit("move");
    app.submit("pick 0,0");
    app.submit("pick 0,50");

    let moved = match &app.tab_mut().markup.existing(0).unwrap().objects()[0].geom {
        cad_kernel::Geom::Line(l) => l.a.y,
        _ => unreachable!(),
    };
    assert!((moved - 150.0).abs() < 1e-6, "the move did not happen: y={moved}");

    app.submit("undo");
    let back = match &app.tab_mut().markup.existing(0).unwrap().objects()[0].geom {
        cad_kernel::Geom::Line(l) => l.a.y,
        _ => unreachable!(),
    };
    assert!((back - 100.0).abs() < 1e-6, "the move was not undone: y={back}");
}

/// **Reported from use: a shape drawn earlier got undone instead of a
/// text box just added a moment later.** `undo` always tried the markup
/// layer's own stack first, no matter which of it and the document's own
/// command history had actually changed most recently — wrong here,
/// since drawing the line happened first and adding the text happened
/// last. A plain "undo" has to reverse the *last* thing that happened,
/// whichever of the two stacks that turns out to live on.
#[test]
fn undo_reverses_whichever_stack_changed_more_recently() {
    let mut app = app();
    app.submit("l 10,10 100,100");
    assert_eq!(marks(&app), 1, "the line did not draw:\n{}", said(&app));
    // A frame's worth of polling between the two actions — see
    // `track_undo_recency`'s own doc for why order is only told apart
    // this way, not by comparing two raw counts at the end.
    app.track_undo_recency();

    app.write_text_at(0, AppPoint { x: 200.0, y: 200.0 }, "HELLO").expect("written");
    let runs_before = app.tab_mut().doc.as_ref().unwrap().session.text_runs(0).expect("runs");
    assert!(runs_before.iter().any(|r| r.text.contains("HELLO")), "the text was not written");

    app.submit("undo");

    assert_eq!(marks(&app), 1, "the line was undone instead of the text");
    let runs_after = app.tab_mut().doc.as_ref().unwrap().session.text_runs(0).expect("runs");
    assert!(
        !runs_after.iter().any(|r| r.text.contains("HELLO")),
        "the text was still there — the wrong stack was undone:\n{}",
        said(&app)
    );
}

/// The same, in the other order: text written first, a shape drawn
/// after it, so undo must reverse the *shape* this time.
#[test]
fn undo_still_prefers_the_layer_when_that_is_what_changed_last() {
    let mut app = app();
    app.write_text_at(0, AppPoint { x: 200.0, y: 200.0 }, "HELLO").expect("written");
    app.track_undo_recency();
    app.submit("l 10,10 100,100");
    assert_eq!(marks(&app), 1, "the line did not draw:\n{}", said(&app));

    app.submit("undo");

    assert_eq!(marks(&app), 0, "the shape should have been the one undone:\n{}", said(&app));
    let runs_after = app.tab_mut().doc.as_ref().unwrap().session.text_runs(0).expect("runs");
    assert!(
        runs_after.iter().any(|r| r.text.contains("HELLO")),
        "the text was undone instead of the shape"
    );
}

#[test]
fn with_nothing_to_undo_it_says_so_honestly() {
    let mut app = app();
    app.submit("undo");
    assert!(
        said(&app).contains("nothing to undo"),
        "no honest answer:\n{}",
        said(&app)
    );
}

#[test]
fn an_operation_that_did_nothing_leaves_no_step() {
    // Undoing "nothing happened" looks broken, because nothing moves.
    let mut app = app();
    app.submit("l 10,10 100,100");
    app.submit("erase"); // nothing selected
    app.submit("undo");

    assert_eq!(marks(&app), 0, "undo hit an empty step first:\n{}", said(&app));
}
