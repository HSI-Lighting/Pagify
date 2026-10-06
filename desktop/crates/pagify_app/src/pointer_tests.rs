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

fn at(x: f64, y: f64) -> AppPoint {
    AppPoint { x, y }
}

fn marks(app: &PagifyApp) -> usize {
    app.tab().markup.existing(app.tab().page).map(|l| l.len()).unwrap_or(0)
}

fn said(app: &PagifyApp) -> String {
    app.cmd.history().iter().map(|e| e.text.as_str()).collect::<Vec<_>>().join("\n")
}

// -- drawing ----------------------------------------------------------

/// Pick the tool, then click twice. This is the whole interaction, and
/// nothing tested it: every drawing test until now typed both coordinates
/// on one line, which skips arming, picking and the readiness check.
#[test]
fn the_line_tool_draws_from_two_clicks() {
    let mut app = app("single-page.pdf");
    app.submit("line");
    assert!(app.tab_mut().pending.is_some(), "line did not arm:\n{}", said(&app));

    app.take_pick(at(10.0, 10.0));
    assert_eq!(marks(&app), 0, "one click drew a line");
    app.take_pick(at(100.0, 100.0));

    assert_eq!(marks(&app), 1, "two clicks drew nothing:\n{}", said(&app));
    // **Still in hand.** A tool is something you pick up and keep using;
    // one that lets go after a single line means going back to the ribbon
    // between every line, and a box becomes four trips.
    assert!(app.tab_mut().pending.is_some(), "the line tool let go after one line");

    app.take_pick(at(120.0, 10.0));
    app.take_pick(at(200.0, 90.0));
    assert_eq!(marks(&app), 2, "the second line needed the tool choosing again");

    app.escape();
    assert!(app.tab_mut().pending.is_none(), "escape did not put the tool down");
}

#[test]
fn the_circle_tool_draws_from_two_clicks() {
    let mut app = app("single-page.pdf");
    app.submit("circle");
    app.take_pick(at(80.0, 80.0));
    app.take_pick(at(120.0, 80.0));
    assert_eq!(marks(&app), 1, "circle drew nothing:\n{}", said(&app));
}

/// A polyline has no fixed number of points, so it ends on a command
/// rather than on a count. If that is wrong it never finishes.
#[test]
fn a_polyline_takes_any_number_of_clicks_and_then_finishes() {
    let mut app = app("single-page.pdf");
    app.submit("pline");
    for p in [(10.0, 10.0), (60.0, 30.0), (110.0, 10.0), (160.0, 40.0)] {
        app.take_pick(at(p.0, p.1));
    }
    assert_eq!(marks(&app), 0, "a polyline finished on its own");

    // The typed form of pressing Enter. Without it a polyline can only be
    // ended with the keyboard, and a recorded session could never end one.
    app.submit("done");
    assert_eq!(marks(&app), 1, "polyline never finished:\n{}", said(&app));
}

/// Fillet needs two *objects*, not two points — the picking path is a
/// different one, and it is the path that was silently performing a move.
/// Every coordinate here arrives the way the pointer delivers it.
///
/// Deliberately not mixed with typed ones: a typed `l 30,250` is in the
/// kernel's space, y up from the bottom-left, and a click is in app space,
/// y down from the top-left. The two are the same numbers and different
/// places, and a test that mixes them misses by the height of the page.
#[test]
fn fillet_picks_two_marks_and_joins_them() {
    let mut app = app("single-page.pdf");

    app.submit("line");
    app.take_pick(at(30.0, 60.0));
    app.take_pick(at(170.0, 60.0));
    app.submit("line");
    app.take_pick(at(170.0, 60.0));
    app.take_pick(at(170.0, 160.0));
    let before = marks(&app);
    assert_eq!(before, 2, "the two lines were not drawn:\n{}", said(&app));

    app.submit("fillet 25");
    app.take_pick(at(100.0, 60.0));
    app.take_pick(at(170.0, 120.0));

    assert!(marks(&app) > before, "fillet did nothing:\n{}", said(&app));
}

/// Clicking where there is no mark must say so rather than consuming the
/// click, or the operation silently collects nonsense.
#[test]
fn picking_empty_paper_for_an_object_says_so() {
    let mut app = app("single-page.pdf");
    app.submit("l 10,10 100,100");
    app.submit("fillet 20");
    app.take_pick(at(500.0, 500.0));

    assert!(said(&app).contains("nothing there"), "no complaint:\n{}", said(&app));
}

/// Scratch: is a real catalogue page hit-testable?
#[test]
#[ignore]
fn catalogue_hit_check() {
    let path = "/Users/hsilighting/Downloads/HSI CATALOG 2026.pdf";
    let mut app = PagifyApp::new(Some(path));
    assert!(app.tab_mut().doc.is_some(), "did not open");

    for page in 0..6usize {
        let Some(chars) = app.characters(page) else {
            println!("page {}: no characters at all", page + 1);
            continue;
        };
        let n = chars.len();
        let rects = chars.line_rects(0..n.min(1));
        let Some(r) = rects.into_iter().next() else {
            println!("page {}: {n} characters, NO BOXES", page + 1);
            continue;
        };
        let mid = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
        let hit = chars.hit(mid.0, mid.1);
        println!(
            "page {}: {n} chars, first box [{:.1},{:.1} {:.1},{:.1}], hit at {mid:?} -> {hit:?}",
            page + 1, r.left, r.top, r.right, r.bottom
        );
    }
}

/// Extract Text on a page that already has some real text must not write a
/// second copy of it. Measured on a real report before the fix: 384
/// characters became 1,687 and `Component` appeared twice.
#[test]
#[ignore]
fn extract_text_does_not_duplicate_text_already_on_the_page() {
    let path = format!("{}/Desktop/8_144498923727031132.pdf", std::env::var("HOME").unwrap());
    if !std::path::Path::new(&path).exists() {
        eprintln!("skipping: the report is not on the Desktop");
        return;
    }
    let mut app = PagifyApp::new(Some(&path));
    assert!(app.tab_mut().doc.is_some(), "the report did not open");
    if PagifyApp::model_directory().is_none() {
        eprintln!("skipping: no recognition models");
        return;
    }

    let before = app.characters(0).map(|c| c.text()).unwrap_or_default();
    let native = before.matches("Component").count();
    assert_eq!(native, 1, "the fixture no longer says `Component` exactly once");

    app.submit("extracttext");
    app.wait_for_reading();
    app.tab_mut().doc.as_mut().unwrap().caches.text = None;

    let after = app.characters(0).map(|c| c.text()).unwrap_or_default();
    assert_eq!(
        after.matches("Component").count(),
        1,
        "a word already on the page was written a second time:\n{}",
        said(&app)
    );
    assert!(
        after.chars().count() > before.chars().count() + 200,
        "the outlined prose was not added ({} -> {} characters)",
        before.chars().count(),
        after.chars().count()
    );
}

/// The window must keep running while a page is being read. Recognition is
/// about a second of solid CPU; on the UI thread that is a second of dead
/// window, and twenty seconds on a twenty-page document.
/// Ignored by default with the rest of the recognition tests: a debug build
/// reads a page in about three minutes against one second in release. Run
/// with `cargo test -p pagify_app --release extract -- --ignored`.
#[test]
#[ignore]
fn reading_a_page_does_not_block_the_window() {
    let mut app = app("outlined.pdf");
    if PagifyApp::model_directory().is_none() {
        eprintln!("skipping: no recognition models");
        return;
    }

    let start = std::time::Instant::now();
    app.submit("extracttext");
    let dispatched = start.elapsed();

    assert!(
        app.tab_mut().reading.is_some(),
        "the read finished inline, so it ran on this thread:\n{}",
        said(&app)
    );
    assert!(
        dispatched < std::time::Duration::from_millis(250),
        "asking for a read took {dispatched:?} — it is still being done inline"
    );

    app.wait_for_reading();
    assert!(app.tab_mut().reading.is_none());
}

/// A second request while one is running must be refused rather than
/// queued or run alongside: two pages at once doubles the peak memory, and
/// one page already measured at 482 MB.
/// Ignored by default with the rest of the recognition tests: a debug build
/// reads a page in about three minutes against one second in release. Run
/// with `cargo test -p pagify_app --release extract -- --ignored`.
#[test]
#[ignore]
fn a_second_read_is_refused_while_one_is_running() {
    let mut app = app("outlined.pdf");
    if PagifyApp::model_directory().is_none() {
        return;
    }
    app.submit("extracttext");
    app.submit("extracttext");

    assert!(said(&app).contains("already reading"), "no refusal:\n{}", said(&app));
    app.wait_for_reading();
}

/// Ignored by default with the rest of the recognition tests: a debug build
/// reads a page in about three minutes against one second in release. Run
/// with `cargo test -p pagify_app --release extract -- --ignored`.
#[test]
#[ignore]
fn escape_gives_up_on_a_page_being_read() {
    let mut app = app("outlined.pdf");
    if PagifyApp::model_directory().is_none() {
        return;
    }
    app.submit("extracttext");
    app.escape();
    app.wait_for_reading();

    assert!(said(&app).contains("stopped after"), "escape did nothing:\n{}", said(&app));
    assert_eq!(
        app.characters(0).map(|c| c.len()).unwrap_or(0),
        0,
        "a declined read wrote its layer anyway"
    );
}

// -- closing with unsaved work -----------------------------------------

/// The trap this replaces: the window would not close, and the only way out
/// was a command the user had to be told about. Abandoning work is a normal
/// thing to do — a mark put down to measure something, a line drawn to
/// check a distance — and a program that will not let go of it is one you
/// have to kill.
#[test]
fn closing_with_unsaved_marks_asks_rather_than_refusing() {
    let mut app = app("single-page.pdf");
    app.submit("l 10,10 100,100");
    assert!(app.unsaved().is_some(), "the mark was not counted as unsaved");

    app.submit("close");
    assert_eq!(app.tab().closing, Some(Closing::Document), "close did not ask:\n{}", said(&app));
    assert!(app.tab_mut().doc.is_some(), "it closed anyway, losing the mark");
}

#[test]
fn quitting_with_unsaved_marks_asks_too() {
    let mut app = app("single-page.pdf");
    app.submit("l 10,10 100,100");
    app.submit("quit");
    assert_eq!(app.tab_mut().closing, Some(Closing::Program));
}

/// Opening something else no longer touches the current document at all
/// — it lands in a new tab, so there is nothing to ask about.
#[test]
fn opening_another_file_does_not_ask() {
    let mut app = app("single-page.pdf");
    app.submit("l 10,10 100,100");
    app.submit(&format!("open \"{}\"", fixture("two-column.pdf")));

    assert_eq!(app.tab().closing, None, "opening should never need to ask:\n{}", said(&app));
    assert_eq!(app.tabs.len(), 2, "opening another file should have made a second tab");
}

// -- tabs ---------------------------------------------------------------

/// Two tabs are independent — paging or zooming one leaves the other
/// showing exactly what it was showing when it was left.
#[test]
fn each_tab_keeps_its_own_page_and_zoom_across_a_switch() {
    let mut app = app("pages-ladder.pdf");
    app.submit("page 3");
    app.tab_mut().zoom = ZoomMode::Factor(2.0);
    let (page_a, zoom_a) = (app.tab().page, app.tab().zoom);

    app.submit(&format!("open \"{}\"", fixture("pages-ladder.pdf")));
    assert_eq!(app.tabs.len(), 2, "opening another file should have made a second tab");
    app.submit("page 1");
    app.tab_mut().zoom = ZoomMode::Factor(1.0);

    // The newest tab is the leftmost: the first document is now the second.
    app.active_tab = 1;
    assert_eq!(app.tab().page, page_a, "switching back lost the first tab's page");
    assert_eq!(app.tab().zoom, zoom_a, "switching back lost the first tab's zoom");

    app.active_tab = 0;
    assert_ne!(app.tab().page, page_a, "the second tab's own page should be unaffected");
    assert_eq!(app.tab().zoom, ZoomMode::Factor(1.0));
}

/// Closing a tab with unsaved marks prompts, exactly like closing today's
/// one and only document — and leaves the other tab, and its own marks,
/// completely alone.
#[test]
fn closing_a_tab_with_unsaved_marks_prompts_without_touching_the_other() {
    let mut app = app("single-page.pdf");
    app.submit("l 10,10 100,100");
    app.submit(&format!("open \"{}\"", fixture("two-column.pdf")));
    assert_eq!(app.tabs.len(), 2);
    // The newest tab is the leftmost.
    assert_eq!(app.active_tab, 0, "opening should have switched to the new tab");
    app.submit("l 20,20 120,120");
    assert_eq!(app.tabs[0].markup.existing(0).map(|l| l.len()), Some(1));

    // The first document, now the second tab, has the unsaved mark.
    app.close_tab(1);
    assert_eq!(app.tabs.len(), 2, "a tab with unsaved work should not vanish on its own");
    assert_eq!(app.active_tab, 1, "asking about a tab should bring it to the front");
    assert_eq!(app.tab().closing, Some(Closing::Tab(1)));

    // Discard, driven directly the same way `discarding_actually_closes`
    // below drives `Closing::Document` — rendering the real modal needs
    // a full egui frame that a plain `Context::default()` cannot supply.
    let ctx = egui::Context::default();
    app.tab_mut().closing = None;
    app.finish_closing(Closing::Tab(1), &ctx);

    assert_eq!(app.tabs.len(), 1, "discarding should have closed just the one tab");
    assert!(
        app.tab().doc.as_ref().unwrap().session.path().to_string_lossy().contains("two-column"),
        "the wrong tab was closed"
    );
    assert_eq!(
        app.tab().markup.existing(0).map(|l| l.len()),
        Some(1),
        "the surviving tab's own mark should be untouched"
    );
}

/// A tab with nothing unsaved closes immediately, no different from
/// today's `close!` — and the tab that is left becomes the one showing.
#[test]
fn closing_a_clean_tab_removes_it_without_asking() {
    let mut app = app("single-page.pdf");
    app.submit(&format!("open \"{}\"", fixture("two-column.pdf")));
    assert_eq!(app.tabs.len(), 2);

    // single-page.pdf, the first one opened, is now the second tab.
    app.close_tab(1);
    assert_eq!(app.tabs.len(), 1, "a clean tab should close outright");
    assert!(app.tab().closing.is_none(), "a clean tab has nothing to ask about");
    assert!(
        app.tab().doc.as_ref().unwrap().session.path().to_string_lossy().contains("two-column"),
        "the remaining tab should be the one that was not closed"
    );
}

/// The point of the whole change.
#[test]
fn discarding_actually_closes() {
    let mut app = app("single-page.pdf");
    app.submit("l 10,10 100,100");
    app.submit("close");
    assert!(app.tab_mut().closing.is_some());

    let ctx = egui::Context::default();
    app.tab_mut().closing = None;
    app.finish_closing(Closing::Document, &ctx);

    assert!(app.tab_mut().doc.is_none(), "discarding did not close the document");
    assert!(app.unsaved().is_none(), "the marks came with it");
}

/// The forcing form still works, for anyone who already knows it and for
/// recorded sessions, which have no one to answer a dialog.
#[test]
fn the_forcing_form_still_discards_without_asking() {
    let mut app = app("single-page.pdf");
    app.submit("l 10,10 100,100");
    app.submit("close!");

    assert!(app.tab_mut().closing.is_none(), "close! put up a dialog");
    assert!(app.tab_mut().doc.is_none(), "close! did not close:\n{}", said(&app));
}

// -- marking text ------------------------------------------------------

/// `highlight` used to answer "select some text first" and then do nothing
/// once you had. All four markups now act on the selection.
#[test]
fn marking_the_selection_writes_one_annotation_per_selection() {
    for command in ["highlight", "underline", "strikeout", "squiggly"] {
        let mut app = app("text-lines.pdf");
        let chars = app.characters(0).expect("characters").clone();
        let n = chars.len().min(12);
        app.tab_mut().text_selection = Some(0..n);
        app.tab_mut().selection_page = 0;

        app.submit(command);

        let marks = app.tab_mut()
            .doc
            .as_ref()
            .unwrap()
            .session
            .annotations(0)
            .expect("read annotations");
        assert_eq!(
            marks.len(),
            1,
            "`{command}` wrote {} annotations:\n{}",
            marks.len(),
            said(&app)
        );
    }
}

/// A selection spanning several lines is one thing the reader made, so it
/// must be one mark — erasing it is then one action rather than three.
#[test]
fn a_selection_over_several_lines_is_a_single_mark() {
    let mut app = app("text-lines.pdf");
    let (len, lines) = {
        let chars = app.characters(0).expect("characters");
        (chars.len(), chars.line_rects(0..chars.len()).len())
    };
    app.tab_mut().text_selection = Some(0..len);
    app.tab_mut().selection_page = 0;

    assert!(lines > 1, "the fixture is only one line, so this proves nothing");

    app.submit("underline");
    let marks = app.tab_mut().doc.as_ref().unwrap().session.annotations(0).expect("read");
    assert_eq!(marks.len(), 1, "{lines} lines became {} marks", marks.len());
}

/// With nothing selected the tool is **picked up**, not refused.
///
/// It used to answer "select some text first", which meant the tool could
/// only ever be applied once per selection and read as a button that
/// scolded you rather than one that did something.
#[test]
fn marking_with_nothing_selected_picks_the_tool_up() {
    let mut app = app("text-lines.pdf");
    app.submit("underline");

    assert!(app.tab_mut().markup_armed.is_some(), "the tool was not picked up:\n{}", said(&app));
    assert!(
        said(&app).contains("drag across the text"),
        "it did not say what to do next:\n{}",
        said(&app)
    );
    assert!(
        app.tab_mut().doc.as_ref().unwrap().session.annotations(0).unwrap().is_empty(),
        "picking the tool up marked something"
    );
}

// -- extract text ------------------------------------------------------

/// `outlined.pdf` is exactly the case this exists for: real glyph contours
/// drawn as filled paths, no text object anywhere. It renders as words and
/// contains not one character to select.
/// Ignored by default: recognition is ~1 s a page in release and over a
/// minute in a debug build, which is not a cost the ordinary suite should
/// pay on every run. Run with
/// `cargo test -p pagify_app --release extract -- --ignored`.
#[test]
#[ignore]
fn extract_text_makes_an_outlined_page_selectable() {
    let mut app = app("outlined.pdf");
    if PagifyApp::model_directory().is_none() {
        eprintln!("skipping: no recognition models");
        return;
    }

    assert!(
        app.characters(0).map(|c| c.len()).unwrap_or(0) < 4,
        "the fixture already has selectable text, so this proves nothing"
    );

    app.submit("extracttext");
    app.wait_for_reading();
    app.tab_mut().doc.as_mut().unwrap().caches.text = None;
    let after = app.characters(0).map(|c| c.len()).unwrap_or(0);
    assert!(after > 4, "nothing became selectable:\n{}", said(&app));

    // And it must be selectable, not merely present: a layer in the wrong
    // place reads back fine and cannot be clicked on.
    let chars = app.characters(0).expect("characters").clone();
    let r = chars.line_rects(0..1).into_iter().next().expect("no box");
    let mid = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
    assert!(
        chars.hit(mid.0, mid.1).is_some(),
        "the text layer is not where its own boxes say it is"
    );
}

/// It changes the document, so it has to be reversible.
#[test]
#[ignore]
fn an_extracted_text_layer_can_be_undone() {
    let mut app = app("outlined.pdf");
    if PagifyApp::model_directory().is_none() {
        return;
    }
    app.submit("extracttext");
    app.wait_for_reading();
    app.tab_mut().doc.as_mut().unwrap().caches.text = None;
    let added = app.characters(0).map(|c| c.len()).unwrap_or(0);
    assert!(added > 4, "nothing to undo:\n{}", said(&app));

    app.submit("undo");
    app.tab_mut().doc.as_mut().unwrap().caches.text = None;
    let after = app.characters(0).map(|c| c.len()).unwrap_or(0);
    assert!(after < added, "undo left the layer in place ({added} -> {after})");
}

/// `outlined-montserrat.pdf` is the same generator as `outlined.pdf`, but
/// drawn with the face this build actually bundles — a same-font match,
/// measured at ~0.84–0.87 against the 0.5 trustworthiness floor — see the
/// calibration table on `outlined_words_are_trustworthy` itself. Unlike
/// the tests above, this checks not just that the page became selectable
/// but that OCR's lazy loader
/// (`self.recogniser`) was never reached to do it: `Some` would only
/// appear there if some page in the batch had fallen through to OCR.
///
/// That is also why this one is not `#[ignore]`d and does not check
/// `model_directory()`: a correctly working fast path needs no models on
/// disk at all, and this proves exactly that rather than assuming it.
#[test]
fn extract_text_on_a_same_font_outlined_page_never_touches_ocr() {
    let mut app = app("outlined-montserrat.pdf");
    assert!(
        app.characters(0).map(|c| c.len()).unwrap_or(0) < 4,
        "the fixture already has selectable text, so this proves nothing"
    );
    assert!(app.recogniser.is_none(), "OCR was already warm before the test ran");

    app.submit("extracttext");
    app.wait_for_reading();
    app.tab_mut().doc.as_mut().unwrap().caches.text = None;

    let after = app.characters(0).map(|c| c.len()).unwrap_or(0);
    assert!(after > 4, "nothing became selectable:\n{}", said(&app));
    assert!(
        app.recogniser.is_none(),
        "the page was read by OCR, not the vector-match fast path"
    );
}

/// A bad path must be refused the moment it is typed, before it ever
/// reaches disk as a saved setting — see `add_outlined_font`, which only
/// calls `OutlinedFonts::save` on the `Ok` branch. Run with no document
/// open, since adding a font is a preference, not a per-document action.
///
/// Checks only the error text, not the list's overall length: this app
/// instance loads the same real, on-disk settings file every other test
/// in this suite does, so another test's own font can legitimately be
/// sitting in it at the moment this one runs. What must be true
/// regardless is that *this* bad path never joined it.
#[test]
fn outlinedfont_reports_a_clear_error_for_a_bad_path() {
    let bad = "/definitely/not/a/real/path/9f3a.ttf";
    let mut app = PagifyApp::new(None);
    app.submit(&format!("outlinedfont {bad}"));
    assert!(said(&app).contains("could not read"), "unhelpful error:\n{}", said(&app));
    assert!(!app.outlined_fonts.paths.iter().any(|p| p == std::path::Path::new(bad)));
}

/// The end-to-end proof that adding a font actually changes what
/// `extracttext` can do without OCR — not just that `OutlinedFonts` can
/// hold a path (`outlined_fonts.rs` already covers that in isolation).
///
/// `outlined.pdf` is authored with system Arial — see
/// `make_text_fixtures.rs` — which the bundled Montserrat alone does not
/// resolve (~0.27 similarity, measured below the 0.5 trustworthiness
/// floor; the two `#[ignore]`d tests above exercise that fallback for
/// real). Adding Arial itself as an extra outlined font must therefore
/// flip this exact fixture onto the fast path — the only variable that
/// changed is the font list this test controls.
///
/// Self-cleaning: removes what it added, so a real font a person has
/// configured on this machine is never at risk from running the suite.
#[test]
fn a_user_added_font_unlocks_the_fast_path_for_a_page_the_bundled_fonts_do_not_match() {
    let arial = "/System/Library/Fonts/Supplemental/Arial.ttf";
    if !std::path::Path::new(arial).is_file() {
        eprintln!("skipping: {arial} not present on this machine");
        return;
    }

    let mut app = app("outlined.pdf");
    assert!(
        app.characters(0).map(|c| c.len()).unwrap_or(0) < 4,
        "the fixture already has selectable text, so this proves nothing"
    );

    app.submit(&format!("outlinedfont {arial}"));
    assert!(
        app.outlined_fonts.paths.iter().any(|p| p == std::path::Path::new(arial)),
        "the font was not added:\n{}",
        said(&app)
    );

    app.submit("extracttext");
    app.wait_for_reading();
    app.tab_mut().doc.as_mut().unwrap().caches.text = None;

    let after = app.characters(0).map(|c| c.len()).unwrap_or(0);
    let recognised_without_ocr = app.recogniser.is_none();

    app.submit(&format!("outlinedfont remove {arial}"));
    // Checks that Arial specifically is gone, not that the list is empty
    // outright — see the bad-path test above for why: another test's own
    // font may legitimately share this same on-disk settings file.
    assert!(
        !app.outlined_fonts.paths.iter().any(|p| p == std::path::Path::new(arial)),
        "cleanup left the font on the list"
    );

    assert!(after > 4, "nothing became selectable:\n{}", said(&app));
    assert!(
        recognised_without_ocr,
        "the page was read by OCR — the added font did not reach the worker"
    );
}

// -- encrypted files ---------------------------------------------------

/// A great many real working documents are encrypted — an extract saved out
/// of another editor, a drawing issued under restriction. Refusing them
/// with "document is password protected" and no way to supply one is a dead
/// end in a program whose entire interface is a place to type things.
#[test]
fn an_encrypted_file_asks_for_its_password() {
    let mut app = PagifyApp::new(None);
    app.submit(&format!("open \"{}\"", fixture("encrypted.pdf")));

    assert!(
        app.tab_mut().awaiting_password.is_some(),
        "an encrypted file did not ask:\n{}",
        said(&app)
    );
    assert!(said(&app).contains("encrypted"), "no explanation:\n{}", said(&app));
}

/// The password must reach PDFium and nothing else. `CommandBox::submit`
/// echoes every line into the visible history and the recorder keeps it for
/// replay, so a password typed as an ordinary command would be written into
/// both — and a recorded session would carry it to whoever it is shared
/// with.
#[test]
fn a_password_is_never_written_into_the_history() {
    let mut app = PagifyApp::new(None);
    app.submit(&format!("open \"{}\"", fixture("encrypted.pdf")));
    assert!(app.tab_mut().awaiting_password.is_some(), "the fixture did not ask for a password");

    app.submit("hunter2");
    let history = said(&app);
    assert!(
        !history.contains("hunter2"),
        "the password was echoed into the history:\n{history}"
    );
}

#[test]
fn a_wrong_password_asks_again_rather_than_giving_up() {
    let mut app = PagifyApp::new(None);
    app.submit(&format!("open \"{}\"", fixture("encrypted.pdf")));
    let path = app.awaiting_open().expect("it did not ask for a password");
    app.answer_open_password(&path, "not-the-password");

    assert!(
        app.tab_mut().awaiting_password.is_some(),
        "one wrong attempt ended it:\n{}",
        said(&app)
    );
    // The window says so, where the person is looking, rather than the
    // status line underneath it.
    assert_eq!(
        app.tab_mut().password_problem.as_deref(),
        Some("That password was not accepted."),
        "the window would say nothing about the wrong password"
    );
}

#[test]
fn the_right_password_opens_it() {
    let mut app = PagifyApp::new(None);
    app.submit(&format!("open \"{}\"", fixture("encrypted.pdf")));
    let path = app.awaiting_open().expect("it did not ask for a password");
    app.answer_open_password(&path, "pagify");

    assert!(app.tab_mut().awaiting_password.is_none(), "still asking:\n{}", said(&app));
    assert!(app.tab_mut().doc.is_some(), "the right password did not open it:\n{}", said(&app));
}

/// **The password that opened the file locks its own content too.**
///
/// Reported from use: having just typed the one password this document
/// asks for, being asked for it again to lock a passage inside the same
/// file read as the program not remembering a password it was just given
/// — not as a second, deliberately different one. Modelled on
/// `a_second_lock_uses_the_passcode_the_first_one_was_given`, which pins
/// the same rule for two locks in a row; this pins it for the password
/// that opened the document in the first place.
#[test]
fn the_password_that_opened_the_file_locks_its_own_content_too() {
    let mut app = PagifyApp::new(None);
    app.submit(&format!("open \"{}\"", fixture("encrypted.pdf")));
    let path = app.awaiting_open().expect("it did not ask for a password");
    app.answer_open_password(&path, "pagify");
    assert!(app.tab_mut().doc.is_some(), "control: it should have opened");

    app.ask_or_reuse_passcode(Awaiting::LockPages(vec![0]), "should never be shown");
    assert!(
        app.tab_mut().awaiting_password.is_none(),
        "it asked for a passcode it was already given to open the file"
    );
    assert!(
        !said(&app).contains("should never be shown"),
        "it showed the prompt anyway:\n{}",
        said(&app)
    );
    assert!(
        app.tab_mut().doc.as_ref().is_some_and(|d| !d.session.locked_pages().is_empty()),
        "the lock did not happen:\n{}",
        said(&app)
    );
}

/// **The same thing, but typed into the box and submitted the way the
/// window's own field would be** — `submit`, not `answer_open_password`
/// directly, so this actually goes through `consume_password_line`.
/// Every other password test in this file calls the lower-level answer
/// methods instead, which is exactly how an inverted guard in
/// `consume_password_line` (it cleared and swallowed the typed password
/// whenever one *was* being asked for, the opposite of what it must do)
/// went unnoticed: nothing else ever asked it to actually consume a line.
#[test]
fn typing_the_right_password_and_submitting_opens_it() {
    let mut app = PagifyApp::new(None);
    app.submit(&format!("open \"{}\"", fixture("encrypted.pdf")));
    assert!(app.tab_mut().awaiting_password.is_some(), "the fixture did not ask for a password");

    app.submit("pagify");

    assert!(app.tab_mut().awaiting_password.is_none(), "still asking:\n{}", said(&app));
    assert!(app.tab_mut().doc.is_some(), "typing the right password did not open it:\n{}", said(&app));
}

#[test]
fn escape_gives_up_on_the_password() {
    let mut app = PagifyApp::new(None);
    app.submit(&format!("open \"{}\"", fixture("encrypted.pdf")));
    app.escape();
    assert!(app.tab_mut().awaiting_password.is_none(), "escape did not give up");
}

// -- the two standing tools -------------------------------------------

/// Both were `Planned`, which is the worst thing they could have been: they
/// sit first on every tab, they are what anybody reaches for before
/// anything else, and answering "planned for the selection phase" reads as
/// *this program cannot select*.
#[test]
fn hand_and_select_actually_switch_the_pointer() {
    use pagify_shell::verbs::PointerMode;
    let mut app = app("text-lines.pdf");
    assert_eq!(app.tab_mut().pointer, PointerMode::Select, "the default is not Select");

    app.submit("hand");
    assert_eq!(app.tab().pointer, PointerMode::Pan, "hand did nothing:\n{}", said(&app));

    app.submit("selecttool");
    assert_eq!(app.tab().pointer, PointerMode::Select, "select did nothing:\n{}", said(&app));
}

/// Changing tool mid-pick has to abandon the pick. Otherwise the next click
/// on the page lands in an operation the user has already walked away from.
#[test]
fn switching_to_the_pointer_abandons_a_half_finished_tool() {
    let mut app = app("single-page.pdf");
    app.submit("line");
    app.take_pick(at(10.0, 10.0));
    assert!(app.tab_mut().pending.is_some());

    app.submit("selecttool");
    assert!(app.tab_mut().pending.is_none(), "the line tool survived the switch");

    app.take_pick(at(100.0, 100.0));
    assert_eq!(marks(&app), 0, "a click after switching still drew something");
}

#[test]
fn escape_returns_to_selecting() {
    use pagify_shell::verbs::PointerMode;
    let mut app = app("text-lines.pdf");
    app.submit("hand");
    app.escape();
    assert_eq!(
        app.tab_mut().pointer,
        PointerMode::Select,
        "escape left the pointer in Hand — which is not stopping"
    );
}

// -- text selection ---------------------------------------------------

/// The rule the canvas uses to decide between selecting text and selecting
/// marks: a drag that begins on a character selects text. If `hit` cannot
/// find a character under a point that is plainly on one, every drag
/// becomes a box-select and text can never be selected at all.
#[test]
fn a_point_on_a_character_is_recognised_as_text() {
    let mut app = app("text-lines.pdf");
    let chars = app.characters(0).expect("no characters").clone();
    assert!(chars.len() > 0, "the fixture has no characters");

    let first = chars.line_rects(0..1).into_iter().next().expect("no box for character 0");
    let mid = ((first.left + first.right) / 2.0, (first.top + first.bottom) / 2.0);

    assert!(
        chars.hit(mid.0, mid.1).is_some(),
        "the middle of the first character ({mid:?}) is not on text — \
         every drag would box-select instead"
    );
}

#[test]
fn dragging_across_a_line_selects_the_characters_between() {
    let mut app = app("text-lines.pdf");
    let chars = app.characters(0).expect("no characters").clone();
    let n = chars.len();
    assert!(n > 4, "fixture has too little text to drag across");

    let centre = |i: usize| {
        let r = chars.line_rects(i..i + 1).into_iter().next().expect("no box");
        ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0)
    };

    let range = chars.range_between(centre(0), centre(4)).expect("no range across one line");
    assert!(!range.is_empty(), "the range came back empty");
    assert!(range.end <= n, "range {range:?} runs past {n} characters");
    assert!(
        !chars.text_of(range.clone()).trim().is_empty(),
        "selected {range:?} and got no text out of it"
    );
}

/// Selecting has to survive the page being asked for twice — the character
/// cache is keyed by page, and a stale key silently selects on the wrong
/// one.
#[test]
fn the_character_cache_follows_the_page() {
    let mut app = app("two-column.pdf");
    let first = app.characters(0).map(|c| c.text()).unwrap_or_default();
    assert!(!first.is_empty(), "page 1 has no text");

    let again = app.characters(0).map(|c| c.text()).unwrap_or_default();
    assert_eq!(first, again, "asking twice gave different text");
}
