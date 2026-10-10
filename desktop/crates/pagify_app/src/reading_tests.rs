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
fn find_reports_what_it_found_and_lands_on_the_first_match() {
    let mut app = PagifyApp::new(Some(&fixture("text-lines.pdf")));
    app.submit("find fox");

    assert_eq!(app.tab_mut().panels.find_hits.len(), 1, "history:\n{}", said(&app));
    assert_eq!(app.tab_mut().panels.find_at, 0);
    // The match is also the selection, so ⌘C copies what was found.
    assert!(app.tab_mut().text_selection.is_some(), "the match was not selected");
}

#[test]
fn a_match_selects_exactly_the_words_searched_for() {
    let mut app = PagifyApp::new(Some(&fixture("text-lines.pdf")));
    app.submit("find brown");

    let range = app.tab_mut().text_selection.clone().expect("a selection");
    let found = app.characters(0).expect("characters").text_of(range);
    assert_eq!(found, "brown", "the highlight is over the wrong characters");
}

#[test]
fn search_is_case_insensitive() {
    let mut app = PagifyApp::new(Some(&fixture("text-lines.pdf")));
    app.submit("find QUICK");
    assert_eq!(app.tab_mut().panels.find_hits.len(), 1, "history:\n{}", said(&app));
}

#[test]
fn stepping_wraps_round_rather_than_stopping_dead() {
    // Two columns of prose, so there is more than one of a common word.
    let mut app = PagifyApp::new(Some(&fixture("two-column.pdf")));
    app.submit("find the");
    let count = app.tab_mut().panels.find_hits.len();
    assert!(count > 2, "only {count} matches — is the fixture right?");

    for _ in 0..count {
        app.submit("findnext");
    }
    assert_eq!(app.tab_mut().panels.find_at, 0, "stepping through every match did not wrap");

    app.submit("findprev");
    assert_eq!(app.tab_mut().panels.find_at, count - 1, "stepping back from the first did not wrap");
}

#[test]
fn a_search_that_finds_nothing_says_so_and_changes_nothing() {
    let mut app = PagifyApp::new(Some(&fixture("text-lines.pdf")));
    app.submit("find zzzznotpresent");

    assert!(app.tab_mut().panels.find_hits.is_empty());
    assert!(app.tab_mut().text_selection.is_none(), "a failed search left a selection behind");
    assert!(said(&app).contains("no matches"), "no explanation:\n{}", said(&app));
}

#[test]
fn stepping_before_searching_explains_rather_than_doing_nothing() {
    let mut app = PagifyApp::new(Some(&fixture("text-lines.pdf")));
    app.submit("findnext");
    assert!(said(&app).contains("find"), "no guidance:\n{}", said(&app));
}

#[test]
fn find_crosses_pages_and_jumps_to_the_page_the_match_is_on() {
    let mut app = PagifyApp::new(Some(&fixture("pages-ladder.pdf")));
    // No text in that fixture at all — the honest result is nothing found,
    // and crucially not a panic or a jump to a page that has no match.
    app.submit("find anything");
    assert!(app.tab_mut().panels.find_hits.is_empty());
    assert_eq!(app.tab_mut().page, 0);
}

#[test]
fn find_needs_something_to_search() {
    let mut app = PagifyApp::new(Some(&fixture("text-lines.pdf")));
    app.submit("find");
    assert!(said(&app).contains("usage"), "no usage shown:\n{}", said(&app));
}

/// **"when searched word is replaced with a replacement word it should
/// have all the same properties of the replaced word."** Checked
/// against the run's own size and colour, read before and after —
/// `replace_all` sends `TextStyle::default()`, which is what keeps the
/// byte-safe path from touching anything but the words themselves.
#[test]
fn replace_all_swaps_the_word_and_keeps_the_runs_own_style() {
    let mut app = PagifyApp::new(Some(&fixture("text-lines.pdf")));
    let before = app.tab_mut().doc.as_ref().unwrap().session.text_runs(0).expect("runs")[0].clone();
    assert!(before.text.contains("fox"), "fixture assumption: {:?}", before.text);

    let said = app.replace_all("fox", "wolf").expect("replace failed");
    assert!(said.contains('1'), "should have reported one replacement: {said}");

    let after = app.tab_mut().doc.as_ref().unwrap().session.text_runs(0).expect("runs");
    let changed = after
        .iter()
        .find(|r| r.object == before.object)
        .expect("the run should still be there");
    assert!(
        changed.text.contains("wolf") && !changed.text.contains("fox"),
        "the word was not swapped: {:?}",
        changed.text
    );
    assert_eq!(changed.size, before.size, "the size changed");
    assert_eq!(
        (changed.color.r, changed.color.g, changed.color.b),
        (before.color.r, before.color.g, before.color.b),
        "the colour changed"
    );
    // The box itself is free to grow — "wolf" is wider than "fox" — but
    // where it starts and its own line must not move.
    assert_eq!(changed.rect.left, before.rect.left, "the run's start moved");
    assert_eq!(changed.rect.top, before.rect.top, "the run changed line");
    assert_eq!(changed.rect.bottom, before.rect.bottom, "the run changed line");
}

#[test]
fn replace_all_reaches_every_page_and_every_matching_run() {
    // "the" appears in more than one of this fixture's two columns.
    let mut app = PagifyApp::new(Some(&fixture("two-column.pdf")));
    let before = app.tab_mut().doc.as_ref().unwrap().session.characters(0).expect("chars");
    let before_hits = before.find("the").len();
    assert!(before_hits > 2, "only {before_hits} matches — is the fixture right?");

    // Not "THE": `Characters::find` case-folds, so it would still find
    // its own replacement and the check below would prove nothing.
    let said = app.replace_all("the", "XYZ").expect("replace failed");
    assert!(
        said.contains(&before_hits.to_string()),
        "should have reported every occurrence, said: {said}"
    );

    let after = app.tab_mut().doc.as_ref().unwrap().session.characters(0).expect("chars");
    assert_eq!(after.find("the").len(), 0, "some occurrences were left behind");
}

#[test]
fn replace_all_says_so_when_nothing_matches() {
    let mut app = PagifyApp::new(Some(&fixture("text-lines.pdf")));
    let before = app.tab_mut().doc.as_ref().unwrap().session.text_runs(0).expect("runs");

    let said = app.replace_all("zzzznotpresent", "anything").expect("should not error");
    assert!(said.contains("not found"), "no explanation: {said}");

    let after = app.tab_mut().doc.as_ref().unwrap().session.text_runs(0).expect("runs");
    assert_eq!(before, after, "nothing should have changed");
}

#[test]
fn replace_all_needs_something_to_search_for() {
    let mut app = PagifyApp::new(Some(&fixture("text-lines.pdf")));
    assert!(app.replace_all("", "anything").is_err());
}

/// **"an option to just search and replace one by one."** Exactly one
/// occurrence changes per call — the rest of the document's own matches
/// are left exactly as they were, in their own size and colour.
#[test]
fn replace_current_changes_only_the_current_match() {
    let mut app = PagifyApp::new(Some(&fixture("two-column.pdf")));
    app.find("the");
    let before_count = app.tab_mut().panels.find_hits.len();
    assert!(before_count > 2, "only {before_count} matches — is the fixture right?");
    let before_style = app.tab_mut().doc.as_ref().unwrap().session.text_runs(0).expect("runs");

    // Not "THE": `Characters::find` case-folds, so counting "the" left
    // afterwards would still count this one's own replacement too.
    let said = app.replace_current("the", "XYZ").expect("replace failed");
    assert!(said.contains('1'), "should say one was replaced: {said}");

    app.find("the");
    assert_eq!(
        app.tab_mut().panels.find_hits.len(),
        before_count - 1,
        "should have changed exactly one match"
    );

    let after_style = app.tab_mut().doc.as_ref().unwrap().session.text_runs(0).expect("runs");
    let mut unchanged = 0;
    for was in &before_style {
        if let Some(now) = after_style.iter().find(|r| r.object == was.object) {
            if now.text == was.text {
                assert_eq!(now.size, was.size, "an untouched run's size changed");
                assert_eq!(
                    (now.color.r, now.color.g, now.color.b),
                    (was.color.r, was.color.g, was.color.b),
                    "an untouched run's colour changed"
                );
                unchanged += 1;
            }
        }
    }
    assert_eq!(
        unchanged,
        before_style.len() - 1,
        "more than one run's text changed"
    );
}

#[test]
fn replace_current_says_when_none_are_left() {
    // "fox" appears exactly once in this fixture.
    let mut app = PagifyApp::new(Some(&fixture("text-lines.pdf")));
    // Searched for first: a match is shown before it is replaced, so
    // Replace with nothing searched for yet only shows it (see
    // `g2_find_view_tests::replace_shows_the_match_before_replacing_it`).
    app.find("fox");
    let said = app.replace_current("fox", "wolf").expect("replace failed");
    assert!(said.contains("none left"), "{said}");
    assert!(app.tab_mut().panels.find_hits.is_empty());
}
