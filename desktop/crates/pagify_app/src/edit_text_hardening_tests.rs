//! The Edit Text paragraph path, hardened: tests for the things the first wiring
//! left open — faux-bold twins, pages too heavy to read in one pass, an editor
//! whose page changed under it, joined groups and caches that outlive the page
//! they were made from, a block whose clicked line cannot be written, the words
//! the person sees when a click cannot open a paragraph, and an apply the engine
//! refuses.
//!
//! A child of the crate root, so it reaches the app's private state the way the
//! tests in `main.rs` do. Real documents are the owner's and are skipped, with a
//! line saying so, where they are not on the machine.

use super::wrap_hyphen_tests::open_page;
use super::*;
use pdf_core::command::Command;
use pdf_core::document::{Rect, TextLineEdit, TextRun};

const MARINA: &str = r"C:\Users\hsili\Desktop\Datasheets - Editors market - Marina mall.pdf";

fn fixture(name: &str) -> String {
    format!("{}/../../../rust/pdf_core/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))
}

fn app(name: &str) -> PagifyApp {
    let app = PagifyApp::new(Some(&fixture(name)));
    assert!(app.tab().doc.is_some(), "{name} did not open");
    app
}

/// The middle of a box, as a click on it.
fn centre(rect: &Rect) -> AppPoint {
    AppPoint { x: ((rect.left + rect.right) / 2.0) as f64, y: ((rect.top + rect.bottom) / 2.0) as f64 }
}

fn runs_on(app: &PagifyApp, page: usize) -> Vec<TextRun> {
    let mut runs = app.tab().doc.as_ref().expect("open").session.text_runs(page).expect("runs");
    runs.sort_by_key(|r| r.object);
    runs
}

fn picture(app: &PagifyApp, page: usize) -> Vec<u8> {
    app.tab().doc.as_ref().expect("open").session.render_page(page, 2.0).expect("render").pixels
}

/// Everything said so far, one line each.
fn said(app: &PagifyApp) -> String {
    app.cmd.history().iter().map(|e| e.text.as_str()).collect::<Vec<_>>().join("\n")
}

/// The session log of `app` redirected to a scratch folder, and where to read it
/// back from. The folder is the caller's to remove.
fn log_into(app: &mut PagifyApp, tag: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("pagify-test-hardening-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    app.session_log = pagify_shell::session_log::SessionLog::start_in(dir.clone());
    let path = app.session_log.path().expect("the scratch folder is writable").to_path_buf();
    (dir, path)
}

/// The text of every log line of this kind, in order.
fn logged(path: &std::path::Path, kind: &str) -> Vec<String> {
    std::fs::read_to_string(path)
        .expect("the log file exists")
        .lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).expect("valid json"))
        .filter(|l| l["kind"] == kind)
        .map(|l| l["text"].as_str().unwrap_or_default().to_string())
        .collect()
}

/// `key=value` out of a pick line (values hold no spaces).
fn field<'a>(line: &'a str, key: &str) -> &'a str {
    line.split(' ')
        .find_map(|pair| pair.strip_prefix(key).and_then(|rest| rest.strip_prefix('=')))
        .unwrap_or_else(|| panic!("no `{key}=` in {line:?}"))
}

/// Click once; what came of it and the one log line it wrote.
fn click_logged(
    app: &mut PagifyApp,
    log: &std::path::Path,
    page: usize,
    at: AppPoint,
) -> (Result<String, String>, String) {
    let before = logged(log, "pick").len();
    let outcome = app.pick_text_run(page, at);
    let lines = logged(log, "pick");
    assert_eq!(lines.len(), before + 1, "a click must write exactly one pick line: {lines:?}");
    (outcome, lines.last().cloned().expect("a line"))
}

fn marina_is_here() -> bool {
    let here = std::path::Path::new(MARINA).is_file();
    if !here {
        eprintln!("skipping: the Marina datasheet is not on this machine");
    }
    here
}

// -- faux-bold twins --------------------------------------------------------------

/// **Retyping a heading that is drawn twice takes both copies off the page.**
/// "Color Options" on pages 1 and 2 of the datasheet is a faux bold: five readable
/// pieces (objects 817 to 821, 2650 to 2654) and five blank copies of them a hair
/// away (826 to 830, 2659 to 2663). Left behind by a retype they go on drawing the
/// old words over the new ones.
///
/// Checked three ways: the page loses exactly the four other pieces and the five
/// twins; what is drawn is **the same words typed once** — pixel for pixel, against a
/// page on which the twins were taken off first and the line retyped after; and one
/// undo puts every object and every pixel back.
#[test]
fn retyping_the_color_options_heading_takes_its_faux_bold_twins_off_the_page() {
    if !marina_is_here() {
        return;
    }
    let _turn = tests_support::one_at_a_time();
    for (page, first, twin_first) in [(0usize, 817usize, 826usize), (1, 2650, 2659)] {
        let what = format!("page {}", page + 1);
        let mut app = PagifyApp::new(Some(MARINA));
        let session = app.tab().doc.as_ref().expect("open").session.clone();
        let runs_before = runs_on(&app, page);
        let objects_before = session.page_scale(page).expect("scale").text_objects;
        let picture_before = picture(&app, page);

        let seed = runs_before.iter().find(|r| r.object == first).expect("the heading's first piece");
        app.pick_text_run(page, centre(&seed.rect)).expect("picked");
        let edit = app.tab().editing_run.as_ref().expect("an editor opened").clone();
        let pieces: Vec<usize> = (first..first + 5).collect();
        let twins: Vec<usize> = (twin_first..twin_first + 5).collect();
        assert_eq!(edit.lines.len(), 1, "{what}: the heading is one line");
        assert_eq!(edit.lines[0].0, pieces, "{what}: its five readable pieces");
        assert_eq!(edit.twins, vec![twins.clone()], "{what}: and its five twins, listed with the line");

        // The same letters in the other order: every glyph is in the font already.
        let typed: String = {
            let mut words: Vec<&str> = edit.buffer.split_whitespace().collect();
            words.reverse();
            words.join(" ")
        };
        assert_ne!(typed.trim(), edit.buffer.trim(), "{what}: setup: the words are not the same as they were");
        app.tab_mut().editing_run.as_mut().expect("editing").buffer = typed.clone();
        assert!(!app.apply_editing_page(), "{what}: refused: {}", said(&app));
        assert!(said(&app).contains("paragraph changed"), "{what}: {}", said(&app));
        let picture_after = picture(&app, page);
        assert_eq!(
            session.page_scale(page).expect("scale").text_objects,
            objects_before - 4 - 5,
            "{what}: the page should lose the four other pieces and the five twins, and nothing else"
        );
        assert!(
            runs_on(&app, page).iter().any(|r| r.text.contains(typed.split_whitespace().next().unwrap())),
            "{what}: the new words are not on the page"
        );
        assert_ne!(picture_after, picture_before, "{what}: setup: the retype changed nothing that is drawn");

        // The same words typed once, the other way round: the twins off first, the
        // line retyped after (nothing to take with it now).
        let mut other = PagifyApp::new(Some(MARINA));
        let other_session = other.tab().doc.as_ref().expect("open").session.clone();
        other_session
            .execute(Command::ReplaceTextLines { page_index: page, edits: vec![TextLineEdit::Remove { objects: twins.clone() }] })
            .expect("the twins come off");
        other.tab_mut().doc.as_mut().expect("open").rendered_is_stale();
        let seed = runs_on(&other, page).into_iter().find(|r| r.object == first).expect("the first piece");
        other.pick_text_run(page, centre(&seed.rect)).expect("picked");
        let again = other.tab().editing_run.as_ref().expect("an editor opened").clone();
        assert!(again.twins.is_empty(), "{what}: setup: no twins are left to list");
        other.tab_mut().editing_run.as_mut().expect("editing").buffer = typed;
        other.apply_editing_page();
        assert!(said(&other).contains("paragraph changed"), "{what}: {}", said(&other));
        assert!(
            picture_after == picture(&other, page),
            "{what}: the heading does not look like the same words typed once — an old word is still drawn"
        );

        // One undo, and the page is exactly what it was.
        let (undone, _) = session.undo().expect("undo");
        assert!(undone, "{what}: the apply did not undo in one step");
        let restored = runs_on(&app, page);
        assert_eq!(restored.len(), runs_before.len(), "{what}: undo did not bring every object back");
        for (was, now) in runs_before.iter().zip(&restored) {
            assert_eq!(was.object, now.object, "{what}: undo renumbered the page");
            assert!(tests_support::same_run(was, now), "{what}: undo did not restore a run exactly: {was:?} now {now:?}");
        }
        assert!(picture(&app, page) == picture_before, "{what}: the page is not pixel-identical after undo");
    }
}

// -- pages too heavy to read ---------------------------------------------------------

#[test]
fn a_page_is_heavy_when_it_holds_too_many_objects_or_too_many_words() {
    let scale = |text_objects, page_objects| pdf_core::document::PageScale { text_objects, page_objects };
    assert!(!page_is_heavy(&scale(0, 0)));
    assert!(!page_is_heavy(&scale(HEAVY_PAGE_TEXT_OBJECTS, HEAVY_PAGE_OBJECTS)), "the limits themselves are allowed");
    assert!(page_is_heavy(&scale(HEAVY_PAGE_TEXT_OBJECTS + 1, 0)), "too many words");
    assert!(page_is_heavy(&scale(10, HEAVY_PAGE_OBJECTS + 1)), "too many objects");
    // The datasheet's own pages (883, 1 488 and 1 394 text objects) are nowhere near.
    assert!(!page_is_heavy(&scale(1_488, 6_000)));
}

/// A page of a few lines of text and `rects` filled rectangles — a drawing, for
/// the purpose of how much the page holds.
fn drawing(name: &str, rects: usize) -> PagifyApp {
    let mut shapes = String::with_capacity(rects * 14);
    for _ in 0..rects {
        shapes.push_str("0 0 1 1 re f\n");
    }
    open_page(
        name,
        &[(100.0, 700.0, "first line of the words"), (100.0, 688.0, "second line of the words")],
        &shapes,
    )
}

/// **A page too heavy to read in one pass opens the word alone — and reads nothing
/// else.** The first click on a drawing of tens of thousands of objects (or a plan
/// of nearly a million) used to read the whole page, five times over, and render
/// all of it to sample a background colour, with the window stood still the while.
/// Now one count of the page's objects decides; past the limit the click finds the
/// word from its rectangle, reads that word, and stops.
#[test]
fn a_page_too_heavy_to_read_opens_the_word_alone_and_reads_nothing_else() {
    let mut app = drawing("heavy-click", HEAVY_PAGE_OBJECTS + 100);
    let (dir, log) = log_into(&mut app, "heavy");
    let weight = app.page_weight(0).expect("the page can be counted");
    assert!(page_is_heavy(&weight), "setup: {weight:?}");
    let run = runs_on(&app, 0).into_iter().next().expect("a run");

    let (outcome, line) = click_logged(&mut app, &log, 0, centre(&run.rect));
    let message = outcome.expect("the word opens");
    assert!(message.starts_with("opened this word alone: this page is very large"), "{message}");
    let edit = app.tab().editing_run.as_ref().expect("an editor opened");
    assert_eq!(edit.lines, vec![(vec![run.object], run.rect)], "the word alone, not its paragraph");
    assert_eq!(edit.buffer.trim(), run.text.trim());
    assert_eq!(field(&line, "path"), "single", "{line}");
    assert_eq!((field(&line, "frags"), field(&line, "blocks")), ("0", "0"), "nothing was detected: {line}");
    // **Nothing of the heavy read was made**: not the page's paragraphs, not the
    // picture the background colour is sampled from, and no font read for the box.
    let doc = app.tab().doc.as_ref().expect("open");
    assert!(doc.caches.page_blocks.borrow().is_none(), "the page was read for paragraphs");
    assert!(doc.caches.sampling.borrow().is_none(), "the page was rendered to sample a background");
    assert_eq!(edit.background, Color { r: 255, g: 255, b: 255, a: 255 }, "the box is drawn on white");
    assert!(app.editor_face.is_none() && app.pending_face.is_none(), "a font program was asked for");
    assert!(
        logged(&log, "pick-note").iter().any(|note| note.contains("too many to read for paragraphs")),
        "the guard did not say it fired: {:?}",
        logged(&log, "pick-note")
    );
    for note in logged(&log, "pick-note") {
        assert!(!note.contains("first line"), "a pick note holds the page's words: {note}");
    }

    // Counted once and read once: the second click costs no further walk of the page.
    let again = doc.caches.weight.get().expect("the count was kept");
    let (with_words, light) = doc.caches.rect_page.borrow().as_ref().map(|(w, p)| (*w, p.clone())).expect("the page's runs were kept");
    assert!(with_words, "a heavy page's words are read once, for every click after the first");
    app.tab_mut().editing_run = None;
    let _ = click_logged(&mut app, &log, 0, centre(&run.rect));
    let doc = app.tab().doc.as_ref().expect("open");
    assert_eq!(doc.caches.weight.get(), Some(again));
    assert!(
        std::rc::Rc::ptr_eq(&light, &doc.caches.rect_page.borrow().as_ref().expect("still kept").1),
        "the page's runs were read again"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A page of ordinary weight is still read for its paragraphs, and counted once.
#[test]
fn a_page_of_ordinary_weight_is_still_read_for_its_paragraphs() {
    let mut app = open_page(
        "light-click",
        &[
            (100.0, 700.0, "first line of the words"),
            (100.0, 688.0, "second line of the words"),
            (100.0, 676.0, "third line of the words"),
        ],
        "",
    );
    let (dir, log) = log_into(&mut app, "light");
    let run = runs_on(&app, 0).into_iter().next().expect("a run");
    let (outcome, line) = click_logged(&mut app, &log, 0, centre(&run.rect));
    assert!(outcome.expect("opens").starts_with("editing a paragraph of 3 lines"), "{line}");
    assert_eq!(field(&line, "path"), "block", "{line}");
    let doc = app.tab().doc.as_ref().expect("open");
    assert!(doc.caches.page_blocks.borrow().is_some() && doc.caches.weight.get().is_some());
    assert!(doc.caches.sampling.borrow().is_some(), "the background is still sampled from a picture of the page");
    assert!(logged(&log, "pick-note").is_empty(), "{:?}", logged(&log, "pick-note"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A click on bare paper of a page with that many objects does not look for words
/// the page draws as shapes — which walks and matches every one of them — and says
/// so rather than answering as though the click had missed.
#[test]
fn a_click_on_bare_paper_of_a_heavy_page_does_not_search_it_for_drawn_words() {
    let mut app = drawing("heavy-paper", HEAVY_PAGE_OBJECTS + 100);
    let (dir, log) = log_into(&mut app, "heavy-paper");
    let (outcome, line) = click_logged(&mut app, &log, 0, AppPoint { x: 20.0, y: 20.0 });
    let said = outcome.expect_err("bare paper");
    assert!(said.contains("no text there") && said.contains("too many for words drawn as shapes"), "{said}");
    assert_eq!(field(&line, "path"), "none", "{line}");
    assert!(app.tab().doc.as_ref().unwrap().caches.drawn_words.is_none(), "the page was searched for drawn words");
    let _ = std::fs::remove_dir_all(&dir);
}

// -- an editor whose page changed under it -------------------------------------------

/// An editor picked from a four-line page with a rectangle on it (object 4), with
/// its words changed and not yet applied.
fn editor_with_a_rectangle(name: &str) -> PagifyApp {
    let mut app = open_page(
        name,
        &[
            (100.0, 700.0, "first line of the words"),
            (100.0, 688.0, "second line of the words"),
            (100.0, 676.0, "third line of the words"),
            (100.0, 664.0, "fourth line of the words"),
        ],
        "0 0 50 50 re f\n",
    );
    let first = runs_on(&app, 0).into_iter().next().expect("a run");
    app.pick_text_run(0, centre(&first.rect)).expect("picked");
    let edit = app.tab().editing_run.as_ref().expect("an editor opened").clone();
    assert_eq!(edit.lines.len(), 4, "setup: the four lines are one paragraph");
    let retyped = edit.buffer.replacen("second", "SECOND", 1);
    app.tab_mut().editing_run.as_mut().expect("editing").buffer = retyped;
    app
}

/// **A page change that is not in the undo history refuses the open editor.** The
/// Layers panel restacks an object: the page is rewritten, every object number
/// after it moves, and the undo generation does not — the stamp the editor carried
/// before. The words typed would have landed on the objects that now have the old
/// numbers (reported with a reproduction: a word outside the paragraph was written
/// over). The epoch is the other half of the stamp.
#[test]
fn an_editor_whose_page_was_restacked_is_refused_and_the_page_is_untouched() {
    let mut app = editor_with_a_rectangle("stale-restack");
    let generation = app.doc_generation();
    // The rectangle to the back: it is object 0 now, and the text objects are 1 to 4.
    app.restack(0, 4, pdf_core::document::Stacking::Back).expect("restacked");
    assert_eq!(app.doc_generation(), generation, "setup: a restack does not move the history");
    let (runs_before, picture_before) = (runs_on(&app, 0), picture(&app, 0));
    assert!(runs_before.iter().all(|r| r.object >= 1), "setup: the page was renumbered: {runs_before:?}");

    assert!(!app.apply_editing_page(), "a stale editor is not a refusal to keep");
    assert!(said(&app).contains(STALE_EDITOR_MESSAGE), "{}", said(&app));
    assert!(app.tab().editing_run.is_none(), "the editor should be closed");
    let runs_after = runs_on(&app, 0);
    assert_eq!(runs_after.len(), runs_before.len());
    assert!(runs_before.iter().zip(&runs_after).all(|(a, b)| tests_support::same_run(a, b)), "the page changed");
    assert!(picture(&app, 0) == picture_before, "the page's picture changed");
    // The words that were typed are not lost with it.
    assert!(
        app.text_to_offer.as_deref().is_some_and(|text| text.contains("SECOND")),
        "the typed words were not offered back: {:?}",
        app.text_to_offer
    );
}

/// The same refusal for the other things that move the stamp: the view turned
/// (a false alarm, and a harmless one: the text is clicked again), a page inserted
/// (a history command), and the history moved behind the editor's back.
#[test]
fn an_editor_whose_view_was_turned_or_whose_history_moved_is_refused_too() {
    for (what, change) in [
        ("the view turned", 0),
        ("a page inserted", 1),
        ("an undo", 2),
    ] {
        let mut app = editor_with_a_rectangle(&format!("stale-{}", what.replace(' ', "-")));
        match change {
            0 => app.rotate_view(90),
            1 => app.insert_page(),
            _ => {
                // Something to undo first: picked again after it, so the editor is
                // fresh when the undo comes.
                let first = runs_on(&app, 0).into_iter().next().expect("a run");
                app.tab_mut().editing_run = None;
                app.pick_text_run(0, centre(&first.rect)).expect("picked");
                let retyped = app.tab().editing_run.as_ref().expect("an editor opened").buffer.replacen("first", "FIRST", 1);
                app.tab_mut().editing_run.as_mut().expect("editing").buffer = retyped;
                app.apply_editing_page();
                app.tab_mut().editing_run = None;
                app.pick_text_run(0, centre(&first.rect)).expect("picked again");
                let retyped = app.tab().editing_run.as_ref().expect("an editor opened").buffer.replacen("second", "SECOND", 1);
                app.tab_mut().editing_run.as_mut().expect("editing").buffer = retyped;
                let (undone, _) = app.tab().doc.as_ref().expect("open").session.undo().expect("undo");
                assert!(undone, "setup: there was something to undo");
            }
        }
        let before = runs_on(&app, 0);
        assert!(!app.apply_editing_page());
        assert!(said(&app).contains(STALE_EDITOR_MESSAGE), "{what}: {}", said(&app));
        assert!(app.tab().editing_run.is_none(), "{what}: the editor should be closed");
        let after = runs_on(&app, 0);
        assert!(
            before.len() == after.len() && before.iter().zip(&after).all(|(a, b)| tests_support::same_run(a, b)),
            "{what}: the page changed"
        );
    }
}

/// **An editor whose page changed under it is closed on the next frame**, without
/// anybody pressing Apply to be told: it names objects by number, and the words in
/// it are not worth writing onto other ones.
#[test]
fn an_editor_whose_page_changed_is_closed_on_the_next_frame() {
    let mut h = crate::ui_tests::harness("two-column.pdf");
    let runs = runs_on(h.state(), 0);
    let run = runs[runs.len() / 2].clone();
    h.state_mut().submit("edittext");
    h.run_steps(1);
    let at = h.state().tab().last_view.expect("the page was drawn").to_screen(centre(&run.rect));
    crate::ui_tests::click(&mut h, at);
    h.run_steps(2);
    assert!(h.state().tab().editing_run.is_some(), "setup: the click opened an editor");
    let typed = h.state().tab().editing_run.as_ref().unwrap().buffer.replacen('e', "E", 1);
    h.state_mut().tab_mut().editing_run.as_mut().unwrap().buffer = typed;
    h.run_steps(2);
    assert!(h.state().tab().editing_run.is_some(), "an editor over an unchanged page stays open");

    // The page is changed by a path that is not in the history.
    h.state_mut().tab_mut().doc.as_mut().expect("open").rendered_is_stale();
    h.run_steps(2);
    assert!(h.state().tab().editing_run.is_none(), "the editor was left open over a page that changed");
    assert!(said(h.state()).contains(STALE_EDITOR_MESSAGE), "{}", said(h.state()));
    assert!(h.state().text_to_offer.is_none(), "the typed words were left unsaid");
}

// -- joined groups -----------------------------------------------------------------

/// Join everything on the page of `app` into one group, by selecting all of its
/// characters the way a person does.
fn join_everything(app: &mut PagifyApp) {
    let total = app.characters(0).expect("characters").len();
    app.tab_mut().text_selection = Some(0..total);
    app.tab_mut().selection_page = 0;
    app.join_selected_text().expect("join");
    app.tab_mut().editing_run = None;
}

/// **A joined group is checked against the page before it is used.** Joined, then
/// the page renumbered by a path that is not in the history (a restack): the group
/// still names the old numbers, which now hold other words. Used, it opened a joined
/// editor over two unrelated words glued together — and applying would have written
/// over both. A group whose fingerprint no longer matches is forgotten.
#[test]
fn a_joined_group_whose_page_changed_is_not_used_and_is_forgotten() {
    let mut app = app("two-column.pdf");
    let (dir, log) = log_into(&mut app, "join-stale");
    let runs = runs_on(&app, 0);
    join_everything(&mut app);
    assert_eq!(app.tab().joined_groups.len(), 1, "setup: one group");
    let middle = runs[runs.len() / 2].clone();

    // While nothing changed the group is the editor a click opens.
    let (_, line) = click_logged(&mut app, &log, 0, centre(&middle.rect));
    assert_eq!(field(&line, "path"), "joined", "{line}");
    app.tab_mut().editing_run = None;

    // Then the page changes under it: the first object to the back renumbers all the rest.
    app.restack(0, runs.len() - 1, pdf_core::document::Stacking::Back).expect("restacked");
    let after = runs_on(&app, 0);
    let moved = after.iter().find(|r| r.text == middle.text).expect("the word is still on the page");
    let (_, line) = click_logged(&mut app, &log, 0, centre(&moved.rect));
    assert_ne!(field(&line, "path"), "joined", "a group that no longer matches the page was used: {line}");
    assert!(app.tab().joined_groups.is_empty(), "the stale group was kept: {:?}", app.tab().joined_groups);
    assert!(
        logged(&log, "pick-note").iter().any(|note| note.contains("joined group") && note.contains("forgotten")),
        "the log does not say it: {:?}",
        logged(&log, "pick-note")
    );
    // And no click anywhere on the page is a joined one any more.
    for run in &after {
        app.tab_mut().editing_run = None;
        let (_, line) = click_logged(&mut app, &log, 0, centre(&run.rect));
        assert_ne!(field(&line, "path"), "joined", "{line}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A join survives a page that cannot be read for a moment: it is the page that is
/// unreadable then, not the join that is wrong, and it comes back with the next
/// click that can read.
#[test]
fn a_joined_group_is_kept_while_the_page_cannot_be_read() {
    struct Restore;
    impl Drop for Restore {
        fn drop(&mut self) {
            tests_support::SNAPSHOT_FAILS.with(|fails| fails.set(false));
        }
    }
    let _restore = Restore;
    let mut app = app("two-column.pdf");
    let (dir, log) = log_into(&mut app, "join-unreadable");
    let runs = runs_on(&app, 0);
    join_everything(&mut app);
    let run = runs[runs.len() / 2].clone();

    // The reading the join was made from is no longer kept (a page that changed),
    // and the next one fails.
    app.tab_mut().doc.as_mut().expect("open").rendered_is_stale();
    tests_support::SNAPSHOT_FAILS.with(|fails| fails.set(true));
    let (_, line) = click_logged(&mut app, &log, 0, centre(&run.rect));
    assert_eq!(field(&line, "path"), "single", "{line}");
    assert_eq!(app.tab().joined_groups.len(), 1, "the join was deleted by a page that could not be read");

    tests_support::SNAPSHOT_FAILS.with(|fails| fails.set(false));
    app.tab_mut().editing_run = None;
    let (_, line) = click_logged(&mut app, &log, 0, centre(&run.rect));
    assert_eq!(field(&line, "path"), "joined", "the join did not come back: {line}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The fingerprint is of the runs' numbers, words and boxes — and of how many.
#[test]
fn a_fingerprint_changes_with_the_words_the_box_the_number_and_the_count() {
    let app = app("two-column.pdf");
    let (pb, _) = app.page_blocks(0).expect("the page");
    let objects: Vec<usize> = (0..4).collect();
    let base = group_fingerprint(&pb, &objects).expect("on the page");
    assert_eq!(group_fingerprint(&pb, &objects), Some(base), "stable");
    assert_ne!(group_fingerprint(&pb, &objects[..3]), Some(base), "the count");
    assert_ne!(group_fingerprint(&pb, &[1, 0, 2, 3]), Some(base), "the numbers, in their order");
    assert_eq!(group_fingerprint(&pb, &[0, 99_999]), None, "an object that is not on the page");
    let mut moved = (*pb).clone();
    moved.runs.get_mut(&2).expect("a run").rect.left += 1.0;
    assert_ne!(group_fingerprint(&moved, &objects), Some(base), "the box");
    let mut retyped = (*pb).clone();
    retyped.runs.get_mut(&2).expect("a run").text.push('x');
    assert_ne!(group_fingerprint(&retyped, &objects), Some(base), "the words");
}

// -- caches ------------------------------------------------------------------------

/// **The drawn words are recognised again once the page has changed.** The cache was
/// kept by page index and never emptied: a word taken off the page by a redaction was
/// still listed, and a click on where it had been opened an editor for it.
#[test]
fn drawn_words_are_recognised_again_after_the_page_changed() {
    let mut app = app("outlined-montserrat.pdf");
    let words = app.drawn_words_on(0).to_vec();
    assert!(!words.is_empty(), "no drawn words were recognised at all");
    let key = app.tab().doc.as_ref().unwrap().caches.drawn_words.as_ref().expect("kept").0;
    assert_eq!(key, app.doc_stamp(0), "kept under the stamp of the page it was made for");
    let word = words
        .iter()
        .filter(|w| !w.text.trim().is_empty())
        .max_by_key(|w| w.text.trim().chars().count())
        .cloned()
        .expect("a word");

    // Taken off the page through the history, as a redaction is.
    let faces = app.outlined_font_bytes();
    let outcome = app.tab().doc.as_ref().expect("open").session.execute(Command::Redact {
        page_index: 0,
        area: word.rect,
        fill: None,
        allow_incomplete: false,
        outlined_fonts: faces,
    });
    if outcome.is_err() {
        eprintln!("skipping: this page's shapes cannot be separated");
        return;
    }
    let now = app.drawn_words_on(0).to_vec();
    assert_ne!(app.tab().doc.as_ref().unwrap().caches.drawn_words.as_ref().expect("kept").0, key, "the old reading was served for a changed page");
    assert!(
        now.len() < words.len() && !now.iter().any(|w| w.text == word.text && w.rect == word.rect),
        "the redacted word is still listed: {} words then, {} now",
        words.len(),
        now.len()
    );
}

fn a_face(label: u8) -> CachedFace {
    CachedFace { program: Some(std::rc::Rc::new(vec![label])), metrics: None, key: u64::from(label), coverage: None }
}

/// Bounded, most recent first, and dropped with the stamp.
#[test]
fn the_face_cache_is_bounded_keeps_the_most_recent_and_is_dropped_with_the_stamp() {
    let mut cache = FaceCache::default();
    for font in 0..(FaceCache::CAPACITY as u32 + 4) {
        cache.put((1, 0, 0, 0, font), a_face(font as u8));
    }
    assert_eq!(cache.entries.len(), FaceCache::CAPACITY, "bounded");
    assert_eq!(cache.entries[0].0 .4, FaceCache::CAPACITY as u32 + 3, "the newest is first");
    assert!(cache.get(&(1, 0, 0, 0, 0)).is_none(), "the oldest was let go");
    // A hit becomes the most recent.
    let oldest_kept = cache.entries.last().expect("an entry").0;
    assert!(cache.get(&oldest_kept).is_some());
    assert_eq!(cache.entries[0].0, oldest_kept);
    // Another page in the same state is still good; another state of the same document is not.
    cache.put((1, 3, 0, 0, 7), a_face(70));
    assert!(cache.get(&(1, 3, 0, 0, 7)).is_some());
    assert!(cache.get(&(1, 0, 1, 0, 7)).is_none(), "another epoch");
    assert!(cache.entries.iter().all(|(key, _)| key.2 == 1 || key.0 != 1), "the old state's entries are dropped: {:?}", cache.entries.iter().map(|e| e.0).collect::<Vec<_>>());
    // Another document's entries are not this one's to throw away.
    cache.put((2, 0, 5, 5, 1), a_face(1));
    assert!(cache.get(&(1, 0, 9, 9, 1)).is_none());
    assert!(cache.entries.iter().any(|(key, _)| key.0 == 2), "another tab's entry was dropped");
}

/// **The editor's font is asked of the engine once per font per state of the page**:
/// two clicks on words set in one font share the same program (the same allocation,
/// not an equal copy), and a change of the page makes the next click ask again.
#[test]
fn the_editor_face_is_asked_of_the_engine_once_per_font_per_state_of_the_page() {
    if !marina_is_here() {
        return;
    }
    let _turn = tests_support::one_at_a_time();
    let mut app = PagifyApp::new(Some(MARINA));
    let runs: std::collections::HashMap<usize, TextRun> = runs_on(&app, 0).into_iter().map(|r| (r.object, r)).collect();
    app.pick_text_run(0, centre(&runs[&986].rect)).expect("picked");
    assert_eq!(app.face_cache.entries.len(), 1, "the body font was kept");
    let first = app.face_cache.entries[0].1.program.clone().expect("a program");
    let face = app.editor_face;
    assert!(face.is_some(), "setup: the editor asked for a face");

    // Another word of the same paragraph — the same font.
    app.tab_mut().editing_run = None;
    app.pick_text_run(0, centre(&runs[&990].rect)).expect("picked");
    assert_eq!(app.face_cache.entries.len(), 1, "one font, one entry");
    assert!(
        std::rc::Rc::ptr_eq(&first, app.face_cache.entries[0].1.program.as_ref().expect("a program")),
        "the second click asked the engine for the font again"
    );
    assert_eq!(app.editor_face, face);

    // The heading above it is another font: a second entry, most recent first.
    app.tab_mut().editing_run = None;
    app.pick_text_run(0, centre(&runs[&981].rect)).expect("picked");
    assert_eq!(app.face_cache.entries.len(), 2);
    assert_ne!(app.editor_face, face, "the heading's own face");

    // The page changes: nothing of the old state is served, and none of it is kept.
    app.tab_mut().editing_run = None;
    app.tab_mut().doc.as_mut().expect("open").rendered_is_stale();
    app.pick_text_run(0, centre(&runs[&986].rect)).expect("picked");
    assert_eq!(app.face_cache.entries.len(), 1, "the old state's fonts were kept: {}", app.face_cache.entries.len());
    assert!(
        !std::rc::Rc::ptr_eq(&first, app.face_cache.entries[0].1.program.as_ref().expect("a program")),
        "a font of the page as it was served the page as it is"
    );
}

// -- a click on a word on a line the page draws part of as shapes ------------------------

/// **A word beside a drawn word opens alone, and can be retyped.** The user's
/// paragraph has two lines with a word the page draws as shapes in them ("efficacy",
/// objects 1026 and 1035); the pieces around them are on frozen lines, which an apply
/// never writes. In the box that holds the paragraph the person typed into them and
/// the new words were left out ("left as it is"); the click now opens the piece
/// itself, which is the thing that can be changed.
#[test]
fn clicking_beside_a_word_drawn_as_shapes_opens_that_word_alone_and_retypes_only_it() {
    if !marina_is_here() {
        return;
    }
    let _turn = tests_support::one_at_a_time();
    let mut app = PagifyApp::new(Some(MARINA));
    let runs: std::collections::HashMap<usize, TextRun> = runs_on(&app, 0).into_iter().map(|r| (r.object, r)).collect();
    let (dir, log) = log_into(&mut app, "frozen-seed");
    for neighbour in [1025usize, 1034] {
        app.tab_mut().editing_run = None;
        let (outcome, line) = click_logged(&mut app, &log, 0, centre(&runs[&neighbour].rect));
        let message = outcome.expect("picked");
        assert!(
            message.starts_with("opened this word alone: its line is partly drawn as shapes"),
            "object {neighbour}: {message}"
        );
        assert_eq!(field(&line, "path"), "single", "{line}");
        let edit = app.tab().editing_run.as_ref().expect("an editor opened");
        assert_eq!(edit.lines, vec![(vec![neighbour], runs[&neighbour].rect)], "object {neighbour}: the word alone");
        assert_eq!(edit.frozen, [false]);
    }
    assert!(
        logged(&log, "pick-note").iter().all(|note| note.contains("drawn lettering")),
        "{:?}",
        logged(&log, "pick-note")
    );

    // Retyped, only that word changes.
    app.tab_mut().editing_run = None;
    app.pick_text_run(0, centre(&runs[&1025].rect)).expect("picked");
    let before = runs_on(&app, 0);
    let typed = format!("{} COB", app.tab().editing_run.as_ref().expect("an editor").buffer.trim_end());
    app.tab_mut().editing_run.as_mut().expect("editing").buffer = typed.clone();
    assert!(!app.apply_editing_page(), "{}", said(&app));
    let after = runs_on(&app, 0);
    tests_support::assert_page_after("the word beside a drawn word", &before, &after, &[(1025, &typed)], &[]);
    let _ = std::fs::remove_dir_all(&dir);
}

// -- what is said ------------------------------------------------------------------------

#[test]
fn a_count_says_one_line_and_many_lines() {
    assert_eq!(count_of(1, "line", "lines"), "1 line");
    assert_eq!(count_of(0, "line", "lines"), "0 lines");
    assert_eq!(count_of(13, "line", "lines"), "13 lines");
    assert_eq!(count_of(1, "character", "characters"), "1 character");
}

/// **"editing a paragraph of 1 lines"**, said on every click on a one-line block of
/// several pieces (the datasheet's "Description:" heading, 148 of them in the census).
#[test]
fn a_paragraph_of_one_line_says_1_line_and_not_1_lines() {
    let mut app = super::paragraph_apply_tests::two_piece_page("one-line", &[("Lorem ipsum dolor ", "sit amet consectetur")]);
    let first = runs_on(&app, 0).into_iter().next().expect("a run");
    let said = app.pick_text_run(0, centre(&first.rect)).expect("picked");
    assert!(said.starts_with("editing a paragraph of 1 line ("), "{said}");
    assert!(!said.contains("1 lines") && !said.contains(" 1 characters"), "{said}");
}

/// **A scan says so.** A click on a page that is a picture of words used to answer as
/// though it had missed some: "no text there — click on some words".
#[test]
fn a_scanned_page_says_it_is_scanned_and_what_reads_it() {
    let mut app = app("scan-300dpi.pdf");
    let (dir, log) = log_into(&mut app, "scan");
    let (outcome, line) = click_logged(&mut app, &log, 0, AppPoint { x: 300.0, y: 300.0 });
    let said = outcome.expect_err("nothing to click on a scan");
    assert!(said.contains("no text on this page") && said.contains("scanned") && said.contains("extracttext"), "{said}");
    assert_eq!(field(&line, "path"), "none", "{line}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// **A block the guard refuses opens the word alone, and says so** — one short
/// sentence at the head of the pick's message — besides the line in the log. Two
/// runs drawn at one origin are a block that cannot be written to safely.
#[test]
fn a_block_the_guard_refuses_opens_the_word_alone_and_says_so() {
    let mut app = open_page(
        "guard-refuses",
        &[
            (100.0, 700.0, "alpha beta gamma"),
            (100.0, 700.0, "zzz zzz zzz zzz zzz"),
            (100.0, 688.0, "second line of the paragraph"),
        ],
        "",
    );
    let (dir, log) = log_into(&mut app, "guard");
    let seed = runs_on(&app, 0).into_iter().find(|r| r.text.contains("second")).expect("a run");
    let (outcome, line) = click_logged(&mut app, &log, 0, centre(&seed.rect));
    let message = outcome.expect("opens");
    assert!(
        message.starts_with("opened this word alone: the block could not be read safely"),
        "the person was not told why the paragraph did not open: {message} ({line})"
    );
    assert_eq!(field(&line, "path"), "single", "{line}");
    let notes = logged(&log, "pick-note");
    assert!(notes.iter().any(|note| note.contains("not opened") && note.contains("picking the run alone")), "{notes:?}");
    for note in &notes {
        assert!(!note.contains("second line") && !note.contains("alpha"), "a pick note holds the page's words: {note}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

// -- a line the engine cannot stretch ---------------------------------------------------

/// A paragraph of lines that are exactly as wide as each other — the same letters in
/// another order — and so justified as far as the app can tell.
const FLUSH: [(f32, f32, &str); 5] = [
    (100.0, 700.0, "alpha beta gamma delta"),
    (100.0, 688.0, "delta gamma beta alpha"),
    (100.0, 676.0, "gamma delta alpha beta"),
    (100.0, 664.0, "beta alpha delta gamma"),
    (100.0, 652.0, "alpha beta"),
];

/// **A line the engine cannot stretch is written ragged, and the person is told.**
/// A retyped line of a justified paragraph is asked to keep the width it had; a
/// two-letter word cannot be spread over a whole line, and the engine refuses the
/// whole replace without saying which line asked too much. The apply asks again
/// without the width, and says the line came out ragged.
#[test]
fn a_line_the_engine_cannot_stretch_is_written_ragged_and_said() {
    let mut app = open_page("cannot-stretch", &FLUSH, "");
    let first = runs_on(&app, 0).into_iter().next().expect("a run");
    app.pick_text_run(0, centre(&first.rect)).expect("picked");
    let edit = app.tab().editing_run.as_ref().expect("an editor opened").clone();
    assert_eq!(edit.lines.len(), 5, "setup: one paragraph");
    assert!(paragraph_lines::ends_at_one_margin(&edit.lines) && paragraph_should_justify(&edit.lines), "setup: justified");
    let mut lines: Vec<String> = edit.buffer.split('\n').map(str::to_string).collect();
    lines[1] = "ab".to_string();
    app.tab_mut().editing_run.as_mut().expect("editing").buffer = lines.join("\n");
    assert!(!app.apply_editing_page(), "{}", said(&app));
    assert!(said(&app).contains("paragraph changed"), "{}", said(&app));
    assert!(
        said(&app).contains("could not be stretched to the width they had"),
        "the apply did not say the line came out ragged: {}",
        said(&app)
    );
    let after = runs_on(&app, 0);
    assert!(after.iter().any(|r| r.text.trim() == "ab"), "the words are not on the page: {after:?}");
}

// -- a refused apply -------------------------------------------------------------------

/// One run on a page, with a character no font here can write typed over its words.
fn a_run_with_an_unwritable_word(name: &str) -> PagifyApp {
    let mut app = open_page(name, &[(100.0, 700.0, "one lonely line")], "");
    let run = runs_on(&app, 0).into_iter().next().expect("a run");
    app.pick_text_run(0, centre(&run.rect)).expect("picked");
    app.tab_mut().editing_run.as_mut().expect("editing").buffer = "\u{3A9}".to_string();
    app
}

/// **A refused apply does not throw the typing away.** The engine refuses text it
/// cannot write (here an Omega, in a font that has none): the box used to be gone, with
/// the words in it, and a red line in a history that is shut. It stays, with the
/// words, and the reason beside Apply.
#[test]
fn a_refused_run_keeps_its_editor_and_its_words_and_the_page_is_untouched() {
    let mut app = a_run_with_an_unwritable_word("single-refused");
    let before = (runs_on(&app, 0), picture(&app, 0));
    assert!(app.apply_editing_page(), "the refusal was not reported: {}", said(&app));
    let last = app.cmd.history().last().expect("something was said").clone();
    assert!(matches!(last.kind, Kind::Error), "{last:?}");
    let kept = app.tab().editing_run.as_ref().expect("a refused edit keeps its editor");
    assert_eq!(kept.buffer, "\u{3A9}", "the typing was thrown away");
    let refusal = kept.refusal.as_ref().expect("the refusal is remembered");
    assert_eq!(refusal.reason, last.text);
    assert!(!app.editor_is_stale(kept), "the box was stamped for a page that is not there");
    let after = (runs_on(&app, 0), picture(&app, 0));
    assert!(
        before.0.len() == after.0.len() && before.0.iter().zip(&after.0).all(|(a, b)| tests_support::same_run(a, b)),
        "the page changed"
    );
    assert!(before.1 == after.1, "the page's picture changed");
}

/// **A refused edit is not a trap.** The first click away applies it and is refused;
/// the box stays (and the click does not also pick what is under it). A second click
/// away from the same edit lets it go, with the words on the clipboard — it would be
/// refused for the same reason, and only Escape would otherwise be a way out.
#[test]
fn a_second_click_away_from_the_same_refused_edit_lets_it_go_with_the_words_offered_back() {
    let mut app = a_run_with_an_unwritable_word("click-away-twice");
    assert!(app.leave_editor_by_click(), "the first click away should keep the refused editor");
    assert!(app.tab().editing_run.is_some());
    assert!(app.text_to_offer.is_none(), "the words were offered back while the box was still open");

    assert!(!app.leave_editor_by_click(), "the second click away should let it go");
    assert!(app.tab().editing_run.is_none(), "the editor is still there");
    assert_eq!(app.text_to_offer.as_deref(), Some("\u{3A9}"), "the typed words were not offered back");
    assert!(said(&app).contains("let go of the edit the engine refused"), "{}", said(&app));
    // Nothing was ever written.
    assert_eq!(runs_on(&app, 0)[0].text.trim(), "one lonely line");
}

/// An edit that **has changed** since it was refused is tried again by a click away:
/// the person corrected it, and the engine may take it now.
#[test]
fn an_edit_changed_after_a_refusal_is_tried_again_by_a_click_away() {
    let mut app = a_run_with_an_unwritable_word("click-away-changed");
    assert!(app.leave_editor_by_click(), "setup: refused once");
    app.tab_mut().editing_run.as_mut().expect("editing").buffer = "one LONELY line".to_string();
    assert!(!app.leave_editor_by_click(), "{}", said(&app));
    assert!(app.tab().editing_run.is_none());
    assert_eq!(runs_on(&app, 0)[0].text.trim(), "one LONELY line");
    assert!(app.text_to_offer.is_none(), "words that were written were offered back");
}

/// Escape lets a refused edit go too, and offers the words back.
#[test]
fn escape_after_a_refusal_offers_the_words_back() {
    let mut app = a_run_with_an_unwritable_word("escape-refused");
    assert!(app.apply_editing_page());
    app.escape();
    assert!(app.tab().editing_run.is_none());
    assert_eq!(app.text_to_offer.as_deref(), Some("\u{3A9}"));
    assert!(said(&app).contains("what was typed is on the clipboard"), "{}", said(&app));
    // An Escape with nothing refused offers nothing: the clipboard is not the app's to take.
    let mut plain = open_page("escape-plain", &[(100.0, 700.0, "one lonely line")], "");
    let run = runs_on(&plain, 0).into_iter().next().expect("a run");
    plain.pick_text_run(0, centre(&run.rect)).expect("picked");
    plain.tab_mut().editing_run.as_mut().expect("editing").buffer = "one LONELY line".to_string();
    plain.escape();
    assert!(plain.text_to_offer.is_none());
}

/// **The stamp stays good across the restore, and still refuses what it should**: the
/// refusal marks the page stale as a safety net (the epoch moves), and the box that
/// comes back is stamped for the page as it is — so a corrected edit applies — but a
/// page changed after the refusal is refused as any other.
#[test]
fn a_restored_editor_applies_when_corrected_and_is_refused_when_the_page_changed_meanwhile() {
    let mut app = a_run_with_an_unwritable_word("restore-stamp");
    assert!(app.apply_editing_page());
    app.tab_mut().editing_run.as_mut().expect("editing").buffer = "one LONELY line".to_string();
    assert!(!app.apply_editing_page(), "{}", said(&app));
    assert_eq!(runs_on(&app, 0)[0].text.trim(), "one LONELY line");

    let mut app = a_run_with_an_unwritable_word("restore-stale");
    assert!(app.apply_editing_page());
    app.rotate_view(90);
    app.tab_mut().editing_run.as_mut().expect("editing").buffer = "one LONELY line".to_string();
    assert!(!app.apply_editing_page());
    assert!(said(&app).contains(STALE_EDITOR_MESSAGE), "{}", said(&app));
    assert_eq!(runs_on(&app, 0)[0].text.trim(), "one lonely line", "a stale editor wrote");
}

/// **A line typed through the command path is not kept open when it is refused**: that is
/// a script, a recording or a test, nobody is at the box to correct it, and the next line
/// must run as the command it is — not be taken for more words to type into a box that is
/// still there.
#[test]
fn a_typed_line_the_engine_refuses_does_not_swallow_the_next_command() {
    let mut app = open_page("script-refused", &[(100.0, 700.0, "one lonely line")], "");
    let run = runs_on(&app, 0).into_iter().next().expect("a run");
    app.pick_text_run(0, centre(&run.rect)).expect("picked");
    app.submit("\u{3A9}");
    assert!(app.tab().editing_run.is_none(), "the refused line left the editor open for the next one");
    assert!(app.cmd.history().iter().any(|e| matches!(e.kind, Kind::Error)), "{}", said(&app));
    // So the next line is a command: `edittext` arms the tool rather than becoming a word.
    app.submit("edittext");
    assert!(app.tab().pending.is_some(), "the next line was not run as a command");
    assert_eq!(runs_on(&app, 0)[0].text.trim(), "one lonely line");
}

/// **Through the page, as a person meets it**: Apply is refused, the reason is on
/// screen beside the button with the box still open, and a click on other text does not
/// also pick it — the refusal is the answer to the gesture.
#[test]
fn a_refusal_is_shown_beside_apply_and_a_click_away_does_not_pick_what_is_under_it() {
    use egui_kittest::kittest::Queryable;

    let mut h = crate::ui_tests::harness("two-column.pdf");
    let runs = runs_on(h.state(), 0);
    let (target, other) = (runs[1].clone(), runs[runs.len() - 1].clone());
    h.state_mut().submit("edittext");
    h.run_steps(1);
    let screen = |h: &egui_kittest::Harness<'static, PagifyApp>, run: &TextRun| {
        h.state().tab().last_view.expect("the page was drawn").to_screen(centre(&run.rect))
    };
    let at = screen(&h, &target);
    crate::ui_tests::click(&mut h, at);
    h.run_steps(2);
    let opened = h.state().tab().editing_run.as_ref().expect("a click opened the editor").clone();
    let mut typed: Vec<String> = opened.buffer.split('\n').map(str::to_string).collect();
    typed[0] = "\u{3A9}".to_string();
    h.state_mut().tab_mut().editing_run.as_mut().expect("editing").buffer = typed.join("\n");
    h.run_steps(1);

    // Apply, refused: the box and the words stay, and the reason is on screen.
    h.get_by_label("Apply").click();
    h.run_steps(2);
    let kept = h.state().tab().editing_run.as_ref().expect("the refused edit's box was closed");
    assert_eq!(kept.buffer, typed.join("\n"), "the typing was thrown away");
    assert!(
        h.query_by_label_contains("Not applied").is_some(),
        "the reason is not on screen beside Apply: {}",
        said(h.state())
    );

    // Typed again with something else the engine refuses; the first click away is the
    // refusal's answer and picks nothing.
    typed[0] = "\u{3A8}".to_string();
    h.state_mut().tab_mut().editing_run.as_mut().expect("editing").buffer = typed.join("\n");
    let at = screen(&h, &other);
    crate::ui_tests::click(&mut h, at);
    h.run_steps(2);
    let still = h.state().tab().editing_run.as_ref().expect("a refused click-away closed the box");
    assert_eq!(still.object, opened.object, "the click also picked what was under it");
    assert_eq!(still.buffer, typed.join("\n"));
}

// -- what a click costs -----------------------------------------------------------------

/// What the page is made of, as the engine counts it — a measurement, run on
/// demand: `PAGIFY_HEAVY_PDFS="path:page;path:page" cargo test -p pagify_app
/// --release heavy_page_costs -- --ignored --nocapture`. For each page: the scale,
/// what reading it in one pass costs, and what a click costs, first and after.
#[test]
#[ignore = "a measurement: set PAGIFY_HEAVY_PDFS and run with --ignored --nocapture"]
fn heavy_page_costs() {
    let list = std::env::var("PAGIFY_HEAVY_PDFS").unwrap_or_else(|_| format!("{MARINA}:1;{MARINA}:2;{MARINA}:3"));
    for item in list.split(';').filter(|item| !item.is_empty()) {
        let (path, page) = item.rsplit_once(':').expect("path:page");
        let page: usize = page.parse::<usize>().expect("a page number") - 1;
        if !std::path::Path::new(path).is_file() {
            eprintln!("heavy: skipping {path}: not on this machine");
            continue;
        }
        let mut app = PagifyApp::new(Some(path));
        let session = app.tab().doc.as_ref().expect("open").session.clone();
        let started = std::time::Instant::now();
        let scale = session.page_scale(page).expect("scale");
        let scale_ms = started.elapsed().as_secs_f32() * 1000.0;
        let started = std::time::Instant::now();
        let read = if page_is_heavy(&scale) { None } else { session.page_text_snapshot(page).map(|s| s.runs.len()).ok() };
        let snapshot_ms = started.elapsed().as_secs_f32() * 1000.0;
        // The click: near the middle of the first text object that is wide enough.
        let target = session
            .text_run_rects(page)
            .ok()
            .and_then(|rects| rects.into_iter().find(|(_, r)| (r.right - r.left) > 4.0 && (r.bottom - r.top) > 4.0));
        let Some((_, rect)) = target else {
            eprintln!("heavy: {path} page {}: {scale:?}, page_scale {scale_ms:.1} ms, no text to click", page + 1);
            continue;
        };
        let mut clicks = Vec::new();
        for _ in 0..4 {
            app.tab_mut().editing_run = None;
            let started = std::time::Instant::now();
            let _ = app.pick_text_run(page, centre(&rect));
            clicks.push(started.elapsed().as_secs_f32() * 1000.0);
        }
        eprintln!(
            "heavy: {path} page {}: {scale:?} heavy={}; page_scale {scale_ms:.1} ms; snapshot {snapshot_ms:.1} ms ({read:?}); \
             clicks (ms) first {:.1}, then {:.1} {:.1} {:.1}",
            page + 1,
            page_is_heavy(&scale),
            clicks[0],
            clicks[1],
            clicks[2],
            clicks[3]
        );
    }
}
