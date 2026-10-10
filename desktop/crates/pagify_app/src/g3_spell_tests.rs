use super::ui_tests::{harness, harness_from};
use super::*;
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;

fn suggest_calls() -> usize {
    spelling::SUGGEST_CALLS.with(|c| c.get())
}

fn typo_harness(text: &str) -> Harness<'static, PagifyApp> {
    let mut h = harness("text-lines.pdf");
    h.state_mut().write_text_at(0, AppPoint { x: 40.0, y: 400.0 }, text).expect("written");
    h
}

fn open_panel(h: &mut Harness<'static, PagifyApp>) {
    h.state_mut().submit("spelling");
    h.state_mut().wait_for_spell_scan();
    h.run_steps(3);
}

fn found_words(h: &Harness<'static, PagifyApp>) -> Vec<String> {
    h.state()
        .tab()
        .panels.spelling
        .as_ref()
        .map(|p| p.found.iter().map(|m| m.word.clone()).collect())
        .unwrap_or_default()
}

fn said(h: &Harness<'static, PagifyApp>) -> String {
    h.state().cmd.history().iter().map(|e| e.text.as_str()).collect::<Vec<_>>().join("\n")
}

/// A two-page PDF whose pages are drawn in plain Helvetica, with a
/// `ToUnicode` map that reads the capital letters A-R as Arabic letters
/// and `_` as a zero-width non-joiner: the only way to put Arabic or
/// Persian text into a fixture without shipping a font. What the checker
/// sees is exactly what extraction returns for a real Arabic page.
fn arabic_pdf(first: &str, second: &str) -> std::path::PathBuf {
    let cmap = "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
                /CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n\
                1 begincodespacerange\n<00> <FF>\nendcodespacerange\n\
                1 beginbfrange\n<41> <52> <0627>\nendbfrange\n\
                6 beginbfchar\n<5F> <200C>\n<53> <067E>\n<54> <0686>\n<55> <06AF>\n<56> <06A9>\n<57> <06CC>\nendbfchar\n\
                endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend";
    let page = |content: &str| format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len() + 1);
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R 5 0 R] /Count 2 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R \
         /Resources << /Font << /F1 7 0 R >> >> >>"
            .to_string(),
        page(&format!("BT /F1 14 Tf 50 700 Td ({first}) Tj ET")),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 6 0 R \
         /Resources << /Font << /F1 7 0 R >> >> >>"
            .to_string(),
        page(&format!("BT /F1 14 Tf 50 700 Td ({second}) Tj ET")),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /ToUnicode 8 0 R >>".to_string(),
        format!("<< /Length {} >>\nstream\n{cmap}\nendstream", cmap.len() + 1),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let xref_at = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in &offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF", objects.len() + 1)
            .as_bytes(),
    );
    // One file per call: the tests run in parallel.
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("pagify-g3-spell-{}-{n}.pdf", std::process::id()));
    std::fs::write(&path, out).expect("the fixture could not be written");
    path
}

fn arabic_harness(first: &str, second: &str) -> Harness<'static, PagifyApp> {
    let path = arabic_pdf(first, second);
    let app = PagifyApp::new(Some(path.to_str().unwrap()));
    assert!(app.tab().doc.is_some(), "the fixture did not open");
    harness_from(app)
}

// ---- 1. suggestions are worked out once per word ---------------------

/// **Reported from use: the panel lags.** `draw_spell_check` ran the
/// suggestion search (hundreds of thousands of edit distances) on every
/// frame it drew, and egui draws a frame for every mouse move.
#[test]
fn suggestions_are_worked_out_once_per_word_not_once_per_frame() {
    let mut h = typo_harness("a tpyo here");
    spelling::SUGGEST_CALLS.with(|c| c.set(0));
    open_panel(&mut h);
    h.run_steps(30);
    assert_eq!(found_words(&h), ["tpyo"], "the fixture should have exactly one typo");
    assert_eq!(
        suggest_calls(),
        1,
        "one word, one search: thirty frames must not each run it again"
    );
}

/// The cache is keyed by the word, not by a position in the list: after
/// Ignore moves on, the panel must offer the *next* word's suggestions.
#[test]
fn the_next_word_shows_its_own_suggestions_after_an_ignore() {
    let mut h = typo_harness("a tpyo and definately so");
    spelling::SUGGEST_CALLS.with(|c| c.set(0));
    open_panel(&mut h);
    assert_eq!(found_words(&h), ["tpyo", "definately"]);

    h.get_by_label("Ignore").click();
    h.run_steps(5);
    assert_eq!(found_words(&h), ["definately"], "Ignore should drop only the first");
    let calls = suggest_calls();
    assert_eq!(calls, 2, "two distinct words, two searches, however many frames");

    let (for_tpyo, for_definately) =
        (spelling::suggest("tpyo", 5), spelling::suggest("definately", 5));
    assert!(for_definately.iter().any(|s| s == "definitely"), "{for_definately:?}");
    for word in &for_definately {
        h.get_by_label(word);
    }
    for stale in for_tpyo.iter().filter(|s| !for_definately.contains(s)) {
        assert!(
            h.query_by_label(stale).is_none(),
            "\"{stale}\" is a suggestion for the word that was ignored, not for this one"
        );
    }
    let panel = h.state().tab().panels.spelling.as_ref().unwrap();
    assert_eq!(panel.replacement, "definitely", "Change to: should start on the new word's best");
}

// ---- 2. what the English list cannot judge is not judged -------------

/// **Arabic and Persian were flagged 100%**: the list holds a-z and
/// nothing else, so every word in another script was unknown by
/// construction. A Persian word with a half-space was also cut in two.
/// Arabic now has a dictionary of its own; Persian still has none, and a
/// page of it must not be flagged against the Arabic one.
#[test]
fn persian_words_are_not_flagged_and_english_typos_still_are() {
    // Page 1: Persian (the font maps S-W to پ چ گ ک ی). Page 2: eight
    // English words, and one Persian word with a half-space.
    let mut h = arabic_harness(
        "STU VWA STU VWA",
        "a tpyo is here and still fine today SA_VW",
    );
    open_panel(&mut h);
    assert_eq!(
        found_words(&h),
        ["tpyo"],
        "only the English typo should be flagged: {:?}",
        found_words(&h)
    );
}

#[test]
fn an_accented_word_is_not_flagged_but_an_english_typo_next_to_it_is() {
    let mut h = typo_harness("a café and a tpyo");
    open_panel(&mut h);
    assert_eq!(found_words(&h), ["tpyo"], "café is French-accented, not misspelled English");
}

/// **"mostly Arabic" says so, once.** A page the list cannot judge is not
/// reported as "no misspelled words", which would read as a clean bill
/// of health for a page nothing looked at.
#[test]
fn a_page_mostly_in_persian_is_skipped_and_the_panel_says_so_once() {
    let mut h = arabic_harness("STU VWA STU VWA", "a tpyo is here and still fine");
    open_panel(&mut h);
    assert_eq!(found_words(&h), ["tpyo"], "the English page is still checked");
    h.run_steps(3);
    h.get_by_label_contains("mostly in a language the spelling check has no dictionary for");
}

#[test]
fn a_document_that_is_entirely_persian_does_not_claim_to_be_clean() {
    let mut h = arabic_harness("STU VWA STU VWA", "VWA STU VWA STU");
    open_panel(&mut h);
    assert!(found_words(&h).is_empty(), "{:?}", found_words(&h));
    h.get_by_label_contains("mostly in a language the spelling check has no dictionary for");
}

/// A half-space (U+200C) or a joiner (U+200D) between two letters keeps
/// them one word; one at either end is not part of it.
#[test]
fn a_half_space_keeps_a_persian_word_in_one_piece() {
    let words = |text| -> Vec<String> {
        spelling::words_in(text).into_iter().map(|(_, w)| w.to_string()).collect()
    };
    assert_eq!(
        words("\u{643}\u{62A}\u{627}\u{628}\u{200C}\u{647}\u{627} \u{62E}\u{648}\u{628}"),
        ["\u{643}\u{62A}\u{627}\u{628}\u{200C}\u{647}\u{627}", "\u{62E}\u{648}\u{628}"]
    );
    assert_eq!(words("\u{200C}abc\u{200D}def\u{200C} ghi"), ["abc\u{200D}def", "ghi"]);
    // The range handed back is the word itself, joiner and all.
    let text = "x \u{200C}ab\u{200C}cd\u{200C}";
    let (range, word) = spelling::words_in(text).into_iter().nth(1).unwrap();
    assert_eq!(&text[range], word);
    assert_eq!(word, "ab\u{200C}cd");
}

/// The rule for what is not judged: anything but plain a-z. A typo in
/// plain English next to them is still found.
#[test]
fn words_with_digits_accents_or_other_scripts_are_not_judged() {
    let runs = ["The gu10 and e27 lamps at 300dpi are caf\u{e9} grade, a tpyo here, \u{43f}\u{440}\u{438}\u{432}\u{435}\u{442}"];
    assert_eq!(spelling::misspelled_in_page(&runs), Some(vec![(0, "tpyo")]));
    // The same words with one typo each are all still found when plain.
    assert_eq!(
        spelling::misspelled_in_page(&["lihgting plan", "a tpyo"]),
        Some(vec![(0, "lihgting"), (1, "tpyo")])
    );
    assert_eq!(spelling::misspelled_in_page(&[]), Some(vec![]));
}

/// Mostly means more than half of the words on the page.
#[test]
fn a_page_is_skipped_only_when_more_than_half_of_it_is_in_another_script() {
    // Russian: a script none of the dictionaries judges.
    let russian = "\u{43f}\u{440}\u{438}\u{432}\u{435}\u{442} \u{43c}\u{438}\u{440}";
    assert_eq!(spelling::misspelled_in_page(&[&format!("tpyo here {russian}")]), Some(vec![(0, "tpyo")]));
    assert_eq!(spelling::misspelled_in_page(&[&format!("tpyo {russian}")]), None);
    assert_eq!(spelling::misspelled_in_page(&[russian]), None);
}

#[test]
fn the_suggestion_cache_is_per_word_and_blind_to_case() {
    let mut panel = SpellCheck::default();
    spelling::SUGGEST_CALLS.with(|c| c.set(0));
    let first = panel.suggestions_for("Definately").to_vec();
    assert!(first.iter().any(|s| s == "definitely"), "{first:?}");
    assert_eq!(panel.suggestions_for("definately"), &first[..]);
    assert_eq!(suggest_calls(), 1, "Definately and definately are one word");
    assert!(panel.suggestions_for("tpyo") != &first[..]);
    assert_eq!(suggest_calls(), 2);
}

#[test]
fn the_skipped_note_names_the_pages() {
    let note = |pages: Vec<usize>| {
        SpellCheck { skipped_pages: pages, ..SpellCheck::default() }.skipped_note()
    };
    assert_eq!(note(vec![]), None);
    assert_eq!(
        note(vec![2]).as_deref(),
        Some("Page 3 is mostly in a language the spelling check has no dictionary for - it was skipped.")
    );
    assert!(note(vec![0, 4]).unwrap().starts_with("Pages 1 and 5 are mostly"));
    assert!(note((0..40).collect()).unwrap().starts_with("40 pages are mostly"));
}

// ---- 3. a Change the engine refuses says so --------------------------

/// **A refused Change was dropped without a word.** The error went to
/// the history bar, and the word left the list as if it had been fixed.
#[test]
fn a_change_the_engine_refuses_says_so_and_keeps_the_word_in_the_list() {
    let mut h = typo_harness("a tpyo here");
    open_panel(&mut h);
    // A control character no font has a glyph for: the engine refuses.
    h.state_mut().tab_mut().panels.spelling.as_mut().unwrap().replacement = "t\u{2}po".into();
    h.get_by_label("Change").click();
    h.run_steps(4);

    assert_eq!(found_words(&h), ["tpyo"], "a word that was not fixed must stay to be decided");
    let anchor = h.get_by_label_contains("word 1 of 1").rect();
    let shown_in_the_panel = h
        .get_all_by_label_contains("could not change")
        .any(|n| (n.rect().left() - anchor.left()).abs() < 1.0);
    assert!(shown_in_the_panel, "the panel did not say the change was refused");
    assert!(said(&h).contains("could not change"), "the history lost it too:\n{}", said(&h));
}

#[test]
fn a_change_all_the_engine_refuses_says_so_and_keeps_the_word_in_the_list() {
    let mut h = typo_harness("a tpyo here");
    open_panel(&mut h);
    h.state_mut().tab_mut().panels.spelling.as_mut().unwrap().replacement = "t\u{2}po".into();
    h.get_by_label("Change All").click();
    h.run_steps(4);

    assert_eq!(found_words(&h), ["tpyo"]);
    let anchor = h.get_by_label_contains("word 1 of 1").rect();
    let shown_in_the_panel = h
        .get_all_by_label_contains("could not change")
        .any(|n| (n.rect().left() - anchor.left()).abs() < 1.0);
    assert!(shown_in_the_panel, "the panel did not say Change All changed nothing");
}

/// The reason is for the action that was refused, not for the ones after.
#[test]
fn a_refusal_is_cleared_by_the_next_action() {
    let mut h = typo_harness("a tpyo here");
    open_panel(&mut h);
    h.state_mut().tab_mut().panels.spelling.as_mut().unwrap().replacement = "t\u{2}po".into();
    h.get_by_label("Change").click();
    h.run_steps(3);
    assert!(h.state().tab().panels.spelling.as_ref().unwrap().notice.is_some());

    h.get_by_label("Ignore").click();
    h.run_steps(3);
    let panel = h.state().tab().panels.spelling.as_ref().unwrap();
    assert!(panel.notice.is_none(), "{:?}", panel.notice);
    assert!(panel.found.is_empty(), "Ignore should still drop the word");
}

/// The other half: a Change that works still fixes the page and moves on.
#[test]
fn a_change_that_works_fixes_the_page_and_moves_on() {
    let mut h = typo_harness("a tpyo here");
    open_panel(&mut h);
    h.state_mut().tab_mut().panels.spelling.as_mut().unwrap().replacement = "typo".into();
    h.get_by_label("Change").click();
    h.run_steps(4);

    assert!(found_words(&h).is_empty(), "{:?}", found_words(&h));
    assert!(h.query_all_by_label_contains("could not change").next().is_none());
    let runs = h.state().tab().doc.as_ref().unwrap().session.text_runs(0).expect("runs");
    assert!(runs.iter().any(|r| r.text.contains("a typo here")), "{runs:?}");
}

// ---- 5. a custom dictionary ---------------------------------------------

/// **Reported from use: "no custom dictionary".** Ignore All lasts one
/// check; Add to Dictionary lasts, so the next check does not flag it.
#[test]
fn add_to_dictionary_drops_every_occurrence_and_the_next_check_does_not_flag_it() {
    let mut h = typo_harness("a zqxpagifyb and zqxpagifyb again");
    open_panel(&mut h);
    assert_eq!(found_words(&h), ["zqxpagifyb", "zqxpagifyb"]);

    h.get_by_label("Add to Dictionary").click();
    h.run_steps(4);
    assert!(found_words(&h).is_empty(), "{:?}", found_words(&h));
    assert!(said(&h).contains("added to your dictionary"), "{}", said(&h));

    open_panel(&mut h);
    assert!(found_words(&h).is_empty(), "flagged again on the next check: {:?}", found_words(&h));
}

// ---- 4. the scan does not hold the window up ---------------------------

/// **Reported from use: "spell check freezes".** Opening the panel starts the
/// scan and returns: until a frame collects the answer the panel is the
/// "checking" one and holds no words, so nothing was read on this thread.
#[test]
fn opening_the_panel_starts_a_scan_and_returns_without_waiting_for_it() {
    let mut h = typo_harness("a tpyo here");
    h.state_mut().submit("spelling");
    let tab = h.state().tab();
    assert!(tab.panels.spell_scan.is_some(), "no scan was started");
    let panel = tab.panels.spelling.as_ref().expect("the panel did not open at once");
    assert!(panel.scanning.is_some() && panel.found.is_empty(), "words were read before any frame ran");

    h.state_mut().wait_for_spell_scan();
    h.run_steps(3);
    assert!(h.state().tab().panels.spelling.as_ref().unwrap().scanning.is_none());
    assert_eq!(found_words(&h), ["tpyo"]);
}

#[test]
fn while_the_scan_runs_the_panel_shows_how_far_it_is_and_can_be_cancelled() {
    let mut h = typo_harness("a tpyo here");
    h.state_mut().tab_mut().panels.spelling =
        Some(SpellCheck { scanning: Some((3, 10)), ..SpellCheck::default() });
    h.run_steps(3);
    h.get_by_label_contains("page 3 of 10");
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert!(h.state().tab().panels.spelling.is_none(), "Cancel left the panel open");
    assert!(h.state().tab().panels.spell_scan.is_none());
}

/// A scan nobody is waiting for must not carry on reading a document.
#[test]
fn dropping_a_scan_tells_its_thread_to_stop() {
    let (_tx, done) = std::sync::mpsc::channel();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    drop(SpellScan { done, stop: stop.clone(), pages: 5 });
    assert!(stop.load(std::sync::atomic::Ordering::Relaxed));

    // And the scan itself honours it, between pages.
    let h = typo_harness("a tpyo here");
    let session = h.state().tab().doc.as_ref().unwrap().session.clone();
    let stopped = std::sync::atomic::AtomicBool::new(true);
    assert!(PagifyApp::scan_spelling(&session, 1, &stopped, |_| {}).is_none());
}

