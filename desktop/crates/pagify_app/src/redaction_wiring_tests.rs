use super::*;

fn fixture(name: &str) -> String {
    format!(
        "{}/../../../rust/pdf_core/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn app(name: &str) -> PagifyApp {
    let app = PagifyApp::new(Some(&fixture(name)));
    assert!(app.tab().doc.is_some(), "{name} did not open");
    app
}

fn said(app: &PagifyApp) -> String {
    app.cmd.history().iter().map(|e| e.text.as_str()).collect::<Vec<_>>().join("\n")
}

fn page_text(app: &PagifyApp, page: usize) -> String {
    app.tab().doc
        .as_ref()
        .expect("open")
        .session
        .characters(page)
        .map(|c| c.text().to_string())
        .unwrap_or_default()
}

/// **The verb used to lie.** `redact` sat in the shell's planned table
/// answering "a later phase" long after the engine could do it, so the one
/// destructive feature in the program was unreachable from the one place a
/// user would look for it.
#[test]
fn the_redact_verb_arms_the_tool_rather_than_refusing() {
    let mut app = app("text-lines.pdf");
    app.submit("redact");
    assert!(
        matches!(app.tab_mut().tool.as_ref().map(|t| &t.kind), Some(Tool::Redact)),
        "redact did not arm anything:\n{}",
        said(&app)
    );
    assert!(said(&app).contains("first corner"), "it did not say what to do next");
}

/// The tool stays in hand until it is put down, like every other tool.
#[test]
fn the_redaction_tool_stays_armed() {
    assert!(Tool::Redact.repeats());
}

/// **The signature tool is the odd one out.** Every other one-point
/// tool stays armed so a run of stamps does not mean a trip to the
/// ribbon between each one. A signature is different: the click right
/// after placing one is almost always aimed at adjusting the picture
/// just placed, not starting another, and a tool still in hand would
/// have taken that click for itself. `Tool::Signature.repeats()` answers
/// `false` for exactly that reason, and `resolve` consults it before any
/// re-arm — on success or on failure.
#[test]
fn the_signature_tool_does_not_stay_armed() {
    let mut app = app("single-page.pdf");
    app.arm_tool(Tool::Signature, 0);
    assert!(app.tab_mut().tool.is_some(), "setup: the tool should have armed");

    app.take_pick(AppPoint { x: 100.0, y: 100.0 });

    assert!(
        app.tab_mut().tool.is_none(),
        "the signature tool must not still be in hand after one click, even \
         though placing it failed (no signature saved yet) — unlike a \
         repeating tool such as redaction, a signature never re-arms"
    );
}

/// **Snapping is off for it.** A redaction is placed against words, and the
/// nearest drawn line has nothing to do with where the words are.
#[test]
fn the_redaction_tool_does_not_snap_to_geometry() {
    assert!(!Tool::Redact.wants_snapping());
}

/// A clean redaction goes straight through, and the words are gone.
#[test]
fn redacting_a_clear_area_destroys_the_text() {
    let mut app = app("text-lines.pdf");
    assert!(page_text(&app, 0).contains("The quick brown fox"), "control");

    // "The quick brown fox" sits at L40.3 T49.8 R165.2 B62.8.
    let outcome = app.redact(0, AppPoint::new(35.0, 45.0), AppPoint::new(170.0, 66.0));
    assert!(outcome.is_ok(), "{outcome:?}");
    assert!(app.tab_mut().secure_state.asking_to_redact.is_none(), "it asked about a clear area");

    let after = page_text(&app, 0);
    assert!(!after.contains("The quick brown fox"), "the words survived: {after:?}");
    assert!(after.contains("jumps over the lazy dog"), "it took more than it was pointed at");
}

/// And it undoes, through the same command stack as everything else.
#[test]
fn a_redaction_undoes_from_the_command_box() {
    let mut app = app("text-lines.pdf");
    let before = page_text(&app, 0);
    app.redact(0, AppPoint::new(35.0, 45.0), AppPoint::new(170.0, 66.0)).expect("redact");
    assert!(!page_text(&app, 0).contains("The quick brown fox"));

    app.submit("undo");
    assert_eq!(page_text(&app, 0), before, "undo did not put the page back:\n{}", said(&app));
}

/// **With a bundled font, an outlined page is no longer an automatic
/// refusal.** The app now hands `preview_redaction`/`redact` real candidate
/// faces — see `BUNDLED_OUTLINED_FONTS` — so a rectangle over outlined type
/// genuinely gets surveyed, and most of what a broad rectangle covers
/// matches and becomes removable. What replaces the old blanket refusal is
/// the same acknowledgement path an image beside real text already uses:
/// offered as a question when something inside the rectangle could not be
/// identified, not silently guessed at and not silently dropped.
#[test]
fn an_outlined_page_now_offers_a_real_redaction_with_the_bundled_font() {
    let mut app = app("outlined.pdf");
    let outcome = app.redact(0, AppPoint::new(10.0, 10.0), AppPoint::new(300.0, 300.0));
    assert!(outcome.is_ok(), "a font-assisted redaction was refused outright: {outcome:?}");

    let asking = app.tab_mut().secure_state.asking_to_redact.as_ref().expect(
        "a broad rectangle over real outlined type matched everything — unexpected, \
         but not itself wrong; if this legitimately now clears in one step, this test's \
         premise needs revisiting rather than the assertion loosened blindly",
    );
    assert!(asking.report.objects > 0, "the bundled font matched nothing at all: {:?}", asking.report);
    // And whatever it could not identify is still named as outlined type,
    // not silently folded into "removed" or a generic blocker.
    assert!(
        asking
            .report
            .uncleared
            .iter()
            .any(|u| matches!(u, pdf_core::document::Uncleared::OutlinedText { .. })),
        "nothing was left as an honest blocker: {:?}",
        asking.report
    );
}

/// **The refusal that must still not be overridable.** A scan's words are
/// pixels, not paths — no font, bundled or otherwise, changes that, unlike
/// outlined type above.
#[test]
fn a_scan_is_refused_the_same_way() {
    let mut app = app("scan-300dpi.pdf");
    let outcome = app.redact(0, AppPoint::new(50.0, 50.0), AppPoint::new(200.0, 100.0));
    assert!(outcome.is_err());
    assert!(app.tab_mut().secure_state.asking_to_redact.is_none());
}

#[test]
fn an_area_with_no_size_is_refused_before_anything_else() {
    let mut app = app("text-lines.pdf");
    assert!(app.redact(0, AppPoint::new(50.0, 50.0), AppPoint::new(50.2, 50.2)).is_err());
}

/// **The trap the whole feature turns on.** Saving a redacted document
/// incrementally keeps the original bytes and appends a delta, so every word
/// removed is still in the file at its old offset. The app must ask before
/// it picks a save mode, not assume the one that is right the rest of the
/// time.
#[test]
fn a_redacted_document_no_longer_saves_incrementally() {
    let mut app = app("text-lines.pdf");
    assert!(
        !app.tab_mut().doc.as_ref().unwrap().session.must_save_full_copy(),
        "nothing has been redacted yet"
    );

    app.redact(0, AppPoint::new(35.0, 45.0), AppPoint::new(170.0, 66.0)).expect("redact");
    assert!(
        app.tab_mut().doc.as_ref().unwrap().session.must_save_full_copy(),
        "the app would have appended a delta and left the words in the file"
    );
}

/// Saving really does write a file the words are not in.
#[test]
fn saving_after_a_redaction_writes_a_file_without_the_words() {
    let dir = std::env::temp_dir().join("pagify-redaction-wiring");
    let _ = std::fs::create_dir_all(&dir);
    let target = dir.join("redacted.pdf");
    let _ = std::fs::remove_file(&target);

    let mut app = app("text-lines.pdf");
    app.redact(0, AppPoint::new(35.0, 45.0), AppPoint::new(170.0, 66.0)).expect("redact");
    app.save(Some(target.clone()));
    assert!(target.exists(), "nothing was written:\n{}", said(&app));

    let reopened = PagifyApp::new(Some(target.to_str().expect("path")));
    let after = page_text(&reopened, 0);
    assert!(
        !after.contains("The quick brown fox"),
        "the saved file still has the words: {after:?}"
    );
    assert!(after.contains("jumps over the lazy dog"), "the rest of the page went missing");

    let _ = std::fs::remove_file(&target);
}
