use super::*;

fn fixture(name: &str) -> String {
    format!(
        "{}/../../../rust/pdf_core/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn said(app: &PagifyApp) -> String {
    app.cmd.history().iter().map(|e| e.text.as_str()).collect::<Vec<_>>().join("\n")
}

#[test]
fn a_clean_document_reports_nothing_to_review() {
    // Two pangrams — every word real.
    let mut app = PagifyApp::new(Some(&fixture("text-lines.pdf")));
    app.submit("spelling");
    app.wait_for_spell_scan();

    let panel = app.tab_mut().spelling.as_ref().expect("the panel did not open");
    assert!(panel.found.is_empty(), "flagged real words: {:?}", panel.found);
    assert!(said(&app).contains("no misspelled"), "{}", said(&app));
}

/// **"text the user might have added"** — a word typed in through the
/// app is ordinary page content by the time `text_runs` reads it back
/// (see `scan_spelling`'s own doc), so it is found exactly the way a
/// typo already in the file would be.
#[test]
fn a_typo_just_typed_in_is_found() {
    let mut app = PagifyApp::new(Some(&fixture("text-lines.pdf")));
    app.write_text_at(0, AppPoint { x: 40.0, y: 400.0 }, "This is a tpyo")
        .expect("written");

    app.submit("spelling");
    app.wait_for_spell_scan();
    let panel = app.tab_mut().spelling.as_ref().expect("the panel did not open");
    assert!(
        panel.found.iter().any(|m| m.word == "tpyo"),
        "the typo was not found: {:?}",
        panel.found
    );
}

/// **Not "DALI" — a comprehensive dictionary is not choosy, and "dali"
/// happens to already be one of its 370,000 entries.** "PDF" has no such
/// luck, so it stands in as the acronym a plain English word list was
/// never going to know regardless of the all-capitals rule below.
#[test]
fn an_acronym_is_not_flagged() {
    let mut app = PagifyApp::new(Some(&fixture("text-lines.pdf")));
    app.write_text_at(0, AppPoint { x: 40.0, y: 400.0 }, "a PDF fixture")
        .expect("written");

    app.submit("spelling");
    app.wait_for_spell_scan();
    let panel = app.tab_mut().spelling.as_ref().expect("the panel did not open");
    assert!(
        panel.found.iter().all(|m| m.word != "PDF"),
        "an all-capitals acronym was flagged: {:?}",
        panel.found
    );
}

/// **"it should have all the same properties of the replaced word"** —
/// the same guarantee `replace_current` gives, checked here for a fix
/// applied from the spelling panel specifically.
#[test]
fn changing_a_typo_fixes_it_and_keeps_the_runs_style() {
    let mut app = PagifyApp::new(Some(&fixture("text-lines.pdf")));
    app.write_text_at(0, AppPoint { x: 40.0, y: 400.0 }, "This is a tpyo")
        .expect("written");
    let before = app.tab_mut()
        .doc
        .as_ref()
        .unwrap()
        .session
        .text_runs(0)
        .expect("runs")
        .into_iter()
        .find(|r| r.text.contains("tpyo"))
        .expect("the written run");

    app.apply_spelling_change(0, before.object, "tpyo", "typo").expect("change failed");

    let after = app.tab_mut().doc.as_ref().unwrap().session.text_runs(0).expect("runs");
    let changed = after
        .iter()
        .find(|r| r.object == before.object)
        .expect("the run should still be there");
    assert!(changed.text.contains("typo"), "not fixed: {:?}", changed.text);
    assert_eq!(changed.size, before.size, "the size changed");
    assert_eq!(
        (changed.color.r, changed.color.g, changed.color.b),
        (before.color.r, before.color.g, before.color.b),
        "the colour changed"
    );

    // And a fresh scan no longer finds it.
    app.submit("spelling");
    app.wait_for_spell_scan();
    let panel = app.tab_mut().spelling.as_ref().expect("panel");
    assert!(!panel.found.iter().any(|m| m.word == "tpyo"), "still flagged after the fix");
}

#[test]
fn suggestions_offer_the_intended_word() {
    let mut app = PagifyApp::new(Some(&fixture("text-lines.pdf")));
    app.write_text_at(0, AppPoint { x: 40.0, y: 400.0 }, "quite definately so")
        .expect("written");

    app.submit("spelling");
    app.wait_for_spell_scan();
    let panel = app.tab_mut().spelling.as_ref().expect("panel");
    let found = panel
        .found
        .iter()
        .find(|m| m.word == "definately")
        .expect("the typo should be found");
    let suggestions = spelling::suggest(&found.word, 5);
    assert!(
        suggestions.iter().any(|s| s == "definitely"),
        "expected \"definitely\" among {suggestions:?}"
    );
}

#[test]
fn the_panel_needs_a_document_open() {
    let mut app = PagifyApp::new(None);
    app.submit("spelling");
    app.wait_for_spell_scan();
    assert!(app.tab_mut().spelling.is_none(), "opened with nothing to check");
    assert!(said(&app).contains("nothing open"), "{}", said(&app));
}
