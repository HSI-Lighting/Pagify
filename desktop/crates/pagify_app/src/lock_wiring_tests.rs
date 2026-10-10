use super::*;

/// **Reported from use, with a screenshot**: deleting most of a
/// paragraph left the deleted lines faintly but clearly visible
/// instead of blending into the page — not wrong words landing on the
/// wrong object (the shift corruption this session's other fixes
/// address), but the hide mechanism's own idea of "the page's
/// background here" coming out a few shades under true white. Targets
/// the exact paragraph and file reported from use: sampling its
/// background came back `rgb(248,248,248)` before the fix, snapped to
/// `rgb(255,255,255)` after it.
#[test]
fn a_dense_paragraphs_background_sample_is_not_left_a_few_shades_under_white() {
    let path = r"C:\Users\hsili\Desktop\Datasheets - Editors market - Marina mall.pdf";
    let mut app = PagifyApp::new(Some(path));
    let Some(_) = &app.tab_mut().doc else {
        eprintln!("skipping: Marina mall datasheet not present on this machine");
        return;
    };
    let runs = app.tab_mut().doc.as_ref().unwrap().session.text_runs(0).expect("runs");
    let mut found = false;
    for run in &runs {
        if !run.text.contains("perfect choice") {
            continue;
        }
        app.pick_text_run(0, AppPoint {
            x: ((run.rect.left + run.rect.right) / 2.0) as f64,
            y: ((run.rect.top + run.rect.bottom) / 2.0) as f64,
        })
        .expect("pick");
        found = true;
        break;
    }
    assert!(found, "the 'VEGA is the perfect choice' paragraph was not found — has the fixture changed?");

    let background = app.tab_mut().edit.editing_run.as_ref().expect("still editing").background;
    assert_eq!(
        background,
        pdf_core::document::Color { r: 255, g: 255, b: 255, a: 255 },
        "a background this close to white should snap to it, not stay a few shades under — \
         hidden text recoloured to it would otherwise stay faintly visible against the \
         page's own true white"
    );
}

#[test]
fn plain_click_selects_only_that_page() {
    let mut selected = vec![3, 4];
    let mut anchor = Some(4);
    PagifyApp::apply_organize_click(&mut selected, &mut anchor, 1, egui::Modifiers::NONE);
    assert_eq!(selected, vec![1]);
    assert_eq!(anchor, Some(1));
}

#[test]
fn ctrl_click_toggles_one_page_without_touching_the_rest() {
    let mut selected = vec![1, 2];
    let mut anchor = Some(2);
    PagifyApp::apply_organize_click(&mut selected, &mut anchor, 5, egui::Modifiers::COMMAND);
    assert_eq!(selected, vec![1, 2, 5], "ctrl+click on an unselected page adds it");

    PagifyApp::apply_organize_click(&mut selected, &mut anchor, 2, egui::Modifiers::COMMAND);
    assert_eq!(selected, vec![1, 5], "ctrl+click on an already-selected page removes just it");
}

#[test]
fn shift_click_selects_the_contiguous_range_from_the_anchor() {
    let mut selected = vec![2];
    let mut anchor = Some(2);
    PagifyApp::apply_organize_click(&mut selected, &mut anchor, 5, egui::Modifiers::SHIFT);
    assert_eq!(selected, vec![2, 3, 4, 5]);
    assert_eq!(anchor, Some(2), "shift+click does not move the anchor");

    // Shift-clicking back toward (or past) the anchor shrinks the range
    // from the same anchor, the way a file manager's does.
    PagifyApp::apply_organize_click(&mut selected, &mut anchor, 3, egui::Modifiers::SHIFT);
    assert_eq!(selected, vec![2, 3]);
}

#[test]
fn shift_click_with_no_anchor_yet_behaves_like_a_plain_click() {
    let mut selected = Vec::new();
    let mut anchor = None;
    PagifyApp::apply_organize_click(&mut selected, &mut anchor, 4, egui::Modifiers::SHIFT);
    assert_eq!(selected, vec![4]);
}

#[test]
fn nearest_drop_picks_the_closer_cell_and_the_right_side_of_it() {
    let cells = vec![
        (0, egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(100.0, 100.0))),
        (1, egui::Rect::from_min_size(egui::pos2(100.0, 0.0), egui::vec2(100.0, 100.0))),
        (2, egui::Rect::from_min_size(egui::pos2(200.0, 0.0), egui::vec2(100.0, 100.0))),
    ];

    // Left half of cell 1 — drop before cell 1.
    let (before, indicator_x, _) = PagifyApp::nearest_drop(&cells, egui::pos2(120.0, 50.0)).unwrap();
    assert_eq!(before, 1);
    assert_eq!(indicator_x, 100.0, "the indicator should sit at cell 1's own left edge");

    // Right half of cell 1 — drop after cell 1 (before cell 2).
    let (before, indicator_x, _) = PagifyApp::nearest_drop(&cells, egui::pos2(180.0, 50.0)).unwrap();
    assert_eq!(before, 2);
    assert_eq!(indicator_x, 200.0, "the indicator should sit at cell 2's own left edge");

    // Past every cell — drops at the end, indicator at the last cell's
    // own right edge since there is no "before" cell to anchor on.
    let (before, indicator_x, _) = PagifyApp::nearest_drop(&cells, egui::pos2(280.0, 50.0)).unwrap();
    assert_eq!(before, 3);
    assert_eq!(indicator_x, 300.0);
}

#[test]
fn nearest_drop_with_no_cells_finds_nowhere_to_drop() {
    assert!(PagifyApp::nearest_drop(&[], egui::pos2(10.0, 10.0)).is_none());
}

fn rect(left: f32, top: f32, right: f32, bottom: f32) -> pdf_core::document::Rect {
    pdf_core::document::Rect { left, top, right, bottom }
}

#[test]
fn standard_sheet_sizes_are_named() {
    assert_eq!(PagifyApp::paper_size_label(595.0, 842.0), "A4");
    assert_eq!(PagifyApp::paper_size_label(612.0, 792.0), "Letter");
}

#[test]
fn a_standard_sheet_turned_sideways_is_named_landscape() {
    assert_eq!(PagifyApp::paper_size_label(2384.0, 1684.0), "A1 Landscape");
    assert_eq!(PagifyApp::paper_size_label(1684.0, 2384.0), "A1");
}

#[test]
fn a_size_close_to_standard_within_rounding_still_matches() {
    // PDF page boxes are routinely off a point or two from the nominal
    // size — a real-world export, not a hand-typed test fixture.
    assert_eq!(PagifyApp::paper_size_label(594.5, 841.2), "A4");
}

#[test]
fn a_non_standard_size_falls_back_to_millimetres() {
    assert_eq!(PagifyApp::paper_size_label(1000.0, 1000.0), "353×353mm");
}

#[test]
fn a_screen_rect_converts_to_page_points_through_the_view() {
    let view = PageView { origin: egui::pos2(100.0, 50.0), scale: 2.0 };
    // 2x zoom: 20 screen px either side of the origin is 10 page points.
    let screen = egui::Rect::from_min_max(egui::pos2(100.0, 50.0), egui::pos2(140.0, 90.0));
    let page_rect = PagifyApp::page_rect_from_screen(view, screen, 1000.0, 1000.0);
    assert_eq!(page_rect, rect(0.0, 0.0, 20.0, 20.0));
}

#[test]
fn a_screen_rect_past_the_page_edge_is_clamped_to_it() {
    let view = PageView { origin: egui::pos2(0.0, 0.0), scale: 1.0 };
    // Asking for -50..150 on a 100x100 page — half of it is off the page
    // on every side.
    let screen = egui::Rect::from_min_max(egui::pos2(-50.0, -50.0), egui::pos2(150.0, 150.0));
    let page_rect = PagifyApp::page_rect_from_screen(view, screen, 100.0, 100.0);
    assert_eq!(page_rect, rect(0.0, 0.0, 100.0, 100.0));
}

#[test]
fn a_tile_covering_more_than_whats_visible_is_reused() {
    let tile_crop = rect(0.0, 0.0, 100.0, 100.0);
    let visible = rect(10.0, 10.0, 90.0, 90.0);
    assert!(PagifyApp::detail_tile_covers(0, 7, tile_crop, 0, 7, visible));
}

#[test]
fn a_tile_for_a_different_page_is_not_reused() {
    let tile_crop = rect(0.0, 0.0, 100.0, 100.0);
    let visible = rect(10.0, 10.0, 90.0, 90.0);
    assert!(!PagifyApp::detail_tile_covers(0, 7, tile_crop, 1, 7, visible));
}

#[test]
fn a_tile_at_a_stale_zoom_step_is_not_reused() {
    let tile_crop = rect(0.0, 0.0, 100.0, 100.0);
    let visible = rect(10.0, 10.0, 90.0, 90.0);
    assert!(!PagifyApp::detail_tile_covers(0, 7, tile_crop, 0, 8, visible));
}

#[test]
fn a_tile_that_no_longer_covers_whats_visible_is_not_reused() {
    let tile_crop = rect(0.0, 0.0, 100.0, 100.0);
    // Scrolled past the tile's own right edge.
    let visible = rect(50.0, 10.0, 150.0, 90.0);
    assert!(!PagifyApp::detail_tile_covers(0, 7, tile_crop, 0, 7, visible));
}

#[test]
fn a_tile_exactly_matching_whats_visible_is_reused() {
    let crop = rect(0.0, 0.0, 100.0, 100.0);
    assert!(PagifyApp::detail_tile_covers(0, 7, crop, 0, 7, crop));
}

/// **The one test that most directly justifies building the page
/// clipboard on `extract_to`/`import_from` rather than a new
/// `Session` method**: `extract_to` reads a tab's *live* document
/// state, so copying a page that exists only in memory (never saved,
/// never on disk) and pasting it into a different, independently-opened
/// tab on the same underlying file must still bring it across.
#[test]
fn copying_an_unsaved_page_from_one_tab_pastes_into_another() {
    let mut app = app("text-lines.pdf");
    let original_count = app.tab().doc.as_ref().expect("fixture should open").page_count;

    // A blank page that exists only in this tab's own memory — `insert_page`
    // inserts at the current page (0), so the new blank page becomes page 0
    // and the fixture's own content shifts to page 1.
    app.insert_page();
    assert_eq!(app.tab().doc.as_ref().unwrap().page_count, original_count + 1);

    app.tab_mut().organize.organize_selected = vec![0];
    assert!(app.copy_organize_selection(), "copy should have found a selection to extract");
    assert!(app.page_clipboard.is_some(), "copy should have filled the page clipboard");

    // A second, independent tab opened fresh from the same file on disk —
    // it never saw the blank page `insert_page` added to the first tab.
    app.submit(&format!("open \"{}\"", fixture("text-lines.pdf")));
    assert_eq!(app.tabs.len(), 2, "opening another file should make a second tab");
    let before = app.tab().doc.as_ref().unwrap().page_count;
    assert_eq!(before, original_count, "the second tab should not already have the blank page");

    app.tab_mut().organize.organize_selected.clear();
    app.paste_organize_selection();

    let after = app.tab().doc.as_ref().unwrap().page_count;
    assert_eq!(
        after,
        before + 1,
        "the page copied from the first tab's own unsaved state should have landed in the second"
    );
}

/// **Reported from use**: copy a page, paste it, then copy and paste
/// again in the same tab — the second paste failed with "the system
/// cannot find the file specified". `PageClipboard`'s backing file is
/// the same path every time (one clipboard per running app instance,
/// not per copy — see its own doc comment), and replacing
/// `self.page_clipboard` used to drop, and so delete, that shared path
/// *after* the second copy had already written its fresh content there.
#[test]
fn copying_and_pasting_twice_in_a_row_does_not_lose_the_second_copy() {
    let mut app = app("text-lines.pdf");
    let original_count = app.tab().doc.as_ref().unwrap().page_count;

    app.tab_mut().organize.organize_selected = vec![0];
    assert!(app.copy_organize_selection(), "first copy should find a selection");
    app.tab_mut().organize.organize_selected.clear();
    app.paste_organize_selection();
    assert_eq!(
        app.tab().doc.as_ref().unwrap().page_count,
        original_count + 1,
        "first paste should have landed: {}",
        said(&app)
    );

    app.tab_mut().organize.organize_selected = vec![0];
    assert!(app.copy_organize_selection(), "second copy should also report success");
    app.tab_mut().organize.organize_selected.clear();
    app.paste_organize_selection();
    assert_eq!(
        app.tab().doc.as_ref().unwrap().page_count,
        original_count + 2,
        "second paste should also have landed: {}",
        said(&app)
    );
    assert!(
        !said(&app).to_lowercase().contains("i/o error"),
        "the second copy's file must still exist when the second paste reads it: {}",
        said(&app)
    );
}

/// **Reported from use: "page copy and paste only works between two files
/// opened in the same window; opened separately, it doesn't paste".** The
/// clipboard was remembered by the one process that made it. Two apps
/// sharing a clipboard folder — which is what two Pagify windows do — now
/// paste each other's pages.
#[test]
fn pages_copied_in_one_window_paste_into_another_window() {
    let mut first = app("text-lines.pdf");
    let mut second = app("two-column.pdf");
    second.clipboard_dir = first.clipboard_dir.clone();
    let before = second.tab().doc.as_ref().unwrap().page_count;

    first.tab_mut().organize.organize_selected = vec![0];
    assert!(first.copy_organize_selection(), "copy should find a selection");
    second.tab_mut().organize.organize_selected.clear();
    second.paste_organize_selection();

    assert_eq!(
        second.tab().doc.as_ref().unwrap().page_count,
        before + 1,
        "the page copied in the first window did not paste in the second: {}",
        said(&second)
    );
}

/// The copy belongs to every window, so closing the window that made it
/// does not take the pages with it.
#[test]
fn a_copy_outlives_the_window_that_made_it() {
    let mut first = app("text-lines.pdf");
    let mut second = app("two-column.pdf");
    second.clipboard_dir = first.clipboard_dir.clone();
    let before = second.tab().doc.as_ref().unwrap().page_count;

    first.tab_mut().organize.organize_selected = vec![0];
    assert!(first.copy_organize_selection());
    drop(first);
    second.paste_organize_selection();

    assert_eq!(second.tab().doc.as_ref().unwrap().page_count, before + 1, "{}", said(&second));
}

/// A newer copy, from either window, replaces the older one for both; and
/// copying something that is not pages takes the pages back; and a copy
/// older than an hour is not offered to a window that did not make it.
#[test]
fn the_newest_copy_wins_and_a_stale_or_replaced_one_is_not_offered() {
    let mut first = app("text-lines.pdf");
    let mut second = app("two-column.pdf");
    second.clipboard_dir = first.clipboard_dir.clone();

    first.tab_mut().organize.organize_selected = vec![0];
    assert!(first.copy_organize_selection());
    second.tab_mut().organize.organize_selected = vec![0];
    assert!(second.copy_organize_selection());
    let (newest, _) = first.current_page_clipboard().expect("a copy");
    assert_eq!(
        Some(newest),
        second.page_clipboard.as_ref().map(|c| c.temp_file.clone()),
        "the first window did not see the second window's newer copy"
    );

    // Copying a shape or picture takes the pages back, for every window.
    second.forget_copied_pages();
    assert!(first.current_page_clipboard().is_some(), "the first window's own copy is still its own");
    first.forget_copied_pages();
    assert!(first.current_page_clipboard().is_none() && second.current_page_clipboard().is_none());

    // An hour-old manifest is not offered to a window with no copy of its own.
    let mut third = app("text-lines.pdf");
    let mut fourth = app("two-column.pdf");
    fourth.clipboard_dir = third.clipboard_dir.clone();
    third.tab_mut().organize.organize_selected = vec![0];
    assert!(third.copy_organize_selection());
    let manifest = third.clipboard_dir.join("latest.txt");
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(2 * 3600);
    std::fs::OpenOptions::new().write(true).open(&manifest).expect("manifest").set_modified(old).expect("age it");
    assert!(fourth.current_page_clipboard().is_none(), "a two-hour-old copy was offered");
    assert!(third.current_page_clipboard().is_some(), "the window that made it keeps its own copy");
}

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

const FOX: (f32, f32, f32, f32) = (35.0, 45.0, 170.0, 66.0);

fn fox_area() -> pdf_core::document::Rect {
    pdf_core::document::Rect { left: FOX.0, top: FOX.1, right: FOX.2, bottom: FOX.3 }
}

/// **The survey reports without changing anything.**
#[test]
fn hiddendata_says_what_is_there_and_offers_to_remove_it() {
    let mut app = app("two-column.pdf");
    app.submit("hiddendata");

    let said = said(&app);
    assert!(!said.is_empty(), "it said nothing at all");
    // Either there is nothing, or it says how to remove what there is.
    assert!(
        said.contains("nothing hidden") || said.contains("hiddendata clean"),
        "it reported findings without saying what to do about them: {said}"
    );
}

/// And cleaning says what went, then leaves nothing for a second survey.
#[test]
fn hiddendata_clean_removes_and_a_second_survey_is_empty() {
    let mut app = app("two-column.pdf");
    app.submit("hiddendata clean");
    assert!(
        !said(&app).is_empty() && !said(&app).contains("don't know"),
        "cleaning failed: {}",
        said(&app)
    );

    app.submit("hiddendata");
    assert!(
        said(&app).contains("nothing hidden"),
        "a second survey still found something: {}",
        said(&app)
    );
}

/// **What it says it removed is what the file no longer has.** Found by
/// audit: "removed: 1 embedded file(s); JavaScript" while both were still
/// in the file. Now the line comes from a survey of the cleaned bytes, and
/// the saved file is checked here the way anyone else would read it.
#[test]
fn hiddendata_clean_reports_from_the_cleaned_file_not_from_the_survey() {
    let mut app = app("hidden-things.pdf");
    app.submit("hiddendata");
    let before = said(&app);
    assert!(before.contains("embedded file") && before.contains("JavaScript"), "{before}");

    app.submit("hiddendata clean");
    let told = said(&app);
    assert!(told.contains("removed:"), "{told}");
    assert!(!told.contains("STILL THERE"), "{told}");
    for expected in ["embedded file", "JavaScript", "XMP", "Author"] {
        assert!(told.contains(expected), "{expected:?} not reported as removed: {told}");
    }

    let out = std::env::temp_dir().join(format!("pagify-hiddendata-{}.pdf", std::process::id()));
    app.submit(&format!("saveas {}", out.display()));
    let bytes = std::fs::read(&out).expect("the file was written");
    let _ = std::fs::remove_file(&out);
    let file = pdf_core::pdf::File::parse(&bytes).expect("parse");
    let after = pdf_core::pdf::hidden::survey(&file, &bytes).expect("survey");
    assert!(after.is_empty(), "the saved file still carries: {}", after.describe());
    assert!(!bytes.windows(16).any(|w| w == b"ATTACHED-PAYLOAD"), "the attachment survived");
    assert!(!bytes.windows(9).any(|w| w == b"app.alert"), "the script survived");
}

/// **The window offers the choice, and warns about the one that shuts
/// everyone out.**
#[test]
fn the_password_window_offers_secure_and_secure_plus() {
    // This module is not the one that usually drives a window, so it brings
    // in what it needs itself.
    use eframe::App as _;
    use egui_kittest::kittest::Queryable;
    use egui_kittest::Harness;

    let app = app("two-column.pdf");
    let mut h = Harness::builder()
        .with_size(egui::vec2(1200.0, 900.0))
        .build_ui_state(
            |ui, app: &mut PagifyApp| {
                let mut frame = eframe::Frame::_new_kittest();
                app.ui(ui, &mut frame);
            },
            app,
        );
    h.run_steps(2);

    h.state_mut().submit("secure");
    h.run();
    h.get_by_label_contains("Choose a password for this file");

    // Both offered, and the ordinary one says it is ordinary.
    h.get_by_label("Secure");
    h.get_by_label("Secure Plus");
    h.get_by_label_contains("Any reader will ask for this password");

    // Choosing the other says plainly what it costs, before it is used.
    h.get_by_label("Secure Plus").click();
    h.run();
    h.get_by_label_contains("Nothing else will open this file");
}

/// And the choice reaches the document.
#[test]
fn choosing_secure_plus_uses_pagifys_own_handler() {
    let mut app = app("two-column.pdf");
    app.submit("secure");
    app.tab_mut().secure_state.password_plus = true;
    app.answer_passcode("Correct-Horse-99-Battery");
    app.answer_passcode("Correct-Horse-99-Battery");

    let doc = app.tab_mut().doc.as_ref().expect("doc");
    assert!(doc.session.is_secured(), "no password was set:\n{}", said(&app));
    assert!(
        doc.session.is_secure_plus(),
        "it used PDF's handler when Secure Plus was chosen"
    );
}

/// **A certified document is unsaved work until it is saved, and what
/// the save writes is the signed file.** Found by audit: the signed bytes
/// lived only in memory, marked clean; a save re-serialised the document
/// and broke the signature; a close discarded it without asking.
#[test]
fn certifying_is_unsaved_work_and_saving_writes_the_signed_file() {
    let certificate = std::path::Path::new(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../../rust/pdf_core/fixtures/test-signer-sm2.p12"),
    );
    if !certificate.is_file() {
        eprintln!("skipping: no test certificate");
        return;
    }
    let mut app = app("two-column.pdf");
    assert!(!app.would_lose_work());
    app.submit(&format!("certify {}", certificate.display()));
    app.answer_passcode("pagify");
    assert!(said(&app).contains("signed as"), "{}", said(&app));
    assert!(app.would_lose_work(), "a signature not yet on disk was not guarded");

    let out = std::env::temp_dir().join(format!("pagify-certified-{}.pdf", std::process::id()));
    app.submit(&format!("saveas {}", out.display()));
    assert!(said(&app).contains("saved"), "{}", said(&app));
    assert!(!app.would_lose_work(), "saved, and still guarded");

    // The file on disk, checked with none of the signing code in the way.
    let bytes = std::fs::read(&out).expect("the file was written");
    let _ = std::fs::remove_file(&out);
    let file = pdf_core::pdf::File::parse(&bytes).expect("parse");
    let found = pdf_core::pdf::validate::check(&file, &bytes).expect("check");
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].verdict, pdf_core::pdf::validate::Verdict::Unaltered, "{:?}", found[0]);
}

/// **Signing says the thing people learn the hard way.**
///
/// A signature covers the file as it stands; the next edit is outside it,
/// and the check will say the document changed after it was signed. That
/// belongs in the line somebody reads when it works, not in a manual.
#[test]
fn signing_says_that_a_later_edit_is_outside_it() {
    let certificate = std::path::Path::new(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../../rust/pdf_core/fixtures/test-signer-sm2.p12"),
    );
    if !certificate.is_file() {
        eprintln!("skipping: no test certificate");
        return;
    }

    let mut app = app("two-column.pdf");
    app.submit("certify");
    assert!(said(&app).contains("not signed"), "{}", said(&app));

    app.submit(&format!("certify {}", certificate.display()));
    assert!(
        matches!(app.tab_mut().secure_state.awaiting_password, Some(Awaiting::Certificate(_))),
        "it did not ask for the certificate's password:\n{}",
        said(&app)
    );

    app.answer_passcode("pagify");
    let told = said(&app);
    assert!(told.contains("signed as"), "it did not sign: {told}");
    assert!(
        told.contains("outside it"),
        "it did not say that an edit afterwards is outside the signature: {told}"
    );

    // And a reader sees it.
    app.submit("certify");
    assert!(said(&app).contains("1 signature"), "{}", said(&app));
}

/// A signature pad, and a scratch file to keep it in — never the real one.
///
/// `label` is the *test's* name rather than the fixture's, because these
/// run in parallel in one process: two tests sharing a path delete each
/// other's file, and the one that notices is whichever lost the race.
fn with_signature_pad(name: &str, label: &str) -> (PagifyApp, std::path::PathBuf) {
    let mut app = app(name);
    let path = std::env::temp_dir()
        .join(format!("pagify-test-signatures-{}-{label}.json", std::process::id()));
    let _ = std::fs::remove_file(&path);
    app.signatures = Default::default();
    app.signatures_path = Some(path.clone());
    (app, path)
}

/// A rough scrawl, in a pad's own pixels.
fn scrawl() -> Vec<Vec<(f32, f32)>> {
    vec![
        vec![(20.0, 120.0), (60.0, 60.0), (100.0, 130.0), (150.0, 55.0)],
        vec![(30.0, 110.0), (170.0, 110.0)],
    ]
}

/// **Reaching for the tool with nothing drawn opens the pad.**
///
/// Rather than refusing with "no signature" and leaving somebody to find
/// the word that makes one. The first press does the first thing.
#[test]
fn signature_with_nothing_drawn_yet_opens_the_pad() {
    let (mut app, _path) = with_signature_pad("two-column.pdf", "opens-pad");
    app.submit("signature");

    assert!(app.pad.is_some(), "the pad did not open:\n{}", said(&app));
    assert!(
        app.pad.as_ref().is_some_and(|p| p.then_place),
        "it will not carry on to the click somebody wanted"
    );
    assert!(app.tab_mut().tool.is_none(), "it armed a click with nothing to place");
    assert!(said(&app).contains("this computer"), "{}", said(&app));
}

/// **What is drawn is kept, and placed where it is clicked.**
#[test]
fn a_drawn_signature_places_on_the_line_that_was_clicked() {
    let (mut app, path) = with_signature_pad("two-column.pdf", "places");
    let told = app.save_drawn_signature("mine", &scrawl()).expect("kept");
    assert!(told.contains("this computer"), "{told}");
    assert!(path.is_file(), "it was not written to {}", path.display());

    // Now the tool places rather than opening the pad again.
    app.submit("signature");
    assert!(app.pad.is_none(), "it opened the pad over a signature it already had");
    assert!(
        matches!(app.tab_mut().tool.as_ref().map(|t| &t.kind), Some(Tool::Signature)),
        "the tool was not armed:\n{}",
        said(&app)
    );

    let before = app.tab_mut().doc.as_ref().expect("open").session.annotations(0).expect("read").len();
    let told = app
        .place_signature(0, AppPoint { x: 100.0, y: 400.0 })
        .expect("placed");

    let marks = app.tab_mut().doc.as_ref().expect("open").session.annotations(0).expect("read");
    assert_eq!(marks.len(), before + 1, "nothing was added to the page");
    let ink = marks
        .iter()
        .rev()
        .find_map(|m| match &m.annotation {
            pdf_core::document::Annotation::Ink { strokes, .. } => Some(strokes.clone()),
            _ => None,
        })
        .expect("the signature is not ink on the page");
    assert_eq!(ink.len(), 2, "a stroke went missing");

    // It sits *on* the line, at the width a form expects.
    let bottom = ink.iter().flatten().map(|p| p.y).fold(f32::MIN, f32::max);
    let left = ink.iter().flatten().map(|p| p.x).fold(f32::MAX, f32::min);
    assert!((bottom - 400.0).abs() < 1.0, "it did not sit on the line: {bottom}");
    assert!((left - 100.0).abs() < 1.0, "it did not start where it was clicked: {left}");

    // **And the line says which kind of signature this is.**
    assert!(told.contains("does not prove"), "{told}");
    assert!(told.contains("certify"), "it did not name the tool that does: {told}");

    let _ = std::fs::remove_file(&path);
}

/// A small solid picture, as an uploaded signature file would decode to.
fn solid_rgba(width: u32, height: u32, rgb: [u8; 3]) -> Vec<u8> {
    let mut rgba = vec![0u8; (width * height * 4) as usize];
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
    }
    rgba
}

/// **An uploaded picture places the same way a drawn signature does** —
/// same pad-free path once one exists, same click-to-place, same
/// warning line — as a picture on the page rather than ink.
#[test]
fn an_uploaded_signature_places_as_a_picture_on_the_line_that_was_clicked() {
    let (mut app, path) = with_signature_pad("two-column.pdf", "uploaded");
    let told = app
        .save_uploaded_signature("mine", solid_rgba(8, 4, [30, 120, 210]), 8, 4)
        .expect("kept");
    assert!(told.contains("this computer"), "{told}");
    assert!(path.is_file(), "it was not written to {}", path.display());

    // The tool places rather than opening the pad, exactly as it does
    // for a drawn signature.
    app.submit("signature");
    assert!(app.pad.is_none(), "it opened the pad over a signature it already had");
    assert!(
        matches!(app.tab_mut().tool.as_ref().map(|t| &t.kind), Some(Tool::Signature)),
        "the tool was not armed:
{}",
        said(&app)
    );

    let before = app.tab_mut().doc.as_ref().expect("open").session.annotations(0).expect("read").len();
    let told = app
        .place_signature(0, AppPoint { x: 100.0, y: 400.0 })
        .expect("placed");

    let marks = app.tab_mut().doc.as_ref().expect("open").session.annotations(0).expect("read");
    assert_eq!(marks.len(), before + 1, "nothing was added to the page");
    let (rect, rgba) = marks
        .iter()
        .rev()
        .find_map(|m| match &m.annotation {
            pdf_core::document::Annotation::Image { rect, rgba, .. } => Some((*rect, rgba.clone())),
            _ => None,
        })
        .expect("the signature is not a picture on the page");
    assert_eq!(rgba, solid_rgba(8, 4, [30, 120, 210]), "the pixels do not match what was uploaded");

    // It sits *on* the line, at the width a form expects — the same
    // anchor a drawn signature uses.
    assert!((rect.bottom - 400.0).abs() < 1.0, "it did not sit on the line: {}", rect.bottom);
    assert!((rect.left - 100.0).abs() < 1.0, "it did not start where it was clicked: {}", rect.left);

    // **And the line says the same thing a drawn signature's does.**
    assert!(told.contains("does not prove"), "{told}");
    assert!(told.contains("certify"), "it did not name the tool that does: {told}");

    let _ = std::fs::remove_file(&path);
}

/// **Add Images, end to end**: a real file on disk, decoded and armed by
/// the `addimage` command, then placed centred on the click — unlike a
/// signature, a plain picture has no line to sit on.
#[test]
fn the_addimage_command_decodes_a_file_and_arms_placement() {
    let mut app = app("two-column.pdf");
    let tmp = std::env::temp_dir()
        .join(format!("pagify-test-addimage-{}.png", std::process::id()));
    image::RgbaImage::from_pixel(8, 4, image::Rgba([40, 90, 160, 255]))
        .save(&tmp)
        .expect("write a test picture");

    app.submit(&format!("addimage {}", tmp.display()));
    let _ = std::fs::remove_file(&tmp);
    let Some(Tool::PlaceImage { rgba, width, height }) =
        app.tab_mut().tool.as_ref().map(|t| &t.kind)
    else {
        panic!("the file was not decoded and armed: {}", said(&app));
    };
    assert_eq!((*width, *height), (8, 4));
    let (rgba, width, height) = (rgba.clone(), *width, *height);

    let before = app.tab_mut().doc.as_ref().expect("open").session.annotations(0).expect("read").len();
    // Populate the foreign-marks cache with a stale, pre-placement
    // snapshot — the exact state a person looking at the page already
    // put it in before they ever reached for Add Images.
    let _ = app.foreign_marks(0);
    app.place_image_at(0, AppPoint { x: 300.0, y: 400.0 }, rgba, width, height)
        .expect("placed");

    // **Reported from use: a freshly placed picture could not be
    // selected, moved or resized.** Its hit-test rectangle lived in a
    // cache keyed only on the page number, never invalidated by a new
    // annotation on the *current* page — so the picture was there, on
    // the document, and simply invisible to anything that went looking
    // for it until the page was left and returned to.
    assert!(app.tab_mut().doc.as_ref().unwrap().caches.foreign.is_none(), "the foreign-marks cache was not invalidated");

    let marks = app.tab_mut().doc.as_ref().expect("open").session.annotations(0).expect("read");
    assert_eq!(marks.len(), before + 1, "nothing was added to the page");
    let rect = marks
        .iter()
        .rev()
        .find_map(|m| match &m.annotation {
            pdf_core::document::Annotation::Image { rect, .. } => Some(*rect),
            _ => None,
        })
        .expect("the picture is not on the page");

    let (cx, cy) = ((rect.left + rect.right) / 2.0, (rect.top + rect.bottom) / 2.0);
    assert!((cx - 300.0).abs() < 1.0, "not centred on x: {cx}");
    assert!((cy - 400.0).abs() < 1.0, "not centred on y: {cy}");
    // 8x4 is 2:1 — the width this app places at, and half that for height.
    assert!(
        ((rect.right - rect.left) - 2.0 * (rect.bottom - rect.top)).abs() < 1.0,
        "the aspect ratio was not kept: {rect:?}"
    );
}

/// **A picture with real alpha is composited against the page it lands
/// on, not placed with its background untouched** — the whole point of
/// carrying alpha through from extraction at all. A pixel that was
/// fully opaque survives exactly; a pixel that was fully transparent
/// picks up whatever renders at that spot on the page instead of the
/// arbitrary colour it happened to be uploaded with.
#[test]
fn a_placed_picture_with_alpha_is_composited_against_the_page() {
    let (mut app, path) = with_signature_pad("two-column.pdf", "alpha-composite");
    let rgba = vec![
        10, 20, 30, 255, // opaque: must survive unchanged
        1, 2, 3, 0, // transparent, holding an obviously wrong colour: must not survive
    ];
    app.save_uploaded_signature("mine", rgba, 2, 1).expect("kept");
    app.submit("signature");

    app.place_signature(0, AppPoint { x: 100.0, y: 400.0 }).expect("placed");
    let marks = app.tab_mut().doc.as_ref().expect("open").session.annotations(0).expect("read");
    let placed = marks
        .iter()
        .rev()
        .find_map(|m| match &m.annotation {
            pdf_core::document::Annotation::Image { rgba, .. } => Some(rgba.clone()),
            _ => None,
        })
        .expect("the signature is not a picture on the page");

    assert_eq!(&placed[0..4], &[10, 20, 30, 255], "the opaque pixel should survive exactly");
    assert_ne!(
        &placed[4..7],
        &[1, 2, 3],
        "the transparent pixel should have been composited against the page, not left as uploaded"
    );
    assert_eq!(placed[7], 255, "a placed picture is always opaque");

    let _ = std::fs::remove_file(&path);
}

/// A placed-but-unapplied picture signature is found by
/// [`PagifyApp::signature_at`] — and only there, within its own rect —
/// the pick this session's move/resize is built on.
#[test]
fn a_placed_signature_is_found_at_its_own_rect_and_nowhere_else() {
    let (mut app, path) = with_signature_pad("two-column.pdf", "signature-at");
    app.save_uploaded_signature("mine", solid_rgba(4, 4, [40, 90, 200]), 4, 4).expect("kept");
    app.submit("signature");
    app.place_signature(0, AppPoint { x: 100.0, y: 400.0 }).expect("placed");

    let marks = app.tab_mut().doc.as_ref().expect("open").session.image_signature_marks(0).expect("marks");
    let mark = marks.first().expect("the signature is placed");
    let middle = AppPoint {
        x: ((mark.rect.left + mark.rect.right) / 2.0) as f64,
        y: ((mark.rect.top + mark.rect.bottom) / 2.0) as f64,
    };
    let found = app.signature_at(0, middle);
    assert_eq!(found, Some((mark.index, mark.rect, mark.rotation)), "not found at its own middle");

    let far_away = AppPoint { x: (mark.rect.right + 200.0) as f64, y: (mark.rect.bottom + 200.0) as f64 };
    assert_eq!(app.signature_at(0, far_away), None, "found somewhere it was never placed");

    let _ = std::fs::remove_file(&path);
}

/// **Dragging the body of a selected signature moves it** — the same
/// commit shape `dragging_a_handle_resizes_about_the_opposite_corner`
/// already exercises for the object tool, here through
/// `finish_signature_grab` instead of `finish_grab`, since a signature
/// is an annotation, never page content.
#[test]
fn dragging_a_selected_signature_moves_it() {
    let (mut app, path) = with_signature_pad("two-column.pdf", "signature-move");
    app.save_uploaded_signature("mine", solid_rgba(4, 4, [40, 90, 200]), 4, 4).expect("kept");
    app.submit("signature");
    app.place_signature(0, AppPoint { x: 100.0, y: 400.0 }).expect("placed");

    let mark = app.tab_mut().doc.as_ref().unwrap().session.image_signature_marks(0).unwrap().remove(0);
    let sel = SignatureSelected { page: 0, index: mark.index, rect: mark.rect, rotation: mark.rotation };
    let grab = Grab { handle: None, from: AppPoint { x: mark.rect.left as f64, y: mark.rect.top as f64 }, by: (30.0, -15.0) };
    app.finish_signature_grab(sel, grab);

    let moved = app.tab_mut().doc.as_ref().unwrap().session.image_signature_marks(0).unwrap().remove(0);
    assert!((moved.rect.left - (mark.rect.left + 30.0)).abs() < 0.5, "left did not move");
    assert!((moved.rect.top - (mark.rect.top - 15.0)).abs() < 0.5, "top did not move");
    let (w0, h0) = (mark.rect.right - mark.rect.left, mark.rect.bottom - mark.rect.top);
    let (w1, h1) = (moved.rect.right - moved.rect.left, moved.rect.bottom - moved.rect.top);
    assert!((w0 - w1).abs() < 0.5 && (h0 - h1).abs() < 0.5, "a move changed the size");
    assert_eq!(app.tab_mut().selection.signature_selected, Some(SignatureSelected { page: 0, index: moved.index, rect: moved.rect, rotation: moved.rotation }), "the selection did not follow the move");

    let _ = std::fs::remove_file(&path);
}

/// **Dragging a handle resizes about the opposite corner** — the
/// signature counterpart of the object tool's own handle test, again
/// committed through `finish_signature_grab`.
#[test]
fn dragging_a_signature_handle_resizes_about_the_opposite_corner() {
    let (mut app, path) = with_signature_pad("two-column.pdf", "signature-resize");
    app.save_uploaded_signature("mine", solid_rgba(4, 4, [40, 90, 200]), 4, 4).expect("kept");
    app.submit("signature");
    app.place_signature(0, AppPoint { x: 100.0, y: 400.0 }).expect("placed");

    let mark = app.tab_mut().doc.as_ref().unwrap().session.image_signature_marks(0).unwrap().remove(0);
    let (w, h) = (mark.rect.right - mark.rect.left, mark.rect.bottom - mark.rect.top);
    let sel = SignatureSelected { page: 0, index: mark.index, rect: mark.rect, rotation: mark.rotation };
    let grab = Grab {
        handle: Some(Handle::BottomRight),
        from: AppPoint { x: mark.rect.right as f64, y: mark.rect.bottom as f64 },
        by: (-w / 2.0, -h / 2.0),
    };
    app.finish_signature_grab(sel, grab);

    let resized = app.tab_mut().doc.as_ref().unwrap().session.image_signature_marks(0).unwrap().remove(0);
    assert!((resized.rect.left - mark.rect.left).abs() < 0.5 && (resized.rect.top - mark.rect.top).abs() < 0.5, "the anchored corner moved");
    assert!(((resized.rect.right - resized.rect.left) - w / 2.0).abs() < 1.0, "width did not halve");
    assert!(((resized.rect.bottom - resized.rect.top) - h / 2.0).abs() < 1.0, "height did not halve");

    let _ = std::fs::remove_file(&path);
}

/// **Reported from use: a placed picture could not be selected, moved
/// or resized.** The same three tests as the signature system above,
/// for [`PagifyApp::placed_image_at`] / [`PagifyApp::finish_placed_image_grab`]
/// — proving the parallel system this session added actually works, not
/// just that it compiles.
#[test]
fn a_placed_image_is_found_at_its_own_rect_and_nowhere_else() {
    let mut app = app("two-column.pdf");
    app.place_image_at(0, AppPoint { x: 100.0, y: 400.0 }, solid_rgba(4, 4, [40, 90, 200]), 4, 4)
        .expect("placed");

    let marks = app.tab_mut().doc.as_ref().expect("open").session.placed_image_marks(0).expect("marks");
    let mark = marks.first().expect("the picture is placed");
    let middle = AppPoint {
        x: ((mark.rect.left + mark.rect.right) / 2.0) as f64,
        y: ((mark.rect.top + mark.rect.bottom) / 2.0) as f64,
    };
    let found = app.placed_image_at(0, middle);
    assert_eq!(found, Some((mark.index, mark.rect, mark.rotation)), "not found at its own middle");

    let far_away = AppPoint { x: (mark.rect.right + 200.0) as f64, y: (mark.rect.bottom + 200.0) as f64 };
    assert_eq!(app.placed_image_at(0, far_away), None, "found somewhere it was never placed");
}

#[test]
fn dragging_a_selected_placed_image_moves_it() {
    let mut app = app("two-column.pdf");
    app.place_image_at(0, AppPoint { x: 100.0, y: 400.0 }, solid_rgba(4, 4, [40, 90, 200]), 4, 4)
        .expect("placed");

    let mark = app.tab_mut().doc.as_ref().unwrap().session.placed_image_marks(0).unwrap().remove(0);
    let sel = PlacedImageSelected { page: 0, index: mark.index, rect: mark.rect, rotation: mark.rotation };
    let grab = Grab {
        handle: None,
        from: AppPoint { x: mark.rect.left as f64, y: mark.rect.top as f64 },
        by: (30.0, -15.0),
    };
    app.finish_placed_image_grab(sel, grab);

    let moved = app.tab_mut().doc.as_ref().unwrap().session.placed_image_marks(0).unwrap().remove(0);
    assert!((moved.rect.left - (mark.rect.left + 30.0)).abs() < 0.5, "left did not move");
    assert!((moved.rect.top - (mark.rect.top - 15.0)).abs() < 0.5, "top did not move");
    let (w0, h0) = (mark.rect.right - mark.rect.left, mark.rect.bottom - mark.rect.top);
    let (w1, h1) = (moved.rect.right - moved.rect.left, moved.rect.bottom - moved.rect.top);
    assert!((w0 - w1).abs() < 0.5 && (h0 - h1).abs() < 0.5, "a move changed the size");
    assert_eq!(
        app.tab_mut().selection.placed_image_selected,
        Some(PlacedImageSelected { page: 0, index: moved.index, rect: moved.rect, rotation: moved.rotation }),
        "the selection did not follow the move"
    );
}

#[test]
fn dragging_a_placed_image_handle_resizes_about_the_opposite_corner() {
    let mut app = app("two-column.pdf");
    app.place_image_at(0, AppPoint { x: 100.0, y: 400.0 }, solid_rgba(4, 4, [40, 90, 200]), 4, 4)
        .expect("placed");

    let mark = app.tab_mut().doc.as_ref().unwrap().session.placed_image_marks(0).unwrap().remove(0);
    let (w, h) = (mark.rect.right - mark.rect.left, mark.rect.bottom - mark.rect.top);
    let sel = PlacedImageSelected { page: 0, index: mark.index, rect: mark.rect, rotation: mark.rotation };
    let grab = Grab {
        handle: Some(Handle::BottomRight),
        from: AppPoint { x: mark.rect.right as f64, y: mark.rect.bottom as f64 },
        by: (-w / 2.0, -h / 2.0),
    };
    app.finish_placed_image_grab(sel, grab);

    let resized = app.tab_mut().doc.as_ref().unwrap().session.placed_image_marks(0).unwrap().remove(0);
    assert!(
        (resized.rect.left - mark.rect.left).abs() < 0.5 && (resized.rect.top - mark.rect.top).abs() < 0.5,
        "the anchored corner moved"
    );
    assert!(((resized.rect.right - resized.rect.left) - w / 2.0).abs() < 1.0, "width did not halve");
    assert!(((resized.rect.bottom - resized.rect.top) - h / 2.0).abs() < 1.0, "height did not halve");
}

/// **The property the whole split exists for.** `apply_signatures`
/// burns in every `image_signature_marks` hit; if a plain placed
/// picture ever showed up there, applying signatures would flatten
/// someone's decorative image as though it had been signed with.
#[test]
fn placed_pictures_and_signatures_are_found_separately() {
    let (mut app, path) = with_signature_pad("two-column.pdf", "disjoint");
    app.save_uploaded_signature("mine", solid_rgba(4, 4, [40, 90, 200]), 4, 4).expect("kept");
    app.submit("signature");
    app.place_signature(0, AppPoint { x: 100.0, y: 400.0 }).expect("signature placed");
    app.place_image_at(0, AppPoint { x: 300.0, y: 500.0 }, solid_rgba(6, 6, [10, 200, 60]), 6, 6)
        .expect("picture placed");

    let signatures = app.tab_mut().doc.as_ref().unwrap().session.image_signature_marks(0).unwrap();
    let pictures = app.tab_mut().doc.as_ref().unwrap().session.placed_image_marks(0).unwrap();
    assert_eq!(signatures.len(), 1, "the signature should be found, and only once");
    assert_eq!(pictures.len(), 1, "the picture should be found, and only once");
    assert_ne!(signatures[0].index, pictures[0].index, "they must not resolve to the same annotation");

    let _ = std::fs::remove_file(&path);
}

/// **Reported from use, with screenshots: a real page's paragraph
/// opened for editing with a chunk of its own first line missing —
/// "Camino elitee-plus 3.0" jumping straight to a mid-word "ing
/// solution..." with a stray "h" in between.** The producer had split
/// that line across two runs ("Camino elitee-plus 3.0 " and "is a
/// powerful accent light", side by side on the same line) — a shape the
/// old geometric walk did not know how to include, since a
/// same-line candidate was treated as "not the next line" and simply
/// dropped rather than merged into the line it already was. Now it is
/// gathered into that line and concatenated in reading order, and a
/// row's several runs are edited as one line (see `EditingRun::lines`).
// `looks_rotated` — the actual decision `pick_text_run` refuses on —
// has its own dedicated tests in `looks_rotated_tests`, including the
// exact box dimensions a real rotated dimension label reported. There
// is no way to make `write_text_at` itself produce a rotated-looking
// rect to drive an end-to-end test through the public API with (it
// always lays glyphs out horizontally), so the integration was instead
// confirmed directly against the real page this was reported against:
// picking the rotated "54mm" label there returns
// `Err("that text is rotated on the page...")` rather than opening.

/// **Reported from use, on more than one paragraph, after a tool switch
/// left a run split into individual characters** (see
/// `taking_up_edit_object_puts_an_open_run_editor_down`, the fix for
/// how that happens): Edit Text opened a single letter instead of the
/// line it belonged to, permanently, since the old geometric walk refused
/// to glue lone characters back together on principle — a rule that
/// exists to protect a split that just happened, not one to trap a line
/// at one letter forever. Simulated directly with
/// `split_run_into_characters`, the same call Edit Object's own
/// character-drilling makes.
#[test]
fn edit_text_recovers_a_line_that_was_split_into_characters() {
    let mut app = app("text-lines.pdf");
    let object = app.tab_mut().doc.as_ref().unwrap().session.text_runs(0).expect("runs")[0].object;
    let original_text = app.tab_mut().doc.as_ref().unwrap().session.text_runs(0).expect("runs")[0]
        .text
        .clone();

    app.tab_mut().doc
        .as_mut()
        .unwrap()
        .session
        .split_run_into_characters(0, object)
        .expect("split");
    let split_runs = app.tab_mut().doc.as_ref().unwrap().session.text_runs(0).expect("runs");
    assert!(
        split_runs.len() > 1,
        "setup: splitting should have produced more than one run"
    );

    let one_char = split_runs.iter().find(|r| r.text.trim().chars().count() == 1).expect("a lone character");
    let at = AppPoint {
        x: ((one_char.rect.left + one_char.rect.right) / 2.0) as f64,
        y: ((one_char.rect.top + one_char.rect.bottom) / 2.0) as f64,
    };

    app.submit("edittext");
    app.pick_text_run(0, at).expect("a run was here");

    let edit = app.tab_mut().edit.editing_run.as_ref().expect("should have opened");
    assert_eq!(
        edit.buffer.trim(),
        original_text.trim(),
        "should have reassembled the whole line, not just the letter clicked"
    );
    assert!(
        edit.lines.iter().any(|(objects, _)| objects.len() > 1),
        "should have recorded the recovered line as many objects, not one: {:?}",
        edit.lines
    );
}

/// **Reported from use, with a screenshot**: retyping a field whose line
/// was actually several objects — "CCT:3500K," split the same way
/// `split_run_into_characters` splits one here — replaced only the
/// first object with the whole new text and left every other object on
/// that line drawing its own untouched original text right next to it,
/// so "CCT:3500K," retyped as "5000" came out "5000CT:3500K," (or
/// similar, depending which object happened to be first) instead of
/// cleanly replacing the line. Each further edit compounded more of the
/// same stale text onto the result. `apply_paragraph_edit` already
/// writes the new text into a line's first object and removes the rest —
/// this pins that it actually gets used for a single ROW with several
/// objects, not just for several rows.
#[test]
fn retyping_a_line_split_into_several_objects_does_not_leave_stale_text_behind() {
    let mut app = app("text-lines.pdf");
    let original_text = app.tab_mut().doc.as_ref().unwrap().session.text_runs(0).expect("runs")[0]
        .text
        .clone();
    let object = app.tab_mut().doc.as_ref().unwrap().session.text_runs(0).expect("runs")[0].object;

    app.tab_mut().doc
        .as_mut()
        .unwrap()
        .session
        .split_run_into_characters(0, object)
        .expect("split");
    let split_runs = app.tab_mut().doc.as_ref().unwrap().session.text_runs(0).expect("runs");
    let one_char = split_runs.iter().find(|r| r.text.trim().chars().count() == 1).expect("a lone character");
    let at = AppPoint {
        x: ((one_char.rect.left + one_char.rect.right) / 2.0) as f64,
        y: ((one_char.rect.top + one_char.rect.bottom) / 2.0) as f64,
    };

    app.submit("edittext");
    app.pick_text_run(0, at).expect("a run was here");
    assert!(
        app.tab_mut().edit.editing_run.as_ref().unwrap().lines.iter().any(|(objects, _)| objects.len() > 1),
        "setup: the recovered line must carry several objects, or this proves nothing"
    );

    app.tab_mut().edit.editing_run.as_mut().unwrap().buffer = "REPLACED".to_string();
    app.apply_editing_page();

    let page_text: String = app.tab_mut().doc.as_ref().unwrap().session.characters(0).map(|c| c.text().to_string()).unwrap_or_default();
    assert!(
        page_text.contains("REPLACED"),
        "the new text should be on the page: {page_text:?}"
    );
    assert!(
        original_text.trim().chars().count() < 2
            || !page_text.contains(original_text.trim()),
        "no stale fragment of the original line (\"{}\") should still be drawn on the page: {page_text:?}",
        original_text.trim()
    );
}

/// **Reported from use, with a screenshot comparison against another PDF
/// editor**: a spec sheet laid out as ruled-off "label: / value" rows —
/// "Power Input: / 40W", "Current Input: / 1050mA", each underlined —
/// opened for editing as one garbled multi-row paragraph instead of one
/// box per row, and applying an edit corrupted unrelated rows (a dropped
/// space, "40W" turning into "0"). The other editor drew exactly one box
/// per ruled row. Two rows with a close gap and the identical look
/// (which geometry and font alone would merge, confirmed by the first
/// assertion below) must stay apart when a ruled line — thin, wide,
/// sitting in the gap between them — separates them.
///
/// Run through the same two steps a click makes — the adapter and the
/// detector, [`block_input::build_page_blocks_from`] — on the same two
/// runs and the same rule this test always used; only the thing asked has
/// changed, from the old geometric walk to "which block is each run in".
#[test]
fn a_ruled_line_between_two_rows_stops_the_paragraph_there() {
    let label = pdf_core::document::TextRun {
        object: 0,
        text: "Power Input:".to_string(),
        rect: pdf_core::document::Rect { left: 50.0, right: 150.0, top: 100.0, bottom: 112.0 },
        origin: pdf_core::document::Point { x: 50.0, y: 110.0 },
        size: 10.0,
        color: pdf_core::document::Color { r: 0, g: 0, b: 0, a: 255 },
    };
    let value = pdf_core::document::TextRun {
        object: 1,
        text: "40W".to_string(),
        // A 4pt gap after a 12pt-tall line — a line pitch of 1.6 em, so
        // geometry alone calls this the next line of the same paragraph.
        rect: pdf_core::document::Rect { left: 50.0, right: 150.0, top: 116.0, bottom: 128.0 },
        origin: pdf_core::document::Point { x: 50.0, y: 126.0 },
        size: 10.0,
        color: pdf_core::document::Color { r: 0, g: 0, b: 0, a: 255 },
    };
    let runs = vec![label.clone(), value.clone()];
    // One font for both, as the producer of such a sheet gives every field.
    let style = pdf_core::document::RunStyle { font: 0, stem_milli_em: Some(74), axis: (1.0, 0.0) };
    let styles: HashMap<usize, pdf_core::document::RunStyle> = [(0, style), (1, style)].into_iter().collect();
    let face_names: HashMap<usize, String> =
        [(0, "Helvetica".to_string()), (1, "Helvetica".to_string())].into_iter().collect();
    let blocks_with = |shapes: Vec<pdf_core::document::DrawnObject>| {
        block_input::build_page_blocks_from(0, 0, 0, runs.clone(), styles.clone(), face_names.clone(), shapes)
    };
    let block_of = |page: &PageBlocks, object: usize| page.by_object.get(&object).map(|&(block, _)| block);

    let merged = blocks_with(Vec::new());
    assert!(block_of(&merged, 0).is_some() && block_of(&merged, 1).is_some(), "setup: both runs are in a block");
    assert_eq!(
        block_of(&merged, label.object),
        block_of(&merged, value.object),
        "setup: geometry and look alone must merge these two rows, or the rule below proves nothing"
    );

    let ruled_line = pdf_core::document::DrawnObject {
        object: 2,
        kind: pdf_core::document::DrawnKind::Shape,
        // Thin (1pt) and wide (120pt) — a producer's dividing rule, not a
        // text run — sitting in the 4pt gap between the two rows.
        rect: pdf_core::document::Rect { left: 40.0, right: 160.0, top: 113.5, bottom: 114.5 },
        label: "rule".to_string(),
        depth: 0,
        opacity: 1.0,
        movable: true,
    };
    let kept_apart = blocks_with(vec![ruled_line]);
    assert_ne!(
        block_of(&kept_apart, label.object),
        block_of(&kept_apart, value.object),
        "a ruled line between the two rows should have stopped the paragraph there"
    );
}

/// **Two blocks the automatic heuristic keeps apart on purpose** —
/// picked as far apart on the page as `two-column.pdf` has runs, which
/// the detector refuses to bridge — can still be
/// declared one paragraph by hand, and the grouping persists: closing
/// the editor and clicking either run again re-opens the same joined
/// pair, not just the one under the pointer.
#[test]
fn joining_two_distant_runs_opens_them_as_one_paragraph_and_persists() {
    let mut app = app("two-column.pdf");
    let runs = app.tab_mut().doc.as_ref().unwrap().session.text_runs(0).expect("runs");
    assert!(runs.len() >= 2, "setup: need at least two runs");
    let a = runs.iter().min_by(|x, y| x.rect.top.total_cmp(&y.rect.top)).unwrap().clone();
    let b = runs.iter().max_by(|x, y| x.rect.top.total_cmp(&y.rect.top)).unwrap().clone();
    assert_ne!(a.object, b.object, "setup: need two distinct runs to prove a join did something");

    let (detected, _) = app.page_blocks(0).expect("the page's blocks");
    let block_of = |object: usize| detected.by_object.get(&object).map(|&(block, _)| block);
    assert!(
        block_of(a.object).is_some() && block_of(b.object).is_some(),
        "setup: both runs must be in a block, or the comparison below proves nothing"
    );
    assert_ne!(
        block_of(a.object),
        block_of(b.object),
        "setup: these two must not already share a paragraph automatically, or joining them by hand proves nothing"
    );

    let total_chars = app.characters(0).expect("characters").len();
    app.tab_mut().selection.text_selection = Some(0..total_chars);
    app.tab_mut().organize.selection_page = 0;

    let message = app.join_selected_text().expect("join should succeed");
    assert!(message.contains("paragraph"), "should have opened the paragraph editor: {message}");

    let edit = app.tab_mut().edit.editing_run.as_ref().expect("should have opened an editor");
    assert!(edit.buffer.contains(a.text.trim()), "joined text should include the topmost run");
    assert!(edit.buffer.contains(b.text.trim()), "joined text should include the bottommost run");

    // Persists: closing the editor and clicking the *other* run reopens
    // the same joined group, not just the run under the pointer.
    app.tab_mut().edit.editing_run = None;
    let at_b = AppPoint {
        x: ((b.rect.left + b.rect.right) / 2.0) as f64,
        y: ((b.rect.top + b.rect.bottom) / 2.0) as f64,
    };
    app.pick_text_run(0, at_b).expect("b should still be there");
    let reopened = app.tab_mut().edit.editing_run.as_ref().expect("should have reopened");
    assert!(
        reopened.lines.iter().flat_map(|(objects, _)| objects).any(|o| *o == a.object),
        "reopening on b should have brought a's run back in too"
    );
}

/// Splitting forgets the grouping and nothing else — the page is
/// untouched, and a later click on either run edits it alone again.
#[test]
fn splitting_a_joined_pair_edits_them_separately_again() {
    let mut app = app("two-column.pdf");
    let runs = app.tab_mut().doc.as_ref().unwrap().session.text_runs(0).expect("runs");
    let a = runs.iter().min_by(|x, y| x.rect.top.total_cmp(&y.rect.top)).unwrap().clone();
    let b = runs.iter().max_by(|x, y| x.rect.top.total_cmp(&y.rect.top)).unwrap().clone();

    let total_chars = app.characters(0).expect("characters").len();
    app.tab_mut().selection.text_selection = Some(0..total_chars);
    app.tab_mut().organize.selection_page = 0;
    app.join_selected_text().expect("join should succeed");
    assert!(app.group_containing(0, b.object).is_some(), "setup: should be joined");

    assert!(app.split_group(0, b.object), "split should find the group");
    assert!(app.group_containing(0, a.object).is_none(), "the group should be gone for both runs");
    assert!(app.group_containing(0, b.object).is_none());

    app.tab_mut().edit.editing_run = None;
    let at_b = AppPoint {
        x: ((b.rect.left + b.rect.right) / 2.0) as f64,
        y: ((b.rect.top + b.rect.bottom) / 2.0) as f64,
    };
    app.pick_text_run(0, at_b).expect("b should still be there");
    let reopened = app.tab_mut().edit.editing_run.as_ref().expect("should have reopened");
    assert!(
        !reopened.lines.iter().flat_map(|(objects, _)| objects).any(|o| *o == a.object),
        "after splitting, editing b should not bring a's run back in"
    );
}

/// A selection covering only one run's own words has nothing to join.
#[test]
fn joining_a_single_run_selection_is_refused() {
    let mut app = app("two-column.pdf");
    let runs = app.tab_mut().doc.as_ref().unwrap().session.text_runs(0).expect("runs");
    let a = &runs[0];
    let centre = ((a.rect.left + a.rect.right) / 2.0, (a.rect.top + a.rect.bottom) / 2.0);
    let range = app
        .characters(0)
        .and_then(|c| c.range_between((centre.0, centre.1), (centre.0, centre.1)))
        .expect("a point inside a run's own rect should hit something");
    app.tab_mut().selection.text_selection = Some(range);
    app.tab_mut().organize.selection_page = 0;
    assert!(app.join_selected_text().is_err(), "one run alone is nothing to join");
}

/// The sample's size and colour carry over to a target run whose own
/// size and colour genuinely differ — picked as two separate
/// selections, one after the other, the way the ribbon tool actually
/// works.
#[test]
fn matching_properties_copies_size_and_colour_to_the_target() {
    let mut app = app("two-column.pdf");
    let runs = app.tab_mut().doc.as_ref().unwrap().session.text_runs(0).expect("runs");
    let a = runs.iter().min_by(|x, y| x.rect.top.total_cmp(&y.rect.top)).unwrap().clone();
    let b = runs.iter().max_by(|x, y| x.rect.top.total_cmp(&y.rect.top)).unwrap().clone();
    assert_ne!(a.object, b.object, "setup: need two distinct runs");

    // Force a real mismatch first, so matching them proves something.
    let doc = app.tab_mut().doc.as_ref().unwrap();
    doc.session
        .execute(pdf_core::command::Command::SetTextRun {
            page_index: 0,
            object: b.object,
            text: b.text.clone(),
            style: pdf_core::document::TextStyle { size: Some(a.size + 6.0), ..Default::default() },
        })
        .expect("setup: resize b");
    doc.session
        .execute(pdf_core::command::Command::SetTextRun {
            page_index: 0,
            object: b.object,
            text: b.text.clone(),
            style: pdf_core::document::TextStyle {
                color: Some(pdf_core::document::Color { r: 200, g: 10, b: 10, a: 255 }),
                ..Default::default()
            },
        })
        .expect("setup: recolour b");

    let centre_a = ((a.rect.left + a.rect.right) / 2.0, (a.rect.top + a.rect.bottom) / 2.0);
    let centre_b = ((b.rect.left + b.rect.right) / 2.0, (b.rect.top + b.rect.bottom) / 2.0);

    // The sample: a selection over `a` alone.
    let sample_range = app
        .characters(0)
        .and_then(|c| c.range_between(centre_a, centre_a))
        .expect("a point inside a's own rect");
    app.tab_mut().selection.text_selection = Some(sample_range);
    app.tab_mut().organize.selection_page = 0;
    app.match_properties_sample_from_current_selection().expect("sample should be accepted");
    assert!(
        matches!(app.tab_mut().tool.as_ref().map(|t| &t.kind), Some(Tool::MatchProperties { sample: Some(_) })),
        "the sample should be held"
    );

    // The target: a separate selection over `b` alone.
    let target_range = app
        .characters(0)
        .and_then(|c| c.range_between(centre_b, centre_b))
        .expect("a point inside b's own rect");
    app.tab_mut().selection.text_selection = Some(target_range);
    app.tab_mut().organize.selection_page = 0;
    let message = app.apply_match_properties_to_current_selection().expect("match should succeed");
    assert!(message.contains("matched"), "unexpected message: {message}");
    assert!(
        matches!(app.tab_mut().tool.as_ref().map(|t| &t.kind), Some(Tool::MatchProperties { sample: Some(_) })),
        "the tool should stay in hand"
    );

    let after = app.tab_mut().doc.as_ref().unwrap().session.text_runs(0).expect("runs");
    let b_after = after.iter().find(|r| r.object == b.object).expect("b should still be there");
    assert!(
        (b_after.size - a.size).abs() < 0.01,
        "the target's size should now match the sample's: {} vs {}",
        b_after.size,
        a.size
    );
    assert_eq!(
        (b_after.color.r, b_after.color.g, b_after.color.b),
        (a.color.r, a.color.g, a.color.b),
        "the target's colour should now match the sample's"
    );
}

/// An empty selection has nothing to serve as a sample, or to change.
#[test]
fn an_empty_selection_is_refused_by_both_match_properties_steps() {
    let mut app = app("two-column.pdf");
    app.tab_mut().selection.text_selection = Some(0..0);
    app.tab_mut().organize.selection_page = 0;
    assert!(
        app.match_properties_sample_from_current_selection().is_err(),
        "an empty selection is nothing to copy from"
    );

    let runs = app.tab_mut().doc.as_ref().unwrap().session.text_runs(0).expect("runs");
    let a = &runs[0];
    let sample = app.build_match_properties_sample(0, a);
    app.tab_mut().tool = Some(ArmedTool {
        kind: Tool::MatchProperties { sample: Some(sample) },
        page: 0,
        objects: Vec::new(),
        points: Vec::new(),
    });
    app.tab_mut().selection.text_selection = Some(0..0);
    app.tab_mut().organize.selection_page = 0;
    assert!(
        app.apply_match_properties_to_current_selection().is_err(),
        "an empty selection is nothing to change"
    );
}

/// **Reported from use: the app froze on right-click.** The context
/// menu used to recompute `joinable`/`split_object` itself, inline —
/// each a full-page `text_runs` read — and egui redraws an open popup's
/// contents every frame, so a real few-hundred-run document paid that
/// cost dozens of times a second for as long as the menu stayed open.
/// `compute_right_click_text_actions` is the fix's whole shape: called
/// once, at the click, with its result cached in
/// `right_click_text_actions` for the menu to only ever *read*. This
/// exercises exactly the entry point the click handler calls, on a
/// selection genuinely spanning two joinable runs.
#[test]
fn right_click_text_actions_are_computed_correctly_from_one_call() {
    let mut app = app("two-column.pdf");
    let runs = app.tab_mut().doc.as_ref().unwrap().session.text_runs(0).expect("runs");
    let a = runs.iter().min_by(|x, y| x.rect.top.total_cmp(&y.rect.top)).unwrap().clone();
    let b = runs.iter().max_by(|x, y| x.rect.top.total_cmp(&y.rect.top)).unwrap().clone();
    assert_ne!(a.object, b.object, "setup: need two distinct runs");

    let centre_a = ((a.rect.left + a.rect.right) / 2.0, (a.rect.top + a.rect.bottom) / 2.0);
    let centre_b = ((b.rect.left + b.rect.right) / 2.0, (b.rect.top + b.rect.bottom) / 2.0);
    let range = app
        .characters(0)
        .and_then(|c| c.range_between(centre_a, centre_b))
        .expect("a range covering both runs");
    app.tab_mut().selection.text_selection = Some(range);
    app.tab_mut().organize.selection_page = 0;

    let at = AppPoint { x: centre_a.0 as f64, y: centre_a.1 as f64 };
    let actions = app.compute_right_click_text_actions(0, at);
    assert!(actions.joinable, "two distinct runs in the selection should be joinable: {actions:?}");
    assert_eq!(actions.split_object, None, "neither run belongs to any joined group yet");

    // Join, then ask again pointing at one of the now-joined runs:
    // `split_object` should name it.
    app.join_selected_text().expect("join should succeed");
    let at_b = AppPoint { x: centre_b.0 as f64, y: centre_b.1 as f64 };
    let after_join = app.compute_right_click_text_actions(0, at_b);
    assert_eq!(
        after_join.split_object,
        Some(b.object),
        "pointing at a joined run should offer to split it: {after_join:?}"
    );
}

/// **Reported from use, general this time rather than tied to one
/// paragraph: "you are only fixing the paragraphs i am showing you...
/// description is written in [bold] but when i try to edit it becomes
/// regular."** `majority_look` used to be the only defence against a
/// heading merging into the body under it — whichever style had more
/// *lines* voting for it won, and the minority style, heading or body,
/// was simply overwritten in the editor, on any document where one
/// exists, not just the one in the screenshot. The real fix is upstream
/// of any vote: the detector (`pagify_shell::blocks`) now stops growing a
/// paragraph the moment a candidate line's own style stops matching,
/// so a heading and the body under it become two separate edits, each
/// opening in its own real style, instead of one edit where a vote
/// decides which style to discard.
#[test]
fn a_heading_does_not_merge_into_the_body_beneath_it() {
    use pdf_core::command::Command;
    use pdf_core::document::{Annotation, Color, Glyph};

    let mut app = app("text-lines.pdf");
    let write_sized = |app: &mut PagifyApp, id: i32, at: AppPoint, size: f32, text: &str| {
        let doc = app.tab_mut().doc.as_ref().expect("open");
        doc.session
            .execute(Command::AddAnnotation {
                page_index: 0,
                annotation: Annotation::Text {
                    text: text.to_string(),
                    font: "Helvetica".into(),
                    font_asset: None,
                    size,
                    color: Color { r: 20, g: 20, b: 20, a: 255 },
                    glyphs: vec![Glyph {
                        ch: text.to_string(),
                        id: 0,
                        x: at.x as f32,
                        y: at.y as f32,
                        radians: 0.0,
                    }],
                    id,
                    restore: String::new(),
                    frame: Vec::new(),
                    frame_width: 0.0,
                },
            })
            .expect("written");
    };

    // A bold-sized heading directly above an ordinary-sized paragraph —
    // close enough, vertically, that a purely geometric merge would
    // have pulled them together exactly as reported.
    write_sized(&mut app, 9001, AppPoint { x: 100.0, y: 500.0 }, 20.0, "Description:");
    write_sized(
        &mut app,
        9002,
        AppPoint { x: 100.0, y: 522.0 },
        12.0,
        "This is the ordinary body text underneath it.",
    );

    let runs = app.tab_mut().doc.as_ref().expect("open").session.text_runs(0).expect("runs");
    let heading = runs.iter().find(|r| r.text.contains("Description")).expect("heading");
    let body = runs.iter().find(|r| r.text.contains("ordinary body")).expect("body");
    assert!(
        (heading.rect.bottom - body.rect.top).abs() < 20.0,
        "the two lines need to start out close enough that only style keeps them apart: {:?} / {:?}",
        heading.rect,
        body.rect
    );

    app.submit("edittext");
    let at = AppPoint {
        x: ((heading.rect.left + heading.rect.right) / 2.0) as f64,
        y: ((heading.rect.top + heading.rect.bottom) / 2.0) as f64,
    };
    app.pick_text_run(0, at).expect("the heading was here");
    let edit = app.tab_mut().edit.editing_run.as_ref().expect("should have opened");
    assert!(
        !edit.buffer.contains("ordinary body"),
        "the heading's own edit swallowed the body beneath it: {:?}",
        edit.buffer
    );

    let at = AppPoint {
        x: ((body.rect.left + body.rect.right) / 2.0) as f64,
        y: ((body.rect.top + body.rect.bottom) / 2.0) as f64,
    };
    app.pick_text_run(0, at).expect("the body was here");
    let edit = app.tab_mut().edit.editing_run.as_ref().expect("should have opened");
    assert!(
        !edit.buffer.contains("Description"),
        "the body's own edit reached up and swallowed the heading above it: {:?}",
        edit.buffer
    );
}

/// **"see how the hyphens go missing?" — and the other half: where the
/// page draws none, the box invents none.** End to end, through
/// `pick_paragraph` rather than `join_paragraph_lines` in isolation. Two
/// ordinary lines, one word cut across them — the shape a producer's own
/// wrap-hyphen leaves when it draws that hyphen as a mark rather than a
/// character, and *also* the shape every wrap of a producer that never
/// writes a trailing space leaves.
///
/// **Changed from the first version of this test**, which asserted
/// `"heat dis-\nsipation"` on exactly these two lines: it expected the
/// hyphen from the shape of the break alone, and these lines carry no mark
/// at all — the page drew no hyphen there, so the buffer must not hold
/// one (applying would type it into the page). The positive case — a mark
/// drawn after the line, hyphen restored — is
/// `wrap_hyphen_tests::opening_a_paragraph_restores_a_hyphen_the_page_draws`,
/// on a page that really has one.
#[test]
fn opening_a_paragraph_invents_no_wrap_hyphen_the_page_never_drew() {
    let mut app = app("text-lines.pdf");
    app.write_text_at(0, AppPoint { x: 100.0, y: 500.0 }, "a powerful accent heat dis")
        .expect("first line written");
    app.write_text_at(0, AppPoint { x: 100.0, y: 516.0 }, "sipation and high CRI")
        .expect("second line written");

    let seed = app.tab_mut()
        .doc
        .as_ref()
        .expect("open")
        .session
        .text_runs(0)
        .expect("runs")
        .into_iter()
        .find(|r| r.text.contains("heat dis"))
        .expect("seed");
    let at = AppPoint {
        x: ((seed.rect.left + seed.rect.right) / 2.0) as f64,
        y: ((seed.rect.top + seed.rect.bottom) / 2.0) as f64,
    };

    app.submit("edittext");
    app.pick_text_run(0, at).expect("a run was here");

    let edit = app.tab_mut().edit.editing_run.as_ref().expect("should have opened");
    assert!(
        edit.buffer.contains("heat dis\nsipation"),
        "the two lines should join exactly as the page has them, no hyphen between: {:?}",
        edit.buffer
    );
}

/// **"enter for new line doesnt work... when a new line is typed it
/// should be in the same font, size, color, etc as the text in the text
/// box."** A single run's own editor used to be a `singleline` field —
/// Enter submitted it rather than adding to it, and there was nowhere
/// for a second line to go. Now it can grow one, written in the run's
/// own appearance rather than `write_text_at`'s flat default.
///
/// **A shaped write is one object per glyph, same as any other embedded-
/// font write** — see `write_styled_line_at`'s two branches — so the new
/// line's own text is read by joining every run below the first line
/// left to right, not by looking for one run that already says the
/// whole thing.
#[test]
fn a_single_run_can_grow_a_second_line_in_its_own_style() {
    let mut app = app("text-lines.pdf");
    let seed = app.tab_mut().doc.as_ref().unwrap().session.text_runs(0).expect("runs")[0].clone();
    let at = AppPoint {
        x: ((seed.rect.left + seed.rect.right) / 2.0) as f64,
        y: ((seed.rect.top + seed.rect.bottom) / 2.0) as f64,
    };
    app.submit("edittext");
    app.pick_text_run(0, at).expect("picked");

    {
        let edit = app.tab_mut().edit.editing_run.as_mut().expect("editing");
        let original = edit.buffer.clone();
        edit.buffer = format!("{original}\nSecond line");
    }
    app.apply_editing_page();

    let runs = app.tab_mut().doc.as_ref().unwrap().session.text_runs(0).expect("runs");
    assert!(
        runs.iter().any(|r| r.text.trim() == seed.text.trim()),
        "the first line should still read what it always did: {runs:?}"
    );

    // The nearest line below the seed's own — not every later line on the
    // page, which for this pangram fixture includes two more of its own.
    let after_seed: Vec<_> = runs.iter().filter(|r| r.rect.top > seed.rect.bottom).collect();
    let nearest_top = after_seed
        .iter()
        .map(|r| r.rect.top)
        .fold(f32::INFINITY, f32::min);
    let mut below: Vec<_> = after_seed
        .into_iter()
        .filter(|r| (r.rect.top - nearest_top).abs() < 5.0)
        .collect();
    below.sort_by(|a, b| a.rect.left.total_cmp(&b.rect.left));
    let joined: String = below.iter().map(|r| r.text.as_str()).collect();
    assert!(
        joined.contains("Second line"),
        "the second line was not written at all: {runs:?}"
    );
    let second = *below.first().expect("checked: joined is not empty");

    assert!(
        (second.size - seed.size).abs() < 0.5,
        "the new line's size did not match the run it grew from: {} vs {}",
        second.size,
        seed.size
    );
    assert_eq!(
        (second.color.r, second.color.g, second.color.b),
        (seed.color.r, seed.color.g, seed.color.b),
        "the new line's colour did not match the run it grew from"
    );
    // **The actual regression.** A face the app has no registration for
    // — every ordinary embedded document font — used to fall silently
    // back to plain, unembedded Helvetica for a grown line, regardless
    // of how bold or unusual the run it grew from looked. An embedded
    // font is the one thing that fallback can never produce.
    assert!(
        app.tab_mut().doc
            .as_ref()
            .unwrap()
            .session
            .run_font_is_embedded(0, second.object)
            .unwrap_or(false),
        "the new line fell back to an unembedded standard font instead \
         of the run's own"
    );
}

#[test]
fn a_line_the_producer_split_across_two_runs_is_not_missing_a_chunk() {
    let mut app = app("text-lines.pdf");

    // Two runs on the same baseline, side by side with an ordinary
    // word-sized gap between them — exactly the shape a producer's own
    // mid-line font or kerning change leaves behind, and exactly what
    // the real page this was reported against turned out to have.
    app.write_text_at(0, AppPoint { x: 100.0, y: 500.0 }, "Camino elitee-plus 3.0 ")
        .expect("first chunk written");
    let first = app.tab_mut()
        .doc
        .as_ref()
        .expect("open")
        .session
        .text_runs(0)
        .expect("runs")
        .into_iter()
        .find(|r| r.text.contains("Camino elitee"))
        .expect("first chunk");
    app.write_text_at(0, AppPoint { x: first.rect.right as f64 + 2.0, y: 500.0 }, "is a powerful accent light")
        .expect("second chunk written");

    let seed = app.tab_mut()
        .doc
        .as_ref()
        .expect("open")
        .session
        .text_runs(0)
        .expect("runs")
        .into_iter()
        .find(|r| r.text.contains("Camino elitee"))
        .expect("seed");
    let at = AppPoint {
        x: ((seed.rect.left + seed.rect.right) / 2.0) as f64,
        y: ((seed.rect.top + seed.rect.bottom) / 2.0) as f64,
    };

    app.submit("edittext");
    app.pick_text_run(0, at).expect("a run was here");

    let edit = app.tab_mut().edit.editing_run.as_ref().expect("should have opened");
    assert!(
        edit.buffer.contains("Camino elitee-plus 3.0 is a powerful accent light"),
        "the split line's second run is missing from the reconstructed text: {:?}",
        edit.buffer
    );
    assert!(
        edit.lines.iter().any(|(objects, _)| objects.len() > 1),
        "the split line should have been recorded as one line of two objects: {:?}",
        edit.lines
    );
}

/// **Reported from use, with a screenshot: Edit Text and Edit Object
/// both showed as active on the ribbon at once.** `take_up_object_tool`
/// already clears `self.tab_mut().tool` when Edit Object is picked up; `arm_tool`
/// (what Edit Text and every other picked-then-clicked tool goes
/// through) did not clear `self.tab_mut().tool_state.object_tool` back — so using Edit
/// Object and then Edit Text left both armed, and since
/// `self.tab_mut().tool_state.object_tool.is_some()` is checked first and takes the pointer
/// outright, every click after that went to Edit Object's own
/// character-drilling selection instead of the paragraph pick Edit Text
/// was meant to make.
#[test]
fn arming_edit_text_after_edit_object_puts_the_object_tool_down() {
    let mut app = app("two-column.pdf");
    app.submit("editobject");
    assert!(app.tab_mut().tool_state.object_tool.is_some(), "editobject should have armed the object tool");

    app.submit("edittext");

    assert!(app.tab_mut().tool_state.object_tool.is_none(), "arming Edit Text should have put the object tool down");
    assert!(
        app.tab_mut().tool.is_some(),
        "Edit Text itself should still have armed its own click-to-pick"
    );
}

/// The same fix, checked through every tool `arm_tool` is the entry point
/// for, not just Edit Text — a stale object tool would have silently
/// swallowed clicks meant for any of these exactly the same way.
#[test]
fn arming_any_pending_tool_after_edit_object_puts_it_down() {
    for command in ["edittext", "line", "circle", "redact"] {
        let mut app = app("two-column.pdf");
        app.submit("editobject");
        assert!(app.tab_mut().tool_state.object_tool.is_some(), "{command}: editobject should have armed the object tool");

        app.submit(command);

        assert!(
            app.tab_mut().tool_state.object_tool.is_none(),
            "{command}: arming it should have put the object tool down"
        );
        assert!(app.tab_mut().tool.is_some(), "{command}: should itself be armed");
    }
}

/// **Reported from use: a run picked with Edit Text, still open,
/// got split into individual characters the moment Edit Object was
/// clicked without an Escape in between.** `take_up_object_tool` cleared
/// `self.tab_mut().tool` and the object-tool's own selection state, but never
/// `self.tab_mut().edit.editing_run` — so the run Edit Text still thought it was
/// editing sat there, orphaned, while Edit Object's own click handler
/// went on to split whatever the next click landed on into individual
/// characters, with nothing to say a different tool had already claimed
/// that exact run.
/// **Not `app.submit("editobject")`** — `submit` is the command box's
/// own entry point, and while a run editor is open it intercepts every
/// line as a replacement for the run being edited rather than as a
/// command (see `PagifyApp::submit`'s own doc). A ribbon button calls
/// `take_up_object_tool` directly, which is exactly what a click on it
/// does and what this calls too.
#[test]
fn taking_up_edit_object_puts_an_open_run_editor_down() {
    let mut app = app("two-column.pdf");
    app.submit("edittext");
    let word = {
        let runs = app.tab_mut().doc.as_ref().unwrap().session.text_runs(0).expect("runs");
        runs[0].clone()
    };
    let at = AppPoint {
        x: ((word.rect.left + word.rect.right) / 2.0) as f64,
        y: ((word.rect.top + word.rect.bottom) / 2.0) as f64,
    };
    app.pick_text_run(0, at).expect("a run was here");
    assert!(app.tab_mut().edit.editing_run.is_some(), "setup: the run editor should be open");

    app.take_up_object_tool(true, 0);

    assert!(
        app.tab_mut().edit.editing_run.is_none(),
        "taking up Edit Object should have put the open run editor down"
    );
    assert!(app.tab_mut().tool_state.object_tool.is_some(), "Edit Object itself should still be armed");
}

/// The same fix, for arming a `pending`-based tool over an open run
/// editor rather than taking up Edit Object — see the sibling test's
/// own doc for why this calls `arm` directly rather than through
/// `submit`.
#[test]
fn arming_a_pending_tool_puts_an_open_run_editor_down() {
    let mut app = app("two-column.pdf");
    app.submit("edittext");
    let word = {
        let runs = app.tab_mut().doc.as_ref().unwrap().session.text_runs(0).expect("runs");
        runs[0].clone()
    };
    let at = AppPoint {
        x: ((word.rect.left + word.rect.right) / 2.0) as f64,
        y: ((word.rect.top + word.rect.bottom) / 2.0) as f64,
    };
    app.pick_text_run(0, at).expect("a run was here");
    assert!(app.tab_mut().edit.editing_run.is_some(), "setup: the run editor should be open");

    app.arm_tool(Tool::Draw(DrawKind::Line), 0);

    assert!(
        app.tab_mut().edit.editing_run.is_none(),
        "arming a different tool should have put the open run editor down"
    );
    assert!(app.tab_mut().tool.is_some(), "the newly armed tool should itself be armed");
}

/// **Reported from use: a selected drawn or inserted object could not
/// be deleted.** Delete already reached the object tool's own selection
/// and the markup layer, but a placed picture and a placed signature —
/// both a bare annotation picked with no tool armed — had never been
/// wired in at all.
#[test]
fn deleting_a_selected_placed_picture_removes_it() {
    let mut app = app("two-column.pdf");
    app.place_image_at(0, AppPoint { x: 100.0, y: 400.0 }, solid_rgba(4, 4, [40, 90, 200]), 4, 4)
        .expect("placed");
    let mark = app.tab_mut().doc.as_ref().unwrap().session.placed_image_marks(0).unwrap().remove(0);
    app.tab_mut().selection.placed_image_selected =
        Some(PlacedImageSelected { page: 0, index: mark.index, rect: mark.rect, rotation: mark.rotation });

    app.delete_selection();

    let remaining = app.tab_mut().doc.as_ref().unwrap().session.placed_image_marks(0).unwrap();
    assert!(remaining.is_empty(), "the picture should be gone");
    assert!(app.tab_mut().selection.placed_image_selected.is_none(), "the selection should have cleared with it");
}

#[test]
fn deleting_a_selected_signature_removes_it() {
    let (mut app, path) = with_signature_pad("two-column.pdf", "delete-signature");
    app.save_uploaded_signature("mine", solid_rgba(4, 4, [40, 90, 200]), 4, 4).expect("kept");
    app.submit("signature");
    app.place_signature(0, AppPoint { x: 100.0, y: 400.0 }).expect("placed");
    let mark = app.tab_mut().doc.as_ref().unwrap().session.image_signature_marks(0).unwrap().remove(0);
    app.tab_mut().selection.signature_selected =
        Some(SignatureSelected { page: 0, index: mark.index, rect: mark.rect, rotation: mark.rotation });

    app.delete_selection();

    let remaining = app.tab_mut().doc.as_ref().unwrap().session.image_signature_marks(0).unwrap();
    assert!(remaining.is_empty(), "the signature should be gone");
    assert!(app.tab_mut().selection.signature_selected.is_none(), "the selection should have cleared with it");

    let _ = std::fs::remove_file(&path);
}

/// `copy` then `paste` on a drawn shape — the same "select a shape,
/// duplicate it" a reader asked for alongside movability.
#[test]
fn copying_and_pasting_a_selected_shape_makes_a_second_one() {
    let mut app = app("pictures.pdf");
    app.submit("circle");
    app.submit("pick 300,300");
    app.submit("pick 340,300");
    app.submit("all");
    assert_eq!(app.tab_mut().markup.existing(0).unwrap().len(), 1, "the circle should be the only object so far");

    assert!(app.copy_object_selection(), "the selected circle should have been copied");
    app.paste_object_selection();

    let layer = app.tab_mut().markup.existing(0).expect("the layer exists");
    assert_eq!(layer.len(), 2, "paste should have added a second shape");
    let original = layer.objects()[0].geom.bbox();
    let pasted = layer.objects()[1].geom.bbox();
    assert_ne!(original, pasted, "the paste landed exactly on the original instead of beside it");
    assert_eq!(layer.selection().len(), 1, "the pasted copy should end up selected, not the original");
}

/// **The fill this session added must survive a copy.** A Hatch is not
/// independently selectable (see `Layer::hit`), so a click-selected
/// shape's copy never carried one — this proves the fill still comes
/// along through `is_filled`/`set_filled` rather than being silently
/// dropped.
#[test]
fn copying_and_pasting_preserves_a_filled_shapes_fill() {
    let mut app = app("pictures.pdf");
    app.submit("circle");
    app.submit("pick 300,300");
    app.submit("pick 340,300");
    app.submit("all");
    app.tab_mut().markup.existing_mut(0).unwrap().set_filled(0, true);

    assert!(app.copy_object_selection());
    app.paste_object_selection();

    let layer = app.tab_mut().markup.existing(0).expect("the layer exists");
    // The original circle and its Hatch, plus a pasted circle and the
    // *new* Hatch `set_filled` pairs with it — not a Hatch smuggled
    // straight out of the clipboard onto the original's own handle.
    assert_eq!(layer.len(), 4, "expected original+Hatch and pasted+Hatch");
    assert!(layer.is_filled(2), "the pasted copy should be filled too");
}

/// **Requested alongside splines: "when a line is drawn in properties
/// let there be an option to pick ends."** The arrow tool is that
/// choice made in advance — a line whose head lands where the drawer
/// aimed, not the tail.
#[test]
fn finishing_an_arrow_draws_a_line_with_a_head_where_it_was_aimed() {
    let mut app = app("pictures.pdf");
    app.submit("arrow");
    app.submit("pick 100,100");
    app.submit("pick 200,150");

    let layer = app.tab_mut().markup.existing(0).expect("the layer exists");
    assert_eq!(layer.len(), 1);
    assert!(matches!(&layer.objects()[0].geom, cad_kernel::Geom::Line(_)));
    assert_eq!(
        layer.arrow_ends(0),
        (false, true),
        "the head belongs at the second point, not the first"
    );
}

/// The other half of "we also need... splines" — enough points make a
/// real curve, not a silently-truncated one.
#[test]
fn a_finished_spline_keeps_every_point_it_was_given() {
    let mut app = app("pictures.pdf");
    app.submit("spline");
    app.submit("pick 100,100");
    app.submit("pick 150,120");
    app.submit("pick 200,100");
    app.submit("pick 250,140");
    app.submit("done");

    let layer = app.tab_mut().markup.existing(0).expect("the layer exists");
    assert_eq!(layer.len(), 1);
    match &layer.objects()[0].geom {
        cad_kernel::Geom::Spline(s) => assert_eq!(s.control_points.len(), 4),
        other => panic!("expected a spline, got {other:?}"),
    }
}

/// A degree-3 B-spline needs more control points than its degree —
/// finishing early must say so, not draw a shortened curve nobody asked
/// for.
#[test]
fn a_spline_finished_too_early_is_refused() {
    let mut app = app("pictures.pdf");
    app.submit("spline");
    app.submit("pick 100,100");
    app.submit("pick 150,120");
    app.submit("done");

    assert_eq!(
        app.tab_mut().markup.existing(0).map(|l| l.len()).unwrap_or(0),
        0,
        "nothing should have been added"
    );
    assert!(said(&app).contains("at least four"), "{}", said(&app));
}

/// **"the thickness of the lines should also be adjustable."** A shape
/// with no lineweight of its own resolves to nothing special; giving it
/// one is what the properties panel's Thickness slider does, and it must
/// survive being read back exactly.
#[test]
fn giving_a_shape_a_lineweight_makes_it_resolve_to_that_thickness() {
    let mut app = app("pictures.pdf");
    app.submit("circle");
    app.submit("pick 300,300");
    app.submit("pick 340,300");

    assert!(
        app.tab_mut().markup.existing(0).unwrap().resolved_lineweight_mm(0).is_none(),
        "nothing chosen yet"
    );
    let layer = app.tab_mut().markup.existing_mut(0).expect("the layer exists");
    assert!(layer.set_lineweight_mm(0, 0.5));
    assert_eq!(layer.resolved_lineweight_mm(0), Some(0.5));
}

#[test]
fn copying_and_pasting_a_selected_placed_picture_makes_a_second_one() {
    let mut app = app("two-column.pdf");
    app.place_image_at(0, AppPoint { x: 100.0, y: 400.0 }, solid_rgba(4, 4, [40, 90, 200]), 4, 4)
        .expect("placed");
    let mark = app.tab_mut().doc.as_ref().unwrap().session.placed_image_marks(0).unwrap().remove(0);
    app.tab_mut().selection.placed_image_selected =
        Some(PlacedImageSelected { page: 0, index: mark.index, rect: mark.rect, rotation: mark.rotation });

    assert!(app.copy_object_selection(), "the selected picture should have been copied");
    app.paste_object_selection();

    let pictures = app.tab_mut().doc.as_ref().unwrap().session.placed_image_marks(0).unwrap();
    assert_eq!(pictures.len(), 2, "paste should have added a second picture");
    assert!(
        (pictures[0].rect.left - pictures[1].rect.left).abs() > 1.0
            || (pictures[0].rect.top - pictures[1].rect.top).abs() > 1.0,
        "the paste landed exactly on the original instead of beside it"
    );
}

#[test]
fn pasting_with_nothing_copied_says_so() {
    let mut app = app("two-column.pdf");
    app.paste_object_selection();
    assert!(said(&app).contains("nothing to paste"), "expected a plain refusal, got: {}", said(&app));
}

/// A drag too small to mean anything (egui's own click-vs-drag noise
/// floor) commits nothing — no call into `pdf_core` at all, and the
/// selection is left exactly where it was, not nudged by a fraction of
/// a point on every stray pixel of mouse jitter.
#[test]
fn a_negligible_drag_on_a_signature_changes_nothing() {
    let (mut app, path) = with_signature_pad("two-column.pdf", "signature-noop");
    app.save_uploaded_signature("mine", solid_rgba(4, 4, [40, 90, 200]), 4, 4).expect("kept");
    app.submit("signature");
    app.place_signature(0, AppPoint { x: 100.0, y: 400.0 }).expect("placed");

    let mark = app.tab_mut().doc.as_ref().unwrap().session.image_signature_marks(0).unwrap().remove(0);
    let sel = SignatureSelected { page: 0, index: mark.index, rect: mark.rect, rotation: mark.rotation };
    app.tab_mut().selection.signature_selected = Some(sel.clone());
    let grab = Grab { handle: None, from: AppPoint { x: mark.rect.left as f64, y: mark.rect.top as f64 }, by: (0.1, -0.1) };
    app.finish_signature_grab(sel.clone(), grab);

    let still = app.tab_mut().doc.as_ref().unwrap().session.image_signature_marks(0).unwrap().remove(0);
    assert_eq!(still.rect, mark.rect, "a negligible drag moved the signature");
    assert_eq!(app.tab_mut().selection.signature_selected, Some(sel), "the selection should be exactly what was passed in, untouched");

    let _ = std::fs::remove_file(&path);
}

/// **Applying signatures clears a signature selection** — the
/// annotation it named is gone, burnt into the page, and a stale index
/// must not linger to be dragged next.
#[test]
fn applying_signatures_clears_the_signature_selection() {
    let (mut app, path) = with_signature_pad("two-column.pdf", "signature-apply-clears");
    app.save_uploaded_signature("mine", solid_rgba(4, 4, [40, 90, 200]), 4, 4).expect("kept");
    app.submit("signature");
    app.place_signature(0, AppPoint { x: 100.0, y: 400.0 }).expect("placed");

    let mark = app.tab_mut().doc.as_ref().unwrap().session.image_signature_marks(0).unwrap().remove(0);
    app.tab_mut().selection.signature_selected = Some(SignatureSelected { page: 0, index: mark.index, rect: mark.rect, rotation: mark.rotation });

    app.submit("applysignatures");
    assert!(app.tab_mut().selection.signature_selected.is_none(), "a stale selection survived applying");

    let _ = std::fs::remove_file(&path);
}

/// **`angle_from_drag` measures a clockwise sweep from the rect's own
/// centre** — pure geometry, no document or pointer event involved, so
/// this checks the arithmetic directly at angles a render-based test
/// only ever samples near. Dragging from "3 o'clock" to "6 o'clock"
/// relative to the centre is a quarter turn clockwise: +90 degrees.
/// Dragging back the other way is the same amount, negative.
#[test]
fn angle_from_drag_measures_a_clockwise_sweep_from_the_centre() {
    let rect = pdf_core::document::Rect { left: 0.0, top: 0.0, right: 100.0, bottom: 100.0 };
    // Centre is (50, 50). "3 o'clock" is directly right of centre, at
    // (90, 50); "6 o'clock" is directly below it, at (50, 90) — the
    // drag from one to the other is (-40, +40).
    let three_oclock = AppPoint { x: 90.0, y: 50.0 };
    let to_six_oclock = (-40.0, 40.0);

    let degrees = PagifyApp::angle_from_drag(&rect, 0.0, three_oclock, to_six_oclock);
    assert!((degrees - 90.0).abs() < 1.0, "expected a quarter turn clockwise, got {degrees}");

    // From 6 o'clock (50, 90) back to 3 o'clock (90, 50): (+40, -40).
    let back = PagifyApp::angle_from_drag(&rect, 90.0, AppPoint { x: 50.0, y: 90.0 }, (40.0, -40.0));
    assert!(back.abs() < 1.0, "turning back the same amount should return to 0, got {back}");
}

/// **Dragging the rotate handle turns the signature, committed through
/// `rotate_image_signature`** — the same shape as the move and resize
/// tests above, this time with `Grab.handle == Some(Handle::Rotate)`.
#[test]
fn dragging_the_rotate_handle_turns_the_signature() {
    let (mut app, path) = with_signature_pad("two-column.pdf", "signature-rotate");
    app.save_uploaded_signature("mine", solid_rgba(4, 4, [40, 90, 200]), 4, 4).expect("kept");
    app.submit("signature");
    app.place_signature(0, AppPoint { x: 100.0, y: 400.0 }).expect("placed");

    let mark = app.tab_mut().doc.as_ref().unwrap().session.image_signature_marks(0).unwrap().remove(0);
    assert_eq!(mark.rotation, 0.0, "a freshly placed signature should start unrotated");
    let centre = (
        (mark.rect.left + mark.rect.right) as f64 / 2.0,
        (mark.rect.top + mark.rect.bottom) as f64 / 2.0,
    );
    let sel = SignatureSelected { page: 0, index: mark.index, rect: mark.rect, rotation: mark.rotation };
    // From directly right of centre to directly below it: +90 degrees.
    let grab = Grab {
        handle: Some(Handle::Rotate),
        from: AppPoint { x: centre.0 + 20.0, y: centre.1 },
        by: (-20.0, 20.0),
    };
    app.finish_signature_grab(sel, grab);

    let turned = app.tab_mut().doc.as_ref().unwrap().session.image_signature_marks(0).unwrap().remove(0);
    assert!((turned.rotation - 90.0).abs() < 1.0, "expected roughly a 90-degree turn, got {}", turned.rotation);
    assert_eq!(turned.rect, mark.rect, "rotating must not move the picture's own rect");
    assert_eq!(
        app.tab_mut().selection.signature_selected,
        Some(SignatureSelected { page: 0, index: turned.index, rect: turned.rect, rotation: turned.rotation }),
        "the selection should carry the new rotation forward"
    );

    let _ = std::fs::remove_file(&path);
}

/// A rotate-drag too small to mean anything commits nothing, the same
/// noise floor the move and resize paths already have.
#[test]
fn a_negligible_rotate_drag_changes_nothing() {
    let (mut app, path) = with_signature_pad("two-column.pdf", "signature-rotate-noop");
    app.save_uploaded_signature("mine", solid_rgba(4, 4, [40, 90, 200]), 4, 4).expect("kept");
    app.submit("signature");
    app.place_signature(0, AppPoint { x: 100.0, y: 400.0 }).expect("placed");

    let mark = app.tab_mut().doc.as_ref().unwrap().session.image_signature_marks(0).unwrap().remove(0);
    let sel = SignatureSelected { page: 0, index: mark.index, rect: mark.rect, rotation: mark.rotation };
    app.tab_mut().selection.signature_selected = Some(sel.clone());
    let centre_ish = AppPoint {
        x: ((mark.rect.left + mark.rect.right) / 2.0) as f64 + 20.0,
        y: ((mark.rect.top + mark.rect.bottom) / 2.0) as f64,
    };
    // A drag of a fraction of a degree — nowhere near the 0.5-degree floor.
    let grab = Grab { handle: Some(Handle::Rotate), from: centre_ish, by: (0.0, 0.01) };
    app.finish_signature_grab(sel.clone(), grab);

    let still = app.tab_mut().doc.as_ref().unwrap().session.image_signature_marks(0).unwrap().remove(0);
    assert_eq!(still.rotation, 0.0, "a negligible drag should not have rotated the signature");
    assert_eq!(app.tab_mut().selection.signature_selected, Some(sel), "the selection should be untouched");

    let _ = std::fs::remove_file(&path);
}

/// **The rotate handle is found where it is drawn** — a fixed distance
/// above the rect's top-centre in screen space, not one of the eight
/// resize handles `Handle::ALL` already covers.
#[test]
fn signature_handle_at_finds_the_rotate_handle() {
    let (mut app, path) = with_signature_pad("two-column.pdf", "signature-rotate-handle");
    app.save_uploaded_signature("mine", solid_rgba(4, 4, [40, 90, 200]), 4, 4).expect("kept");
    app.submit("signature");
    app.place_signature(0, AppPoint { x: 100.0, y: 400.0 }).expect("placed");

    let mark = app.tab_mut().doc.as_ref().unwrap().session.image_signature_marks(0).unwrap().remove(0);
    app.tab_mut().selection.signature_selected =
        Some(SignatureSelected { page: 0, index: mark.index, rect: mark.rect, rotation: mark.rotation });

    // A simple 1:1 view with the page's own origin at the screen origin.
    let view = PageView { origin: egui::Pos2::new(0.0, 0.0), scale: 1.0 };
    let handle_screen = PagifyApp::rotate_handle_screen_pos(&mark.rect, view);
    let at_handle = view.to_page(handle_screen);
    assert_eq!(app.signature_handle_at(at_handle, view), Some(Handle::Rotate));

    // Well inside the body, away from every handle: none of them.
    let middle = AppPoint {
        x: ((mark.rect.left + mark.rect.right) / 2.0) as f64,
        y: ((mark.rect.top + mark.rect.bottom) / 2.0) as f64,
    };
    assert_eq!(app.signature_handle_at(middle, view), None);

    let _ = std::fs::remove_file(&path);
}

/// **The whole upload path, not just the part that skips the file.**
/// A real PNG, written to a scratch file and handed to `upload_signature`
/// exactly as the file dialog would — decoded, named from the file, and
/// kept, with the same pixels that went in.
#[test]
fn a_picture_file_is_decoded_and_kept_by_its_own_name() {
    let (mut app, sig_path) = with_signature_pad("two-column.pdf", "upload-file");

    let png_path = std::env::temp_dir()
        .join(format!("pagify-test-upload-{}.png", std::process::id()));
    let image = image::RgbaImage::from_raw(3, 2, solid_rgba(3, 2, [90, 200, 40]))
        .expect("a 3x2 buffer fits a 3x2 image");
    image.save(&png_path).expect("write the scratch PNG");

    app.upload_signature(&png_path);
    assert!(said(&app).contains("this computer"), "{}", said(&app));
    assert!(said(&app).contains("does not prove"), "{}", said(&app));

    let kept = app.signatures.current().expect("a signature was kept");
    let expected_name = png_path.file_stem().unwrap().to_string_lossy().into_owned();
    assert_eq!(kept.name, expected_name, "it was not named from the file");
    let stored = kept.image.as_ref().expect("it is a picture, not ink");
    assert_eq!((stored.width, stored.height), (3, 2));
    assert_eq!(stored.rgba, solid_rgba(3, 2, [90, 200, 40]), "the pixels changed in the round trip");

    let _ = std::fs::remove_file(&png_path);
    let _ = std::fs::remove_file(&sig_path);
}

/// A PNG whose IHDR declares an absurd size — checked purely from the
/// bytes of a real header, no encoder involved, since a legitimate one
/// refuses to write anything this large.
fn png_declaring(width: u32, height: u32) -> Vec<u8> {
    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc: u32 = 0xFFFF_FFFF;
        for &byte in bytes {
            crc ^= byte as u32;
            for _ in 0..8 {
                let mask = (crc & 1).wrapping_neg();
                crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
            }
        }
        !crc
    }
    fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        out.extend_from_slice(&body);
        out.extend_from_slice(&crc32(&body).to_be_bytes());
        out
    }
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit RGBA, no interlace
    out.extend(chunk(b"IHDR", &ihdr));
    // The decoder wants one to exist at all before it will report
    // anything — its actual bytes are never reached, since the size
    // this is testing is refused first.
    out.extend(chunk(b"IDAT", &[0, 0, 0, 0]));
    out.extend(chunk(b"IEND", &[]));
    out
}

/// **Found by audit.** A picture's declared size is read from its own
/// header before a single pixel is decoded — the `image` crate caps a
/// decoder's allocation at 512 MB but not its dimensions, and what
/// follows (`to_rgba8`, orientation, signature extraction) each make
/// another full-size copy on top.
#[test]
fn a_picture_declaring_an_absurd_size_is_refused_before_it_is_decoded() {
    let (mut app, sig_path) = with_signature_pad("two-column.pdf", "upload-huge");
    let png_path = std::env::temp_dir()
        .join(format!("pagify-test-upload-huge-{}.png", std::process::id()));
    std::fs::write(&png_path, png_declaring(50_000, 50_000)).expect("write the scratch PNG");

    app.upload_signature(&png_path);
    assert!(said(&app).contains("too large"), "{}", said(&app));
    assert!(app.signatures.is_empty(), "something was kept from an oversized picture");

    let _ = std::fs::remove_file(&png_path);
    let _ = std::fs::remove_file(&sig_path);
}

/// **Found by audit.** A script that names itself — or two that name
/// each other — has nothing else to stop it recursing forever; the
/// existing "stop at the first bad step" guard does not apply, since a
/// further `replay` dispatches just fine every time.
#[test]
fn a_script_that_replays_itself_stops_rather_than_recursing_forever() {
    let mut app = app("two-column.pdf");
    let path = std::env::temp_dir()
        .join(format!("pagify-test-replay-self-{}.json", std::process::id()));
    let script = Script {
        version: pagify_shell::automate::SCRIPT_VERSION,
        name: "self".into(),
        steps: vec![format!("replay {}", path.display())],
    };
    std::fs::write(&path, script.to_json()).expect("write the scratch script");

    app.replay(&path);

    assert!(said(&app).contains("scripts deep"), "{}", said(&app));
    assert_eq!(app.replay_depth, 0, "the depth counter was not unwound");

    let _ = std::fs::remove_file(&path);
}

/// **A file that is not a picture is refused, plainly, and nothing is
/// kept.**
#[test]
fn a_file_that_is_not_a_picture_is_refused() {
    let (mut app, sig_path) = with_signature_pad("two-column.pdf", "upload-bad");
    let bad_path = std::env::temp_dir()
        .join(format!("pagify-test-upload-bad-{}.png", std::process::id()));
    std::fs::write(&bad_path, b"not a picture").expect("write the scratch file");

    app.upload_signature(&bad_path);
    assert!(said(&app).contains("not a picture"), "{}", said(&app));
    assert!(app.signatures.is_empty(), "something was kept from a file that could not decode");

    let _ = std::fs::remove_file(&bad_path);
    let _ = std::fs::remove_file(&sig_path);
}

/// A scratch file for kept words — never the real one.
fn with_snippets(name: &str, label: &str) -> (PagifyApp, std::path::PathBuf) {
    let mut app = app(name);
    let path = std::env::temp_dir()
        .join(format!("pagify-test-predefined-{}-{label}.json", std::process::id()));
    let _ = std::fs::remove_file(&path);
    app.predefined = Default::default();
    app.predefined_path = Some(path.clone());
    (app, path)
}

/// **Words handed to the tool are kept, and armed for a click.**
#[test]
fn predefined_text_keeps_the_words_and_waits_for_a_click() {
    let (mut app, path) = with_snippets("two-column.pdf", "keep");
    app.submit("predefinedtext Jane Smith");

    assert!(said(&app).contains("this computer"), "{}", said(&app));
    assert_eq!(app.predefined.current(), Some("Jane Smith"));
    assert!(
        matches!(app.tab_mut().tool.as_ref().map(|t| &t.kind), Some(Tool::Write(t)) if t == "Jane Smith"),
        "it did not arm the click that writes them:\n{}",
        said(&app)
    );
    assert_eq!(
        pagify_shell::predefined::Predefined::load_from(&path).current(),
        Some("Jane Smith"),
        "it was not written to disk"
    );

    let _ = std::fs::remove_file(&path);
}

/// Using the same words again does not say it kept them a second time, and
/// does not keep a second copy.
#[test]
fn using_kept_words_again_keeps_no_second_copy() {
    let (mut app, path) = with_snippets("two-column.pdf", "again");
    app.submit("predefinedtext Jane Smith");
    app.submit("predefinedtext jane@example.com");
    app.submit("predefinedtext Jane Smith");

    assert_eq!(app.predefined.entries().len(), 2, "{:?}", app.predefined.entries());
    assert_eq!(app.predefined.current(), Some("Jane Smith"), "using it did not bring it back");
    let _ = std::fs::remove_file(&path);
}

/// **Nothing typed onto a page reaches the list.**
///
/// The decision this tool is built around: a form field holds somebody's
/// name and account number, and copying that to disk because it might be
/// handy later is not this program's decision to make.
#[test]
fn writing_text_on_a_page_does_not_keep_it() {
    let (mut app, path) = with_snippets("two-column.pdf", "not-kept");
    app.submit("addtext Something private");
    app.write_text_at(0, AppPoint { x: 100.0, y: 200.0 }, "Something private")
        .expect("written");

    assert!(app.predefined.is_empty(), "the typewriter filled the list: {:?}", app.predefined.entries());
    assert!(!path.exists(), "it wrote a list nobody asked for");
    let _ = std::fs::remove_file(&path);
}

/// The panel opens, and says so when there is nothing in it.
#[test]
fn the_predefined_text_panel_opens_and_says_when_it_is_empty() {
    let (mut app, path) = with_snippets("two-column.pdf", "panel");
    app.submit("predefinedtext");
    assert!(app.snippets.is_some(), "the panel did not open");
    assert!(said(&app).contains("nothing kept yet"), "{}", said(&app));
    let _ = std::fs::remove_file(&path);
}

/// The Search & Replace panel opens from its own command, the same way
/// every other small popup does — see `the_predefined_text_panel_opens_
/// and_says_when_it_is_empty` just above.
#[test]
fn the_search_and_replace_panel_opens_from_its_command() {
    let mut app = app("two-column.pdf");
    app.submit("replace");
    assert!(app.tab_mut().panels.find_replace.is_some(), "the panel did not open");
}

#[test]
fn bookmarking_the_page_opens_the_panel_with_a_default_title() {
    let mut app = app("two-column.pdf");
    app.submit("bookmark");

    let panel = app.tab_mut().panels.bookmark_panel.as_ref().expect("the panel did not open");
    assert_eq!(panel.entries, vec![("Page 1".to_string(), 0)]);
}

/// **"there is no indication that it is bookmarked"** — `bookmarked_pages`
/// is the cache `draw_pages` reads to paint the corner icon; this is
/// what proves the cache is kept honest without needing to render a
/// frame to check it.
#[test]
fn bookmarking_a_page_marks_it_for_the_corner_icon() {
    let mut app = app("two-column.pdf");
    assert!(app.tab_mut().bookmarked_pages.is_empty(), "a fresh document should start with none");

    app.submit("bookmark");
    assert!(app.tab_mut().bookmarked_pages.contains(&0));

    app.submit("undo");
    assert!(
        app.tab_mut().bookmarked_pages.is_empty(),
        "the icon's own cache should have followed the undo"
    );
}

/// **Named from what was already picked, not a bare page number** — the
/// same instinct as writing a caption from selected text.
#[test]
fn bookmarking_with_a_selection_uses_it_as_the_title() {
    let mut app = app("two-column.pdf");
    let range = app.characters(0).expect("chars").find("the").first().cloned().expect("a match");
    app.tab_mut().selection.text_selection = Some(range);
    app.tab_mut().organize.selection_page = 0;

    app.submit("bookmark");

    let panel = app.tab_mut().panels.bookmark_panel.as_ref().expect("the panel did not open");
    assert_eq!(panel.entries.len(), 1);
    assert_eq!(panel.entries[0].0.to_lowercase(), "the");
    assert_eq!(panel.entries[0].1, 0);
}

#[test]
fn undoing_a_bookmark_removes_it_from_the_document() {
    let mut app = app("two-column.pdf");
    app.submit("bookmark");
    assert_eq!(
        app.tab_mut().doc.as_ref().unwrap().session.bookmarks().expect("read").len(),
        1
    );

    app.submit("undo");
    assert!(
        app.tab_mut().doc.as_ref().unwrap().session.bookmarks().expect("read").is_empty(),
        "the bookmark should be gone after undo"
    );
}

#[test]
fn weblinks_arms_when_nothing_is_selected() {
    let mut app = app("two-column.pdf");
    app.submit("weblinks");
    assert!(app.tab_mut().tool.is_some(), "the tool was not armed");
    assert!(app.tab_mut().panels.pending_link.is_none());
}

#[test]
fn weblinks_opens_the_prompt_when_text_is_already_selected() {
    let mut app = app("two-column.pdf");
    let range = app.characters(0).expect("chars").find("the").first().cloned().expect("a match");
    app.tab_mut().selection.text_selection = Some(range);
    app.tab_mut().organize.selection_page = 0;

    app.submit("weblinks");

    assert!(app.tab_mut().panels.pending_link.is_some(), "the prompt did not open");
    assert!(app.tab_mut().selection.text_selection.is_none(), "the selection should have been consumed");
    assert!(app.tab_mut().tool.is_none(), "arming is only for when nothing was selected yet");
}

/// **The ribbon's "Link & Join Text" button, reported as missing its
/// own effect**: it was still `Verb::Planned` in the command table, so
/// pressing it only ever said "not built yet" — even though the
/// underlying join/split feature this session built was fully working
/// from a right-click. With nothing selected, pressing the button says
/// how to use it rather than doing nothing silently.
#[test]
fn jointext_with_nothing_selected_says_how_to_use_it() {
    let mut app = app("two-column.pdf");
    app.submit("jointext");
    let said_something = app.cmd.history().iter().any(|e| e.text.contains("drag across"));
    assert!(said_something, "should have explained what to do: {:?}", app.cmd.history());
}

#[test]
fn jointext_joins_an_already_made_selection_at_once() {
    let mut app = app("two-column.pdf");
    let runs = app.tab_mut().doc.as_ref().unwrap().session.text_runs(0).expect("runs");
    let a = runs.iter().min_by(|x, y| x.rect.top.total_cmp(&y.rect.top)).unwrap().clone();
    let b = runs.iter().max_by(|x, y| x.rect.top.total_cmp(&y.rect.top)).unwrap().clone();
    assert_ne!(a.object, b.object, "setup: need two distinct runs");

    let centre_a = ((a.rect.left + a.rect.right) / 2.0, (a.rect.top + a.rect.bottom) / 2.0);
    let centre_b = ((b.rect.left + b.rect.right) / 2.0, (b.rect.top + b.rect.bottom) / 2.0);
    let range = app
        .characters(0)
        .and_then(|c| c.range_between(centre_a, centre_b))
        .expect("a range covering both runs");
    app.tab_mut().selection.text_selection = Some(range);
    app.tab_mut().organize.selection_page = 0;

    app.submit("jointext");

    assert!(app.tab_mut().edit.editing_run.is_some(), "should have opened the joined paragraph editor");
    assert!(app.group_containing(0, a.object).is_some(), "the two runs should now be a joined group");
}

/// **"it should have all the same properties of the replaced word"** does
/// not apply here — a link adds a mark, it does not rewrite the text —
/// but the equivalent guarantee does: the link lands exactly over the
/// selection, one annotation per line, all to the same address.
#[test]
fn applying_a_web_link_makes_one_link_per_line() {
    let mut app = app("two-column.pdf");
    let rects = vec![
        pdf_core::document::Rect { left: 20.0, top: 30.0, right: 180.0, bottom: 44.0 },
        pdf_core::document::Rect { left: 20.0, top: 46.0, right: 100.0, bottom: 60.0 },
    ];
    let pending = PendingLink { page: 0, rects: rects.clone(), url: "example.com".to_string() };

    let said = app.apply_web_link(&pending).expect("apply failed");
    assert!(said.contains('2'), "should report two lines linked: {said}");

    let marks = app.tab_mut().doc.as_ref().unwrap().session.annotations(0).expect("read");
    let links: Vec<&pdf_core::document::Rect> = marks
        .iter()
        .filter_map(|m| match &m.annotation {
            pdf_core::document::Annotation::Link { rect, uri } => {
                // A bare domain needs a scheme, or the address opens
                // nowhere in most readers.
                assert_eq!(uri, "https://example.com");
                Some(rect)
            }
            _ => None,
        })
        .collect();
    assert_eq!(links.len(), 2, "expected one link per line: {marks:?}");
}

/// **"show it in blue font color but dont change the font style."**
#[test]
fn linked_text_turns_blue_but_keeps_its_words_and_size() {
    let mut app = app("two-column.pdf");
    let before = app.tab_mut().doc.as_ref().unwrap().session.text_runs(0).expect("runs")[0].clone();

    let pending =
        PendingLink { page: 0, rects: vec![before.rect], url: "example.com".to_string() };
    app.apply_web_link(&pending).expect("apply failed");

    let after = app.tab_mut().doc.as_ref().unwrap().session.text_runs(0).expect("runs");
    let changed = after
        .iter()
        .find(|r| r.object == before.object)
        .expect("the run should still be there");
    assert_eq!(changed.text, before.text, "the words should not have changed");
    assert_eq!(changed.size, before.size, "the size should not have changed");
    assert_eq!(
        (changed.color.r, changed.color.g, changed.color.b),
        (5, 99, 193),
        "should have turned the standard hyperlink blue"
    );

    let marks = app.tab_mut().doc.as_ref().unwrap().session.annotations(0).expect("read");
    assert!(
        marks.iter().any(|m| matches!(&m.annotation, pdf_core::document::Annotation::Link { .. })),
        "the link itself should still be there after recolouring: {marks:?}"
    );
    assert!(
        !marks.iter().any(|m| matches!(&m.annotation, pdf_core::document::Annotation::Underline { .. })),
        "the underline fallback should not fire when recolouring already succeeded: {marks:?}"
    );
}

/// Recolouring is scoped to what the link actually covers — a run
/// nowhere near the link's own rects keeps whatever colour it already
/// had.
#[test]
fn only_the_linked_run_turns_blue() {
    let mut app = app("two-column.pdf");
    let runs = app.tab_mut().doc.as_ref().unwrap().session.text_runs(0).expect("runs");
    let linked = runs[0].clone();
    let untouched =
        runs.iter().find(|r| r.object != linked.object).cloned().expect("a second run");

    let pending =
        PendingLink { page: 0, rects: vec![linked.rect], url: "example.com".to_string() };
    app.apply_web_link(&pending).expect("apply failed");

    let after = app.tab_mut().doc.as_ref().unwrap().session.text_runs(0).expect("runs");
    let still = after
        .iter()
        .find(|r| r.object == untouched.object)
        .expect("the unrelated run should still be there");
    assert_eq!(
        (still.color.r, still.color.g, still.color.b),
        (untouched.color.r, untouched.color.g, untouched.color.b),
        "an unrelated run should not have changed colour"
    );
}

#[test]
fn applying_a_web_link_with_no_address_is_refused() {
    let mut app = app("two-column.pdf");
    let pending = PendingLink {
        page: 0,
        rects: vec![pdf_core::document::Rect { left: 0.0, top: 0.0, right: 10.0, bottom: 10.0 }],
        url: String::new(),
    };
    assert!(app.apply_web_link(&pending).is_err());
}

/// **"once added theres no way of clicking it so it will take it the
/// user to that webpage"** — `link_uri_at` is the lookup the click
/// handler uses; this is what proves it actually finds the address
/// rather than just the annotation's existence.
#[test]
fn link_uri_at_finds_the_address_of_a_link_just_added() {
    let mut app = app("two-column.pdf");
    let pending = PendingLink {
        page: 0,
        rects: vec![pdf_core::document::Rect { left: 20.0, top: 30.0, right: 180.0, bottom: 44.0 }],
        url: "example.com".to_string(),
    };
    app.apply_web_link(&pending).expect("apply failed");

    assert_eq!(app.link_uri_at(0, 1), Some("https://example.com".to_string()));
}

#[test]
fn link_uri_at_is_none_for_a_mark_that_is_not_a_link() {
    let mut app = app("two-column.pdf");
    app.tab_mut().doc
        .as_ref()
        .unwrap()
        .session
        .highlight(
            0,
            vec![pdf_core::document::Rect { left: 20.0, top: 30.0, right: 180.0, bottom: 44.0 }],
            pdf_core::document::Color { r: 255, g: 224, b: 102, a: 128 },
        )
        .expect("highlight failed");

    assert_eq!(app.link_uri_at(0, 1), None);
}

#[test]
fn an_article_box_draws_a_border_and_a_note_for_its_title() {
    let mut app = app("two-column.pdf");
    let pending = PendingArticleBox {
        page: 0,
        rect: pdf_core::document::Rect { left: 20.0, top: 20.0, right: 200.0, bottom: 120.0 },
        title: "Reading order 1".to_string(),
    };

    let said = app.apply_article_box(&pending).expect("apply failed");
    assert!(said.contains("Reading order 1"), "{said}");

    let marks = app.tab_mut().doc.as_ref().unwrap().session.annotations(0).expect("read");
    let has_border = marks
        .iter()
        .any(|m| matches!(&m.annotation, pdf_core::document::Annotation::Ink { strokes, .. } if strokes.len() == 1 && strokes[0].len() == 5));
    assert!(has_border, "no bordered region found: {marks:?}");

    let has_title = marks.iter().any(|m| {
        matches!(&m.annotation, pdf_core::document::Annotation::Note { contents, .. } if contents == "Reading order 1")
    });
    assert!(has_title, "no title note found: {marks:?}");
}

#[test]
fn an_article_box_with_no_title_skips_the_note() {
    let mut app = app("two-column.pdf");
    let pending = PendingArticleBox {
        page: 0,
        rect: pdf_core::document::Rect { left: 20.0, top: 20.0, right: 200.0, bottom: 120.0 },
        title: String::new(),
    };
    app.apply_article_box(&pending).expect("apply failed");

    let marks = app.tab_mut().doc.as_ref().unwrap().session.annotations(0).expect("read");
    assert!(
        !marks.iter().any(|m| matches!(&m.annotation, pdf_core::document::Annotation::Note { .. })),
        "a note was added despite no title: {marks:?}"
    );
}

/// **Any change to the document asks before it is thrown away.**
///
/// Reported from use: a document was edited, closed, and the work went
/// without a word. Marks were guarded and a pending password was guarded;
/// everything that changes the document *in the document* — edited words, a
/// whiteout, a signature, a box, a page moved — was not.
#[test]
fn any_change_to_the_document_is_guarded_on_close() {
    // One representative of each way a document changes, all of which go
    // through the engine rather than through the markup layer.
    let mut boxed = app("two-column.pdf");
    assert!(!boxed.would_lose_work(), "a freshly opened document already looks changed");
    boxed
        .stamp_box(0, AppPoint { x: 40.0, y: 40.0 }, AppPoint { x: 200.0, y: 120.0 })
        .expect("box");
    assert!(boxed.would_lose_work(), "a box drawn on the page was not guarded");

    let mut ruled = app("two-column.pdf");
    ruled
        .stamp_line(0, AppPoint { x: 40.0, y: 100.0 }, AppPoint { x: 300.0, y: 100.0 })
        .expect("line");
    assert!(ruled.would_lose_work(), "a ruled line was not guarded");

    let mut painted = app("two-column.pdf");
    painted
        .whiteout(0, AppPoint { x: 40.0, y: 40.0 }, AppPoint { x: 200.0, y: 120.0 })
        .expect("whiteout");
    assert!(painted.would_lose_work(), "a whiteout was not guarded");
}

/// And the close itself stops rather than going through.
#[test]
fn closing_an_edited_document_asks_first() {
    let mut app = app("two-column.pdf");
    app.stamp_box(0, AppPoint { x: 40.0, y: 40.0 }, AppPoint { x: 200.0, y: 120.0 })
        .expect("box");

    app.submit("close");
    assert!(
        matches!(app.tab_mut().closing, Some(Closing::Document)),
        "it closed without asking:\n{}",
        said(&app)
    );
    assert!(app.tab_mut().doc.is_some(), "the document was closed anyway");
}

/// Opening another document alongside unsaved edits doesn't touch them —
/// it lands in a new tab instead of asking.
#[test]
fn opening_another_document_over_unsaved_edits_opens_a_new_tab() {
    let mut app = app("two-column.pdf");
    app.stamp_box(0, AppPoint { x: 40.0, y: 40.0 }, AppPoint { x: 200.0, y: 120.0 })
        .expect("box");

    app.submit(&format!("open {}", fixture("pages-ladder.pdf")));
    assert_eq!(app.tab().closing, None, "opening should never need to ask:\n{}", said(&app));
    assert_eq!(app.tabs.len(), 2, "opening another file should have made a second tab");
}

/// **Typing words the document has no letters for, through the app.**
///
/// The engine can write a font into a document; this checks the app hands
/// it one. Nothing else in the chain has ever needed a font, so the wiring
/// at `open_with` is the part with nobody watching it.
#[test]
fn editing_to_letters_the_document_lacks_writes_a_font_in() {
    let mut app = app("text-lines.pdf");
    let runs = app.tab_mut().doc.as_ref().expect("open").session.text_runs(0).expect("runs");
    let target = runs
        .iter()
        .find(|r| r.text.trim().chars().count() > 4)
        .cloned()
        .expect("a run with words in it");

    app.pick_text_run(
        0,
        AppPoint {
            x: ((target.rect.left + target.rect.right) / 2.0) as f64,
            y: ((target.rect.top + target.rect.bottom) / 2.0) as f64,
        },
    )
    .expect("picked");

    // Characters a subset font is unlikely to carry.
    app.tab_mut().edit.editing_run.as_mut().expect("editing").buffer = "Zwölf Ünique".into();
    app.apply_editing_page();

    let told = said(&app);
    if told.contains("cannot be changed") || told.contains("drawn in a way") {
        eprintln!("skipping: this run cannot be edited at all");
        return;
    }
    assert!(told.contains("Zwölf Ünique"), "it did not report the change: {told}");

    // The words are on the page.
    let mut app = app;
    app.tab_mut().doc.as_mut().unwrap().caches.text = None;
    let page = app.characters(0).map(|c| c.text()).unwrap_or_default();
    assert!(page.contains("Zwölf Ünique"), "the words are not on the page:\n{page}");

    // And if a face was substituted, the line says which and why.
    if told.contains("written in") {
        assert!(told.contains("Montserrat"), "it did not name the face: {told}");
        assert!(
            told.contains("not match its neighbours"),
            "it did not say the look changed: {told}"
        );
    }
}

/// **Picking a font from the font picker actually writes the run in it —
/// end to end through the real `pdf_core` path, not a mock.**
///
/// Distinct from the test above in the one way that matters: that one
/// exercises the *automatic* fallback (no font asked for, one chosen
/// because the current one cannot spell the words), which reports "does
/// not match its neighbours" — a warning about an unasked-for
/// substitution. An explicit pick is not a surprise, so it must say the
/// choice as a choice, not as a warning — see the message match in
/// `apply_one_edit`. Also checks a picked font that *cannot* spell the
/// words is refused by name, not silently substituted for something else.
#[test]
fn picking_a_font_writes_the_run_in_it() {
    let mut app = app("text-lines.pdf");
    let runs = app.tab_mut().doc.as_ref().expect("open").session.text_runs(0).expect("runs");
    let target = runs
        .iter()
        .find(|r| r.text.trim().chars().count() > 4)
        .cloned()
        .expect("a run with words in it");

    // Every opened document is handed the bundled fonts as typing fonts
    // (see `open_with`), so a bundled face is already available to pick
    // without adding anything — the same list `writing_faces` surfaces
    // for the outline matcher.
    let picked = app
        .writing_faces()
        .into_iter()
        .find(|n| n.to_ascii_lowercase().contains("montserrat"))
        .expect("Montserrat is bundled");

    app.pick_text_run(
        0,
        AppPoint {
            x: ((target.rect.left + target.rect.right) / 2.0) as f64,
            y: ((target.rect.top + target.rect.bottom) / 2.0) as f64,
        },
    )
    .expect("picked");

    // Picking a font alone, the text unchanged — still a real edit, not
    // "unchanged", and it must not be judged against `was` the way a
    // size/colour/position tweak is (see `changed_look` in
    // `apply_one_edit`).
    app.tab_mut().edit.editing_run.as_mut().expect("editing").style.face = Some(picked.clone());
    app.apply_editing_page();

    let told = said(&app);
    assert!(!told.contains("unchanged"), "a font pick alone was reported as no edit: {told}");
    assert!(told.contains(&picked), "it did not name the picked face: {told}");
    assert!(
        !told.contains("not match its neighbours"),
        "an explicit pick was reported as an unasked-for substitution: {told}"
    );

    // The real signal, not just the message: `substituted_face()` is what
    // `apply_one_edit` itself reads to decide what to say, so this is
    // the document actually reporting which face it swapped to.
    let substituted = app.tab_mut().doc.as_ref().unwrap().session.substituted_face();
    assert_eq!(
        substituted.as_deref(),
        Some(picked.as_str()),
        "the document did not record the picked face as what it wrote the run in"
    );

    // And the words themselves are still really on the page, at the run
    // this all started from.
    let after = app.tab_mut().doc.as_ref().unwrap().session.text_runs(0).unwrap();
    assert!(
        after.iter().any(|r| r.text.trim() == target.text.trim()),
        "the words are not on the page after the font-only edit: {after:?}"
    );

    // A font that plainly cannot spell the word (only ever ASCII words in
    // Latin fixtures) is refused by name — proven with a request that
    // asks for a face nothing on the list is, so the lookup itself must
    // fail rather than silently fall back.
    app.pick_text_run(
        0,
        AppPoint {
            x: ((target.rect.left + target.rect.right) / 2.0) as f64,
            y: ((target.rect.top + target.rect.bottom) / 2.0) as f64,
        },
    )
    .expect("picked again");
    app.tab_mut().edit.editing_run.as_mut().expect("editing").style.face = Some("NotARealFontName".into());
    app.apply_editing_page();
    let refused = said(&app);
    assert!(
        refused.contains("NotARealFontName") && refused.contains("not one of the fonts"),
        "an unknown requested face was not refused by name: {refused}"
    );
}

/// **The session log actually receives what the app does, not just what
/// `SessionLog` itself can be made to record in isolation.**
///
/// `session_log.rs`'s own tests prove the file format; this proves the
/// three call sites in this file (`PagifyApp::submit`, the ribbon/typed
/// dispatch in `ui`, and `say_info`/`say_error`) actually reach it — the
/// wiring a unit test of the module alone cannot see. `PagifyApp::new`
/// gives every test build a no-op logger (see its own comment on
/// `session_log`), so this swaps in a real one pointed at a temp
/// directory before doing anything.
#[test]
fn commands_and_outcomes_both_reach_the_session_log() {
    let mut app = app("two-column.pdf");
    let dir = std::env::temp_dir()
        .join(format!("pagify-test-session-log-wiring-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    app.session_log = pagify_shell::session_log::SessionLog::start_in(dir.clone());
    let log_path = app.session_log.path().expect("the temp dir is writable").to_path_buf();

    app.submit("zoom fit");
    app.say_error("a deliberate test error");

    let text = std::fs::read_to_string(&log_path).expect("the log file exists");
    let lines: Vec<serde_json::Value> = text
        .lines()
        .map(|l| serde_json::from_str(l).expect("valid json"))
        .collect();

    assert!(
        lines.iter().any(|l| l["kind"] == "command" && l["text"] == "zoom fit"),
        "the submitted command was not logged: {lines:?}"
    );
    assert!(
        lines.iter().any(|l| l["kind"] == "error" && l["text"] == "a deliberate test error"),
        "the error message was not logged: {lines:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// **A word replaced by itself leaves the page looking the same.**
///
/// The only check that can catch a replacement landing at the wrong size or
/// off the line, because it compares pictures rather than intentions: set
/// the same word, in the same face, and the ink should be where it was.
///
/// Both numbers here were earned. The size came back a quarter too small
/// until it was measured from the width the word occupies rather than taken
/// from the recogniser's estimate, and the baseline sat three points high
/// until it was measured from the bottom of the word's own ink.
#[test]
fn a_word_replaced_by_itself_lands_where_it_was() {
    let mut app = app("outlined-montserrat.pdf");
    let words = app.drawn_words_on(0).to_vec();
    let Some(word) = words
        .iter()
        .filter(|w| !w.text.trim().is_empty())
        .max_by_key(|w| w.text.trim().chars().count())
        .cloned()
    else {
        eprintln!("skipping: nothing recognised on the fixture");
        return;
    };
    let same = word.text.trim().to_string();

    // The ink in the word's own box: how much, how far left it starts, and
    // where its bottom edge sits.
    let measure = |app: &PagifyApp| -> (f32, f32, f32) {
        const SCALE: f32 = 4.0;
        let raster = app.tab()
            .doc
            .as_ref()
            .expect("open")
            .session
            .render_page(0, SCALE)
            .expect("render");
        let width = raster.width as usize;
        let r = word.rect;
        let (x0, y0) = ((r.left * SCALE) as usize, ((r.top - 4.0) * SCALE) as usize);
        let (x1, y1) = ((r.right * SCALE) as usize, ((r.bottom + 4.0) * SCALE) as usize);
        let (mut dark, mut seen) = (0usize, 0usize);
        let (mut left, mut bottom) = (usize::MAX, 0usize);
        for y in y0..y1 {
            for x in x0..x1 {
                let at = (y * width + x) * 4;
                if at + 2 < raster.pixels.len() {
                    seen += 1;
                    if raster.pixels[at] < 200 {
                        dark += 1;
                        left = left.min(x);
                        bottom = bottom.max(y);
                    }
                }
            }
        }
        (
            100.0 * dark as f32 / seen.max(1) as f32,
            if left == usize::MAX { 0.0 } else { left as f32 / SCALE },
            bottom as f32 / SCALE,
        )
    };
    let (ink_before, left_before, baseline_before) = measure(&app);

    app.pick_text_run(
        0,
        AppPoint {
            x: ((word.rect.left + word.rect.right) / 2.0) as f64,
            y: ((word.rect.top + word.rect.bottom) / 2.0) as f64,
        },
    )
    .expect("picked");
    app.tab_mut().edit.editing_run.as_mut().expect("editing").buffer = same.clone();
    app.apply_editing_page();

    let said = said(&app);
    if said.contains("could not be taken off") {
        eprintln!("skipping: this page's shapes cannot be separated");
        return;
    }
    let (ink_after, left_after, baseline_after) = measure(&app);

    assert!(
        (baseline_after - baseline_before).abs() < 1.0,
        "the words did not land on the line they came off: {baseline_before} then {baseline_after}"
    );
    assert!(
        (left_after - left_before).abs() < 1.5,
        "the words did not start where the old ones started: {left_before} then {left_after}"
    );
    // Set at the wrong size this halves; vector outlines and rendered type
    // never agree to the last pixel, so the bar is "the same word", not
    // "the same bytes".
    assert!(
        (ink_after - ink_before).abs() < ink_before * 0.25,
        "the words are not the size they were: {ink_before}% then {ink_after}%"
    );
}

/// **A page whose words are already text is not read again.**
///
/// Reported from use, and the cause of a bug that looked like three
/// different ones. Extracting on such a page ran OCR for over a minute —
/// measured on a real report: sixty-eight seconds on a page carrying
/// twenty-nine perfectly good text runs — and then laid a *second*,
/// invisible copy of the words over the real ones.
///
/// Afterwards a click picked the copy. So the reported text changed, the
/// page did not, and deleting a word left it plainly visible underneath:
/// "if i delete something the text embedded below still shows".
#[test]
fn extracting_skips_a_page_that_already_has_text() {
    let mut app = app("two-column.pdf");
    let before = app.tab_mut().doc.as_ref().expect("open").session.text_runs(0).expect("runs");
    assert!(!before.is_empty(), "the fixture has no text, so this proves nothing");

    app.submit("extracttext");

    // No worker was started at all: the answer is immediate.
    assert!(app.tab_mut().reading.is_none(), "it went off to read a page that needs no reading");
    let told = said(&app);
    assert!(told.contains("already has text"), "{told}");
    assert!(
        told.contains("edit it directly"),
        "it did not say what to do instead: {told}"
    );

    // And nothing was added — least of all an invisible copy of the words.
    let after = app.tab_mut().doc.as_ref().expect("open").session.text_runs(0).expect("runs");
    assert_eq!(after.len(), before.len(), "the page gained runs");
    assert_eq!(
        after.iter().filter(|r| r.color.a == 0).count(),
        0,
        "a transparent copy of the words was written over the real ones"
    );

    // So a click still picks the real words, which is what makes editing
    // change what anybody can see.
    let target = before
        .iter()
        .find(|r| r.text.trim().chars().count() > 5)
        .cloned()
        .expect("a run with words");
    app.pick_text_run(
        0,
        AppPoint {
            x: ((target.rect.left + target.rect.right) / 2.0) as f64,
            y: ((target.rect.top + target.rect.bottom) / 2.0) as f64,
        },
    )
    .expect("picked");
    assert!(
        app.tab_mut().edit.editing_run.as_ref().is_some_and(|e| !e.drawn),
        "it picked something other than the page's own words"
    );
}

/// **A word that is drawn can be replaced with one that is written.**
///
/// The page this exists for carries no text at all: its words are Bézier
/// paths, type converted to outlines when the file was made. Reported from
/// use — "editing it doesn't change the text, because if i delete something
/// the text embedded below still shows" — which is what editing a
/// transparent extraction layer over artwork does.
///
/// So the two things that actually change the page: the paths come off, and
/// real text goes on. Checked by reading the page back, not by asking the
/// program what it thinks it did.
#[test]
fn a_drawn_word_can_be_replaced_with_real_text() {
    let mut app = app("outlined-montserrat.pdf");
    assert_eq!(
        app.tab_mut().doc.as_ref().expect("open").session.text_runs(0).expect("runs").len(),
        0,
        "the fixture has text on it, so it proves nothing about drawn words"
    );

    let words = app.drawn_words_on(0).to_vec();
    assert!(!words.is_empty(), "no drawn words were recognised at all");
    let word = words
        .iter()
        .filter(|w| !w.text.trim().is_empty())
        .max_by_key(|w| w.text.trim().chars().count())
        .cloned()
        .expect("a word");

    // Picking one takes **no extraction**, which is the point: a layer
    // written over the page re-emits it, and objects that end up nested in
    // a form cannot be taken off by a redaction afterwards.
    let told = app
        .pick_text_run(
            0,
            AppPoint {
                x: ((word.rect.left + word.rect.right) / 2.0) as f64,
                y: ((word.rect.top + word.rect.bottom) / 2.0) as f64,
            },
        )
        .expect("a drawn word was not picked");
    assert!(told.contains("drawn, not written"), "{told}");
    assert!(app.tab_mut().edit.editing_run.as_ref().is_some_and(|e| e.drawn));

    app.tab_mut().edit.editing_run.as_mut().expect("editing").buffer = "REPLACED".into();
    app.apply_editing_page();

    let said = said(&app);
    if said.contains("could not be taken off") {
        // The honest other answer: some pages draw a whole line as one
        // path, and one word cannot be lifted out of it. Nothing changed.
        eprintln!("skipping: this page's shapes cannot be separated");
        return;
    }
    assert!(said.contains("replaced the drawn word"), "{said}");
    // **Set in the face the page was set in**, not a stand-in: these words
    // were recognised *by* that font, so it is the one that matches.
    assert!(
        said.contains("Montserrat"),
        "it did not use the document's own face: {said}"
    );

    // The page really says it now, read back the way anything else reads it.
    app.tab_mut().doc.as_mut().unwrap().caches.text = None;
    let page = app.characters(0).map(|c| c.text()).unwrap_or_default();
    assert!(page.contains("REPLACED"), "the words are not on the page:\n{page}");
}

/// **Words that are drawn rather than written say so, before anything is
/// typed.**
///
/// Reported from use, and the worst kind of failure: on a page whose words
/// are vector outlines, `extracttext` writes a transparent text layer over
/// the artwork so it can be searched. Editing that layer appears to work —
/// the reported text changes — and the page looks exactly as it did,
/// because the printed words are paths and nothing touched them.
#[test]
fn editing_drawn_words_says_they_will_be_replaced() {
    let mut app = app("text-lines.pdf");

    // A transparent run, written the way `extracttext` writes its layer.
    let doc = app.tab_mut().doc.as_ref().expect("open");
    doc.session
        .execute(pdf_core::command::Command::AddAnnotation {
            page_index: 0,
            annotation: pdf_core::document::Annotation::Text {
                text: "invisible".into(),
                font: "Helvetica".into(),
                font_asset: None,
                size: 12.0,
                color: pdf_core::document::Color { r: 0, g: 0, b: 0, a: 0 },
                glyphs: vec![pdf_core::document::Glyph {
                    ch: "invisible".into(),
                    id: 0,
                    x: 40.0,
                    y: 700.0,
                    radians: 0.0,
                }],
                id: 7,
                restore: String::new(),
                frame: Vec::new(),
                frame_width: 0.0,
            },
        })
        .expect("wrote a transparent run");

    let runs = app.tab_mut().doc.as_ref().expect("open").session.text_runs(0).expect("runs");
    let ghost = runs
        .iter()
        .find(|r| r.color.a == 0 && r.text.contains("invisible"))
        .cloned()
        .expect("the transparent run is not on the page");

    let told = app
        .pick_text_run(
            0,
            AppPoint {
                x: ((ghost.rect.left + ghost.rect.right) / 2.0) as f64,
                y: ((ghost.rect.top + ghost.rect.bottom) / 2.0) as f64,
            },
        )
        .expect("picked");

    assert!(told.contains("drawn, not written"), "{told}");
    assert!(
        told.contains("takes the drawn shapes off"),
        "it did not say what changing them does: {told}"
    );
    assert!(
        told.contains("will not match"),
        "it did not say the face changes: {told}"
    );
    assert!(
        app.tab_mut().edit.editing_run.as_ref().is_some_and(|e| e.drawn),
        "the edit did not remember what it had picked"
    );
}

/// **Edit Object reaches for the picture; Move reaches for the words.**
///
/// The same two clicks either way, and either will take the other when
/// there is nothing else under the pointer — what differs is which of two
/// overlapping things was meant. A caption on a photograph is the caption
/// when you are moving things, and the photograph when you are editing
/// objects.
#[test]
fn edit_object_prefers_the_picture_and_move_prefers_the_words() {
    let mut app = app("two-column.pdf");
    let runs = app.tab_mut().doc.as_ref().expect("open").session.text_runs(0).expect("runs");
    let target = runs
        .iter()
        .find(|r| r.text.trim().chars().count() > 5)
        .cloned()
        .expect("a run");
    let middle = AppPoint {
        x: ((target.rect.left + target.rect.right) / 2.0) as f64,
        y: ((target.rect.top + target.rect.bottom) / 2.0) as f64,
    };

    // Words under the pointer and no picture: both tools find the words,
    // because either will take what is actually there.
    let (_, _, by_move) = app.thing_at(0, middle, false).expect("the words");
    let (_, _, by_object) = app.thing_at(0, middle, true).expect("the words");
    assert_eq!(by_move, "the words");
    assert_eq!(by_object, "the words", "Edit Object refused words when that is all there is");
}

/// Both tools arm, and each says which it is aiming at.
#[test]
fn both_moving_tools_arm_and_say_what_they_take() {
    let mut first = app("two-column.pdf");
    first.submit("editobject");
    assert_eq!(first.tab().tool_state.object_tool, Some(true), "edit object did not arm:\n{}", said(&first));
    assert!(said(&first).contains("picture"), "{}", said(&first));

    let mut second = app("two-column.pdf");
    second.submit("moveobject");
    assert_eq!(second.tab().tool_state.object_tool, Some(false));
    assert!(said(&second).contains("words or a picture"), "{}", said(&second));
}

/// **Words and pictures can be picked up and put down.**
///
/// Two clicks: what to move, and where it goes. Whatever is under the first
/// one — a caption sitting on a photograph is the caption, because the
/// **A picture moves where it is put, and takes nothing with it.**
///
/// This is the "Edit Object" gesture: `pictures_first`, so a click over a
/// picture picks the picture rather than any words on top of it. Reported
/// from use as the picture disappearing, so this asserts it is still on the
/// page afterwards as well as where it went.
#[test]
fn a_picture_can_be_moved_and_is_still_on_the_page() {
    // Not `app`: the helper of that name is called again inside the loop,
    // and a local would shadow it.
    let opened = app("pictures.pdf");
    let before = opened.tab().doc.as_ref().expect("open").session.images_on(0).expect("images");
    assert_eq!(before.len(), 2, "the fixture should have two pictures");
    let words = page_text(&opened, 0);
    drop(opened);

    for target in &before {
        let mut carrying = app("pictures.pdf");
        let middle = |r: &pdf_core::document::Rect| AppPoint {
            x: ((r.left + r.right) / 2.0) as f64,
            y: ((r.top + r.bottom) / 2.0) as f64,
        };
        let from = middle(&target.rect);
        let to = AppPoint { x: from.x + 30.0, y: from.y + 18.0 };

        let told = carrying.move_thing(0, from, to, true).expect("moved");
        assert!(told.contains("the picture"), "it did not pick the picture: {told}");

        let after =
            carrying.tab().doc.as_ref().expect("open").session.images_on(0).expect("images");
        assert_eq!(after.len(), 2, "a picture left the page");
        let now = after
            .iter()
            .find(|i| i.object == target.object)
            .expect("the picture that was moved is gone");
        assert!(
            (now.rect.left - target.rect.left - 30.0).abs() < 0.5
                && (now.rect.top - target.rect.top - 18.0).abs() < 0.5,
            "object {} went to {:?} from {:?}",
            target.object,
            now.rect,
            target.rect
        );
        assert_eq!(page_text(&carrying, 0), words, "moving a picture changed the words");
    }
}

/// **A rectangle dragged over more than one thing picks up all of
/// them**, not just whichever one a plain click would have landed on —
/// the fixture's two pictures, both inside one marquee.
#[test]
fn dragging_a_rectangle_over_two_pictures_selects_both() {
    let mut app = app("pictures.pdf");
    app.submit("editobject");
    let pictures = app.tab_mut().doc.as_ref().expect("open").session.images_on(0).expect("images");
    assert_eq!(pictures.len(), 2, "the fixture should have two pictures");

    let mut bounds = pictures[0].rect;
    for p in &pictures[1..] {
        bounds.left = bounds.left.min(p.rect.left);
        bounds.top = bounds.top.min(p.rect.top);
        bounds.right = bounds.right.max(p.rect.right);
        bounds.bottom = bounds.bottom.max(p.rect.bottom);
    }
    let pad = 5.0;
    app.select_group_in(
        0,
        AppPoint { x: (bounds.left - pad) as f64, y: (bounds.top - pad) as f64 },
        AppPoint { x: (bounds.right + pad) as f64, y: (bounds.bottom + pad) as f64 },
        false,
    );
    assert!(app.tab_mut().selection.selected.is_none(), "a group should not also leave a single selection");
    assert_eq!(app.tab_mut().selection.group.len(), 2, "both pictures should be in the group: {:?}", app.tab_mut().selection.group);
    assert!(said(&app).contains("2 things selected"), "{}", said(&app));
}

/// **A marquee over just one thing behaves like clicking it** — full
/// [`PagifyApp::selected`], handles and all, not a one-member group with
/// no way to resize it.
#[test]
fn a_marquee_over_just_one_thing_selects_it_normally() {
    let mut app = app("pictures.pdf");
    app.submit("editobject");
    let target = app.tab_mut().doc.as_ref().expect("open").session.images_on(0).expect("images")[0].clone();
    let pad = 5.0;
    app.select_group_in(
        0,
        AppPoint { x: (target.rect.left - pad) as f64, y: (target.rect.top - pad) as f64 },
        AppPoint { x: (target.rect.right + pad) as f64, y: (target.rect.bottom + pad) as f64 },
        false,
    );
    assert!(app.tab_mut().selection.group.is_empty(), "one thing should not become a group");
    assert_eq!(app.tab_mut().selection.selected.as_ref().map(|s| s.object), Some(target.object));
}

/// **Clicking one line of a paragraph in Edit Text opens the whole
/// paragraph to retype, not just that line** — the fixture's left-hand
/// column is eight single-spaced lines, close enough together that
/// they must all merge into one paragraph.
#[test]
fn clicking_a_line_in_edit_text_opens_the_whole_paragraph_it_sits_in() {
    let mut app = app("two-column.pdf");
    let runs = app.tab_mut().doc.as_ref().expect("open").session.text_run_rects(0).expect("runs");

    // The left column: everything left of the gap before the right
    // column starts.
    let left_column: std::collections::HashSet<usize> =
        runs.iter().filter(|(_, r)| r.left < 300.0).map(|(o, _)| *o).collect();
    assert_eq!(left_column.len(), 8, "expected the fixture's eight left-column lines");

    // A line from the middle of the column, not an edge, so a click has
    // lines on both sides of it to gather.
    let mut left_runs: Vec<(usize, pdf_core::document::Rect)> =
        runs.iter().filter(|(o, _)| left_column.contains(o)).cloned().collect();
    left_runs.sort_by(|(_, a), (_, b)| a.top.total_cmp(&b.top));
    let (_, seed_rect) = left_runs[left_runs.len() / 2];
    let at = AppPoint {
        x: ((seed_rect.left + seed_rect.right) / 2.0) as f64,
        y: ((seed_rect.top + seed_rect.bottom) / 2.0) as f64,
    };

    app.submit("edittext");
    app.pick_text_run(0, at).expect("a run was here");

    let edit = app.tab_mut().edit.editing_run.as_ref().expect("edit text should have opened");
    let opened: std::collections::HashSet<usize> =
        edit.lines.iter().flat_map(|(o, _)| o.iter().copied()).collect();
    assert_eq!(
        opened, left_column,
        "the paragraph opened for editing did not match the left column exactly"
    );
    assert_eq!(
        edit.buffer.lines().count(),
        8,
        "the combined buffer should hold all eight lines: {:?}",
        edit.buffer
    );
}

/// Arms Edit Text and clicks the middle of the fixture's eight-line
/// left column, the same way the test above finds it — shared by the
/// three `apply_paragraph_edit` tests below so each starts from the
/// same open paragraph rather than repeating the setup.
fn open_left_column_paragraph(app: &mut PagifyApp) -> Vec<usize> {
    let runs = app.tab_mut().doc.as_ref().expect("open").session.text_run_rects(0).expect("runs");
    let mut left_runs: Vec<(usize, pdf_core::document::Rect)> =
        runs.iter().filter(|(_, r)| r.left < 300.0).cloned().collect();
    left_runs.sort_by(|(_, a), (_, b)| a.top.total_cmp(&b.top));
    assert_eq!(left_runs.len(), 8, "expected the fixture's eight left-column lines");
    let (_, seed_rect) = left_runs[left_runs.len() / 2];
    let at = AppPoint {
        x: ((seed_rect.left + seed_rect.right) / 2.0) as f64,
        y: ((seed_rect.top + seed_rect.bottom) / 2.0) as f64,
    };

    app.submit("edittext");
    app.pick_text_run(0, at).expect("a run was here");
    app.tab_mut().edit.editing_run
        .as_ref()
        .expect("edit text should have opened")
        .lines
        .iter()
        .map(|(objects, _)| *objects.first().expect("a line is never empty"))
        .collect()
}

/// **Retyping a paragraph rewrites each of its lines in place** — the
/// object at index `i` gets the `i`th line of what was typed, the same
/// safe in-place swap a single run's own edit already uses.
#[test]
fn applying_a_paragraph_edit_rewrites_each_line_in_place() {
    let mut app = app("two-column.pdf");
    let objects = open_left_column_paragraph(&mut app);

    let new_text = "ONE\nTWO\nTHREE\nFOUR\nFIVE\nSIX\nSEVEN\nEIGHT";
    app.tab_mut().edit.editing_run.as_mut().unwrap().buffer = new_text.to_string();
    app.apply_editing_page();

    let after = app.tab_mut().doc.as_ref().expect("open").session.text_runs(0).expect("runs");
    let by_object: std::collections::HashMap<usize, String> =
        after.into_iter().map(|r| (r.object, r.text)).collect();
    for (i, object) in objects.iter().enumerate() {
        let expected = new_text.split('\n').nth(i).unwrap();
        assert_eq!(
            by_object.get(object).map(|s| s.trim()),
            Some(expected),
            "line {i} (object {object}) did not get its own new text"
        );
    }
}

/// **Typing more lines than the paragraph had adds the rest below it**
/// — the existing lines stay exactly where they were; only the new
/// ones are placed.
///
/// **A shaped write is one object per glyph**, same as any other
/// embedded-font write — see `write_styled_line_at` — so the new line's
/// own objects are read by joining every one that is not among the
/// paragraph's own original objects, not by looking for a single run
/// that already says the whole thing.
#[test]
fn applying_a_grown_paragraph_edit_adds_new_lines_below() {
    let mut app = app("two-column.pdf");
    let objects = open_left_column_paragraph(&mut app);

    let mut new_text: Vec<String> = (0..objects.len()).map(|i| format!("L{i}")).collect();
    new_text.push("EXTRALINE".to_string());
    app.tab_mut().edit.editing_run.as_mut().unwrap().buffer = new_text.join("\n");
    app.apply_editing_page();

    let after = app.tab_mut().doc.as_ref().expect("open").session.text_runs(0).expect("runs");
    let mut added: Vec<_> = after
        .iter()
        .filter(|r| !objects.contains(&r.object) && !r.text.trim().is_empty())
        .collect();
    added.sort_by(|a, b| a.rect.left.total_cmp(&b.rect.left));
    let joined: String = added.iter().map(|r| r.text.as_str()).collect();
    assert!(
        joined.contains("EXTRALINE"),
        "the extra line was not added: {:?}",
        after.iter().map(|r| r.text.trim()).collect::<Vec<_>>()
    );
    // And the original eight are still there, each with its own new text.
    let by_object: std::collections::HashMap<usize, String> =
        after.into_iter().map(|r| (r.object, r.text)).collect();
    for (i, object) in objects.iter().enumerate() {
        assert_eq!(by_object.get(object).map(|s| s.trim()), Some(format!("L{i}").as_str()));
    }
}

/// **Typing fewer lines than the paragraph had REMOVES the rest** — the
/// lines' objects come off the page, words and all, and nothing else on it
/// changes. They used to be painted in the page's colour and left in the
/// file: an old line stayed searchable, was drawn after the new words (and
/// could erase part of them), and was found again by the next pick.
#[test]
fn applying_a_shrunk_paragraph_edit_removes_the_extra_lines() {
    let mut app = app("two-column.pdf");
    let objects = open_left_column_paragraph(&mut app);
    let before = tests_support::runs_in_order(&app);

    app.tab_mut().edit.editing_run.as_mut().unwrap().buffer = "ONLYONE".to_string();
    app.apply_editing_page();
    let after = tests_support::runs_in_order(&app);

    // One object per line on this fixture: the first line holds the new
    // words, the other seven are gone, and the right column is untouched.
    tests_support::assert_page_after(
        "a paragraph shrunk to one line",
        &before,
        &after,
        &[(objects[0], "ONLYONE")],
        &objects[1..],
    );
    assert_eq!(after.len(), before.len() - (objects.len() - 1));
}

/// **Edit Object never groups a click on text into a paragraph** —
/// that belongs to Edit Text (see the test above); Edit Object always
/// answers at word or letter granularity, on the same fixture's
/// eight-line column that would have merged were this still there.
#[test]
fn clicking_a_line_in_edit_object_does_not_select_the_paragraph() {
    let mut app = app("two-column.pdf");
    app.submit("editobject");
    let runs = app.tab_mut().doc.as_ref().expect("open").session.text_run_rects(0).expect("runs");
    let mut left_runs: Vec<(usize, pdf_core::document::Rect)> =
        runs.iter().filter(|(_, r)| r.left < 300.0).cloned().collect();
    left_runs.sort_by(|(_, a), (_, b)| a.top.total_cmp(&b.top));
    let (_, seed_rect) = left_runs[left_runs.len() / 2];
    let at = AppPoint {
        x: ((seed_rect.left + seed_rect.right) / 2.0) as f64,
        y: ((seed_rect.top + seed_rect.bottom) / 2.0) as f64,
    };

    app.select_thing_at(0, at);

    assert!(app.tab_mut().selection.group.is_empty(), "edit object grouped a click into a paragraph");
    assert!(app.tab_mut().selection.selected.is_some(), "the click should still have selected something");
}

/// **The paragraph detector has exactly one production caller — Edit
/// Text's own `pick_text_run` — and Edit Object's character split
/// (`split_run_into_characters`) is only ever reachable through a
/// different tool, gated behind its own `self.tab_mut().tool_state.object_tool`.** The two
/// can never fire back to back in one action: reaching the detector
/// with a just-split letter as its seed always means a tool switch (and
/// therefore real, elapsed use) happened first, never that the split
/// itself is still in flight. So the split's neighbouring one-letter
/// runs are exactly what the detector exists to reassemble — see
/// `edit_text_recovers_a_line_that_was_split_into_characters` for the
/// same fix exercised through the real Edit Text entry point.
///
/// Asked of the page's blocks directly: the block holding the letter has
/// more than the letter in it, and its line reads as the run did before
/// it was split (the zero-size spaces a split leaves are not objects of
/// their own; PDFium's own trailing spaces keep the words apart).
#[test]
fn splitting_a_run_leaves_its_letters_recoverable_as_one_paragraph() {
    let mut app = app("two-column.pdf");
    let runs = app.tab_mut().doc.as_ref().expect("open").session.text_runs(0).expect("runs");
    let target = runs
        .iter()
        .find(|r| r.text.trim().chars().count() > 5)
        .cloned()
        .expect("a run with words");

    app.tab_mut().doc
        .as_ref()
        .expect("open")
        .session
        .split_run_into_characters(0, target.object)
        .expect("split");

    // A point a quarter of the way into where the run used to be — the
    // same aim `clicking_a_run_splits_it_and_selects_one_letter` uses,
    // chosen there because the exact centre can land in a gap between
    // characters. Whichever of the newly split characters' rects
    // contains it is the letter this test starts from.
    let at = (
        target.rect.left + (target.rect.right - target.rect.left) * 0.25,
        (target.rect.top + target.rect.bottom) / 2.0,
    );
    // Fresh, not the `runs` fetched before the split: those still name
    // the one whole object that no longer exists. (No page was read
    // before the split, so none is cached from before it either.)
    let (detected, _) = app.page_blocks(0).expect("the page's blocks");
    let letter = detected
        .runs
        .values()
        .find(|r| {
            at.0 >= r.rect.left.min(r.rect.right)
                && at.0 <= r.rect.left.max(r.rect.right)
                && at.1 >= r.rect.top.min(r.rect.bottom)
                && at.1 <= r.rect.top.max(r.rect.bottom)
        })
        .expect("the click point should land on one of the split characters");
    assert_ne!(letter.object, target.object, "setup: the split gave the letter an object of its own");

    let &(block, line) = detected.by_object.get(&letter.object).expect("the letter is in a block");
    let members = &detected.blocks[block].lines[line].objects;
    assert!(
        members.len() > 1,
        "a split letter should have found its same-line siblings again: {members:?}"
    );
    let text: String = members.iter().map(|o| detected.runs[o].text.as_str()).collect();
    assert_eq!(
        text.trim(),
        target.text.trim(),
        "the letters' line should read as the run did before it was split"
    );
}

/// **Found by audit-style report: a marquee must only take what it fully
/// encloses.** A rectangle that merely crosses a picture's edge — the old
/// "any overlap counts" rule — must leave it out.
#[test]
fn a_marquee_that_only_clips_a_picture_does_not_select_it() {
    let mut app = app("pictures.pdf");
    app.submit("editobject");
    let pictures = app.tab_mut().doc.as_ref().expect("open").session.images_on(0).expect("images");
    let target = pictures[0].clone();

    // The right half of the target's own bounding box: its left edge is
    // outside this rectangle, so it is crossed, not enclosed.
    let mid_x = (target.rect.left + target.rect.right) / 2.0;
    app.select_group_in(
        0,
        AppPoint { x: mid_x as f64, y: (target.rect.top - 5.0) as f64 },
        AppPoint { x: (target.rect.right + 5.0) as f64, y: (target.rect.bottom + 5.0) as f64 },
        false,
    );
    assert_ne!(
        app.tab_mut().selection.selected.as_ref().map(|s| s.object),
        Some(target.object),
        "a picture only half inside the marquee was selected"
    );
}

/// **Shift-click adds to the selection instead of replacing it.**
#[test]
fn shift_clicking_a_second_picture_adds_it_to_the_selection() {
    let mut app = app("pictures.pdf");
    app.submit("editobject");
    let pictures = app.tab_mut().doc.as_ref().expect("open").session.images_on(0).expect("images");
    let (first, second) = (pictures[0].clone(), pictures[1].clone());

    let at = |r: pdf_core::document::Rect| AppPoint {
        x: ((r.left + r.right) / 2.0) as f64,
        y: ((r.top + r.bottom) / 2.0) as f64,
    };
    app.select_thing_at(0, at(first.rect));
    assert_eq!(app.tab_mut().selection.selected.as_ref().map(|s| s.object), Some(first.object));

    app.extend_selection_at(0, at(second.rect));
    assert!(app.tab_mut().selection.selected.is_none(), "a two-member selection must be a group, not `selected`");
    let objects: Vec<usize> = app.tab_mut().selection.group.iter().map(|s| s.object).collect();
    assert!(objects.contains(&first.object) && objects.contains(&second.object), "{objects:?}");
}

/// **Shift-clicking a member already in the selection drops it again** —
/// the toggle every other multi-select gesture uses.
#[test]
fn shift_clicking_a_selected_picture_again_removes_it() {
    let mut app = app("pictures.pdf");
    app.submit("editobject");
    let pictures = app.tab_mut().doc.as_ref().expect("open").session.images_on(0).expect("images");
    let (first, second) = (pictures[0].clone(), pictures[1].clone());
    app.tab_mut().selection.group = vec![
        Selected { page: 0, object: first.object, rect: first.rect, what: "the picture" },
        Selected { page: 0, object: second.object, rect: second.rect, what: "the picture" },
    ];

    app.extend_selection_at(
        0,
        AppPoint {
            x: ((first.rect.left + first.rect.right) / 2.0) as f64,
            y: ((first.rect.top + first.rect.bottom) / 2.0) as f64,
        },
    );
    assert_eq!(app.tab_mut().selection.group.len(), 0, "removing one of two should leave one, folded into `selected`");
    assert_eq!(app.tab_mut().selection.selected.as_ref().map(|s| s.object), Some(second.object));
}

/// **Shift-dragging a marquee adds to the selection rather than
/// replacing it.**
#[test]
fn shift_dragging_a_marquee_extends_an_existing_selection() {
    let mut app = app("pictures.pdf");
    app.submit("editobject");
    let pictures = app.tab_mut().doc.as_ref().expect("open").session.images_on(0).expect("images");
    let (first, second) = (pictures[0].clone(), pictures[1].clone());
    app.tab_mut().selection.selected = Some(Selected { page: 0, object: first.object, rect: first.rect, what: "the picture" });

    let pad = 5.0;
    app.select_group_in(
        0,
        AppPoint { x: (second.rect.left - pad) as f64, y: (second.rect.top - pad) as f64 },
        AppPoint { x: (second.rect.right + pad) as f64, y: (second.rect.bottom + pad) as f64 },
        true,
    );
    assert!(app.tab_mut().selection.selected.is_none());
    let objects: Vec<usize> = app.tab_mut().selection.group.iter().map(|s| s.object).collect();
    assert!(
        objects.contains(&first.object) && objects.contains(&second.object),
        "the extended marquee lost the picture already selected: {objects:?}"
    );
}

/// **Dragging the group moves every member by the same amount.**
#[test]
fn dragging_the_group_moves_every_member_by_the_same_amount() {
    let mut app = app("pictures.pdf");
    app.submit("editobject");
    let pictures = app.tab_mut().doc.as_ref().expect("open").session.images_on(0).expect("images");
    app.tab_mut().selection.group = pictures
        .iter()
        .map(|p| Selected { page: 0, object: p.object, rect: p.rect, what: "the picture" })
        .collect();

    app.finish_group_grab(Grab { handle: None, from: AppPoint { x: 0.0, y: 0.0 }, by: (12.0, -7.0) }, 1.0);

    let after = app.tab_mut().doc.as_ref().expect("open").session.images_on(0).expect("images");
    for before in &pictures {
        let now = after.iter().find(|i| i.object == before.object).expect("still on the page");
        assert!(
            (now.rect.left - before.rect.left - 12.0).abs() < 0.5
                && (now.rect.top - before.rect.top + 7.0).abs() < 0.5,
            "object {} moved to {:?} from {:?}",
            before.object,
            now.rect,
            before.rect
        );
    }
    assert_eq!(app.tab_mut().selection.group.len(), 2, "the group should still hold both, at their new spots");
}

/// **Dragging one of the group's own handles resizes every member about
/// the same shared anchor** — the group's own bounding box, the same
/// way a single object's own handle anchors on its own opposite corner.
#[test]
fn resizing_the_group_scales_every_member_about_the_same_anchor() {
    let mut app = app("pictures.pdf");
    app.submit("editobject");
    let pictures = app.tab_mut().doc.as_ref().expect("open").session.images_on(0).expect("images");
    app.tab_mut().selection.group = pictures
        .iter()
        .map(|p| Selected { page: 0, object: p.object, rect: p.rect, what: "the picture" })
        .collect();
    let bounds = app.group_bounds(0).expect("bounds");
    let (w, h) = (bounds.right - bounds.left, bounds.bottom - bounds.top);

    // Drag the bottom-right handle out by the group's own width and
    // height — doubling it, anchored at the group's top-left.
    app.finish_group_grab(
        Grab {
            handle: Some(Handle::BottomRight),
            from: AppPoint { x: bounds.right as f64, y: bounds.bottom as f64 },
            by: (w, h),
        },
        1.0,
    );

    let after = app.tab_mut().doc.as_ref().expect("open").session.images_on(0).expect("images");
    for before in &pictures {
        let now = after.iter().find(|i| i.object == before.object).expect("still on the page");
        let want_left = bounds.left + (before.rect.left - bounds.left) * 2.0;
        let want_top = bounds.top + (before.rect.top - bounds.top) * 2.0;
        assert!(
            (now.rect.left - want_left).abs() < 1.0 && (now.rect.top - want_top).abs() < 1.0,
            "object {} did not scale about the group's own anchor: now {:?}, wanted left {want_left} top {want_top}",
            before.object,
            now.rect
        );
    }
    assert_eq!(app.tab_mut().selection.group.len(), 2, "the group should still hold both, at their new sizes");
    let bounds_after = app.group_bounds(0).expect("bounds");
    assert!(
        (bounds_after.right - bounds_after.left - w * 2.0).abs() < 1.0,
        "the group's own bounds should have doubled: was {w}, now {}",
        bounds_after.right - bounds_after.left
    );
}

/// **Deleting the group removes every member** — highest object index
/// first, so the second removal is not reading a page whose earlier
/// objects have already shifted down underneath it.
#[test]
fn deleting_the_group_removes_every_member() {
    let mut app = app("pictures.pdf");
    app.submit("editobject");
    let before = app.tab_mut().doc.as_ref().expect("open").session.drawn_objects(0).expect("objects");
    assert_eq!(before.len(), 5, "the fixture should draw five things");
    let pictures = app.tab_mut().doc.as_ref().expect("open").session.images_on(0).expect("images");
    assert_eq!(pictures.len(), 2);
    app.tab_mut().selection.group = pictures
        .iter()
        .map(|p| Selected { page: 0, object: p.object, rect: p.rect, what: "the picture" })
        .collect();

    app.delete_group();

    assert!(app.tab_mut().selection.group.is_empty(), "the group should be spent after deleting it");
    let after = app.tab_mut().doc.as_ref().expect("open").session.images_on(0).expect("images");
    assert!(after.is_empty(), "both pictures should be gone: {after:?}");
    let remaining = app.tab_mut().doc.as_ref().expect("open").session.drawn_objects(0).expect("objects");
    assert_eq!(remaining.len(), 3, "only the two pictures should have been removed");
    assert!(said(&app).contains("2 things removed"), "{}", said(&app));
}

/// **Reported from use: copying a group left the clipboard untouched, so
/// `paste` quietly put down whatever had been copied before it instead.**
/// `copy_object_selection` had a branch for the single-item `selected` but
/// none for `DocTab::group`, so copying more than one thing fell all the
/// way through to the text-selection fallback's "nothing selected" and a
/// stale `object_clipboard` from an earlier copy survived, unreplaced.
#[test]
fn copying_a_group_overwrites_whatever_was_copied_before_it() {
    let mut app = app("pictures.pdf");
    app.submit("editobject");
    let pictures = app.tab_mut().doc.as_ref().expect("open").session.images_on(0).expect("images");
    assert_eq!(pictures.len(), 2);

    // An earlier, single-item copy — the "previously selected object" that
    // must not survive the group copy below.
    app.tab_mut().selection.selected =
        Some(Selected { page: 0, object: pictures[0].object, rect: pictures[0].rect, what: "the picture" });
    assert!(app.copy_object_selection(), "the first, single copy should succeed");

    app.tab_mut().selection.selected = None;
    app.tab_mut().selection.group = pictures
        .iter()
        .map(|p| Selected { page: 0, object: p.object, rect: p.rect, what: "the picture" })
        .collect();

    assert!(app.copy_object_selection(), "copying a multi-object group must succeed, not silently do nothing");

    app.paste_object_selection();
    app.place_paste_ghost(0, AppPoint { x: 500.0, y: 500.0 });

    let placed = app.tab_mut().doc.as_ref().expect("open").session.placed_image_marks(0).expect("marks");
    assert_eq!(placed.len(), 2, "both group members should have been pasted, not just the one copied earlier");
    assert!(
        (placed[0].rect.left - placed[1].rect.left).abs() > 1.0 || (placed[0].rect.top - placed[1].rect.top).abs() > 1.0,
        "the group's own two members should not have landed on top of each other: {:?}",
        placed
    );
}

/// **Reported from use: a short label PDFium reports as several adjacent
/// word-runs came back out of copy/paste as visual noise, each run
/// overlapping its neighbour.** `copy_object_selection`'s group branch
/// offset every member from its own rect's *centre*, regardless of kind —
/// right for a picture or shape, since `place_clipboard_content` centres
/// those on the point given, but `Text` starts from its own top-left (the
/// pen's origin), so every pasted run landed shifted left by about half its
/// own width, crowding into whatever sat to its left. Proven directly on
/// the stored offsets — not by reading glyph pixels back — since that is
/// exactly where the mismatch lived.
#[test]
fn a_text_groups_own_offsets_are_each_members_left_edge_not_its_centre() {
    let mut app = app("two-column.pdf");
    app.submit("editobject");
    let mut runs = app.tab_mut().doc.as_ref().expect("open").session.text_runs(0).expect("runs");
    runs.retain(|r| r.text.trim().chars().count() >= 2);
    runs.sort_by(|a, b| a.rect.left.total_cmp(&b.rect.left));
    let (a, b) = (runs[0].clone(), runs[1].clone());
    assert!(
        (a.rect.right - a.rect.left - (b.rect.right - b.rect.left)).abs() > 0.5,
        "need two differently-sized runs for a centre-vs-edge mix-up to show: {a:?} {b:?}"
    );

    app.tab_mut().selection.group = vec![
        Selected { page: 0, object: a.object, rect: a.rect, what: "the words" },
        Selected { page: 0, object: b.object, rect: b.rect, what: "the words" },
    ];
    let bounds = app.group_bounds(0).expect("bounds");
    let (cx, cy) = ((bounds.left + bounds.right) / 2.0, (bounds.top + bounds.bottom) / 2.0);

    assert!(app.copy_object_selection());
    let Some(ObjectClipboard::Group(items)) = app.object_clipboard.clone() else {
        panic!("expected a Group on the clipboard");
    };
    assert_eq!(items.len(), 2);
    for (sel, (content, dx, dy)) in [a, b].iter().zip(&items) {
        assert!(matches!(content, ObjectClipboard::Text { .. }), "expected text");
        assert!((*dx - (sel.rect.left - cx)).abs() < 0.01, "offset {dx} is not the left edge {}: centred instead?", sel.rect.left - cx);
        // The vertical half anchors on the run's own *baseline* (`origin.y`),
        // not `rect.top` — a box's top sits above its baseline by that run's
        // own ascent, which is not the same number for every run (see the
        // sibling test using a real ascender/x-height split for why that
        // matters in practice).
        assert!((*dy - (sel.origin.y - cy)).abs() < 0.01, "offset {dy} is not the baseline {}: boxed top instead?", sel.origin.y - cy);
    }
}

/// **Copying and pasting words each leave one content-free log line** — the
/// same rule `pick` lines already follow
/// (`pick_wiring_tests::every_click_writes_exactly_one_content_free_pick_line_to_the_session_log`):
/// a PDF someone is editing may hold confidential text, so the session log
/// — which gets read back to diagnose a report like "the paste doesn't look
/// like what I copied" — must carry the *shape* of what moved (character
/// count, size, face) and never the words themselves.
#[test]
fn copying_and_pasting_words_logs_their_shape_but_never_their_text() {
    let mut app = app("two-column.pdf");
    let dir = std::env::temp_dir().join(format!("pagify-test-copy-paste-log-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    app.session_log = pagify_shell::session_log::SessionLog::start_in(dir.clone());
    let log_path = app.session_log.path().expect("the temp dir is writable").to_path_buf();

    app.submit("editobject");
    let runs = app.tab_mut().doc.as_ref().expect("open").session.text_runs(0).expect("runs");
    let target = runs.iter().find(|r| r.text.trim().chars().count() > 5).cloned().expect("a run with words");
    app.tab_mut().selection.selected = Some(Selected { page: 0, object: target.object, rect: target.rect, what: "the words" });

    assert!(app.copy_object_selection(), "the selected run should have been copied");
    assert!(app.start_paste_ghost(None), "a copy should have something to pick up");
    app.place_paste_ghost(0, AppPoint { x: 500.0, y: 500.0 });

    let lines: Vec<serde_json::Value> = std::fs::read_to_string(&log_path)
        .expect("the log file exists")
        .lines()
        .map(|l| serde_json::from_str(l).expect("valid json"))
        .collect();
    let copy_line = lines.iter().find(|l| l["kind"] == "copy").expect("a copy line").clone();
    let paste_line = lines.iter().find(|l| l["kind"] == "paste").expect("a paste line").clone();

    let words: Vec<&str> = target.text.split_whitespace().filter(|w| w.chars().count() >= 4).collect();
    for line in [&copy_line, &paste_line] {
        let text = line["text"].as_str().expect("text field");
        for word in &words {
            assert!(!text.contains(word), "the log line holds the page's own word {word:?}: {text}");
        }
    }
    let chars = target.text.trim().chars().count().to_string();
    assert!(
        copy_line["text"].as_str().unwrap().contains(&format!("chars={chars}")),
        "the copy line should carry the character count, content-free: {copy_line}"
    );
    assert!(
        paste_line["text"].as_str().unwrap().contains(&format!("chars={chars}")),
        "the paste line should carry the character count, content-free: {paste_line}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// smaller thing is what was aimed at.
#[test]
fn a_run_of_words_can_be_moved_across_the_page() {
    let mut app = app("two-column.pdf");
    let before = app.tab_mut().doc.as_ref().expect("open").session.text_runs(0).expect("runs");
    let target = before
        .iter()
        .find(|r| r.text.trim().chars().count() > 5)
        .cloned()
        .expect("a run with words");

    let told = app
        .move_thing(
            0,
            AppPoint {
                x: ((target.rect.left + target.rect.right) / 2.0) as f64,
                y: ((target.rect.top + target.rect.bottom) / 2.0) as f64,
            },
            AppPoint {
                x: ((target.rect.left + target.rect.right) / 2.0 + 30.0) as f64,
                y: ((target.rect.top + target.rect.bottom) / 2.0 + 18.0) as f64,
            },
            false,
        )
        .expect("moved");
    assert!(told.contains("the words"), "{told}");

    let after = app.tab_mut().doc.as_ref().expect("open").session.text_runs(0).expect("runs");
    let now = after
        .iter()
        .find(|r| r.object == target.object)
        .expect("the run vanished");
    assert!(
        (now.rect.left - target.rect.left - 30.0).abs() < 0.5
            && (now.rect.top - target.rect.top - 18.0).abs() < 0.5,
        "it did not go where it was sent: {:?} then {:?}",
        target.rect,
        now.rect
    );

    // And every other run stayed put — the whole reason moving is done this
    // way rather than by rewriting the page.
    for was in &before {
        if was.object == target.object {
            continue;
        }
        let still = after.iter().find(|r| r.object == was.object).expect("a run vanished");
        assert!(
            (still.rect.left - was.rect.left).abs() < 0.5
                && (still.rect.top - was.rect.top).abs() < 0.5,
            "moving one run shifted another"
        );
    }
}

/// **Clicking into a run of words splits it into characters and selects
/// just the one that was clicked** — not the whole sentence, which used
/// to be the only thing Edit Object could ever select or move.
///
/// `text-lines.pdf`, not `two-column.pdf`: its lines sit far enough
/// apart that each one is its own paragraph of one — see
/// `pagify_shell::blocks` — so a click still drills straight to a
/// single letter instead of picking up a multi-line group.
#[test]
fn clicking_a_run_splits_it_and_selects_one_letter() {
    let mut app = app("text-lines.pdf");
    app.submit("editobject");
    let before_runs = app.tab_mut().doc.as_ref().expect("open").session.text_runs(0).expect("runs");
    let target = before_runs
        .iter()
        .find(|r| r.text.trim().chars().count() > 5)
        .cloned()
        .expect("a run with words");
    let before_objects = app.tab_mut().doc.as_ref().expect("open").session.drawn_objects(0).expect("objects").len();

    // A quarter of the way in rather than dead centre — the exact
    // midpoint of a run with an even split can land in the gap between
    // two characters (a space, say) rather than inside either one.
    let at = AppPoint {
        x: (target.rect.left + (target.rect.right - target.rect.left) * 0.25) as f64,
        y: ((target.rect.top + target.rect.bottom) / 2.0) as f64,
    };
    app.select_thing_at(0, at);

    let sel = app.tab_mut().selection.selected.clone().expect("something should be selected");
    assert_eq!(sel.what, "the letter", "should have split down to one letter:\n{}", said(&app));
    let width = sel.rect.right - sel.rect.left;
    let run_width = target.rect.right - target.rect.left;
    assert!(
        width < run_width * 0.6,
        "the selection should be about one letter wide, not the whole run: {width} of {run_width}"
    );

    let after_objects =
        app.tab_mut().doc.as_ref().expect("open").session.drawn_objects(0).expect("objects").len();
    assert!(after_objects > before_objects, "the run should have split into more objects");

    // Clicking the very same spot again is a no-op split — still one
    // letter selected, not an error and not a second split on top of it.
    app.select_thing_at(0, at);
    assert_eq!(app.tab_mut().selection.selected.as_ref().map(|s| s.what), Some("the letter"));
    let again = app.tab_mut().doc.as_ref().expect("open").session.drawn_objects(0).expect("objects").len();
    assert_eq!(again, after_objects, "clicking an already-split letter should change nothing further");
}

/// Clicking bare paper says so rather than moving whatever is nearest.
#[test]
fn moving_nothing_says_there_is_nothing_there() {
    let mut app = app("two-column.pdf");
    let told = app
        .move_thing(0, AppPoint { x: 4.0, y: 4.0 }, AppPoint { x: 40.0, y: 40.0 }, false)
        .expect_err("there is nothing in the corner");
    assert!(told.contains("nothing to move"), "{told}");
}

/// Putting something back where it already is is a slip, not an edit.
#[test]
fn moving_something_nowhere_is_refused() {
    let mut app = app("two-column.pdf");
    let runs = app.tab_mut().doc.as_ref().expect("open").session.text_runs(0).expect("runs");
    let target = runs.first().cloned().expect("a run");
    let at = AppPoint {
        x: ((target.rect.left + target.rect.right) / 2.0) as f64,
        y: ((target.rect.top + target.rect.bottom) / 2.0) as f64,
    };
    assert!(app.move_thing(0, at, at, false).is_err());
}

/// **The editor is set in the document's own face.**
///
/// Typing into a field drawn in the program's typeface is editing a copy of
/// the words; typing in the document's is editing the words. The face comes
/// out of the file itself — and where the file only *names* a font rather
/// than carrying it, there is nothing to install and the editor falls back
/// without complaint.
#[test]
fn picking_a_run_asks_for_the_face_it_is_drawn_in() {
    let mut app = app("two-column.pdf");
    let runs = app.tab_mut().doc.as_ref().expect("open").session.text_runs(0).expect("runs");
    let target = runs
        .iter()
        .find(|r| r.text.trim().chars().count() > 4)
        .cloned()
        .expect("a run");

    // What the document actually carries for that run.
    let carried = app.tab_mut()
        .doc
        .as_ref()
        .expect("open")
        .session
        .run_font_data(0, target.object)
        .expect("asked");

    app.pick_text_run(
        0,
        AppPoint {
            x: ((target.rect.left + target.rect.right) / 2.0) as f64,
            y: ((target.rect.top + target.rect.bottom) / 2.0) as f64,
        },
    )
    .expect("picked");

    match carried {
        Some(bytes) => {
            assert!(
                pdf_core::pdf::embed::metrics(&bytes).is_some(),
                "the run's font came back as something no reader could parse"
            );
            assert!(
                app.pending_face.is_some() || app.editor_face.is_some(),
                "the editor did not ask for the face the words are drawn in"
            );
        }
        None => {
            // A named font — nothing in the file to install.
            assert!(app.editor_face.is_none(), "it installed a face that is not there");
        }
    }

    // And it is never used before egui has had a frame to build it.
    assert!(
        !app.editor_face_ready,
        "the face was marked usable on the frame it was asked for"
    );
}

/// **A click that just misses a word still picks it.**
///
/// Measured at the zoom real documents open at: the median run is 5.1
/// pixels tall in one of them and 6.7 in another. Requiring a click to land
/// inside that band means missing constantly, and a miss is reported as
/// there being no text — which reads as the tool being broken.
#[test]
fn a_click_just_outside_a_word_still_picks_it() {
    let mut app = app("text-lines.pdf");
    let runs = app.tab_mut().doc.as_ref().expect("open").session.text_runs(0).expect("runs");
    let target = runs
        .iter()
        .find(|r| r.text.trim().chars().count() > 4)
        .cloned()
        .expect("a run");

    // Just above the top of the box — a near miss, not a wild one.
    let just_above = AppPoint {
        x: ((target.rect.left + target.rect.right) / 2.0) as f64,
        y: (target.rect.top - 1.0) as f64,
    };
    app.pick_text_run(0, just_above).expect("a near miss should still pick");
    assert_eq!(
        app.tab_mut().edit.editing_run.as_ref().map(|e| e.object),
        Some(target.object),
        "it picked a different run"
    );

    // And a click nowhere near anything is still nothing.
    let mut app = app;
    app.tab_mut().edit.editing_run = None;
    let miles_away = AppPoint {
        x: (target.rect.left) as f64,
        y: (target.rect.bottom + 200.0) as f64,
    };
    assert!(
        app.pick_text_run(0, miles_away).is_err(),
        "it picked a run from the other end of the page"
    );
}

/// **Editing the words must not move the words.**
///
/// Reported from use, twice, with a screenshot: a paragraph came back with
/// one line drawn over the line above it in a different font, and its own
/// place left empty. The engine has a path that swaps the characters in the
/// content stream and touches nothing else — this checks the app actually
/// reaches it.
#[test]
fn editing_the_words_of_a_run_leaves_every_other_run_where_it_was() {
    // Not `two-column.pdf`: its lines are close enough together that
    // they now open as a paragraph (see `pick_paragraph`), and this
    // test is specifically about the single-run path. Widely separated
    // lines, so the run this picks is its own paragraph of one.
    let mut app = app("text-lines.pdf");
    let before = app.tab_mut().doc.as_ref().expect("open").session.text_runs(0).expect("runs");
    assert!(before.len() >= 3, "the fixture has too little text to be a test");

    // A run with words in it, not a stray space.
    let (at, target) = before
        .iter()
        .enumerate()
        .find(|(_, r)| r.text.trim().chars().count() > 8)
        .map(|(i, r)| (i, r.clone()))
        .expect("a run with words in it");

    let middle = AppPoint {
        x: ((target.rect.left + target.rect.right) / 2.0) as f64,
        y: ((target.rect.top + target.rect.bottom) / 2.0) as f64,
    };
    app.pick_text_run(0, middle).expect("picked");
    let picked = app.tab_mut().edit.editing_run.as_ref().expect("editing").object;

    app.tab_mut().edit.editing_run.as_mut().expect("editing").buffer = "Replaced".into();
    app.apply_editing_page();

    let after = app.tab_mut().doc.as_ref().expect("open").session.text_runs(0).expect("runs");

    // Either it made the change or it refused; what it must not do is make
    // it and disturb the page.
    if said(&app).contains("cannot be changed") || said(&app).contains("left alone") {
        eprintln!("this run was refused, which is the honest other answer");
        return;
    }

    assert_eq!(after.len(), before.len(), "the page gained or lost a run:\n{}", said(&app));
    for (was, now) in before.iter().zip(after.iter()) {
        if was.object == picked {
            // The one that was edited. Its words changed; its place did not.
            assert!(
                (was.rect.top - now.rect.top).abs() < 0.6,
                "the edited run moved: was at {}, now at {}",
                was.rect.top,
                now.rect.top
            );
            assert!(
                (was.size - now.size).abs() < 0.2,
                "the edited run changed size: {} then {}",
                was.size,
                now.size
            );
            continue;
        }
        assert_eq!(was.text, now.text, "a run nobody touched changed its words");
        assert!(
            (was.rect.top - now.rect.top).abs() < 0.6
                && (was.rect.left - now.rect.left).abs() < 0.6,
            "a run nobody touched moved: {:?} then {:?}",
            was.rect,
            now.rect
        );
    }
}

/// **Changing how a run looks must not move it either.**
///
/// The second half of the same bug. A restyle *is* PDFium's to apply, and
/// it obeys the position it is given — which the app was taking from the
/// top of the run's box while the engine meant the baseline. Those differ
/// by the font's ascent, so every restyled run rose onto the line above.
#[test]
fn changing_a_runs_colour_leaves_it_where_it_was() {
    let mut app = app("two-column.pdf");
    let before = app.tab_mut().doc.as_ref().expect("open").session.text_runs(0).expect("runs");
    let target = before
        .iter()
        .find(|r| r.text.trim().chars().count() > 8)
        .cloned()
        .expect("a run with words in it");

    // The baseline is below the top of the box, never the same point.
    assert!(
        target.origin.y > target.rect.top,
        "a run's origin is not its box's top: {:?} vs {}",
        target.origin,
        target.rect.top
    );

    app.pick_text_run(
        0,
        AppPoint {
            x: ((target.rect.left + target.rect.right) / 2.0) as f64,
            y: ((target.rect.top + target.rect.bottom) / 2.0) as f64,
        },
    )
    .expect("picked");

    // Only the colour, and the words left alone.
    let edit = app.tab_mut().edit.editing_run.as_mut().expect("editing");
    let object = edit.object;
    edit.style.color = Some(pdf_core::document::Color { r: 200, g: 0, b: 0, a: 255 });
    app.apply_editing_page();

    let after = app.tab_mut().doc.as_ref().expect("open").session.text_runs(0).expect("runs");
    let Some(now) = after.iter().find(|r| r.object == object) else {
        panic!("the run vanished:\n{}", said(&app));
    };
    assert!(
        (now.origin.y - target.origin.y).abs() < 0.6,
        "a recoloured run moved: baseline was {}, now {}",
        target.origin.y,
        now.origin.y
    );
    assert!(
        (now.origin.x - target.origin.x).abs() < 0.6,
        "a recoloured run moved sideways: {} then {}",
        target.origin.x,
        now.origin.x
    );
}

/// **Two lines, and the prompt says which one this is.** As with the two
/// rectangles: one can be picked up again, and this one cannot.
#[test]
fn the_sign_line_says_it_is_not_the_drawing_one() {
    let mut app = app("two-column.pdf");
    app.submit("signline");

    assert!(
        matches!(app.tab_mut().tool.as_ref().map(|t| &t.kind), Some(Tool::SignLine)),
        "the tool was not armed:\n{}",
        said(&app)
    );
    assert!(said(&app).contains("not a drawing"), "{}", said(&app));
}

/// It rules onto the page rather than adding something to select.
#[test]
fn a_ruled_line_goes_onto_the_page() {
    let mut app = app("two-column.pdf");
    let before = app.tab_mut().doc.as_ref().expect("open").session.annotations(0).expect("read").len();

    let told = app
        .stamp_line(0, AppPoint { x: 40.0, y: 100.0 }, AppPoint { x: 300.0, y: 100.0 })
        .expect("line");
    assert!(told.contains("page 1"), "{told}");
    assert_eq!(
        app.tab_mut().doc.as_ref().expect("open").session.annotations(0).expect("read").len(),
        before,
        "it added an annotation instead of ruling on the page"
    );
}

/// **The three PagiSign marks are the three `fillsign` makes.**
///
/// Pressed from the ribbon, each arms the tool rather than reporting that
/// it does not exist — which is what all three did until this was noticed.
#[test]
fn the_pagisign_mark_buttons_arm_the_tool_they_name() {
    for (verb, mark) in [
        ("signcheck", pdf_core::document::FillMark::Tick),
        ("signcross", pdf_core::document::FillMark::Cross),
        ("signdot", pdf_core::document::FillMark::Dot),
    ] {
        let mut app = app("two-column.pdf");
        app.submit(verb);
        assert!(
            matches!(
                app.tab_mut().tool.as_ref().map(|t| &t.kind),
                Some(Tool::Fill(armed)) if *armed == mark
            ),
            "{verb} did not arm the mark it names:\n{}",
            said(&app)
        );
    }
}

/// **Two rectangles, and the prompt says which one this is.**
///
/// The drawing tool makes a mark that can be picked up again; this one
/// fills a form in. Identical prompts would be how somebody reaches for the
/// wrong one and only finds out when they try to move it.
#[test]
fn the_sign_rectangle_says_it_is_not_the_drawing_one() {
    let mut app = app("two-column.pdf");
    app.submit("signrectangle");

    assert!(
        matches!(app.tab_mut().tool.as_ref().map(|t| &t.kind), Some(Tool::SignRectangle)),
        "the tool was not armed:\n{}",
        said(&app)
    );
    let told = said(&app);
    assert!(told.contains("not a drawing"), "{told}");
}

/// It draws on the page rather than adding something to select.
#[test]
fn a_sign_rectangle_goes_onto_the_page() {
    let mut app = app("two-column.pdf");
    let before = app.tab_mut().doc.as_ref().expect("open").session.annotations(0).expect("read").len();

    let told = app
        .stamp_box(0, AppPoint { x: 40.0, y: 40.0 }, AppPoint { x: 200.0, y: 120.0 })
        .expect("box");
    assert!(told.contains("page 1"), "{told}");

    assert_eq!(
        app.tab_mut().doc.as_ref().expect("open").session.annotations(0).expect("read").len(),
        before,
        "it added an annotation instead of drawing on the page"
    );
}

/// Two corners in the same place are a slip, not an instruction.
#[test]
fn a_sign_rectangle_with_no_size_is_refused() {
    let mut app = app("two-column.pdf");
    let told = app
        .stamp_box(0, AppPoint { x: 40.0, y: 40.0 }, AppPoint { x: 40.0, y: 40.0 })
        .expect_err("refused");
    assert!(told.contains("no size"), "{told}");
}

/// **The readout on a document with nothing on it says so, line by line.**
///
/// The absence of a password is the status, not the absence of a line about
/// one — somebody checking whether a file is protected needs to be told it
/// is not.
#[test]
fn the_status_of_an_unprotected_document_says_it_is_unprotected() {
    let app = app("two-column.pdf");
    let lines = app.document_status().join("\n");

    assert!(lines.contains("password: none"), "{lines}");
    assert!(lines.contains("anyone who has the file"), "{lines}");
    assert!(lines.contains("signed: no"), "{lines}");
    // Permissions are not mentioned at all: in an unencrypted file they are
    // decoration, and naming them would suggest they hold.
    assert!(!lines.contains("permissions:"), "it claimed permissions apply: {lines}");
    // And it says which tool looks for hidden data instead of pretending to
    // have looked.
    assert!(lines.contains("`hiddendata`"), "{lines}");
}

/// **A password says who can open the file, and what they may then do.**
#[test]
fn the_status_says_what_a_password_permits() {
    let mut app = app("two-column.pdf");
    app.submit("secure readonly");
    app.answer_passcode("Correct-Horse-99-Battery");
    app.answer_passcode("Correct-Horse-99-Battery");

    let lines = app.document_status().join("\n");
    assert!(lines.contains("AES-256"), "{lines}");
    assert!(lines.contains("any PDF reader"), "{lines}");
    assert!(lines.contains("not written until you save"), "{lines}");
    assert!(lines.contains("permissions: reading only"), "{lines}");
}

/// **Secure Plus keeps no permissions, and says so rather than dropping
/// them.** Found by audit: `secure readonly` under Secure Plus stored
/// "everything permitted" and the dialog said nothing.
#[test]
fn secure_readonly_under_secure_plus_is_refused_not_silently_dropped() {
    let mut app = app("two-column.pdf");
    app.submit("secure readonly");
    app.tab_mut().secure_state.password_plus = true;
    app.answer_passcode("Correct-Horse-99-Battery");
    app.answer_passcode("Correct-Horse-99-Battery");

    let told = said(&app);
    assert!(told.contains("keeps no permissions"), "{told}");
    assert!(told.contains("nothing was set"), "{told}");
    let doc = app.tab_mut().doc.as_ref().expect("doc");
    assert!(!doc.session.is_secured(), "a password was set with the restriction dropped");
}

/// And the window says it before a password is typed, with the button
/// held back.
#[test]
fn the_password_window_says_secure_plus_would_lose_the_restriction() {
    use eframe::App as _;
    use egui_kittest::kittest::Queryable;
    use egui_kittest::Harness;

    let app = app("two-column.pdf");
    let mut h = Harness::builder()
        .with_size(egui::vec2(1200.0, 900.0))
        .build_ui_state(
            |ui, app: &mut PagifyApp| {
                let mut frame = eframe::Frame::_new_kittest();
                app.ui(ui, &mut frame);
            },
            app,
        );
    h.run_steps(2);
    h.state_mut().submit("secure readonly");
    h.run();
    h.get_by_label("Secure Plus").click();
    h.run();
    // The command line already quotes "reading only"; the warning is the
    // one that names both.
    h.get_by_label_contains("keeps no permissions, so \"reading only\"");
}

/// Secure Plus says the thing that makes it different.
#[test]
fn the_status_of_a_secure_plus_document_says_only_pagify_opens_it() {
    let mut app = app("two-column.pdf");
    app.submit("secure");
    app.tab_mut().secure_state.password_plus = true;
    app.answer_passcode("Correct-Horse-99-Battery");
    app.answer_passcode("Correct-Horse-99-Battery");

    let lines = app.document_status().join("\n");
    assert!(lines.contains("Secure Plus"), "{lines}");
    assert!(lines.contains("only Pagify"), "{lines}");
}

/// **The headline question: is it signed, and does the signature still
/// hold?**
#[test]
fn the_status_of_a_signed_document_says_whether_it_still_holds() {
    let certificate = std::path::Path::new(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../../rust/pdf_core/fixtures/test-signer-sm2.p12"),
    );
    if !certificate.is_file() {
        eprintln!("skipping: no test certificate");
        return;
    }

    let mut app = app("two-column.pdf");
    app.submit(&format!("certify {}", certificate.display()));
    app.answer_passcode("pagify");

    let lines = app.document_status().join("\n");
    assert!(lines.contains("signed by CN=Pagify SM2 Test Signer,O=Pagify"), "{lines}");
    assert!(lines.contains("unchanged since it was signed"), "{lines}");
    // The second answer travels with the first, apart from it: the test
    // signer is self-signed, and no root Pagify ships vouches for it — so
    // the line says so, and there is no tick.
    assert!(lines.contains("; not issued by a root Pagify trusts"), "{lines}");
    assert!(!lines.contains("✓"), "a signer nobody vouches for earned the tick: {lines}");
}

/// **A third-party document looks as it always did**: its signature is
/// reported as not verified — the scheme named — never as changed, never
/// as tampered, and with no trust said about a signer nobody checked.
#[test]
fn the_status_of_a_document_signed_elsewhere_says_not_verified_never_tampered() {
    let app = app("rsa-signed.pdf");
    let lines = app.document_status().join("\n");
    assert!(lines.contains("could not be checked"), "{lines}");
    assert!(lines.contains("RSA-PKCS#1v1.5 / SHA-256"), "{lines}");
    assert!(lines.contains("does not verify"), "{lines}");
    assert!(!lines.contains("CHANGED"), "{lines}");
    assert!(!lines.contains("NOT VALID"), "{lines}");
    assert!(!lines.contains("root Pagify trusts"), "trust was said of a signer nobody checked: {lines}");
    assert!(!lines.contains("✓"), "{lines}");
}

/// **A signature placed but not applied is reported as what it still is.**
#[test]
fn the_status_names_signatures_that_are_placed_but_not_applied() {
    let (mut app, path) = with_signature_pad("two-column.pdf", "status");
    app.save_drawn_signature("mine", &scrawl()).expect("kept");
    app.place_signature(0, AppPoint { x: 100.0, y: 400.0 }).expect("placed");

    let lines = app.document_status().join("\n");
    assert!(lines.contains("1 drawn signature"), "{lines}");
    assert!(lines.contains("anyone can delete"), "{lines}");
    assert!(lines.contains("`applysignatures`"), "{lines}");

    // And once applied it is no longer a loose end.
    app.submit("applysignatures");
    let lines = app.document_status().join("\n");
    assert!(!lines.contains("drawn signature"), "it is still reported as placed: {lines}");

    let _ = std::fs::remove_file(&path);
}

/// **A placed signature can be applied, and then it is the page.**
#[test]
fn applying_signatures_makes_them_part_of_the_page() {
    let (mut app, path) = with_signature_pad("two-column.pdf", "apply");
    app.save_drawn_signature("mine", &scrawl()).expect("kept");
    app.place_signature(0, AppPoint { x: 100.0, y: 400.0 }).expect("placed");

    let session = &app.tab_mut().doc.as_ref().expect("open").session;
    assert_eq!(session.signature_marks(0).expect("read").len(), 1);

    app.submit("applysignatures");
    let told = said(&app);
    assert!(told.contains("part of the page"), "{told}");
    // The two things somebody needs to know and would not guess.
    assert!(told.contains("select or delete"), "{told}");
    assert!(told.contains("without saving"), "it did not say the way back: {told}");

    let session = &app.tab_mut().doc.as_ref().expect("open").session;
    assert!(session.signature_marks(0).expect("read").is_empty());
    assert!(session.annotations(0).expect("read").is_empty(), "the annotation is still there");

    let _ = std::fs::remove_file(&path);
}

/// With nothing placed it says so, which is not a failure.
#[test]
fn applying_with_nothing_placed_says_how_to_place_one() {
    let mut app = app("two-column.pdf");
    app.submit("applysignatures");
    let told = said(&app);
    assert!(told.contains("no signatures are placed"), "{told}");
    assert!(told.contains("`signature`"), "it did not say what places one: {told}");
}

/// **Choosing one changes what a click places, and it survives.**
#[test]
fn managing_signatures_chooses_which_one_a_click_places() {
    let (mut app, path) = with_signature_pad("two-column.pdf", "choose");
    app.save_drawn_signature("work", &scrawl()).expect("kept");
    app.save_drawn_signature("personal", &scrawl()).expect("kept");
    assert_eq!(app.signatures.current().map(|s| s.name.as_str()), Some("personal"));

    app.submit("managesignatures use work");
    assert_eq!(app.signatures.current().map(|s| s.name.as_str()), Some("work"));
    assert!(said(&app).contains("now places"), "{}", said(&app));

    // And it is on disk, not only in this window.
    let kept = pagify_shell::signatures::Signatures::load_from(&path);
    assert_eq!(kept.current().map(|s| s.name.as_str()), Some("work"));

    let _ = std::fs::remove_file(&path);
}

/// A name that is not there says which names are.
#[test]
fn choosing_a_signature_that_is_not_there_says_what_is() {
    let (mut app, path) = with_signature_pad("two-column.pdf", "choose-missing");
    app.save_drawn_signature("work", &scrawl()).expect("kept");

    app.submit("managesignatures use nobody");
    let told = said(&app);
    assert!(told.contains("no signature called"), "{told}");
    assert!(told.contains("work"), "it did not say what there is: {told}");
    assert_eq!(app.signatures.current().map(|s| s.name.as_str()), Some("work"));

    let _ = std::fs::remove_file(&path);
}

/// **Deleting says the thing that matters about it: it does not come back.**
#[test]
fn forgetting_a_signature_says_that_undo_does_not_reach_it() {
    let (mut app, path) = with_signature_pad("two-column.pdf", "forget");
    app.save_drawn_signature("work", &scrawl()).expect("kept");
    app.save_drawn_signature("personal", &scrawl()).expect("kept");

    app.submit("managesignatures delete work");
    let told = said(&app);
    assert!(told.contains("undo"), "it did not say undo will not help: {told}");
    assert!(app.signatures.find("work").is_none(), "it is still there");
    // The remaining one is a real signature, not a dangling choice.
    assert_eq!(app.signatures.current().map(|s| s.name.as_str()), Some("personal"));
    assert!(
        pagify_shell::signatures::Signatures::load_from(&path).find("work").is_none(),
        "it came back from disk"
    );

    let _ = std::fs::remove_file(&path);
}

/// Renaming acts on the current one, and will not take a name in use.
#[test]
fn renaming_will_not_quietly_destroy_another_drawing() {
    let (mut app, path) = with_signature_pad("two-column.pdf", "rename");
    app.save_drawn_signature("work", &scrawl()).expect("kept");
    app.save_drawn_signature("personal", &scrawl()).expect("kept");

    app.submit("managesignatures rename work");
    assert!(said(&app).contains("already called that"), "{}", said(&app));
    assert_eq!(app.signatures.entries().len(), 2, "a drawing was destroyed");

    app.submit("managesignatures rename my mark");
    assert!(app.signatures.find("my mark").is_some(), "{}", said(&app));
    assert!(app.signatures.find("personal").is_none());

    let _ = std::fs::remove_file(&path);
}

/// The list says which one a click uses, because that is the question.
#[test]
fn listing_signatures_says_which_one_is_current() {
    let (mut app, path) = with_signature_pad("two-column.pdf", "list");
    app.submit("managesignatures list");
    assert!(said(&app).contains("no signatures drawn yet"), "{}", said(&app));

    app.save_drawn_signature("work", &scrawl()).expect("kept");
    app.submit("managesignatures list");
    assert!(said(&app).contains("the one a click places"), "{}", said(&app));

    let _ = std::fs::remove_file(&path);
}

/// Opening the panel with nothing in it says so rather than showing a
/// window somebody has to work out is empty.
#[test]
fn the_signature_panel_opens_and_says_when_it_is_empty() {
    let (mut app, path) = with_signature_pad("two-column.pdf", "panel");
    app.submit("managesignatures");
    assert!(app.signature_list.is_some(), "the panel did not open");
    assert!(said(&app).contains("no signatures yet"), "{}", said(&app));
    let _ = std::fs::remove_file(&path);
}

/// A smudge is refused, and says what to do instead.
#[test]
fn too_little_to_be_a_signature_is_refused() {
    let (mut app, path) = with_signature_pad("two-column.pdf", "smudge");
    let told = app.save_drawn_signature("mine", &[vec![(4.0, 4.0)]]).expect_err("refused");
    assert!(told.contains("draw across the pad"), "{told}");
    assert!(app.signatures.is_empty(), "a smudge was kept anyway");
    let _ = std::fs::remove_file(&path);
}

/// **Placing before drawing says the word that makes one.**
#[test]
fn placing_with_nothing_drawn_names_the_way_to_draw_one() {
    let (mut app, path) = with_signature_pad("two-column.pdf", "no-signature");
    let told = app
        .place_signature(0, AppPoint { x: 100.0, y: 400.0 })
        .expect_err("nothing to place");
    assert!(told.contains("signature draw"), "{told}");
    let _ = std::fs::remove_file(&path);
}

/// **An unsigned document is not a failed check.**
///
/// The distinction a validator gets wrong most often. "No signatures" is a
/// fact about the file, not a verdict against it, and it must not be
/// dressed up as one.
#[test]
fn validating_an_unsigned_document_reports_none_rather_than_failing() {
    let mut app = app("two-column.pdf");
    app.submit("validate");

    let told = said(&app);
    assert!(told.contains("no signatures"), "{told}");
    for alarming in ["altered", "invalid", "failed"] {
        assert!(!told.contains(alarming), "an unsigned file was called {alarming}: {told}");
    }
}

/// **And a signed one says what it checked — and what it did not.**
///
/// The arithmetic says the bytes have not changed. It does not say who
/// signed them, and the line a user reads has to carry that or the check
/// claims more than it did.
#[test]
fn validating_a_signed_document_separates_unchanged_from_who_signed_it() {
    let certificate = std::path::Path::new(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../../rust/pdf_core/fixtures/test-signer-sm2.p12"),
    );
    if !certificate.is_file() {
        eprintln!("skipping: no test certificate");
        return;
    }

    let mut app = app("two-column.pdf");
    app.submit(&format!("certify {}", certificate.display()));
    app.answer_passcode("pagify");
    assert!(said(&app).contains("signed as"), "it did not sign: {}", said(&app));

    app.submit("validate");
    let told = said(&app);
    assert!(told.contains("unchanged since it was signed"), "{told}");
    // The signer named is the certificate's subject — evidence — and the
    // second answer follows, apart from the first: nobody vouches for it.
    assert!(told.contains("by CN=Pagify SM2 Test Signer,O=Pagify"), "it did not name the certificate: {told}");
    assert!(
        told.contains("; not issued by a root Pagify trusts"),
        "it did not say who vouches for the signer: {told}"
    );
    assert!(!told.contains("✓"), "{told}");
}

/// A certificate that is not there is said so before a password is asked
/// for.
#[test]
fn signing_with_a_missing_certificate_says_so_at_once() {
    let mut app = app("two-column.pdf");
    app.submit("certify /tmp/there-is-no-such-certificate.p12");
    assert!(app.tab_mut().secure_state.awaiting_password.is_none(), "it asked for a password anyway");
    assert!(said(&app).contains("no such file"), "{}", said(&app));
}

/// **A tick lands where it was clicked, and stays in the file.**
///
/// Drawn rather than typed, because neither a tick nor a cross is in the
/// fonts a PDF can rely on — typing one gets a blank box.
#[test]
fn fillsign_puts_a_mark_where_it_was_clicked() {
    let mut app = app("pages-ladder.pdf");
    app.submit("fillsign tick");
    assert!(
        matches!(app.tab_mut().tool.as_ref().map(|t| &t.kind), Some(Tool::Fill(_))),
        "the tool was not armed:\n{}",
        said(&app)
    );

    let said = app
        .stamp_mark(
            0,
            pdf_core::document::FillMark::Tick,
            AppPoint { x: 60.0, y: 60.0 },
        )
        .expect("tick");
    assert!(said.contains("tick"), "{said}");

    // It reached the document itself, not a layer waiting to be committed:
    // the page renders differently now.
    let raster = app.tab_mut()
        .doc
        .as_ref()
        .expect("doc")
        .session
        .render_page(0, 2.0)
        .expect("render");
    let inked = raster
        .pixels
        .chunks_exact(4)
        .filter(|p| p[0] < 200 || p[1] < 200 || p[2] < 200)
        .count();
    assert!(inked > 0, "the page draws nothing at all after a tick");
}

/// Bare `fillsign` is the typing half, and says what else is on offer.
#[test]
fn bare_fillsign_offers_typing_and_the_marks() {
    let mut app = app("pages-ladder.pdf");
    app.submit("fillsign");
    let said = said(&app);
    assert!(said.contains("tick"), "it did not mention the marks: {said}");
    assert!(said.contains("addtext"), "it did not say how to write: {said}");
}

/// **A marking says what it is not.**
///
/// Somebody reaching for this may believe it protects the document. It does
/// not — it says what you intend and stops nobody — and the line that comes
/// back is where they will read that.
#[test]
fn marking_a_document_says_it_does_not_enforce_anything() {
    let mut app = app("pages-ladder.pdf");
    app.submit("sensitivity");
    assert!(said(&app).contains("not marked"), "{}", said(&app));

    app.submit("sensitivity confidential");
    let told = said(&app);
    assert!(told.contains("CONFIDENTIAL"), "it did not say what it marked: {told}");
    assert!(
        told.contains("does not enforce"),
        "it let a marking pass for protection: {told}"
    );
    assert!(told.contains("secure"), "it did not point at what does withhold: {told}");

    // And it reads back.
    app.submit("sensitivity");
    assert!(said(&app).contains("CONFIDENTIAL"), "{}", said(&app));

    // And comes off.
    app.submit("sensitivity none");
    app.submit("sensitivity");
    assert!(said(&app).contains("not marked"), "{}", said(&app));
}

/// **Whiteout says what it did not do.**
///
/// Somebody reaching for it may believe it removes what it covers. The one
/// place they are certain to read is the line that comes back, so that is
/// where it says otherwise — and the prompt says it too, before they drag.
#[test]
fn whiteout_says_it_covers_rather_than_removes() {
    let mut app = app("two-column.pdf");
    app.submit("whiteout");

    let armed = said(&app);
    assert!(
        app.tab_mut().tool.is_some(),
        "the tool was not armed:\n{armed}"
    );

    let size = app.tab_mut().doc.as_ref().expect("doc").session.page_size(0).expect("size");
    let before = app
        .characters(0)
        .map(pagify_shell::reader::Characters::text)
        .unwrap_or_default();

    let said = app
        .whiteout(
            0,
            AppPoint { x: 10.0, y: 10.0 },
            AppPoint { x: size.width_pt as f64 - 10.0, y: 120.0 },
        )
        .expect("whiteout");
    assert!(said.contains("still in the file"), "it did not say what it left: {said}");
    assert!(said.contains("redact"), "it did not point at the tool that destroys: {said}");

    // And it really did leave them.
    app.tab_mut().doc.as_mut().unwrap().caches.text = None;
    let after = app
        .characters(0)
        .map(pagify_shell::reader::Characters::text)
        .unwrap_or_default();
    assert_eq!(after, before, "a whiteout removed text it only covered");
}

/// **Words drawn through a form come out, and the file agrees.** The
/// audit's probe: a card number inside a form XObject, which automatic
/// redaction used to paint over and call gone. Now it is cut out of the
/// form's own stream — the caption before it stays.
#[test]
fn smartredact_cuts_words_out_of_a_form_and_the_saved_file_agrees() {
    let mut app = app("secret-in-form.pdf");
    app.submit("smartredact");
    assert!(said(&app).contains("4111"), "the card number was not found: {}", said(&app));

    app.submit("smartredact redact");
    let told = said(&app);
    assert!(told.contains("2 redacted — gone for good"), "{told}");

    let out = std::env::temp_dir().join(format!("pagify-smartredact-{}.pdf", std::process::id()));
    app.submit(&format!("saveas {}", out.display()));
    let text = pdf_core::registry::exclusive(|| {
        use pdf_core::document::Document;
        let doc = pdf_core::document::pdfium_doc::PdfiumDocument::open_path(
            out.to_str().expect("path"),
            None,
        )
        .expect("reopen");
        let text = doc.page(0).expect("page").text().expect("text");
        text
    });
    let _ = std::fs::remove_file(&out);
    assert!(!text.contains("4111"), "the card number survived: {text}");
    assert!(text.contains("Card on file:"), "the caption went with it: {text}");
    assert!(!text.contains("7946"), "the telephone number survived: {text}");
}

/// A form drawn twice on the page: each drawing is its own find, and each
/// is cut from a copy of its own, so both come out and the captions stay.
#[test]
fn smartredact_cuts_each_drawing_of_a_form_drawn_twice() {
    let mut app = app("secret-in-form-twice.pdf");
    app.submit("smartredact redact");
    let told = said(&app);
    assert!(told.contains("3 redacted — gone for good"), "{told}");

    let out = std::env::temp_dir().join(format!("pagify-smartredact-twice-{}.pdf", std::process::id()));
    app.submit(&format!("saveas {}", out.display()));
    let text = pdf_core::registry::exclusive(|| {
        use pdf_core::document::Document;
        let doc = pdf_core::document::pdfium_doc::PdfiumDocument::open_path(
            out.to_str().expect("path"),
            None,
        )
        .expect("reopen");
        let text = doc.page(0).expect("page").text().expect("text");
        text
    });
    let _ = std::fs::remove_file(&out);
    assert!(!text.contains("4111"), "a drawing kept the number: {text}");
    assert_eq!(text.matches("Card on file:").count(), 2, "a caption went too: {text}");
}

/// A form inside a form: the chain is followed down, and the words come
/// out of the inner form.
#[test]
fn smartredact_cuts_words_out_of_a_form_inside_a_form() {
    let mut app = app("secret-in-nested-form.pdf");
    app.submit("smartredact redact");
    let told = said(&app);
    assert!(told.contains("2 redacted — gone for good"), "{told}");
}

/// **And what still cannot be reached is still said, never claimed
/// gone.** Words that are pixels: a picture of the number under invisible
/// text spelling it, as an OCR layer is written. The text comes out; the
/// picture is reported as still holding the words.
#[test]
fn smartredact_says_what_it_could_not_reach_instead_of_claiming_it_gone() {
    let mut app = app("secret-in-pixels-form.pdf");
    app.submit("smartredact redact");
    let told = said(&app);
    assert!(!told.contains("gone for good. Save"), "it claimed everything was gone: {told}");
    assert!(told.contains("NOT fully cleared"), "{told}");
    assert!(told.contains("4111") && told.contains("image"), "{told}");
    assert!(told.contains("1 gone for good"), "the telephone number is counted: {told}");

    let out = std::env::temp_dir().join(format!("pagify-smartredact-pixels-{}.pdf", std::process::id()));
    app.submit(&format!("saveas {}", out.display()));
    let text = pdf_core::registry::exclusive(|| {
        use pdf_core::document::Document;
        let doc = pdf_core::document::pdfium_doc::PdfiumDocument::open_path(
            out.to_str().expect("path"),
            None,
        )
        .expect("reopen");
        let text = doc.page(0).expect("page").text().expect("text");
        text
    });
    let _ = std::fs::remove_file(&out);
    assert!(!text.contains("4111"), "the invisible text survived: {text}");
    assert!(!text.contains("7946"), "the telephone number survived: {text}");
}

/// **A recording's name is a name, and the script lands in Pagify's own
/// folder.** Found by audit: `record ../../x` wrote `../../x.json`
/// relative to wherever the process happened to be.
#[test]
fn a_recording_named_like_a_path_is_refused_and_a_good_one_lands_in_pagifys_folder() {
    let mut app = app("two-column.pdf");
    app.submit("record ../../x");
    assert!(said(&app).contains("not a name"), "{}", said(&app));
    assert!(!app.recorder.is_recording(), "it recorded under a path");

    let dir = std::env::temp_dir().join(format!("pagify-scripts-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    app.scripts_dir = Some(dir.clone());
    app.submit("record stamp every page");
    app.submit("rotate 90");
    app.submit("stop");
    let told = said(&app);
    assert!(told.contains("written to"), "{told}");
    let written = dir.join("stamp-every-page.json");
    assert!(written.is_file(), "the script is not where it was said to be: {told}");
    assert!(!std::path::Path::new("stamp-every-page.json").exists(), "it also wrote beside the process");
    let _ = std::fs::remove_dir_all(&dir);
}

/// **What Pagify keeps about somebody can be cleared, and they are told
/// where the rest is.** Found by audit: the recent list, texts and
/// signatures were kept in plaintext with no way to clear them from
/// inside the app, and nothing said where they were.
#[test]
fn clearhistory_forgets_the_recent_list_and_says_where_the_rest_is_kept() {
    let mut app = app("two-column.pdf");
    app.recent.record(std::path::Path::new("/tmp/something.pdf"), 3, 0);
    assert!(!app.recent.entries.is_empty());
    app.submit("clearhistory");
    let told = said(&app);
    assert!(app.recent.entries.is_empty(), "the list was not cleared: {told}");
    assert!(told.contains("recent-documents list is gone"), "{told}");
    assert!(told.contains("signatures.json"), "it did not say what else is kept: {told}");
    assert!(told.contains("outlined_fonts.json"), "it did not mention outlined_fonts.json: {told}");
}

/// **Smart Redact reports before it acts, and says what it found.**
///
/// What it finds are candidates. Blacking them out unread is how a price
/// somebody meant to send gets hidden and a name nobody recognised does
/// not, so the reporting half has to name them.
#[test]
fn smartredact_names_what_it_found_before_touching_anything() {
    let mut app = app("two-column.pdf");
    app.submit("smartredact");

    let said = said(&app);
    // The fixture has nothing checkable on it, so it must say so plainly
    // rather than leaving somebody wondering whether it ran.
    assert!(
        said.contains("nothing found"),
        "it did not say what happened: {said}"
    );
    assert!(
        said.contains("card numbers") || said.contains("addresses"),
        "it did not say what it looks for: {said}"
    );
}

/// **`unsecure` then `save` leaves a file with no password on it.** Found
/// by audit: the default save appended to the encrypted file, and the
/// password the person had been told was off was still on.
#[test]
fn unsecure_then_save_writes_a_file_anyone_can_open() {
    let out = std::env::temp_dir().join(format!("pagify-unsecure-{}.pdf", std::process::id()));
    let _ = std::fs::remove_file(&out);
    std::fs::copy(fixture("encrypted.pdf"), &out).expect("copy the fixture");

    let mut app = PagifyApp::new(Some(out.to_str().expect("path")));
    let path = app.awaiting_open().expect("it did not ask for a password");
    app.answer_open_password(&path, "pagify");
    assert!(app.tab_mut().doc.is_some(), "the fixture did not open");

    app.submit("unsecure");
    assert!(said(&app).contains("password is off"), "{}", said(&app));
    app.submit("save");
    assert!(said(&app).contains("saved"), "{}", said(&app));

    let bytes = std::fs::read(&out).expect("read it back");
    let _ = std::fs::remove_file(&out);
    let file = pdf_core::pdf::File::parse(&bytes).expect("parse");
    assert!(file.trailer().get(b"Encrypt").is_none(), "the saved file still carries /Encrypt");
    pdf_core::registry::exclusive(|| {
        pdf_core::document::pdfium_doc::PdfiumDocument::open_bytes(bytes.clone(), None)
    })
    .expect("the saved file still wants a password");
}

/// **Changing a password and saving in place really changes it.**
///
/// Reported from use: set a new password, save, close, reopen — and the
/// *old* password still opened it. The engine was right all along; the save
/// was being refused by the guard meant for putting a first password over
/// somebody's only plain copy, so the file was never written and the error
/// scrolled past.
#[test]
fn changing_a_password_and_saving_over_the_file_takes_effect() {
    let out = std::env::temp_dir().join("pagify-change-in-place.pdf");
    let _ = std::fs::remove_file(&out);
    std::fs::copy(fixture("encrypted.pdf"), &out).expect("copy the fixture");

    let mut app = PagifyApp::new(Some(out.to_str().expect("path")));
    let path = app.awaiting_open().expect("it did not ask for a password");
    app.answer_open_password(&path, "pagify");
    assert!(app.tab_mut().doc.is_some(), "the fixture did not open");

    // Change it, exactly as a person would.
    app.submit("secure");
    app.answer_passcode("pagify");
    app.answer_passcode("New-Password-99!");
    app.answer_passcode("New-Password-99!");
    assert!(app.tab_mut().doc.as_ref().expect("doc").session.is_secured());

    // Saved over the file itself — which is what "save" means.
    app.submit("save");
    let said = said(&app);
    assert!(
        !said.contains("saveas"),
        "saving over a file that already had a password was refused:\n{said}"
    );

    let bytes = std::fs::read(&out).expect("read it back");
    assert!(
        pdf_core::registry::exclusive(|| pdf_core::document::pdfium_doc::PdfiumDocument::open_bytes(bytes.clone(), Some("pagify")))
            .is_err(),
        "the old password still opens it"
    );
    pdf_core::registry::exclusive(|| pdf_core::document::pdfium_doc::PdfiumDocument::open_bytes(
        bytes,
        Some("New-Password-99!"),
    ))
    .expect("the new password does not open it");
    let _ = std::fs::remove_file(&out);
}

/// And the guard still holds where it matters: a first password over
/// somebody's only plain copy is not written without their say-so.
#[test]
fn a_first_password_over_a_plain_original_is_still_held_back() {
    let out = std::env::temp_dir().join("pagify-first-password.pdf");
    let _ = std::fs::remove_file(&out);
    std::fs::copy(fixture("two-column.pdf"), &out).expect("copy the fixture");

    let mut app = PagifyApp::new(Some(out.to_str().expect("path")));
    app.submit("secure");
    app.answer_passcode("Correct-Horse-99-Battery");
    app.answer_passcode("Correct-Horse-99-Battery");
    app.submit("save");

    assert!(app.tab_mut().secure_state.asking_to_secure.is_some(), "it wrote over the only plain copy without asking");
    let bytes = std::fs::read(&out).expect("read");
    assert!(
        pdf_core::registry::exclusive(|| pdf_core::document::pdfium_doc::PdfiumDocument::open_bytes(bytes, None)).is_ok(),
        "the plain original was encrypted in place after all"
    );
    let _ = std::fs::remove_file(&out);
}

/// **A password set and not saved is unsaved work.**
///
/// Reported from use: a password is set on the document in memory and only
/// reaches the file on the next save, so closing without saving threw it
/// away — silently, after somebody had typed it twice and been told it was
/// set. The guard that already exists for unsaved marks now covers it.
#[test]
fn closing_with_a_password_set_asks_before_throwing_it_away() {
    let mut app = app("two-column.pdf");
    app.submit("secure");
    app.answer_passcode("Correct-Horse-99-Battery");
    app.answer_passcode("Correct-Horse-99-Battery");
    assert!(app.tab_mut().doc.as_ref().expect("doc").session.is_secured());

    // Nothing was drawn, so the old guard would have let this straight
    // through.
    assert!(app.unsaved().is_none(), "the fixture has unsaved marks after all");
    assert!(app.unsaved_password(), "a set password was not counted as unsaved");

    app.submit("close");
    assert!(
        app.tab_mut().closing.is_some(),
        "it closed without asking:\n{}",
        said(&app)
    );
    assert!(app.tab_mut().doc.is_some(), "the document was closed anyway");
}

/// **A document with a password is changed, not refused.**
///
/// Reported from use twice: first that being refused was no help, then
/// that the route the refusal recommended — `unsecure` then `saveas` —
/// did not work at all, because PDFium keeps a document's encryption when
/// it saves one it opened encrypted. Both are fixed; this pins the flow.
#[test]
fn a_documents_password_can_be_changed_by_giving_the_current_one() {
    let mut app = PagifyApp::new(Some(&fixture("encrypted.pdf")));
    let path = app.awaiting_open().expect("it did not ask for a password");
    app.answer_open_password(&path, "pagify");
    assert!(app.tab_mut().doc.is_some(), "the fixture did not open");

    app.submit("secure");
    assert!(
        matches!(app.tab_mut().secure_state.awaiting_password, Some(Awaiting::SecureCurrent(_))),
        "it did not ask for the current password:\n{}",
        said(&app)
    );

    // A wrong one gets nowhere, and says so in the window.
    app.answer_passcode("not the password");
    assert!(
        matches!(app.tab_mut().secure_state.awaiting_password, Some(Awaiting::SecureCurrent(_))),
        "a wrong current password was accepted"
    );
    assert_eq!(
        app.tab_mut().secure_state.password_problem.as_deref(),
        Some("That is not this document's password.")
    );

    // The right one moves on to choosing a new one, under the rule.
    app.answer_passcode("pagify");
    assert!(
        matches!(app.tab_mut().secure_state.awaiting_password, Some(Awaiting::Secure(_))),
        "the right password did not lead to choosing a new one:\n{}",
        said(&app)
    );

    app.answer_passcode("Correct-Horse-99-Battery");
    app.answer_passcode("Correct-Horse-99-Battery");
    assert!(
        app.tab_mut().doc.as_ref().expect("doc").session.is_secured(),
        "the new password was not recorded:\n{}",
        said(&app)
    );

    // And it is the new password that opens what gets written.
    let out = std::env::temp_dir().join("pagify-changed-password.pdf");
    let _ = std::fs::remove_file(&out);
    app.submit(&format!("saveas {}", out.display()));
    let bytes = std::fs::read(&out).expect("nothing was written");

    assert!(
        pdf_core::registry::exclusive(|| pdf_core::document::pdfium_doc::PdfiumDocument::open_bytes(bytes.clone(), Some("pagify")))
            .is_err(),
        "the old password still opens it"
    );
    pdf_core::registry::exclusive(|| pdf_core::document::pdfium_doc::PdfiumDocument::open_bytes(
        bytes,
        Some("Correct-Horse-99-Battery"),
    ))
    .expect("the new password does not open it");
    let _ = std::fs::remove_file(&out);
}

/// **A password is asked for twice, and a slip sets nothing.**
///
/// Every other passcode in this program guards something the document
/// still contains, so a typo costs a retry. This one *is* the document:
/// somebody who mistypes it finds out when they next open the file, by
/// which time nothing can be done. Reported from use exactly that way.
#[test]
fn a_mismatched_confirmation_sets_no_password() {
    let mut app = app("two-column.pdf");
    app.submit("secure");
    app.answer_passcode("Correct-Horse-99-Battery");
    app.answer_passcode("Correct-Horse-99-Batteryx");

    assert!(
        !app.tab_mut().doc.as_ref().expect("doc").session.is_secured(),
        "it set a password the person only typed once, and differently"
    );
    let said = said(&app);
    assert!(said.contains("did not match"), "it did not say why: {said}");
    assert!(said.contains("nothing was set"), "it left the outcome unclear: {said}");
}

/// **The password you typed is the password that opens it.**
///
/// Reported from use: a document secured in the app would not open with
/// the password that was set. Everything either side of this had been
/// tested — the cipher against PDFium, the verb against the prompt — but
/// not the whole way through, which is where a password gets trimmed,
/// re-encoded, or quietly replaced.
#[test]
fn a_password_typed_in_the_app_opens_the_file_it_saved() {
    // All strong enough to be accepted — the rule applies to choosing one,
    // and this is about whether what was chosen is what opens the file.
    for password in [
        "Correct-Horse-99-Battery",
        "A space in it 1! and more",
        "MiXeD-CASE-1234-abcdef",
        "punctuation!?-_9Abcdefgh",
    ] {
        let mut app = app("two-column.pdf");
        app.submit("secure");
        // The way the window does it — every password is asked for there
        // now, so the command box no longer takes one.
        app.answer_passcode(password);
        app.answer_passcode(password);
        assert!(
            app.tab_mut().doc.as_ref().expect("doc").session.is_secured(),
            "{password:?} was not recorded"
        );

        let out = std::env::temp_dir().join(format!(
            "pagify-secured-{}.pdf",
            password.replace(|c: char| !c.is_ascii_alphanumeric(), "-")
        ));
        let _ = std::fs::remove_file(&out);
        app.submit(&format!("saveas {}", out.display()));
        assert!(out.is_file(), "{password:?}: nothing was written:\n{}", said(&app));

        // The file must refuse everyone else and open for this password.
        let bytes = std::fs::read(&out).expect("read back");
        assert!(
            pdf_core::registry::exclusive(|| pdf_core::document::pdfium_doc::PdfiumDocument::open_bytes(bytes.clone(), None))
                .is_err(),
            "{password:?}: the file opened with no password at all"
        );
        pdf_core::registry::exclusive(|| pdf_core::document::pdfium_doc::PdfiumDocument::open_bytes(bytes, Some(password)))
            .unwrap_or_else(|e| {
                panic!("{password:?}: the password that was set does not open it: {e}")
            });
        let _ = std::fs::remove_file(&out);
    }
}

/// **A password is never written over the only copy quietly.**
///
/// Securing a document and saving it makes a file nobody can read without
/// the password — including the person who typed it, if they mistype or
/// forget it. Doing that to the original in place, on a plain `save`, is
/// irreversible. It happened to a real document during this program's own
/// development. It used to be refused outright; now it is asked, because a
/// refusal with one exit left somebody unable to save at all — see
/// `asking_to_secure`. Either way nothing is written until a person says.
#[test]
fn saving_over_the_original_is_not_done_quietly_while_a_password_is_waiting() {
    let mut app = app("two-column.pdf");
    app.submit("secure");
    app.answer_passcode("Correct-Horse-99-Battery");
    app.answer_passcode("Correct-Horse-99-Battery");
    assert!(app.tab_mut().doc.as_ref().expect("doc").session.is_secured());

    app.submit("save");
    assert!(app.tab_mut().secure_state.asking_to_secure.is_some(), "no question was raised:\n{}", said(&app));
    assert!(
        !said(&app).contains("saved "),
        "the file was written without an answer:\n{}",
        said(&app)
    );
}

/// **Secure asks for a password, and says what it will do.**
#[test]
fn the_secure_verb_asks_for_a_password() {
    let mut app = app("two-column.pdf");
    app.submit("secure");

    assert!(matches!(app.tab_mut().secure_state.awaiting_password, Some(Awaiting::Secure(_))));
    let said = said(&app);
    assert!(
        said.contains("password"),
        "it did not say what it wanted: {said}"
    );
}

/// **The permissions reach the prompt, and are said out loud.**
///
/// A person turning printing off should be told that is what they did —
/// and told plainly that it is a courtesy readers honour rather than a
/// lock, which is all the PDF specification promises.
#[test]
fn secure_carries_the_permissions_it_was_given() {
    let mut only = app("two-column.pdf");
    only.submit("secure readonly");

    let told = said(&only);
    let Some(Awaiting::Secure(options)) = only.tab().secure_state.awaiting_password else {
        panic!("it did not ask for a password:\n{told}");
    };
    assert!(!options.printing && !options.copying);
    assert!(told.contains("reading only"), "{told}");

    // One at a time, combinable.
    let mut combined = app("two-column.pdf");
    combined.submit("secure noprint nocopy");
    let Some(Awaiting::Secure(options)) = combined.tab().secure_state.awaiting_password else {
        panic!("it did not ask for a password");
    };
    assert!(!options.printing && !options.copying);
    assert!(options.editing && options.annotating, "it forbade more than it was told to");
}

/// A word it does not know is refused, not ignored. Silently permitting
/// what somebody just tried to forbid is the worst outcome available.
#[test]
fn secure_refuses_a_permission_it_does_not_understand() {
    let mut app = app("two-column.pdf");
    app.submit("secure nopriting");

    assert!(app.tab_mut().secure_state.awaiting_password.is_none(), "it asked for a password anyway");
    let said = said(&app);
    assert!(said.contains("nopriting"), "it did not say what it could not read: {said}");
    assert!(said.contains("noprint"), "it did not suggest the right word: {said}");
}

/// And typing one records it, to be written on the next save.
#[test]
fn a_password_typed_at_the_prompt_is_recorded() {
    let mut app = app("two-column.pdf");
    app.submit("secure noprint");
    app.answer_passcode("Correct-Horse-99-Battery");
    app.answer_passcode("Correct-Horse-99-Battery");

    assert!(
        app.tab_mut().doc.as_ref().expect("doc").session.is_secured(),
        "the password was not recorded:\n{}",
        said(&app)
    );

    // And it can be taken back off.
    app.submit("unsecure");
    assert!(!app.tab_mut().doc.as_ref().expect("doc").session.is_secured());
}

/// **`lock` asks for a selection, not for two corners.**
///
/// Reported from use: being asked to click opposite corners of a rectangle
/// is a drawing gesture, and not what anyone reaches for when they mean
/// "hide these words".
#[test]
fn the_lock_verb_asks_for_a_selection_rather_than_arming_a_rectangle() {
    let mut app = app("text-lines.pdf");
    app.submit("lock");

    assert!(app.tab_mut().tool.is_none(), "it armed the rectangle tool anyway");
    let said = said(&app);
    assert!(said.contains("select"), "it did not say to select anything: {said}");
}

/// And with a selection already made, it locks that.
#[test]
fn the_lock_verb_takes_the_selection_that_is_already_there() {
    let mut app = app("text-lines.pdf");
    app.tab_mut().doc.as_mut().unwrap().caches.text = None;
    let chars = app.characters(0).expect("characters").clone();
    app.tab_mut().selection.text_selection = Some(0..chars.len().min(8));
    app.tab_mut().organize.selection_page = 0;

    app.submit("lock");
    assert!(
        matches!(app.tab_mut().secure_state.awaiting_password, Some(Awaiting::Lock { .. })),
        "it did not lock the selection:\n{}",
        said(&app)
    );
}

/// The rectangle is still there for what a selection cannot express — an
/// area of a scan has no text to select.
#[test]
fn the_lockarea_verb_still_arms_the_rectangle() {
    let mut app = app("text-lines.pdf");
    app.submit("lockarea");
    assert!(matches!(app.tab_mut().tool.as_ref().map(|t| &t.kind), Some(Tool::Lock)));
}

/// Nothing is locked, so unlock has nothing to ask about — and asking for a
/// passcode that cannot open anything trains people to type it at prompts
/// that do not need it.
#[test]
fn the_unlock_verb_declines_when_nothing_is_locked() {
    let mut app = app("text-lines.pdf");
    app.submit("unlock");
    assert!(app.tab_mut().secure_state.awaiting_password.is_none());
    assert!(said(&app).contains("nothing in this document is locked"));
}

/// **The passcode is asked for after the area is drawn**, so it is typed
/// once and spent immediately rather than held while the user aims.
#[test]
fn drawing_the_area_asks_for_a_passcode_and_locks_nothing_yet() {
    let mut app = app("text-lines.pdf");
    let before = page_text(&app, 0);

    app.tab_mut().tool = Some(ArmedTool {
        kind: Tool::Lock,
        page: 0,
        objects: Vec::new(),
        points: vec![AppPoint::new(FOX.0 as f64, FOX.1 as f64), AppPoint::new(FOX.2 as f64, FOX.3 as f64)],
    });
    app.resolve_tool();

    assert!(
        matches!(app.tab_mut().secure_state.awaiting_password, Some(Awaiting::Lock { page: 0, .. })),
        "it did not ask for a passcode"
    );
    assert_eq!(page_text(&app, 0), before, "it locked before it had a passcode");
}

/// Both halves, through the app: off the page, and back with the passcode.
#[test]
fn a_locked_area_hides_and_the_passcode_brings_it_back() {
    let mut app = app("text-lines.pdf");
    assert!(page_text(&app, 0).contains("The quick brown fox"), "control");

    app.lock_area(0, fox_area(), b"a good passcode", true).expect("lock");
    assert!(
        !page_text(&app, 0).contains("The quick brown fox"),
        "the words are still on the page"
    );

    let said = app.unlock(b"a good passcode").expect("unlock");
    assert!(said.contains("1 page"), "{said}");
    assert!(
        page_text(&app, 0).contains("The quick brown fox"),
        "the passcode did not bring them back"
    );
}

#[test]
fn the_wrong_passcode_brings_nothing_back() {
    let mut app = app("text-lines.pdf");
    app.lock_area(0, fox_area(), b"a good passcode", true).expect("lock");

    assert!(app.unlock(b"a bad passcode").is_err());
    assert!(
        !page_text(&app, 0).contains("The quick brown fox"),
        "a wrong passcode revealed the page anyway"
    );
}

/// **A lock with no passcode is not a lock.**
#[test]
fn an_empty_passcode_is_refused_before_anything_is_hidden() {
    let mut app = app("text-lines.pdf");
    let before = page_text(&app, 0);
    assert!(app.lock_area(0, fox_area(), b"", true).is_err());
    assert_eq!(page_text(&app, 0), before);
}

/// Unlocking goes through the command stack, so it undoes — and the undo
/// label names the page rather than the operation.
#[test]
fn unlocking_undoes() {
    let mut app = app("text-lines.pdf");
    app.lock_area(0, fox_area(), b"a good passcode", true).expect("lock");
    let hidden = page_text(&app, 0);

    app.unlock(b"a good passcode").expect("unlock");
    assert!(page_text(&app, 0).contains("The quick brown fox"));

    app.submit("undo");
    assert_eq!(page_text(&app, 0), hidden, "undo did not re-hide it:\n{}", said(&app));
}

/// **A whole-document unlock clears the badge it just restored.**
/// Reported from use: after `unlock` put a locked passage's words back
/// on the page, its padlock stayed — because the page-wide restore never
/// told the vault the passage it had just put back was no longer locked,
/// only `unlock_item` did that bookkeeping.
#[test]
fn a_whole_document_unlock_clears_the_area_badge_it_restored() {
    let mut app = app("text-lines.pdf");
    app.lock_area(0, fox_area(), b"a good passcode", true).expect("lock");
    assert_eq!(app.locked_items_on(0).len(), 1, "control: the badge should be there");

    app.unlock(b"a good passcode").expect("unlock");
    assert!(page_text(&app, 0).contains("The quick brown fox"));
    assert!(
        app.locked_items_on(0).is_empty(),
        "the badge outlived a whole-document unlock that already put the words back"
    );
}

/// **The passcode is asked for once per document, not once per lock.**
///
/// Reported from use: locking several things meant typing the same passcode
/// for each of them. Every lock in a document shares one vault and so one
/// passcode, so after the first the program was asking a question it
/// already had the answer to.
#[test]
fn a_second_lock_uses_the_passcode_the_first_one_was_given() {
    let mut app = app("text-lines.pdf");

    // The first lock asks, and is answered.
    app.tab_mut().secure_state.awaiting_password =
        Some(Awaiting::Lock { page: 0, shapes: vec![fox_area()], require_complete: true });
    app.answer_lock_passcode("a good passcode");
    assert!(
        !page_text(&app, 0).contains("The quick brown fox"),
        "the first lock did not take:\n{}",
        said(&app)
    );

    // The second does not ask at all.
    app.ask_or_reuse_passcode(Awaiting::LockPages(vec![0]), "should never be shown");
    assert!(
        app.tab_mut().secure_state.awaiting_password.is_none(),
        "it asked again for a passcode it already had"
    );
    assert!(
        !said(&app).contains("should never be shown"),
        "it showed the prompt anyway:\n{}",
        said(&app)
    );
    assert!(
        app.tab_mut().doc.as_ref().is_some_and(|d| !d.session.locked_pages().is_empty()),
        "the second lock did not happen:\n{}",
        said(&app)
    );
}

/// **Unlocking always asks.** Holding the passcode is for putting things
/// away, never for bringing them back — somebody at an unattended screen
/// must not be able to click a padlock open.
#[test]
fn a_held_passcode_does_not_unlock_anything() {
    let mut app = app("text-lines.pdf");
    app.tab_mut().secure_state.awaiting_password =
        Some(Awaiting::Lock { page: 0, shapes: vec![fox_area()], require_complete: true });
    app.answer_lock_passcode("a good passcode");
    assert!(app.tab_mut().secure_state.held_passcode.is_some(), "the passcode was not kept");

    // The gesture that brings something back still asks.
    app.submit("unlock");
    assert!(
        matches!(app.tab_mut().secure_state.awaiting_password, Some(Awaiting::Unlock)),
        "unlocking did not ask for the passcode:\n{}",
        said(&app)
    );
    assert!(
        !page_text(&app, 0).contains("The quick brown fox"),
        "the held passcode brought the words back without being typed:\n{}",
        said(&app)
    );
}

/// **A passcode that stops working is let go**, so the next lock asks
/// instead of failing silently for the rest of the session.
#[test]
fn a_passcode_that_is_refused_is_not_kept() {
    let mut app = app("text-lines.pdf");
    app.tab_mut().secure_state.awaiting_password =
        Some(Awaiting::Lock { page: 0, shapes: vec![fox_area()], require_complete: true });
    app.answer_lock_passcode("a good passcode");
    assert!(app.tab_mut().secure_state.held_passcode.is_some());

    // A wrong one, forced in the way a stale held passcode would arrive.
    app.tab_mut().secure_state.awaiting_password = Some(Awaiting::LockPages(vec![0]));
    app.answer_lock_passcode("the wrong passcode");
    assert!(
        app.tab_mut().secure_state.held_passcode.is_none(),
        "a refused passcode was kept, so every lock after it would fail quietly"
    );
}

/// **And it does not follow the reader to the next document.**
#[test]
fn closing_a_document_lets_go_of_its_passcode() {
    let mut app = app("text-lines.pdf");
    app.tab_mut().secure_state.awaiting_password =
        Some(Awaiting::Lock { page: 0, shapes: vec![fox_area()], require_complete: true });
    app.answer_lock_passcode("a good passcode");
    assert!(app.tab_mut().secure_state.held_passcode.is_some());

    app.submit("close!");
    assert!(app.tab_mut().doc.is_none(), "it did not close:\n{}", said(&app));
    assert!(app.tab_mut().secure_state.held_passcode.is_none(), "the passcode outlived the document");
}

/// **A selection over two lines locks the selection, not the two lines.**
///
/// The app used to collapse it into the smallest rectangle holding it,
/// which also holds the head of the first line and the tail of the last.
/// Reported from use as "the exact selected text isn't getting locked, the
/// whole line is" — and the engine was never the part that was wrong.
#[test]
fn locking_a_selection_across_lines_sends_the_lines_not_their_union() {
    let mut app = app("two-column.pdf");
    let page = app.tab_mut().view_state.page;
    let chars = app.characters(page).expect("characters");
    let text: String = chars.text();
    let phrase = "luminaire housing is formed from";
    let at = text.find(phrase).expect("the fixture phrase");
    let at = text[..at].chars().count();
    // Past the line break, so the selection is genuinely two lines.
    let to = at + phrase.chars().count() + 8;

    app.tab_mut().organize.selection_page = page;
    app.tab_mut().selection.text_selection = Some(at..to);
    app.lock_selection();

    match app.tab_mut().secure_state.awaiting_password.take() {
        Some(Awaiting::Lock { shapes, .. }) => {
            assert!(
                shapes.len() >= 2,
                "the selection was collapsed into {} shape(s) — the union is \
                 wider than the selection and takes words either side of it",
                shapes.len()
            );
        }
        other => panic!("no lock was armed: {other:?}"),
    }
}

/// **The layer rail says what the page draws, topmost first.**
#[test]
fn layers_lists_what_the_page_draws() {
    let mut app = app("pictures.pdf");
    assert!(!app.show_layers, "it should start closed");

    app.submit("layers");
    assert!(app.show_layers, "`layers` did not open it:\n{}", said(&app));

    let listed = app.layers_on(0).to_vec();
    assert_eq!(listed.len(), 5, "the fixture draws five things: {listed:#?}");
    assert!(
        listed.iter().any(|d| d.kind == pdf_core::document::DrawnKind::Picture),
        "no picture was listed: {listed:#?}"
    );

    app.submit("layers");
    assert!(!app.show_layers, "it did not close again");
}

/// **Restacking needs something picked**, and says so rather than guessing
/// which of the things on the page was meant.
#[test]
fn bringing_to_front_with_nothing_picked_says_to_pick_something() {
    let mut app = app("pictures.pdf");
    app.submit("bringtofront");
    assert!(
        said(&app).contains("pick something"),
        "it did not say what to do:\n{}",
        said(&app)
    );
}

/// **And with something picked, it goes to the front.**
#[test]
fn a_picked_layer_can_be_brought_to_the_front() {
    let mut app = app("pictures.pdf");
    let listed = app.layers_on(0).to_vec();
    // The bottom-most thing, which is the one a reader would be reaching
    // for when something has covered it.
    let bottom = listed.first().cloned().expect("something is drawn");

    app.tab_mut().selection.picked_layer = Some(bottom.object);
    app.submit("bringtofront");
    assert!(
        said(&app).contains("brought to the front"),
        "it did not restack:\n{}",
        said(&app)
    );

    // The list is read again, because the page has been rewritten.
    let now = app.layers_on(0).to_vec();
    assert_eq!(now.len(), listed.len(), "something left the page");
    assert_eq!(
        now.last().map(|d| d.label.clone()),
        Some(bottom.label.clone()),
        "it is not at the front: {now:#?}"
    );
    // And the pick follows it to its new place, so the next move acts on
    // the same thing rather than on whatever now sits at the old row.
    assert_eq!(
        app.tab_mut().selection.picked_layer,
        Some(now.len() - 1),
        "the pick did not follow the thing it was on"
    );
}

/// **A picture that has gone behind something can be got back.**
///
/// Reported from use: *"images are going behind layers I can't edit"*. The
/// fixture draws a picture and then paints a panel over it, which is the
/// ordinary layout of a brochure and the ordinary way a picture ends up
/// underneath something.
///
/// The whole route is asserted, because each half of it was wrong on its
/// own: right-clicking where the picture *was* points at the panel now
/// covering it, so the menu has to offer the **stack** under the pointer
/// rather than the top of it — otherwise "bring to front" raises the very
/// panel somebody is trying to get out from behind.
#[test]
fn a_picture_under_a_panel_can_be_found_and_brought_back() {
    let mut app = app("covered.pdf");
    let listed = app.layers_on(0).to_vec();
    assert_eq!(listed.len(), 3, "the fixture draws a picture, a panel and a line: {listed:#?}");

    let picture = listed
        .iter()
        .find(|d| d.kind == pdf_core::document::DrawnKind::Picture)
        .cloned()
        .expect("the picture");
    let panel = listed
        .iter()
        .find(|d| d.kind == pdf_core::document::DrawnKind::Shape)
        .cloned()
        .expect("the panel");
    assert!(
        listed.iter().position(|d| d.object == picture.object)
            < listed.iter().position(|d| d.object == panel.object),
        "the fixture should draw the picture first, so it is underneath"
    );

    // A point where the panel covers the picture — where somebody looking
    // for their picture would click.
    let middle = AppPoint {
        x: ((picture.rect.left.max(panel.rect.left) + picture.rect.right.min(panel.rect.right))
            / 2.0) as f64,
        y: ((picture.rect.top.max(panel.rect.top) + picture.rect.bottom.min(panel.rect.bottom))
            / 2.0) as f64,
    };

    // The stack under the pointer, topmost first: the panel is on top and
    // the picture is under it.
    let under = app.layers_under(0, middle);
    assert!(under.len() >= 2, "only one thing found under the pointer: {under:?}");
    let kinds: Vec<pdf_core::document::DrawnKind> =
        under.iter().filter_map(|i| listed.get(*i)).map(|d| d.kind).collect();
    // Topmost first, so the picture — drawn before everything else there —
    // is reported last, under whatever is covering it.
    assert_eq!(
        kinds.last(),
        Some(&pdf_core::document::DrawnKind::Picture),
        "the picture should be reported at the bottom of the stack: {kinds:?}"
    );
    assert!(
        kinds.iter().position(|k| *k == pdf_core::document::DrawnKind::Shape)
            < kinds.iter().position(|k| *k == pdf_core::document::DrawnKind::Picture),
        "the panel should be reported above the picture it covers: {kinds:?}"
    );

    // Pick the picture out of that stack and raise it.
    let at = *under
        .iter()
        .find(|i| listed.get(**i).is_some_and(|d| d.object == picture.object))
        .expect("the picture in the stack");
    app.pick_layer(0, at);
    app.restack_picked(pdf_core::document::Stacking::Front);
    assert!(
        said(&app).contains("brought to the front"),
        "it did not raise the picture:\n{}",
        said(&app)
    );

    // And it is now the last thing the page draws, which is what puts it on
    // top of the panel.
    let now = app.layers_on(0).to_vec();
    assert_eq!(
        now.last().map(|d| d.kind),
        Some(pdf_core::document::DrawnKind::Picture),
        "the picture is still not on top: {now:#?}"
    );
}

/// **A click selects; it does not move.**
///
/// Asked for from use: "clicking an object should select. Just select." A
/// click on bare paper clears the selection and takes nothing else.
#[test]
fn a_click_with_the_object_tool_selects_and_moves_nothing() {
    let mut app = app("covered.pdf");
    let before = app.tab_mut().doc.as_ref().expect("open").session.drawn_objects(0).expect("objects");
    app.submit("editobject");
    assert!(app.tab_mut().tool_state.object_tool.is_some(), "the tool did not arm:\n{}", said(&app));
    assert!(app.tab_mut().tool.is_none(), "the old two-click gesture is still armed");

    // Bare paper: nothing selected.
    assert!(!app.select_thing_at(0, AppPoint { x: 590.0, y: 780.0 }));
    assert!(app.tab_mut().selection.selected.is_none());

    // The panel's far corner, where nothing else is: selected, named, and
    // picked in the layer list — and the page untouched.
    let panel = before.iter().find(|d| d.kind == pdf_core::document::DrawnKind::Shape).expect("panel");
    let corner = AppPoint { x: (panel.rect.right - 20.0) as f64, y: (panel.rect.bottom - 20.0) as f64 };
    assert!(app.select_thing_at(0, corner));
    let sel = app.tab_mut().selection.selected.clone().expect("selected");
    assert_eq!(sel.what, "the shape");
    assert!(said(&app).contains("the shape selected"), "{}", said(&app));
    let picked = app.tab_mut().selection.picked_layer.and_then(|at| app.layers_on(0).get(at).cloned());
    assert_eq!(picked.map(|d| d.kind), Some(pdf_core::document::DrawnKind::Shape));
    let after = app.tab_mut().doc.as_ref().expect("open").session.drawn_objects(0).expect("objects");
    assert_eq!(after.len(), before.len());
    for (a, b) in before.iter().zip(&after) {
        assert_eq!(a.rect, b.rect, "selecting moved something");
    }
}

#[test]
fn distance_to_segment_is_zero_on_the_segment_itself() {
    assert_eq!(PagifyApp::distance_to_segment((5.0, 5.0), (0.0, 0.0), (10.0, 10.0)), 0.0);
}

#[test]
fn distance_to_segment_clamps_past_either_end() {
    // Past (10, 0), not off into where the infinite line would go —
    // the nearest point on the *segment* is its own endpoint.
    let d = PagifyApp::distance_to_segment((20.0, 0.0), (0.0, 0.0), (10.0, 0.0));
    assert_eq!(d, 10.0);
}

#[test]
fn distance_to_segment_measures_perpendicular_to_a_straight_run() {
    let d = PagifyApp::distance_to_segment((5.0, 3.0), (0.0, 0.0), (10.0, 0.0));
    assert_eq!(d, 3.0);
}

/// **The case this was built for**: a click near a long diagonal picks
/// the diagonal, even though a tiny decoy shape's own bounding box also
/// reaches the click — a bounding-box-only test would have picked the
/// decoy for being smaller, regardless of which one the click actually
/// landed near.
#[test]
fn distance_to_outline_finds_the_nearest_contour_not_the_smallest_box() {
    let diagonal = vec![vec![(0.0, 0.0), (100.0, 100.0)]];
    let decoy = vec![vec![(94.0, 94.0), (98.0, 98.0), (98.0, 94.0)]];
    let click = (50.0, 51.0); // essentially on the diagonal

    let to_diagonal = PagifyApp::distance_to_outline(click, &diagonal);
    let to_decoy = PagifyApp::distance_to_outline(click, &decoy);
    assert!(
        to_diagonal < to_decoy,
        "the click should read as nearer the diagonal: {to_diagonal} vs {to_decoy}"
    );
}

/// `flatten_segments` (and `object_outline` on top of it) must not
/// inherit `build_outline`'s own "at least three points" filter — a
/// plain two-point line is the single most common shape a CAD drawing
/// draws, and that filter exists for glyph-recognition noise, not for
/// "is this geometry real". Caught before this ever shipped: an earlier
/// draft of `object_outline` called `build_outline` directly and this
/// test failed with an empty outline.
#[test]
fn a_two_point_line_is_not_dropped_as_too_short_to_be_real() {
    let mut app = app("single-page.pdf");
    app.tab_mut().view_state.page = 0;
    app.stamp_line(0, AppPoint { x: 100.0, y: 700.0 }, AppPoint { x: 500.0, y: 700.0 })
        .expect("stamp a line");

    let outline = app
        .tab_mut()
        .doc
        .as_ref()
        .expect("open")
        .session
        .object_outline(0, 0)
        .expect("read the line's own geometry");
    assert!(!outline.is_empty(), "a two-point line must still produce a contour to hit-test against");

    app.submit("editobject");
    assert!(
        app.select_thing_at(0, AppPoint { x: 300.0, y: 700.0 }),
        "clicking the middle of the line should select it"
    );
    let sel = app.tab_mut().selection.selected.clone().expect("selected");
    assert_eq!(sel.what, "the shape");
}

/// **Dragging the body of the selection moves it, once, on release.**
#[test]
fn dragging_a_selection_moves_it_when_let_go() {
    let mut app = app("covered.pdf");
    app.submit("editobject");
    let panel = app.layers_on(0).iter().find(|d| d.kind == pdf_core::document::DrawnKind::Shape).cloned().expect("panel");
    let corner = AppPoint { x: (panel.rect.right - 20.0) as f64, y: (panel.rect.bottom - 20.0) as f64 };
    assert!(app.select_thing_at(0, corner));
    let sel = app.tab_mut().selection.selected.clone().expect("selected");

    // Mid-drag, nothing has changed in the document.
    app.tab_mut().selection.grab = Some(Grab { handle: None, from: corner, by: (30.0, 18.0) });
    let unmoved = app.layers_on(0).iter().find(|d| d.kind == pdf_core::document::DrawnKind::Shape).cloned().expect("panel");
    assert_eq!(unmoved.rect, panel.rect, "the page changed before the pointer was let go");

    // Let go.
    let grab = app.tab_mut().selection.grab.take().expect("grab");
    app.finish_grab(sel, grab, 1.0);
    let moved = app.layers_on(0).iter().find(|d| d.kind == pdf_core::document::DrawnKind::Shape).cloned().expect("panel");
    assert!(
        (moved.rect.left - panel.rect.left - 30.0).abs() < 0.5 && (moved.rect.top - panel.rect.top - 18.0).abs() < 0.5,
        "it did not move by the drag: {:?} then {:?}",
        panel.rect,
        moved.rect
    );
    // And it is still selected, where it now is.
    let still = app.tab_mut().selection.selected.clone().expect("still selected");
    assert!((still.rect.left - moved.rect.left).abs() < 0.5, "the selection did not follow the thing");
}

/// **Text placed by Add Text is real page content, so it is found and
/// dragged the same way any other run of words is** — the whole run,
/// not the one letter a plain click would drill into.
/// `select_thing_at_drilling(.., false)` is what `interact_objects`
/// actually calls when a drag starts fresh on unselected text; using it
/// here rather than `select_thing_at` is what makes this test exercise
/// the drag path's real bug rather than the click path's intended one.
#[test]
fn text_placed_by_add_text_can_be_selected_and_dragged_like_any_other_run() {
    let mut app = app("single-page.pdf");
    let at = AppPoint { x: 300.0, y: 700.0 };
    app.write_text_at(0, at, "FRESHLYPLACED").expect("written");

    app.submit("editobject");
    assert!(
        app.select_thing_at_drilling(0, at, false),
        "the freshly written words were not found"
    );
    let sel = app.tab_mut().selection.selected.clone().expect("selected");
    assert_eq!(sel.what, "the words");

    let grab = Grab { handle: None, from: at, by: (25.0, 12.0) };
    app.finish_grab(sel.clone(), grab, 1.0);

    let moved = app.tab_mut().selection.selected.clone().expect("still selected after the drag");
    assert_eq!(moved.what, "the words", "the drag left it split down to a single letter");
    assert!(
        (moved.rect.left - sel.rect.left - 25.0).abs() < 1.0
            && (moved.rect.top - sel.rect.top - 12.0).abs() < 1.0,
        "the freshly placed text did not move: {:?} then {:?}",
        sel.rect,
        moved.rect
    );
    assert!(
        (moved.rect.right - moved.rect.left - (sel.rect.right - sel.rect.left)).abs() < 1.0,
        "the whole run should have moved together, not shrunk to one letter's width: {:?} then {:?}",
        sel.rect,
        moved.rect
    );
}

/// **The bug this whole fix was for**: before `select_thing_at_drilling`
/// existed, `interact_objects` used the drilling `select_thing_at` to
/// decide what a fresh drag had landed on — so starting a drag on an
/// ordinary, isolated line of text silently split it into one object per
/// character and moved only the one under the pointer, leaving the rest
/// of the line exactly where it was.
#[test]
fn dragging_an_isolated_line_of_text_moves_the_whole_line_not_one_letter() {
    let mut app = app("single-page.pdf");
    let at = AppPoint { x: 300.0, y: 700.0 };
    app.write_text_at(0, at, "FRESHLYPLACED").expect("written");
    app.submit("editobject");

    // The exact shape of `interact_objects`'s own drag-start branch:
    // select fresh, then drag the body.
    assert!(app.select_thing_at_drilling(0, at, false));
    let sel = app.tab_mut().selection.selected.clone().expect("selected");
    let leftmost = |rects: &[(usize, pdf_core::document::Rect)]| {
        rects.iter().map(|(_, r)| r.left).fold(f32::INFINITY, f32::min)
    };
    let before = app.tab_mut().doc.as_ref().unwrap().session.text_run_rects(0).unwrap();
    let before_count = before.len();
    let before_left = leftmost(&before);

    let grab = Grab { handle: None, from: at, by: (60.0, 0.0) };
    app.finish_grab(sel, grab, 1.0);

    let after = app.tab_mut().doc.as_ref().unwrap().session.text_run_rects(0).unwrap();
    assert_eq!(after.len(), before_count, "the run was split into characters by a drag");
    assert!(
        (leftmost(&after) - (before_left + 60.0)).abs() < 1.0,
        "the run did not move as one piece: {before_left} then {:?}",
        leftmost(&after)
    );
}

/// **`undo` puts a dragged object back** — Edit Object's move, resize
/// and delete used to bypass the command stack entirely, so there was
/// nothing for `undo` to find; now that a drag reaches the document
/// through [`pdf_core::command::Command`], the same `undo` that already
/// reverses everything else reverses these too, with no new wiring on
/// this side beyond routing the call through it.
#[test]
fn undo_puts_a_dragged_object_back() {
    let mut app = app("covered.pdf");
    app.submit("editobject");
    let panel = app
        .layers_on(0)
        .iter()
        .find(|d| d.kind == pdf_core::document::DrawnKind::Shape)
        .cloned()
        .expect("panel");
    let corner = AppPoint { x: (panel.rect.right - 20.0) as f64, y: (panel.rect.bottom - 20.0) as f64 };
    assert!(app.select_thing_at(0, corner));
    let sel = app.tab_mut().selection.selected.clone().expect("selected");

    app.finish_grab(sel, Grab { handle: None, from: corner, by: (30.0, 18.0) }, 1.0);
    let moved = app
        .layers_on(0)
        .iter()
        .find(|d| d.kind == pdf_core::document::DrawnKind::Shape)
        .cloned()
        .expect("panel");
    assert!(
        (moved.rect.left - panel.rect.left - 30.0).abs() < 0.5,
        "the drag did not move it, so undoing it proves nothing"
    );

    app.submit("undo");
    let back = app
        .layers_on(0)
        .iter()
        .find(|d| d.kind == pdf_core::document::DrawnKind::Shape)
        .cloned()
        .expect("panel");
    assert!(
        (back.rect.left - panel.rect.left).abs() < 0.5 && (back.rect.top - panel.rect.top).abs() < 0.5,
        "undo did not put the panel back: was {:?}, moved to {:?}, undo left it at {:?}",
        panel.rect,
        moved.rect,
        back.rect
    );
}

/// **A click's own tiny wobble does not move what it selected**,
/// measured against the screen rather than the page: the same couple of
/// page points of press-to-release drift is under a pixel at one zoom
/// and several pixels at another. Reported from use as clicking
/// something moving it — invisible on a whole sentence, glaring once a
/// click can pick out a single letter of one. A drag that really is a
/// few screen pixels, at a closer zoom, still moves it.
#[test]
fn a_tiny_wobble_does_not_move_the_selection_but_a_real_drag_still_does() {
    let mut app = app("covered.pdf");
    app.submit("editobject");
    let panel = app
        .layers_on(0)
        .iter()
        .find(|d| d.kind == pdf_core::document::DrawnKind::Shape)
        .cloned()
        .expect("panel");
    let middle = AppPoint {
        x: ((panel.rect.left + panel.rect.right) / 2.0) as f64,
        y: ((panel.rect.top + panel.rect.bottom) / 2.0) as f64,
    };
    assert!(app.select_thing_at(0, middle));
    // Not necessarily the panel itself: `covered.pdf` draws a picture
    // right under it, and Edit Object looks at pictures first — tracked
    // by whatever `select_thing_at` actually picked up, not assumed.
    let sel = app.tab_mut().selection.selected.clone().expect("selected");
    let object = sel.object;
    let before = app
        .layers_on(0)
        .iter()
        .find(|d| d.object == object)
        .cloned()
        .expect("the selected thing");

    // Two page points of drift at a zoom where that is under three
    // screen pixels (scale 1.0) — should change nothing.
    app.finish_grab(sel.clone(), Grab { handle: None, from: middle, by: (2.0, 0.0) }, 1.0);
    let still = app
        .layers_on(0)
        .iter()
        .find(|d| d.object == object)
        .cloned()
        .expect("the selected thing");
    assert_eq!(still.rect, before.rect, "a sub-threshold wobble moved the selection");

    // The identical two points of movement, but zoomed in enough (scale
    // 3.0) that it is a real few-pixel drag — should move it.
    app.finish_grab(sel, Grab { handle: None, from: middle, by: (2.0, 0.0) }, 3.0);
    let moved = app
        .layers_on(0)
        .iter()
        .find(|d| d.object == object)
        .cloned()
        .expect("the selected thing");
    assert!(
        (moved.rect.left - before.rect.left - 2.0).abs() < 0.5,
        "a real drag at this zoom should still move it: {:?} then {:?}",
        before.rect,
        moved.rect
    );
}

/// **Dragging a handle resizes about the opposite side.**
#[test]
fn dragging_a_handle_resizes_about_the_opposite_corner() {
    let mut app = app("covered.pdf");
    app.submit("editobject");
    let panel = app.layers_on(0).iter().find(|d| d.kind == pdf_core::document::DrawnKind::Shape).cloned().expect("panel");
    let corner = AppPoint { x: (panel.rect.right - 20.0) as f64, y: (panel.rect.bottom - 20.0) as f64 };
    assert!(app.select_thing_at(0, corner));
    let sel = app.tab_mut().selection.selected.clone().expect("selected");

    // Drag the bottom-right handle in by half the width and height.
    let (w, h) = (panel.rect.right - panel.rect.left, panel.rect.bottom - panel.rect.top);
    let grab = Grab {
        handle: Some(Handle::BottomRight),
        from: AppPoint { x: panel.rect.right as f64, y: panel.rect.bottom as f64 },
        by: (-w / 2.0, -h / 2.0),
    };
    app.finish_grab(sel, grab, 1.0);
    assert!(said(&app).contains("resized to 50%"), "{}", said(&app));

    let now = app.layers_on(0).iter().find(|d| d.kind == pdf_core::document::DrawnKind::Shape).cloned().expect("panel");
    assert!((now.rect.left - panel.rect.left).abs() < 0.5 && (now.rect.top - panel.rect.top).abs() < 0.5, "the anchored corner moved");
    assert!(((now.rect.right - now.rect.left) - w / 2.0).abs() < 1.0, "width did not halve");
    assert!(((now.rect.bottom - now.rect.top) - h / 2.0).abs() < 1.0, "height did not halve");
}

/// **`opacity 50` fades the selected thing**, and the list reports it.
#[test]
fn the_opacity_verb_fades_the_selected_thing() {
    let mut app = app("framed.pdf");
    app.submit("editobject");
    let picture = app.layers_on(0).iter().find(|d| d.kind == pdf_core::document::DrawnKind::Picture).cloned().expect("picture");
    let middle = AppPoint {
        x: ((picture.rect.left + picture.rect.right) / 2.0) as f64,
        y: ((picture.rect.top + picture.rect.bottom) / 2.0) as f64,
    };
    assert!(app.select_thing_at(0, middle));

    app.submit("opacity 50");
    assert!(said(&app).contains("opacity 50%"), "{}", said(&app));
    let faded = app.layers_on(0).iter().find(|d| d.kind == pdf_core::document::DrawnKind::Picture).cloned().expect("picture");
    assert!((faded.opacity - 0.5).abs() < 0.02, "the list does not report the fade: {}", faded.opacity);

    app.submit("opacity 200");
    assert!(said(&app).contains("between 0 and 100"), "{}", said(&app));
}

/// **The layer window steps things over what they overlap, and keeps them
/// picked.**
#[test]
fn a_picked_layer_can_be_nudged_up_and_stays_picked() {
    let mut app = app("covered.pdf");
    let listed = app.layers_on(0).to_vec();
    let picture_at = listed.iter().position(|d| d.kind == pdf_core::document::DrawnKind::Picture).expect("the picture");
    app.tab_mut().selection.picked_layer = Some(picture_at);

    // Up passes the panel it was under, and says so.
    app.restack_picked(pdf_core::document::Stacking::Up);
    assert!(said(&app).contains("now in front of the shape"), "{}", said(&app));
    let now = app.layers_on(0).to_vec();
    let picture_now = now.iter().position(|d| d.kind == pdf_core::document::DrawnKind::Picture).expect("still there");
    let panel_now = now.iter().position(|d| d.kind == pdf_core::document::DrawnKind::Shape).expect("panel");
    assert!(picture_now > panel_now, "the picture should now be over the panel: {now:#?}");
    // Still picked, at its new row, so the next nudge acts on the same thing.
    assert_eq!(app.tab_mut().selection.picked_layer, Some(picture_now), "the pick did not follow the thing it was on");

    // Down puts it back under.
    app.restack_picked(pdf_core::document::Stacking::Down);
    assert!(said(&app).contains("now behind the shape"), "{}", said(&app));
    let back = app.layers_on(0).to_vec();
    assert_eq!(back.iter().position(|d| d.kind == pdf_core::document::DrawnKind::Picture), Some(picture_at));
}

/// **A lock that never took is finished when the document opens.**
///
/// Reported from use with a screenshot: a grey layer over a picture that
/// could not be selected or sent back. It was the chequerboard drawn over a
/// locked picture that was never actually taken off the page. The engine
/// test builds that document; this checks the app repairs it on open and
/// says so.
#[test]
fn a_document_with_a_lock_that_never_took_is_repaired_on_open() {
    use pdf_core::document::{Document, DocumentMut};

    // Build the broken document the way the engine test does.
    let path = fixture("pictures.pdf");
    let original = pdf_core::registry::exclusive(|| {
        let mut doc = pdf_core::document::pdfium_doc::PdfiumDocument::open_path(&path, None)
            .expect("open");
        let mut bytes = Vec::new();
        doc.save_full_copy(&mut bytes).expect("save");
        let file = pdf_core::pdf::File::parse(&bytes).expect("parse");
        (1..64u32)
            .find_map(|number| match file.object(number) {
                Ok(pdf_core::pdf::Object::Stream(dict, span))
                    if dict.get(b"Subtype").and_then(pdf_core::pdf::Object::as_name)
                        == Some(&b"Image"[..])
                        && dict.get(b"Width").and_then(pdf_core::pdf::Object::as_i64)
                            == Some(4) =>
                {
                    Some((number, pdf_core::pdf::write_stream(&dict, &bytes[span])))
                }
                _ => None,
            })
            .expect("the first picture")
    });
    let broken = pdf_core::registry::exclusive(|| {
        let mut doc = pdf_core::document::pdfium_doc::PdfiumDocument::open_path(&path, None)
            .expect("open");
        let picture = doc.images_on(0).expect("images")[0].object;
        doc.lock_image(0, picture, b"a good passcode").expect("lock");
        let mut bytes = Vec::new();
        doc.save_full_copy(&mut bytes).expect("save");
        let file = pdf_core::pdf::File::parse(&bytes).expect("parse");
        file.rewrite(&[original]).expect("put the picture back")
    });
    let scratch = std::env::temp_dir().join(format!(
        "pagify-stale-lock-{}.pdf",
        std::process::id()
    ));
    std::fs::write(&scratch, broken).expect("write");

    let app = PagifyApp::new(Some(scratch.to_str().expect("path")));
    assert!(app.tab().doc.is_some(), "did not open");
    assert!(
        said(&app).contains("had never taken effect"),
        "the repair was not reported:\n{}",
        said(&app)
    );
    let badges = app.locked_items_on(0);
    assert_eq!(badges.len(), 1);
    assert!(!badges[0].stale, "the badge is still stale after open");
    let _ = std::fs::remove_file(&scratch);
}

/// Typing into the command box must never take the program down.
///
/// Seen from outside while looking at a real document: the app vanished
/// the moment `goto 3` was typed — twice, with no crash report, which is
/// what a panic looks like from the desktop.
#[test]
fn typing_a_page_number_does_not_panic() {
    let mut app = app("pages-ladder.pdf");
    for line in ["goto 3", "goto", "page 3", "3", "goto 99", "goto x"] {
        app.submit(line);
    }
    assert!(app.tab_mut().doc.is_some());
}

/// **A document opened with its password saves in place after an edit.**
///
/// Reported from use: after moving a picture in a password-protected
/// catalogue, `save` refused with "this document has a password waiting
/// — use saveas", which is the guard against writing a *first* password
/// over somebody's only plain copy. This document was encrypted before and
/// is encrypted after; the guard has no business firing.
#[test]
fn a_document_opened_with_a_password_can_still_be_saved_after_an_edit() {
    let scratch = std::env::temp_dir().join(format!("pagify-encrypted-{}.pdf", std::process::id()));
    std::fs::copy(fixture("encrypted.pdf"), &scratch).expect("copy");
    let mut app = PagifyApp::new(None);
    app.open_with(scratch.to_str().expect("path"), Some("pagify"));
    assert!(app.tab_mut().doc.is_some(), "did not open:\n{}", said(&app));

    let run = app.tab_mut()
        .doc
        .as_ref()
        .expect("open")
        .session
        .text_runs(0)
        .expect("runs")
        .first()
        .cloned()
        .expect("a run");
    let from = AppPoint {
        x: ((run.rect.left + run.rect.right) / 2.0) as f64,
        y: ((run.rect.top + run.rect.bottom) / 2.0) as f64,
    };
    app.move_thing(0, from, AppPoint { x: from.x + 10.0, y: from.y + 6.0 }, false)
        .expect("move");

    app.submit("save");
    let told = said(&app);
    assert!(
        !told.contains("password waiting"),
        "save was refused on a document that already had a password:\n{told}"
    );
    assert!(told.contains("saved"), "it did not say it saved:\n{told}");
    let _ = std::fs::remove_file(&scratch);
}

/// **Saving with a first password over the only plain copy asks; it does
/// not refuse.**
///
/// Reported from use as "why can't I just save": Secure Document had been
/// pressed on a plain file, and every `save` after that came back with a
/// message that offered `saveas` and nothing else. The irreversible thing
/// still takes a deliberate click; the two safe things are beside it.
#[test]
fn saving_a_first_password_over_the_original_asks_and_can_be_declined() {
    let scratch = std::env::temp_dir().join(format!("pagify-plain-{}.pdf", std::process::id()));
    std::fs::copy(fixture("text-lines.pdf"), &scratch).expect("copy");
    let before = std::fs::read(&scratch).expect("read");
    let mut app = PagifyApp::new(Some(scratch.to_str().expect("path")));
    assert!(app.tab_mut().doc.is_some());

    // Secure Document, answered with a strong password, twice.
    app.submit("secure");
    app.answer_passcode("Correct-Horse-99-Battery");
    app.answer_passcode("Correct-Horse-99-Battery");
    assert!(app.unsaved_password(), "the password was not set:\n{}", said(&app));

    // A plain save asks rather than refusing, and writes nothing yet.
    assert_eq!(app.save(None), SaveOutcome::Asking);
    assert!(app.tab_mut().secure_state.asking_to_secure.is_some(), "no question was raised");
    assert_eq!(std::fs::read(&scratch).expect("read"), before, "the file was written before the answer");
    assert!(!said(&app).contains("use `saveas"), "the old refusal is back:\n{}", said(&app));

    // Taking the password off and saving is one of the answers, and it works.
    app.tab_mut().secure_state.asking_to_secure = None;
    app.tab_mut().doc.as_ref().expect("open").session.unsecure_document().expect("unsecure");
    assert_eq!(app.save(None), SaveOutcome::Done, "{}", said(&app));
    assert!(said(&app).contains("saved"), "{}", said(&app));
    let _ = std::fs::remove_file(&scratch);
}

/// **And writing it over the original, once confirmed, goes through.**
#[test]
fn a_confirmed_password_is_written_over_the_original() {
    let scratch = std::env::temp_dir().join(format!("pagify-plain-2-{}.pdf", std::process::id()));
    std::fs::copy(fixture("text-lines.pdf"), &scratch).expect("copy");
    let mut app = PagifyApp::new(Some(scratch.to_str().expect("path")));
    app.submit("secure");
    app.answer_passcode("Correct-Horse-99-Battery");
    app.answer_passcode("Correct-Horse-99-Battery");
    assert_eq!(app.save(None), SaveOutcome::Asking);

    // The deliberate click.
    app.tab_mut().secure_state.asking_to_secure = None;
    app.tab_mut().secure_state.secure_in_place_confirmed = true;
    assert_eq!(app.save(None), SaveOutcome::Done, "{}", said(&app));

    // The file now needs the password; the confirmation was spent.
    let written = std::fs::read(&scratch).expect("read");
    assert!(
        String::from_utf8_lossy(&written).contains("/Encrypt"),
        "the password was not written into the file"
    );
    assert!(!app.tab_mut().secure_state.secure_in_place_confirmed, "the confirmation must not outlive one save");
    let _ = std::fs::remove_file(&scratch);
}

/// **Escape gives up on a passcode prompt** without locking anything, and
/// says which prompt it abandoned.
#[test]
fn escape_abandons_a_lock_that_was_waiting_on_a_passcode() {
    let mut app = app("text-lines.pdf");
    let before = page_text(&app, 0);
    app.tab_mut().secure_state.awaiting_password =
        Some(Awaiting::Lock { page: 0, shapes: vec![fox_area()], require_complete: true });

    app.escape();
    assert!(app.tab_mut().secure_state.awaiting_password.is_none());
    assert!(said(&app).contains("nothing was locked"), "{}", said(&app));
    assert_eq!(page_text(&app, 0), before);
}

/// A locked document carries its own original, so appending a delta would
/// leave the unsealed page in the file's earlier revision.
#[test]
fn a_locked_document_saves_as_a_full_copy() {
    let mut app = app("text-lines.pdf");
    app.lock_area(0, fox_area(), b"a good passcode", true).expect("lock");
    assert!(app.tab_mut().doc.as_ref().unwrap().session.must_save_full_copy());
}

/// And the saved file really does hide the words and still hold them.
#[test]
fn saving_a_locked_document_keeps_both_halves_true() {
    let dir = std::env::temp_dir().join("pagify-lock-wiring");
    let _ = std::fs::create_dir_all(&dir);
    let target = dir.join("locked.pdf");
    let _ = std::fs::remove_file(&target);

    let mut app = app("text-lines.pdf");
    app.lock_area(0, fox_area(), b"a good passcode", true).expect("lock");
    app.save(Some(target.clone()));
    assert!(target.exists(), "nothing was written:\n{}", said(&app));

    let mut reopened = PagifyApp::new(Some(target.to_str().expect("path")));
    assert!(
        !page_text(&reopened, 0).contains("The quick brown fox"),
        "the saved file still shows the words"
    );
    reopened.unlock(b"a good passcode").expect("unlock the saved file");
    assert!(
        page_text(&reopened, 0).contains("The quick brown fox"),
        "the saved file could not be unlocked"
    );

    let _ = std::fs::remove_file(&target);
}

// -- locking whole pages ------------------------------------------------

/// **Locking the whole document**, reported from use as not being offered
/// at all: `lock` only ever armed a rectangle drag on the page in front of
/// you.
#[test]
fn lock_all_hides_every_page_and_the_passcode_brings_them_back() {
    let mut app = app("text-lines.pdf");
    let before = page_text(&app, 0);
    assert!(before.contains("The quick brown fox"), "control");

    let said = app.lock_pages(&[0], b"a good passcode").expect("lock");
    assert!(said.contains("locked 1 page"), "{said}");
    assert!(page_text(&app, 0).trim().is_empty(), "the page still has its words on it");

    app.unlock(b"a good passcode").expect("unlock");
    assert_eq!(page_text(&app, 0), before, "the passcode did not give the page back");
}

/// The verb asks for a passcode rather than locking on the spot, the same
/// way the area tool does — and nothing is hidden until one is typed.
#[test]
fn lock_all_asks_for_a_passcode_before_it_hides_anything() {
    let mut app = app("text-lines.pdf");
    let before = page_text(&app, 0);

    app.submit("lock all");
    assert!(
        matches!(app.tab_mut().secure_state.awaiting_password, Some(Awaiting::LockPages(_))),
        "it did not ask for a passcode:\n{}",
        said(&app)
    );
    assert_eq!(page_text(&app, 0), before, "it locked before it had a passcode");

    app.escape();
    assert!(said(&app).contains("nothing was locked"), "{}", said(&app));
    assert_eq!(page_text(&app, 0), before);
}

/// A page already locked keeps the way back it has. Without this, locking
/// an area and then the whole page would seal the blank page over the
/// original — the same class of bug as locking two areas.
#[test]
fn locking_a_page_whole_after_locking_an_area_still_gives_everything_back() {
    let mut app = app("text-lines.pdf");
    let before = page_text(&app, 0);

    app.lock_area(0, fox_area(), b"a good passcode", true).expect("lock the area");
    let said = app.lock_pages(&[0], b"a good passcode").expect("lock the page");
    assert!(said.contains("already locked"), "it sealed the page a second time: {said}");

    app.unlock(b"a good passcode").expect("unlock");
    assert_eq!(page_text(&app, 0), before, "the original was lost to the second lock");
}

/// **A text selection is the words, not the area under them.**
///
/// Reported from use: selecting text on a catalogue page and locking it was
/// refused with "this area cannot be fully cleared: object 1 lies under
/// 100% of the area and is part of a scanned image". The image was never
/// what was selected — and there is a right-click on the image itself for
/// anyone who wants that too.
///
/// The catalogue itself is used, because no committed fixture has real text
/// sitting on an image — a pure scan has no text to select at all, and is
/// refused earlier for a different reason. Skips where the file is absent;
/// `the_selection_menu_asks_for_an_incomplete_lock` pins the same rule
/// without it.
#[test]
fn locking_a_selection_over_an_image_is_not_refused_on_the_images_account() {
    let Some(path) = std::env::var("HOME")
        .ok()
        .map(|h| format!("{h}/Downloads/HSI CATALOG 2026.pdf"))
        .filter(|p| std::path::Path::new(p).is_file())
    else {
        eprintln!("skipping: no catalogue in ~/Downloads");
        return;
    };
    let mut app = PagifyApp::new(Some(&path));
    // Skipped rather than failed: this reads a document from the person's
    // own Downloads folder, which they are entitled to change — and one of
    // them acquired a password mid-development, at which point tests about
    // image locking started failing about something else entirely.
    if app.tab_mut().doc.is_none() {
        eprintln!("skipping: the catalogue is present but would not open");
        return;
    }

    // A page with both — text to select and an image beneath it, which is
    // the combination that was refusing.
    let page = (0..60).find(|p| {
        !app.images_on(*p).is_empty()
            && app.tab_mut()
                .doc
                .as_ref()
                .and_then(|d| d.session.page_size(*p).ok())
                .is_some()
            && app.characters(*p).map(|c| c.len()).unwrap_or(0) > 20
    });
    let Some(page) = page else {
        eprintln!("skipping: no page with text over an image");
        return;
    };

    let size = app.tab_mut().doc.as_ref().unwrap().session.page_size(page).expect("size");
    let area = pdf_core::document::Rect {
        left: 0.0,
        top: 0.0,
        right: size.width_pt,
        bottom: size.height_pt,
    };

    // The dragged rectangle still refuses: it means *this area*, and the
    // image in it cannot be cleared.
    //
    // Unless the image *can* be cleared, which depends on the document —
    // this one is read from the person's own Downloads folder, and a copy
    // rebuilt by another program has different images in it. The property
    // is about what a rectangle means, not about that file, so a page that
    // does not exhibit the case is skipped rather than failed.
    if app.lock_area(page, area, b"a good passcode", true).is_ok() {
        eprintln!("skipping: this copy's images can be cleared, so there is nothing to refuse");
        return;
    }

    // The selection does not, because the image was never selected.
    app.lock_area(page, area, b"a good passcode", false)
        .expect("a selection should lock despite the image beneath it");
}

/// And the right-click path really does ask with `require_complete: false`,
/// so the fix above is reachable from the menu rather than only by calling
/// the method directly.
#[test]
fn the_selection_menu_asks_for_an_incomplete_lock() {
    let mut app = app("text-lines.pdf");
    app.tab_mut().doc.as_mut().unwrap().caches.text = None;
    let chars = app.characters(0).expect("characters").clone();
    app.tab_mut().selection.text_selection = Some(0..chars.len().min(8));
    app.tab_mut().organize.selection_page = 0;

    app.lock_selection();
    match &app.tab().secure_state.awaiting_password {
        Some(Awaiting::Lock { require_complete, .. }) => assert!(
            !*require_complete,
            "the selection would still be refused on account of what is under it"
        ),
        other => panic!("expected a lock prompt, got {other:?}"),
    }
}

// -- locking one image --------------------------------------------------

/// **Locking a selected image**, through the app: off the page, a badge
/// where it was, and the passcode puts it back.
#[test]
fn a_locked_image_leaves_a_badge_and_the_passcode_brings_it_back() {
    let mut app = app("scan-300dpi.pdf");
    let images = app.images_on(0);
    assert_eq!(images.len(), 1, "the fixture should have one image");
    let was_at = images[0].rect;

    let said = app.lock_image(0, images[0].object, b"a good passcode").expect("lock");
    assert!(said.contains("padlock"), "it did not say how to get it back: {said}");
    // The object stays and is emptied — blanked in place rather than
    // removed, because removing rewrites the whole page. What matters is
    // that none of the picture is left.
    assert!(
        app.images_on(0)[0].pixel_width <= 1,
        "the image still has its pixels"
    );

    // The badge stands where the image was, which is what makes the lock
    // visible and clickable.
    let badges = app.locked_items_on(0);
    assert_eq!(badges.len(), 1);
    assert!(
        (badges[0].rect.left - was_at.left).abs() < 0.5,
        "the badge is in the wrong place: {:?} vs {:?}",
        badges[0].rect,
        was_at
    );

    app.unlock_item(&badges[0].id, b"a good passcode").expect("unlock");
    assert!(app.images_on(0)[0].pixel_width > 1, "the image did not come back");
    assert!(app.locked_items_on(0).is_empty(), "the badge outlived the lock");
}

/// **The bare `unlock` command finishes the job too, not just
/// `unlock_item`.** Reported from use: after `unlock` put a document's
/// locked pages back, a locked picture's own padlock stayed on the page
/// — because the page-wide restore handed back the original bytes
/// without ever telling the vault the seal it read them from was done.
/// Only unlocking one item at a time did that bookkeeping.
#[test]
fn a_whole_document_unlock_clears_a_locked_images_badge_too() {
    let mut app = app("scan-300dpi.pdf");
    let object = app.images_on(0)[0].object;
    app.lock_image(0, object, b"a good passcode").expect("lock");
    assert_eq!(app.locked_items_on(0).len(), 1, "control: the image should be sealed");

    app.unlock(b"a good passcode").expect("unlock");
    assert!(app.images_on(0)[0].pixel_width > 1, "the image did not come back");
    assert!(
        app.locked_items_on(0).is_empty(),
        "the image's badge outlived a whole-document unlock"
    );
    assert!(
        app.tab_mut().doc.as_ref().unwrap().session.locked_pages().is_empty(),
        "the page still counts as locked once everything on it is back"
    );
}

/// Clicking a badge asks for a passcode rather than unlocking on the spot,
/// and a wrong one leaves the image sealed.
#[test]
fn a_wrong_passcode_leaves_a_locked_image_where_it_is() {
    let mut app = app("scan-300dpi.pdf");
    let object = app.images_on(0)[0].object;
    app.lock_image(0, object, b"a good passcode").expect("lock");
    let id = app.locked_items_on(0)[0].id.clone();

    assert!(app.unlock_item(&id, b"the wrong one").is_err());
    assert!(app.images_on(0)[0].pixel_width <= 1, "it came back anyway");
    assert_eq!(app.locked_items_on(0).len(), 1, "the seal was dropped");
}

/// An empty passcode is refused before anything comes off the page.
#[test]
fn locking_an_image_needs_a_passcode() {
    let mut app = app("scan-300dpi.pdf");
    let object = app.images_on(0)[0].object;

    assert!(app.lock_image(0, object, b"").is_err());
    assert!(app.images_on(0)[0].pixel_width > 1, "the image was cleared anyway");
    assert!(app.locked_items_on(0).is_empty());
}

/// Escape abandons the prompt without locking, and says which prompt it
/// gave up on.
#[test]
fn escape_abandons_an_image_lock_that_was_waiting_on_a_passcode() {
    let mut app = app("scan-300dpi.pdf");
    let object = app.images_on(0)[0].object;
    app.tab_mut().secure_state.awaiting_password = Some(Awaiting::LockImage { page: 0, object });

    app.escape();
    assert!(app.tab_mut().secure_state.awaiting_password.is_none());
    assert!(said(&app).contains("nothing was locked"), "{}", said(&app));
    assert!(app.images_on(0)[0].pixel_width > 1, "the image went anyway");
}

/// Two images on one page lock and unlock independently — the whole point
/// of sealing the object rather than the page it sat on.
///
/// No committed fixture has two images on a page, and the property is worth
/// more than a fixture built to satisfy it: this uses a real catalogue page
/// where several sit together, and skips where that file is not present.
/// `each_sealed_item_opens_on_its_own` pins the same rule in the vault,
/// which does run everywhere.
#[test]
fn one_badge_unlocks_its_own_image_and_leaves_the_others_sealed() {
    let Some(path) = std::env::var("HOME").ok().map(|h| {
        format!("{h}/Downloads/HSI CATALOG 2026.pdf")
    }).filter(|p| std::path::Path::new(p).is_file()) else {
        eprintln!("skipping: no catalogue in ~/Downloads");
        return;
    };
    let mut app = PagifyApp::new(Some(&path));
    // Skipped rather than failed: this reads a document from the person's
    // own Downloads folder, which they are entitled to change — and one of
    // them acquired a password mid-development, at which point tests about
    // image locking started failing about something else entirely.
    if app.tab_mut().doc.is_none() {
        eprintln!("skipping: the catalogue is present but would not open");
        return;
    }
    app.tab_mut().view_state.page = 40;

    let images = app.images_on(40);
    if images.len() < 2 {
        eprintln!("skipping: that page does not have two images");
        return;
    }

    let started_with = images.iter().filter(|i| i.pixel_width > 1).count();
    // Highest object index first: removing one shifts every index above it,
    // so locking the lower one first would then address the wrong object.
    app.lock_image(40, images[1].object, b"a good passcode").expect("second");
    app.lock_image(40, images[0].object, b"a good passcode").expect("first");
    assert_eq!(app.locked_items_on(40).len(), 2, "both should be sealed");
    let live = |a: &PagifyApp| a.images_on(40).iter().filter(|i| i.pixel_width > 1).count();
    assert_eq!(live(&app), started_with - 2, "two should have been cleared");

    let one = app.locked_items_on(40)[0].id.clone();
    app.unlock_item(&one, b"a good passcode").expect("unlock one");

    assert_eq!(live(&app), started_with - 1, "unlocking one brought back more than one");
    assert_eq!(app.locked_items_on(40).len(), 1, "unlocking one released the other");
}

/// A passcode is required, and a wrong one on an already-locked document
/// changes nothing.
#[test]
fn lock_all_refuses_an_empty_or_wrong_passcode() {
    let mut app = app("text-lines.pdf");
    assert!(app.lock_pages(&[0], b"").is_err(), "it locked with no passcode");

    app.lock_pages(&[0], b"a good passcode").expect("lock");
    let locked = page_text(&app, 0);
    assert!(app.lock_pages(&[0], b"the wrong one").is_err());
    assert_eq!(page_text(&app, 0), locked, "a wrong passcode still changed the page");
}

/// **Reported from use, on the real CAMINO file: a long, perceptible
/// pause between clicking a word and its editor opening, and again
/// between applying an edit and the page showing it.** Root-caused to
/// two separate, unconditional full-page text extractions — `text_runs`
/// in `pick_text_run` (to find which run was clicked, needing only its
/// geometry) and `text_runs_all` in `set_run_in_stream` (to read the one
/// run being edited, needing only its own data) — each paying PDFium's
/// real per-run `PdfPageTextObject::text()` cost for every run on the
/// page, most of a second on a page with a few hundred of them. Fixed
/// first by `text_run_rects`/`text_runs_some` (a bounded neighbourhood,
/// not the whole page) on the pick side and `text_run_at` (one run, not
/// all of them) on the apply side — and since the click reads the whole
/// page once for the paragraph detector (`page_text_snapshot`), by making
/// that whole-page read cheap (one shared text page for every object,
/// `PdfPageText::for_object`, tens of milliseconds a page). Measured here
/// rather than merely asserted to succeed, since a slow-but-eventually-
/// correct answer would still be the bug that was reported.
#[test]
fn editing_is_fast_on_a_real_busy_page() {
    let path = r"C:\Users\hsili\Downloads\CAMINO elitee-plus 3.0.pdf";
    let mut app = PagifyApp::new(Some(path));
    let Some(_) = &app.tab_mut().doc else {
        eprintln!("skipping: CAMINO not present on this machine");
        return;
    };

    // **A run that stands alone, not just the first with words in it.**
    // The first run on this page with more than four characters is now
    // the first line of a four-row table, which opens as a paragraph — and
    // typing one reversed line into that editor is a different edit (it
    // shrinks the paragraph and hides three rows), not the plain
    // single-run edit this times. So the target is a run the page reading
    // puts in a block of its own. That reading is made here on the side,
    // from the session, so the click below still pays for the app's own
    // (this test times that, the first pick on a busy page).
    let runs = app.tab_mut().doc.as_ref().unwrap().session.text_runs(0).expect("runs");
    let side = block_input::build_page_blocks(
        0,
        0,
        0,
        app.tab_mut().doc.as_ref().unwrap().session.page_text_snapshot(0).expect("snapshot"),
    );
    let target = runs
        .iter()
        .find(|r| {
            r.text.trim().chars().count() > 4
                && side.by_object.get(&r.object).is_some_and(|&(block, _)| side.blocks[block].objects().len() == 1)
        })
        .cloned()
        .expect("a run with words in it that is a block of its own");
    let at = AppPoint {
        x: ((target.rect.left + target.rect.right) / 2.0) as f64,
        y: ((target.rect.top + target.rect.bottom) / 2.0) as f64,
    };

    // 500ms, not the ~150ms this typically runs in isolation: running as
    // part of the full suite — hundreds of other fixtures opened before
    // it — adds enough ambient jitter to occasionally push an isolated
    // run's own time up, the same reason `matching_properties_on_a_
    // small_selection_is_fast_on_a_busy_page` budgets five whole seconds
    // for a comparably real operation. Still comfortably below the
    // ~800ms the bug this guards against actually took.
    const BUSY_PAGE_BUDGET_MS: u128 = 500;
    let t0 = std::time::Instant::now();
    app.pick_text_run(0, at).expect("picked");
    let pick_time = t0.elapsed();
    eprintln!("timing: the first pick on the CAMINO page (the page is read) took {pick_time:?}");
    assert!(
        pick_time.as_millis() < BUSY_PAGE_BUDGET_MS,
        "picking a word took {pick_time:?} on a busy page"
    );
    let picked = app.tab().edit.editing_run.as_ref().expect("an editor opened");
    assert_eq!(
        (picked.lines.len(), picked.lines[0].0.len()),
        (1, 1),
        "setup: this run should open alone, or what follows is not the single-run edit"
    );

    // Reversed, not something new: the original characters are already
    // in whatever font this run uses, so retyping needs no font
    // substitution — this measures the plain edit, not that separate
    // (and separately expensive) concern.
    let safe_replacement: String = target.text.trim().chars().rev().collect();
    app.tab_mut().edit.editing_run.as_mut().unwrap().buffer = safe_replacement.clone();

    let t1 = std::time::Instant::now();
    app.apply_editing_page();
    let apply_time = t1.elapsed();
    eprintln!("timing: applying that one-line edit on the CAMINO page took {apply_time:?}");
    assert!(
        apply_time.as_millis() < BUSY_PAGE_BUDGET_MS,
        "applying an edit took {apply_time:?} on a busy page"
    );

    app.tab_mut().doc.as_mut().unwrap().caches.text = None;
    let page = app.characters(0).map(|c| c.text()).unwrap_or_default();
    assert!(page.contains(&safe_replacement), "the edit did not land:\n{page}");
}

/// **Reported from use, on the real CAMINO file, a second time: "editing
/// is still very slow... its applying the changes that takes time,"**
/// after `editing_is_fast_on_a_real_busy_page` had already fixed the
/// single-line case. Most of this page's runs are multi-line paragraphs,
/// and `apply_paragraph_edit` used to send one `SetTextRun` per line —
/// each independently paying `set_run_in_stream`'s own per-call cost —
/// so a three-line paragraph took close to a second (roughly three
/// single-line edits' worth) even after that first fix. Root-caused to
/// two separate things, both now fixed: `SetTextRun`s for one paragraph
/// now travel as one `Command::SetTextRuns` (see
/// `DocumentMut::set_text_runs_styled`'s own doc), and
/// `paragraph_edit_commands`'s own `current_text_by_object` — a second,
/// *unconditional* full-page `text_runs()` call, needed only when a
/// shrinking paragraph has to hide a no-longer-wanted line — is now
/// fetched lazily, only when a line in this edit could actually reach
/// that branch. The second of those turned out to be the larger cost:
/// fixing only the first still left a three-line paragraph at ~800ms.
#[test]
fn editing_a_paragraph_is_fast_on_a_real_busy_page() {
    let path = r"C:\Users\hsili\Downloads\CAMINO elitee-plus 3.0.pdf";
    let mut app = PagifyApp::new(Some(path));
    let Some(_) = &app.tab_mut().doc else {
        eprintln!("skipping: CAMINO not present on this machine");
        return;
    };

    let runs = app.tab_mut().doc.as_ref().unwrap().session.text_runs(0).expect("runs");
    // Click every run in turn until one resolves to a real multi-line
    // paragraph — `pick_text_run` already escalates to `pick_paragraph`
    // on its own when that happens.
    let mut line_count = 0usize;
    for run in &runs {
        if run.text.trim().chars().count() <= 4 {
            continue;
        }
        let at = AppPoint {
            x: ((run.rect.left + run.rect.right) / 2.0) as f64,
            y: ((run.rect.top + run.rect.bottom) / 2.0) as f64,
        };
        if app.pick_text_run(0, at).is_ok() {
            let lines = app.tab_mut().edit.editing_run.as_ref().map(|e| e.lines.len()).unwrap_or(0);
            if lines > 1 {
                line_count = lines;
                break;
            }
        }
        app.tab_mut().edit.editing_run = None;
    }
    assert!(line_count > 1, "no multi-line paragraph found on this page to test against");

    let edit = app.tab_mut().edit.editing_run.as_mut().expect("still editing");
    // Reversed per line, not something new — same reasoning as the
    // single-run test: isolate the plain edit from font substitution.
    let reversed_lines: Vec<String> =
        edit.buffer.split('\n').map(|line| line.chars().rev().collect()).collect();
    edit.buffer = reversed_lines.join("\n");

    // Same generous budget as `editing_is_fast_on_a_real_busy_page` —
    // see its own comment for why 500ms, not the ~90ms this typically
    // runs in isolation.
    let t1 = std::time::Instant::now();
    app.apply_editing_page();
    let apply_time = t1.elapsed();
    eprintln!("timing: applying a {line_count}-line paragraph on the CAMINO page took {apply_time:?}");
    assert!(
        apply_time.as_millis() < 500,
        "applying a {line_count}-line paragraph took {apply_time:?} on a busy page"
    );

    app.tab_mut().doc.as_mut().unwrap().caches.text = None;
    let page = app.characters(0).map(|c| c.text()).unwrap_or_default();
    // Each line checked on its own, not all of them concatenated — how a
    // reader's own text extraction joins two separate runs (a space, a
    // synthetic gap, nothing at all) is not this test's concern, and
    // guessing it wrong would fail a paragraph that landed just fine.
    //
    // **Compared with runs of whitespace squeezed to one space, on both
    // sides.** The first multi-line paragraph on this page is now the
    // four rows of the "Photometric Data" table, and one of them is made
    // of pieces that end and begin with a space ("0 " then " lm/w"): its
    // line reads "110  lm/w" with two. Written back it is two space codes,
    // and PDFium's own extraction of the page reads two spaces as one —
    // the words are where they were typed, which is all this checks.
    let squeeze = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");
    let page = squeeze(&page);
    for line in &reversed_lines {
        assert!(
            page.contains(squeeze(line).as_str()),
            "a line did not land: {line:?}\npage:\n{page}"
        );
    }
}

/// **Reported from use, on the real CAMINO file, right after the
/// per-paragraph performance fixes landed**: editing a paragraph,
/// undoing it, then editing the same paragraph again with *shorter*
/// text (removing words from the end, so fewer lines are needed than
/// the paragraph structurally has) corrupted the page — a word cut off
/// mid-way with a stray hyphen, a different font, and a ghosted,
/// wrong-coloured duplicate of a word still visible. Targets the exact
/// paragraph reported — "The Light Source - COB" / "The COB in HSI
/// products..." — which has lines made of several pieces.
///
/// **Rewritten for the replace.** A retyped line now REPLACES its pieces
/// (the first takes the words, the others come off the page) and a line
/// typed away is REMOVED, instead of painted in the page's colour; and the
/// undo is a page snapshot, so it is exact — not "the same words, ignoring
/// trailing whitespace" as it had to be while undo re-typed them. What this
/// checks now: after the first edit and its undo **every object on the page**
/// reads exactly as it did, box and all, and the picture is pixel-identical;
/// the second, shorter edit leaves the lines it kept exactly as they were and
/// takes every piece of the lines it dropped off the page, nothing else; and
/// that edit undoes exactly too.
#[test]
fn undo_then_a_shorter_paragraph_edit_does_not_corrupt_the_page() {
    let path = r"C:\Users\hsili\Downloads\CAMINO elitee-plus 3.0.pdf";
    let mut app = PagifyApp::new(Some(path));
    let Some(_) = &app.tab_mut().doc else {
        eprintln!("skipping: CAMINO not present on this machine");
        return;
    };

    let runs = app.tab_mut().doc.as_ref().unwrap().session.text_runs(0).expect("runs");
    let mut found_paragraph = false;
    for run in &runs {
        if !run.text.contains("manufacturers") {
            continue;
        }
        let at = AppPoint {
            x: ((run.rect.left + run.rect.right) / 2.0) as f64,
            y: ((run.rect.top + run.rect.bottom) / 2.0) as f64,
        };
        if app.pick_text_run(0, at).is_ok() {
            found_paragraph = app.tab_mut().edit.editing_run.is_some();
        }
        break;
    }
    assert!(
        found_paragraph,
        "the 'COB' paragraph reported from use was not found on this page — has the fixture changed?"
    );
    let all_objects: Vec<usize> = app
        .tab_mut()
        .edit.editing_run
        .as_ref()
        .unwrap()
        .lines
        .iter()
        .flat_map(|(objs, _)| objs.iter().copied())
        .collect();
    assert!(
        app.tab_mut().edit.editing_run.as_ref().unwrap().lines.iter().any(|(objs, _)| objs.len() > 1),
        "setup: the paragraph has a line made of several pieces, or nothing here could show a difference"
    );

    // The whole page as it is, every object — not just the paragraph's own.
    let before = tests_support::runs_in_order(&app);
    let picture_before = tests_support::picture(&app);

    // First edit: reversed per line, so every line is retyped.
    let edit = app.tab_mut().edit.editing_run.as_mut().expect("still editing");
    edit.buffer =
        edit.buffer.split('\n').map(|line| line.chars().rev().collect::<String>()).collect::<Vec<_>>().join("\n");
    app.apply_editing_page();
    assert!(
        tests_support::runs_in_order(&app).len() < before.len(),
        "setup: the retyped lines' other pieces should have come off the page"
    );

    // Undo it: ONE step, and the page is exactly what it was.
    let (undone, _) = app.tab_mut().doc.as_ref().unwrap().session.undo().expect("undo");
    assert!(undone, "the first edit did not undo");
    let after_undo = tests_support::runs_in_order(&app);
    assert_eq!(after_undo.len(), before.len(), "undo did not bring every piece back");
    for (was, now) in before.iter().zip(&after_undo) {
        assert_eq!(was.object, now.object, "undo renumbered the page");
        assert!(
            tests_support::same_run(was, now),
            "object {} did not come back exactly as it was after undo: {was:?} now {now:?}",
            was.object
        );
    }
    assert!(
        tests_support::same_picture(&tests_support::picture(&app), &picture_before),
        "the page is not pixel-identical after undo"
    );
    app.tab_mut().doc.as_mut().unwrap().caches.text = None;

    // Second edit, on the same paragraph: pick it again by its own objects'
    // current geometry (whatever undo actually left behind, not assumed),
    // then type something *shorter* — fewer lines than the paragraph
    // structurally has, the exact shape reported from use.
    let mut re_picked = false;
    for &object in &all_objects {
        if let Ok(Some(run)) = app.tab_mut().doc.as_ref().unwrap().session.text_run_at(0, object) {
            let at = AppPoint {
                x: ((run.rect.left + run.rect.right) / 2.0) as f64,
                y: ((run.rect.top + run.rect.bottom) / 2.0) as f64,
            };
            if app.pick_text_run(0, at).is_ok() && app.tab_mut().edit.editing_run.is_some() {
                re_picked = true;
                break;
            }
        }
    }
    assert!(re_picked, "could not pick the same paragraph again after undo");

    let edit = app.tab_mut().edit.editing_run.as_mut().expect("still editing");
    let lines = edit.lines.clone();
    let frozen = edit.frozen.clone();
    let kept_line_count = (lines.len() / 2).max(1);
    let short_text: Vec<String> =
        edit.buffer.split('\n').take(kept_line_count).map(str::to_string).collect();
    edit.buffer = short_text.join("\n");
    let before_second = tests_support::runs_in_order(&app);
    let picture_before_second = tests_support::picture(&app);
    app.apply_editing_page();
    let after_second = tests_support::runs_in_order(&app);

    // **Exactly what it should be — not a search over unrelated page
    // text.** Every line this edit kept says what it said, so nothing of it
    // is written: all its pieces are as they were. Every line beyond what
    // was typed is removed — every one of its pieces — unless it is drawn
    // as shapes, which an edit never touches. Nothing else on the page moves.
    let removed: Vec<usize> = lines
        .iter()
        .zip(&frozen)
        .enumerate()
        .filter(|(i, (_, frozen))| *i >= kept_line_count && !**frozen)
        .flat_map(|(_, ((objects, _), _))| objects.iter().copied())
        .collect();
    assert!(!removed.is_empty(), "setup: the shorter edit should drop at least one line");
    tests_support::assert_page_after(
        "a shorter paragraph after an undo",
        &before_second,
        &after_second,
        &[],
        &removed,
    );

    // And that edit undoes exactly, too.
    let (undone, _) = app.tab_mut().doc.as_ref().unwrap().session.undo().expect("undo");
    assert!(undone, "the second edit did not undo");
    let restored = tests_support::runs_in_order(&app);
    assert_eq!(restored.len(), before_second.len());
    assert!(
        before_second.iter().zip(&restored).all(|(a, b)| tests_support::same_run(a, b)),
        "undo of the shorter edit did not restore the page"
    );
    assert!(
        tests_support::same_picture(&tests_support::picture(&app), &picture_before_second),
        "the page is not pixel-identical after undoing the shorter edit"
    );
}

/// **Reported from use, in the 0.1.11 build that had already fixed the
/// auto-wrap trigger of this exact corruption**: the same stray hyphen
/// and wrong-coloured ghost text came back, with no shrink and no
/// auto-wrap involved at all — just an ordinary Enter press partway
/// through the paragraph, which `pressing_enter_in_the_run_editor_does_
/// not_apply_it` already proves is supported, tested behaviour, not
/// something that could just be disabled. This targets the mechanism
/// directly: inserting one brand new line after the paragraph's own
/// first line, with every other line's *text* left exactly as it was —
/// the shape `paragraph_edit_commands`'s old index-based zip could not
/// survive, since every line after the insertion point would have been
/// read from one index earlier than its own.
#[test]
fn inserting_a_line_mid_paragraph_does_not_shift_the_lines_after_it() {
    let path = r"C:\Users\hsili\Downloads\CAMINO elitee-plus 3.0.pdf";
    let mut app = PagifyApp::new(Some(path));
    let Some(_) = &app.tab_mut().doc else {
        eprintln!("skipping: CAMINO not present on this machine");
        return;
    };

    let runs = app.tab_mut().doc.as_ref().unwrap().session.text_runs(0).expect("runs");
    let mut found_paragraph = false;
    for run in &runs {
        if !run.text.contains("manufacturers") {
            continue;
        }
        let at = AppPoint {
            x: ((run.rect.left + run.rect.right) / 2.0) as f64,
            y: ((run.rect.top + run.rect.bottom) / 2.0) as f64,
        };
        if app.pick_text_run(0, at).is_ok() {
            found_paragraph = app.tab_mut().edit.editing_run.is_some();
        }
        break;
    }
    assert!(
        found_paragraph,
        "the 'COB' paragraph reported from use was not found on this page — has the fixture changed?"
    );

    // Snapshot everything needed out of `edit` as owned values before
    // touching `app` again — `edit` borrows it, and `apply_editing_page`
    // below takes `editing_run` outright.
    let edit = app.tab_mut().edit.editing_run.as_ref().expect("still editing");
    let original_lines: Vec<String> = edit.buffer.split('\n').map(str::to_string).collect();
    let lines_before = edit.lines.clone();
    assert!(
        original_lines.len() >= 3,
        "need at least a prefix, a middle and a suffix line to prove this — got {}",
        original_lines.len()
    );

    let wanted: std::collections::HashSet<usize> =
        lines_before.iter().flat_map(|(objects, _)| objects.iter().copied()).collect();
    let before: std::collections::HashMap<usize, String> = app
        .tab_mut()
        .doc
        .as_ref()
        .unwrap()
        .session
        .text_runs_some(0, &wanted)
        .expect("runs")
        .into_iter()
        .map(|r| (r.object, r.text))
        .collect();

    // Insert one brand new, *blank* line right after the first line —
    // an Enter press at the end of line 0 with nothing typed on the
    // fresh line yet, the single most common real trigger (see
    // `pressing_enter_in_the_run_editor_does_not_apply_it`, which does
    // exactly this to a single run) — and change nothing else. A blank
    // inserted line asks `write_extra_styled_lines` for nothing (it
    // skips empty lines outright), which keeps this test scoped to the
    // shift this fix targets; a non-blank insertion surfaces a
    // different, separate, pre-existing gap in how a brand new
    // overflow annotation interacts with the very next batched
    // `Command::SetTextRuns` — real, but not this bug, and reported on
    // its own rather than folded in here.
    let mut new_lines = original_lines.clone();
    new_lines.insert(1, String::new());
    let edit = app.tab_mut().edit.editing_run.as_mut().expect("still editing");
    edit.buffer = new_lines.join("\n");
    app.apply_editing_page();

    let after: std::collections::HashMap<usize, String> = app
        .tab_mut()
        .doc
        .as_ref()
        .unwrap()
        .session
        .text_runs_some(0, &wanted)
        .expect("runs should still be readable")
        .into_iter()
        .map(|r| (r.object, r.text))
        .collect();

    // Collapses whitespace runs for the comparison only — PDFium's own
    // text shaping is not a guaranteed byte-identical round trip for
    // *spacing* (a line rebuilt from several objects can pick up a
    // doubled space at the join that reads back singled), which is
    // noise this test has no stake in. What it does care about is
    // *which words* ended up on which object.
    // Control characters dropped and a trailing hyphen ignored: this
    // exact file has a font whose own `ToUnicode` entry for its hyphen
    // glyph resolves to a `\u{2}` control code rather than `-` (see
    // `fix_extracted_text`'s own doc) — a defect in the file, not in
    // anything this fix touches. `fix_extracted_text` itself only
    // restores that as a real hyphen with the *next* line's own first
    // letter for context, which a single object's own isolated text
    // does not carry — so whether a word-wrap boundary ends in `-` is
    // not something this test can judge object-by-object, and is not
    // the property it exists to check. What it must still catch is
    // every *word* landing on its own correct object, which a trailing
    // hyphen or control code never obscures.
    fn normalize(s: &str) -> String {
        let no_control: String = s.chars().filter(|c| !c.is_control()).collect();
        no_control.split_whitespace().collect::<Vec<_>>().join(" ").trim_end_matches('-').to_string()
    }

    // Every original line — before *and* after the insertion point —
    // must still hold its own words, where it held them. The old
    // index-based zip would have shifted every line from the insertion
    // point onward onto the *next* line's object, and lost the true last
    // line off the end entirely.
    //
    // **Changed from the first version of this loop, and why.** It
    // asserted that every line's first object (`j == 0`) now held the
    // *whole* line, and that the others were "hidden siblings": an apply
    // used to rewrite every line of the paragraph, changed or not, onto
    // its first object and paint the rest in the page's colour. That
    // collapsed every justified multi-piece line of an untouched paragraph
    // the moment anything changed, and it no longer happens: a line that
    // says what it said is not written (see `plan_paragraph_edit`). This
    // edit changes none of any line's words — a blank line is added and
    // nothing else — so *no* object may change: the first piece of a
    // multi-piece line included, which must still hold only its own piece.
    // The line as a whole must still read as itself too, which is the part
    // of the old assertion that was about words landing where they belong.
    for (i, (objects, _)) in lines_before.iter().enumerate() {
        for (j, &object) in objects.iter().enumerate() {
            let actual = after.get(&object).expect("object should still exist after the edit");
            let expected = before.get(&object).expect("object existed before the edit");
            assert_eq!(
                actual, expected,
                "line {i} position {j} (object {object}) was rewritten by an edit that \
                 changed none of its words"
            );
        }
        let read_back: String = objects.iter().map(|o| after[o].as_str()).collect();
        assert_eq!(
            normalize(&fix_extracted_text(&read_back)),
            normalize(&original_lines[i]),
            "line {i} no longer reads as its own words"
        );
    }

    // A blank inserted line asks for nothing new to be written at all —
    // confirmed separately, not asserted here, since a stray extra
    // object for an *empty* line would itself be a bug were it to
    // appear, but checking that absence is `write_extra_styled_lines`'s
    // own concern, not this fix's.
}

/// **Reported from use, in 0.1.13**: the same corruption recurred —
/// described this time as "the same word is written over it in white"
/// — and the user confirmed the triggering edit was deleting/
/// backspacing *across a line break* (merging two lines into one), not
/// inserting one. `inserting_a_line_mid_paragraph_does_not_shift_the_
/// lines_after_it` covers growth at one hand-picked position; a shrink
/// by merging two lines was hand-traced as correct for one position
/// too, but the real report came from an unknown position inside a
/// 9-line paragraph this fixture's own pick only reaches 8 lines of —
/// so this tries *every* adjacent pair in turn, undoing between
/// trials, rather than trusting one hand-traced position to stand for
/// all of them.
///
/// **Rewritten for the replace.** The merged line is now its first piece
/// holding both halves' words, its other pieces and *every* piece of the
/// line it absorbed come off the page (they used to be painted in the page's
/// colour), and every other object on the page is exactly as it was. And all
/// the trials run on ONE open document, as they first did: undo is a page
/// snapshot, so it restores the page exactly — every object's words, colour,
/// box, origin and size, and the picture — and the next trial finds the whole
/// paragraph again. (While undo re-typed the old words it left a justified
/// line a few points short, the detector read that as the end of a paragraph,
/// and each trial had to open a fresh copy of the file.)
#[test]
fn merging_any_two_adjacent_lines_does_not_corrupt_the_others() {
    let path = r"C:\Users\hsili\Downloads\CAMINO elitee-plus 3.0.pdf";
    let mut app = PagifyApp::new(Some(path));
    let Some(_) = &app.tab_mut().doc else {
        eprintln!("skipping: CAMINO not present on this machine");
        return;
    };

    fn pick_cob_paragraph(app: &mut PagifyApp) -> bool {
        let runs = app.tab_mut().doc.as_ref().unwrap().session.text_runs(0).expect("runs");
        for run in &runs {
            if !run.text.contains("manufacturers") {
                continue;
            }
            let at = AppPoint {
                x: ((run.rect.left + run.rect.right) / 2.0) as f64,
                y: ((run.rect.top + run.rect.bottom) / 2.0) as f64,
            };
            let _ = app.pick_text_run(0, at);
            return app.tab_mut().edit.editing_run.is_some();
        }
        false
    }

    assert!(pick_cob_paragraph(&mut app), "the 'COB' paragraph was not found — has the fixture changed?");
    let line_count = app.tab_mut().edit.editing_run.as_ref().unwrap().lines.len();
    assert!(line_count >= 2, "need at least two lines to merge");
    app.tab_mut().edit.editing_run = None;

    let picture_start = tests_support::picture(&app);
    for merge_at in 0..line_count - 1 {
        assert!(pick_cob_paragraph(&mut app), "merge_at={merge_at}: could not re-pick the paragraph");

        let edit = app.tab_mut().edit.editing_run.as_ref().expect("still editing");
        let original_lines: Vec<String> = edit.buffer.split('\n').map(str::to_string).collect();
        let lines_before = edit.lines.clone();
        assert_eq!(original_lines.len(), line_count, "merge_at={merge_at}: line count drifted between picks");
        assert!(
            !edit.frozen[merge_at] && !edit.frozen[merge_at + 1],
            "merge_at={merge_at}: a line drawn as shapes cannot be merged"
        );
        let before = tests_support::runs_in_order(&app);

        // Merge lines `merge_at` and `merge_at + 1` into one, deleting
        // the newline between them — a Backspace at the start of the
        // second line, the exact gesture the user described.
        let mut new_lines = original_lines.clone();
        let second = new_lines.remove(merge_at + 1);
        new_lines[merge_at] = format!("{}{}", new_lines[merge_at], second);

        let merged_text = new_lines[merge_at].clone();
        let edit = app.tab_mut().edit.editing_run.as_mut().expect("still editing");
        edit.buffer = new_lines.join("\n");
        app.apply_editing_page();
        let after = tests_support::runs_in_order(&app);

        // The merged line is its first piece holding both halves' words;
        // its other pieces and every piece of the absorbed line are gone;
        // every other object on the page — every piece of every other line,
        // and everything outside the paragraph — is exactly as it was.
        let (first, rest) = lines_before[merge_at].0.split_first().expect("the merged line has objects");
        let mut removed: Vec<usize> = rest.to_vec();
        removed.extend(&lines_before[merge_at + 1].0);
        tests_support::assert_page_after(
            &format!("merge_at={merge_at}"),
            &before,
            &after,
            &[(*first, &merged_text)],
            &removed,
        );

        // Undo: one step, and the page is exactly what it was.
        let (undone, _) = app.tab_mut().doc.as_ref().unwrap().session.undo().expect("undo");
        assert!(undone, "merge_at={merge_at}: the merge edit did not undo");
        let restored = tests_support::runs_in_order(&app);
        assert_eq!(restored.len(), before.len(), "merge_at={merge_at}: undo did not bring every piece back");
        for (was, now) in before.iter().zip(&restored) {
            assert_eq!(was.object, now.object, "merge_at={merge_at}: undo renumbered the page");
            assert!(
                tests_support::same_run(was, now),
                "merge_at={merge_at}: object {} did not come back exactly: {was:?} now {now:?}",
                was.object
            );
        }
        app.tab_mut().doc.as_mut().unwrap().caches.text = None;
    }
    assert!(
        tests_support::same_picture(&tests_support::picture(&app), &picture_start),
        "after every merge and undo the page is not pixel-identical to how it started"
    );
}

/// TEMPORARY: the user's own session log, on the real CAMINO file, shows
/// three *successive* paragraph edits in one running session getting
/// slower each time regardless of their own size — 9 lines: 5.3s, 7
/// lines: 8.0s, 11 lines: 15.0s — which a fresh-`PagifyApp`-per-edit test
/// like `editing_a_paragraph_is_fast_on_a_real_busy_page` could never
/// see, since it only ever makes one edit before the process (and the
/// document) goes away. This reuses one `app`/`doc` across several real
/// edits, the way an actual editing session does, to see whether the
/// same growth reproduces here. Delete once the real cause is found.
#[test]
#[ignore = "diagnostic only — always panics to print its own eprintln output"]
fn diag_camino_successive_paragraph_edits() {
    let path = r"C:\Users\hsili\Downloads\CAMINO elitee-plus 3.0.pdf";
    let mut app = PagifyApp::new(Some(path));
    let Some(_) = &app.tab_mut().doc else {
        eprintln!("skipping: CAMINO not present on this machine");
        return;
    };

    let mut tried: std::collections::HashSet<usize> = std::collections::HashSet::new();
    for round in 1..=4 {
        let runs = app.tab_mut().doc.as_ref().unwrap().session.text_runs(0).expect("runs");
        let mut line_count = 0usize;
        for run in &runs {
            if tried.contains(&run.object) || run.text.trim().chars().count() <= 4 {
                continue;
            }
            let at = AppPoint {
                x: ((run.rect.left + run.rect.right) / 2.0) as f64,
                y: ((run.rect.top + run.rect.bottom) / 2.0) as f64,
            };
            if app.pick_text_run(0, at).is_ok() {
                let lines = app.tab_mut().edit.editing_run.as_ref().map(|e| e.lines.len()).unwrap_or(0);
                if lines > 1 {
                    line_count = lines;
                    let objects: Vec<usize> = app
                        .tab_mut()
                        .edit.editing_run
                        .as_ref()
                        .unwrap()
                        .lines
                        .iter()
                        .flat_map(|(objs, _)| objs.iter().copied())
                        .collect();
                    tried.extend(objects);
                    break;
                }
            }
            app.tab_mut().edit.editing_run = None;
            tried.insert(run.object);
        }
        if line_count == 0 {
            eprintln!("round {round}: no fresh multi-line paragraph left to try");
            break;
        }

        let edit = app.tab_mut().edit.editing_run.as_mut().expect("still editing");
        edit.buffer = edit
            .buffer
            .split('\n')
            .map(|line| line.chars().rev().collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");

        let t = std::time::Instant::now();
        app.apply_editing_page();
        eprintln!("round {round}: {line_count}-line paragraph apply took {:?}", t.elapsed());
    }
    panic!("see stderr");
}

/// **Reported from use, on the real CAMINO file, after "Match the font"
/// had already been fixed once to actually change the typeface**: the
/// fix itself corrupted the page. Two adjacent runs — "...Efficacy 11"
/// and the "0" right after it — both went through `embed_typing_font`
/// in the same pass (one for the real font change, one retyping a run
/// that was already the right font for no reason), and the *boundary*
/// between them came back with a stray control character where a space
/// belonged. Root-caused to the second, unnecessary retype: a run
/// already in the target font family needs no edit at all, and skipping
/// it removed the adjacency that caused the corruption. Confirmed here
/// two ways: the page's own words read identically before and after
/// (whitespace-insensitive, since a face swap changing how many objects
/// draw one line can shift exactly where a synthetic word-gap space
/// lands without any real word changing), and the mismatched typeface
/// itself is gone from the page. Ported to Match Properties' own two
/// selections: the same span serves as both, since the sample's own run
/// is already in the target family and is skipped rather than retyped.
#[test]
fn matching_properties_does_not_corrupt_the_page() {
    let path = r"C:\Users\hsili\Downloads\CAMINO elitee-plus 3.0.pdf";
    let mut app = PagifyApp::new(Some(path));
    let Some(_) = &app.tab_mut().doc else {
        eprintln!("skipping: CAMINO not present on this machine");
        return;
    };

    let before_text = app.characters(0).expect("characters").text();

    let from = (278.0, 305.0);
    let to = (300.0, 305.0);
    let sample_range = app.characters(0).and_then(|c| c.range_between(from, to)).expect("a range");
    app.tab_mut().selection.text_selection = Some(sample_range);
    app.tab_mut().organize.selection_page = 0;
    app.match_properties_sample_from_current_selection().expect("sample should be accepted");

    let target_range = app.characters(0).and_then(|c| c.range_between(from, to)).expect("a range");
    app.tab_mut().selection.text_selection = Some(target_range);
    app.tab_mut().organize.selection_page = 0;
    app.apply_match_properties_to_current_selection().expect("match should succeed");

    app.tab_mut().doc.as_mut().unwrap().caches.text = None; // force a fresh read — `characters()` caches per page
    let after_text = app.characters(0).expect("characters").text();
    let squash = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    assert_eq!(
        squash(&after_text),
        squash(&before_text),
        "the page's own words must not change just from matching a font"
    );

    let after_runs = app.tab_mut().doc.as_ref().unwrap().session.text_runs(0).expect("runs");
    let still_arial_mt = after_runs.iter().any(|r| {
        (r.rect.top - 304.0).abs() < 10.0
            && r.rect.left > 270.0
            && r.rect.left < 320.0
            && app.tab_mut()
                .doc
                .as_ref()
                .unwrap()
                .session
                .run_font_name(0, r.object)
                .ok()
                .flatten()
                .as_deref()
                == Some("ArialMT")
    });
    assert!(!still_arial_mt, "the mismatched typeface should be gone from that line");
}

/// **Reported from use, screenshotted as "Pagify (Not Responding)":
/// the app froze solid the moment "Match the font" was clicked, on a
/// real, busy page — for a selection touching only two runs.** Picking
/// the sample used to ask every run on the page for its own font name
/// one at a time to build the list of alternates to try, and each of
/// those questions opened the page fresh to answer it — a few hundred
/// runs meant a few hundred page-opens for one click. See
/// `build_match_properties_sample`'s own doc for the fix. This is the
/// actual reported shape — a small selection, on the real file — timed
/// rather than merely asserted to succeed, since a slow-but-eventually-
/// correct answer would still be the bug.
#[test]
fn matching_properties_on_a_small_selection_is_fast_on_a_busy_page() {
    let path = r"C:\Users\hsili\Downloads\CAMINO elitee-plus 3.0.pdf";
    let mut app = PagifyApp::new(Some(path));
    let Some(_) = &app.tab_mut().doc else {
        eprintln!("skipping: CAMINO not present on this machine");
        return;
    };

    let from = (278.0, 305.0);
    let to = (300.0, 305.0);
    let sample_range = app.characters(0).and_then(|c| c.range_between(from, to)).expect("a range");
    app.tab_mut().selection.text_selection = Some(sample_range);
    app.tab_mut().organize.selection_page = 0;

    let started = std::time::Instant::now();
    app.match_properties_sample_from_current_selection().expect("sample should be accepted");
    let elapsed = started.elapsed();
    assert!(
        elapsed.as_secs() < 5,
        "picking the sample took {elapsed:?} on a busy page — this used to hang the whole app"
    );
}

/// **Reported from use, on the real CAMINO file**: a selection running
/// from inside "Luminaire Efficacy 11" (Montserrat-Light) through
/// "0 lm/w" — the "0" alone set in ArialMT, visibly a different
/// typeface — picked the wrong run as Match Properties' own sample. The
/// wide first run's own *centre* sits under "Luminaire Efficacy", far to
/// the left of where a drag starting mid-run actually lands, so the old
/// centre-point test dropped it and treated something else as the
/// start. Reproduced here without CAMINO: a selection starting near a
/// run's own trailing edge — nowhere near its centre — extending into a
/// second, deliberately mismatched run.
#[test]
fn matching_properties_finds_the_sample_the_selection_only_starts_inside() {
    let mut app = app("two-column.pdf");
    let runs = app.tab_mut().doc.as_ref().unwrap().session.text_runs(0).expect("runs");
    let a = runs.iter().min_by(|x, y| x.rect.top.total_cmp(&y.rect.top)).unwrap().clone();
    let b = runs.iter().max_by(|x, y| x.rect.top.total_cmp(&y.rect.top)).unwrap().clone();
    assert_ne!(a.object, b.object, "setup: need two distinct runs");

    // Deliberately near `a`'s own trailing edge, not its centre — the
    // exact shape that broke before this fix.
    let from = (a.rect.right - 1.0, (a.rect.top + a.rect.bottom) / 2.0);
    let to = ((b.rect.left + b.rect.right) / 2.0, (b.rect.top + b.rect.bottom) / 2.0);
    let range = app.characters(0).and_then(|c| c.range_between(from, to)).expect("a range");
    app.tab_mut().selection.text_selection = Some(range);
    app.tab_mut().organize.selection_page = 0;

    app.match_properties_sample_from_current_selection().expect("sample should be accepted");
    let sample = match app.tab_mut().tool.as_ref().map(|t| &t.kind) {
        Some(Tool::MatchProperties { sample: Some(s) }) => s,
        _ => panic!("sample should be held"),
    };
    assert!(
        (sample.size - a.size).abs() < 0.01,
        "should have found `a` even though the selection only barely starts inside it: \
         sample size {} vs a's own size {}",
        sample.size,
        a.size
    );
}
