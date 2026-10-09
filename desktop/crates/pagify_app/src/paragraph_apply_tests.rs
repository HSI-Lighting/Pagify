use super::tests_support::{assert_page_after, picture, runs_in_order, same_picture, same_run, squeeze};
use super::wrap_hyphen_tests::open_page;
use super::*;
use pdf_core::document::{Color, Rect, TextLineEdit, TextRun};

// -- on a page ----------------------------------------------------------

fn said(app: &PagifyApp) -> String {
    app.cmd.history().iter().map(|e| e.text.as_str()).collect::<Vec<_>>().join("\n")
}

/// Every text object of page 1 with the words and colour it has now.
fn runs_by_object(app: &PagifyApp) -> std::collections::HashMap<usize, (String, Color)> {
    app.tab()
        .doc
        .as_ref()
        .expect("open")
        .session
        .text_runs(0)
        .expect("runs")
        .into_iter()
        .map(|r| (r.object, (r.text, r.color)))
        .collect()
}

/// Arms Edit Text and clicks the middle of text object `object`.
fn pick_object(app: &mut PagifyApp, object: usize) {
    let run = app
        .tab()
        .doc
        .as_ref()
        .expect("open")
        .session
        .text_runs(0)
        .expect("runs")
        .into_iter()
        .find(|r| r.object == object)
        .expect("the object is on the page");
    let at = AppPoint {
        x: ((run.rect.left + run.rect.right) / 2.0) as f64,
        y: ((run.rect.top + run.rect.bottom) / 2.0) as f64,
    };
    app.submit("edittext");
    app.pick_text_run(0, at).expect("a run was here");
}

/// A page of `rows`, each `(left piece, right piece)`, one 12 pt below the
/// last; the right piece starts 3 pt after the left one ends — as a
/// justified line a producer cut at a word gap is. Objects are in drawing
/// order: row 0's left piece is object 0 and its right piece object 1,
/// row 1's are 2 and 3, and so on.
pub(super) fn two_piece_page(name: &str, rows: &[(&str, &str)]) -> PagifyApp {
    let ys: Vec<f32> = (0..rows.len()).map(|i| 700.0 - 12.0 * i as f32).collect();
    // The left pieces alone first, to measure where each one ends: the gap
    // is measured, not typed.
    let lefts: Vec<(f32, f32, &str)> =
        rows.iter().zip(&ys).map(|((left, _), y)| (100.0, *y, *left)).collect();
    let probe = open_page(&format!("{name}-probe"), &lefts, "");
    let measured = runs_by_object(&probe);
    let right_edges: Vec<f32> = (0..rows.len())
        .map(|i| {
            probe
                .tab()
                .doc
                .as_ref()
                .expect("open")
                .session
                .text_run_at(0, i)
                .expect("asked")
                .expect("the left piece")
                .rect
                .right
        })
        .collect();
    assert_eq!(measured.len(), rows.len(), "setup: one object per left piece");
    let mut pieces: Vec<(f32, f32, &str)> = Vec::new();
    for (i, (left, right)) in rows.iter().enumerate() {
        pieces.push((100.0, ys[i], *left));
        pieces.push((right_edges[i] + 3.0, ys[i], *right));
    }
    open_page(name, &pieces, "")
}

const ROWS: [(&str, &str); 5] = [
    ("Lorem ipsum dolor ", "sit amet consectetur"),
    ("adipiscing elit sed ", "do eiusmod tempor"),
    ("incididunt ut labore ", "et dolore magna"),
    ("aliqua ut enim ad ", "minim veniam quis"),
    ("nostrud exercitation ", "ullamco laboris nisi"),
];

/// A five-line, two-pieces-a-line paragraph opened for editing, with one
/// word of the third line retyped in the buffer — not applied yet. Returns
/// what the page reads now, the editor as picked, and the new third line.
fn five_lines_with_a_word_retyped(name: &str) -> (PagifyApp, Vec<TextRun>, EditingRun, String) {
    let mut app = two_piece_page(name, &ROWS);
    pick_object(&mut app, 4);
    let edit = app.tab().editing_run.as_ref().expect("picked").clone();
    assert_eq!(edit.lines.len(), 5, "setup: the five rows should be one paragraph: {:?}", edit.buffer);
    assert!(
        edit.lines.iter().all(|(objects, _)| objects.len() == 2),
        "setup: every line should be its two pieces: {:?}",
        edit.lines
    );
    assert_eq!(edit.frozen, vec![false; 5], "setup: nothing picked from text alone is frozen");

    let before = runs_in_order(&app);
    let retyped = edit.buffer.replace("labore", "LABORE");
    assert_ne!(retyped, edit.buffer, "setup: the word is in the buffer");
    // Whatever the two pieces of the third line join to — the extracted
    // text of a piece does not always keep the space the producer put at
    // its end — with the one word changed.
    let new_third = retyped.split('\n').nth(2).expect("a third line").to_string();
    assert!(new_third.contains("LABORE") && new_third.contains("dolore magna"), "{new_third:?}");
    app.tab_mut().editing_run.as_mut().expect("editing").buffer = retyped;
    (app, before, edit, new_third)
}

/// **Editing one word leaves every other line exactly as it was, and the
/// retyped line REPLACES its pieces.** A five-line paragraph, every line cut
/// in two pieces by a 3 pt gap — the way a justified column is — and one word
/// retyped in the third. The third line is now its first piece, holding the
/// new words; its second piece is *gone from the page* (not painted in the
/// page's colour, which left the old words in the file and let the painted
/// piece erase part of the new ones); the page has lost exactly that one
/// object; and the four lines nobody touched keep their own words, colour and
/// place.
#[test]
fn editing_one_word_leaves_every_other_two_piece_line_exactly_as_it_was() {
    let (mut app, before, edit, new_third) = five_lines_with_a_word_retyped("five-lines");
    app.apply_editing_page();
    let after = runs_in_order(&app);

    let (first, second) = (edit.lines[2].0[0], edit.lines[2].0[1]);
    assert_page_after("one word of line 3", &before, &after, &[(first, &new_third)], &[second]);
    assert!(said(&app).contains("paragraph changed"), "{}", said(&app));
}

const FOUR: [(f32, f32, &str); 4] = [
    (100.0, 700.0, "first line of the words"),
    (100.0, 688.0, "second line of the words"),
    (100.0, 676.0, "third line of the words"),
    (100.0, 664.0, "fourth line of the words"),
];

/// The four-line paragraph opened for editing, then rebuilt white-box into
/// the shape a block with drawn words in it has:
///
/// * line 0 — the first line, ordinary;
/// * line 1 — **nothing at all**: a whole line drawn as shapes, frozen,
///   with no text object;
/// * line 2 — the second line, **frozen**: its text objects share the line
///   with outlined words;
/// * lines 3 and 4 — the third and fourth, ordinary.
///
/// Objects 0 to 3 are the four lines in order.
fn a_paragraph_with_frozen_lines(name: &str) -> (PagifyApp, EditingRun) {
    let mut app = open_page(name, &FOUR, "");
    pick_object(&mut app, 0);
    let mut edit = app.tab().editing_run.as_ref().expect("picked").clone();
    assert_eq!(edit.lines.len(), 4, "setup: the four lines should be one paragraph: {:?}", edit.buffer);
    let lines = edit.lines.clone();
    let gap = Rect {
        left: lines[0].1.left,
        top: lines[0].1.bottom,
        right: lines[0].1.right,
        bottom: lines[0].1.bottom,
    };
    edit.lines = vec![lines[0].clone(), (Vec::new(), gap), lines[1].clone(), lines[2].clone(), lines[3].clone()];
    edit.frozen = vec![false, true, true, false, false];
    edit.original = format!(
        "{}\n\n{}\n{}\n{}",
        FOUR[0].2, FOUR[1].2, FOUR[2].2, FOUR[3].2
    );
    edit.buffer = edit.original.clone();
    (app, edit)
}

/// **A frozen line is never touched by an apply, and nothing panics** — a
/// line with no text object at all in the middle of the paragraph, and a
/// frozen line that has one, with words typed over both and real changes
/// on the lines around them.
#[test]
fn frozen_lines_are_never_touched_by_an_apply_and_nothing_panics() {
    let (mut app, mut edit) = a_paragraph_with_frozen_lines("frozen-middle");
    let (a, b, c, d) = (edit.lines[0].0[0], edit.lines[2].0[0], edit.lines[3].0[0], edit.lines[4].0[0]);
    edit.buffer = "first LINE of the words\ntyped over the empty frozen line\nsecond line TYPED OVER\n\
                   third LINE of the words\nfourth line of the words"
        .to_string();
    // Every line here is one piece, so nothing comes off the page and no
    // object number moves — the ids below stay good through the apply.

    // The commands first: one replace holding exactly the two real changes,
    // and nothing that reaches the frozen line's object (b) or the line
    // nobody touched (d) — neither retyped nor removed.
    let planned = app.paragraph_edit_commands(&edit, &edit.buffer).expect("it lines up");
    let [pdf_core::command::Command::ReplaceTextLines { edits, .. }] = planned.commands.as_slice() else {
        panic!("one replace and nothing else was expected: {:?}", planned.commands)
    };
    let written: Vec<(usize, String, Vec<usize>)> = edits
        .iter()
        .map(|edit| match edit {
            TextLineEdit::Retype { first, text, remove, .. } => (*first, text.clone(), remove.clone()),
            other => panic!("unexpected edit: {other:?}"),
        })
        .collect();
    assert_eq!(
        written,
        [
            (a, "first LINE of the words".to_string(), vec![]),
            (c, "third LINE of the words".to_string(), vec![])
        ]
    );
    assert_eq!(planned.frozen_changed, 2, "both frozen lines had words typed over them");
    assert!(!planned.position_ignored && planned.surplus.is_none());

    // Then for real, through the apply, with a real session log to read back.
    let dir = std::env::temp_dir().join(format!("pagify-test-frozen-log-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    app.session_log = pagify_shell::session_log::SessionLog::start_in(dir.clone());
    let log_path = app.session_log.path().expect("the temp dir is writable").to_path_buf();
    let before = runs_by_object(&app);
    app.apply_one_edit(edit.clone());
    let after = runs_by_object(&app);

    assert_eq!(after[&b], before[&b], "the frozen line's own object was rewritten or hidden");
    assert_eq!(after[&d], before[&d], "the line nobody touched was rewritten");
    assert_eq!(after[&a].0.trim(), "first LINE of the words");
    assert_eq!(after[&c].0.trim(), "third LINE of the words");
    assert_eq!(after.len(), before.len(), "objects were added or lost");

    assert!(
        said(&app).contains("paragraph changed; 2 lines drawn as shapes were left as they are."),
        "the apply did not say what it left alone: {}",
        said(&app)
    );
    let logged = std::fs::read_to_string(&log_path).expect("the log file exists");
    assert!(
        logged.contains("paragraph diff:") && logged.contains("frozen lines [1, 2]"),
        "the frozen lines were not logged: {logged}"
    );
    // Which lines were retyped, removed and left alone — by index and by
    // count, and never by their words.
    assert!(
        logged.contains(
            "retyped lines [0, 3], removed lines [], 0 pieces taken off the page, frozen lines [1, 2]"
        ) && logged.contains("unchanged lines 1"),
        "the diff was not logged as retyped / removed / frozen / unchanged: {logged}"
    );
    for word in ["LINE", "TYPED", "fourth", "second", "words"] {
        let in_the_diff = logged.lines().filter(|l| l.contains("paragraph diff:")).any(|l| l.contains(word));
        assert!(!in_the_diff, "the diff line carries page text {word:?}: {logged}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// **The first line of a block may itself be frozen with no text object**,
/// and the whole buffer is then not trimmed: its empty first line is a line,
/// and trimming it would pair every other line with the one before it.
/// Built through [`PagifyApp::build_editor_from_lines`], applied through the
/// real path.
#[test]
fn an_editor_built_with_an_empty_frozen_first_line_stays_aligned_through_an_apply() {
    let mut app = open_page("frozen-first", &FOUR[..3], "");
    let runs = app.tab().doc.as_ref().expect("open").session.text_runs(0).expect("runs");
    let face_names = app
        .tab()
        .doc
        .as_ref()
        .expect("open")
        .session
        .run_font_names(0)
        .unwrap_or_default();
    let above = Rect {
        left: runs[0].rect.left,
        top: runs[0].rect.top - 12.0,
        right: runs[0].rect.right,
        bottom: runs[0].rect.top,
    };
    let mut lines = vec![ParagraphLine { objects: Vec::new(), rect: above, frozen: true, follows_drawn: false }];
    lines.extend(
        runs.iter().map(|r| ParagraphLine { objects: vec![r.object], rect: r.rect, frozen: false, follows_drawn: false }),
    );
    let (edit, _look) = app
        .build_editor_from_lines(0, lines, &runs, &face_names, &std::collections::HashMap::new(), &[], None)
        .expect("a block with text in it");

    assert_eq!(edit.object, runs[0].object, "the editor sits on the first object of the first line that has one");
    // The frozen line with no text object of its own stands for the words
    // drawn as shapes: its placeholder, one entry like every other line.
    assert_eq!(
        edit.buffer,
        format!(
            "{}\nfirst line of the words\nsecond line of the words\nthird line of the words",
            block_input::OUTLINED_PLACEHOLDER
        )
    );
    assert_eq!(edit.frozen, [true, false, false, false]);
    assert!(edit.lines[0].0.is_empty());
    assert_eq!(edit.original.split('\n').count(), edit.lines.len());

    // Retype the last line only, through the real apply.
    let before = runs_by_object(&app);
    let retyped = edit.buffer.replace("third", "THIRD");
    app.tab_mut().editing_run = Some(edit);
    app.tab_mut().editing_run.as_mut().expect("editing").buffer = retyped;
    app.apply_editing_page();
    let after = runs_by_object(&app);
    assert_eq!(after[&0], before[&0], "the first text line was rewritten: the lines shifted");
    assert_eq!(after[&1], before[&1], "the second text line was rewritten: the lines shifted");
    assert_eq!(after[&2].0.trim(), "THIRD line of the words");
    assert_eq!(after.len(), before.len());
}

/// A block with nothing in it to edit — every line drawn as shapes — has no
/// object to put an editor on, and says so.
#[test]
fn a_block_with_no_text_object_at_all_cannot_be_opened() {
    let mut app = open_page("all-frozen", &FOUR[..1], "");
    let runs = app.tab().doc.as_ref().expect("open").session.text_runs(0).expect("runs");
    let lines = vec![ParagraphLine { objects: Vec::new(), rect: runs[0].rect, frozen: true, follows_drawn: false }];
    let refused = app
        .build_editor_from_lines(
            0,
            lines,
            &runs,
            &std::collections::HashMap::new(),
            &std::collections::HashMap::new(),
            &[],
            None,
        )
        .err()
        .expect("there is nothing to edit");
    assert!(refused.contains("drawn as shapes"), "{refused}");
}

/// **A paragraph that no longer lines up with its own text is refused in a
/// release build too.** The check was a `debug_assert`; in release a line
/// of text too few made the diff index past the end, or pair every later
/// line with the wrong object.
#[test]
fn a_paragraph_that_no_longer_lines_up_is_refused_not_guessed_at() {
    let mut app = open_page("mismatch", &FOUR, "");
    pick_object(&mut app, 0);
    let good = app.tab().editing_run.as_ref().expect("picked").clone();
    assert_eq!(good.lines.len(), 4, "setup: the four lines should be one paragraph");

    let mut too_few_lines_of_text = good.clone();
    too_few_lines_of_text.original = good.original.replacen('\n', " ", 1);
    let mut too_few_flags = good.clone();
    too_few_flags.frozen.pop();
    let mut no_lines = good.clone();
    no_lines.lines.clear();
    no_lines.frozen.clear();
    for (what, broken) in [
        ("one line of text too few", &too_few_lines_of_text),
        ("one frozen flag too few", &too_few_flags),
        ("no lines at all", &no_lines),
    ] {
        let refused = app
            .paragraph_edit_commands(broken, "a\nb\nc\nd")
            .err()
            .unwrap_or_else(|| panic!("{what} was not refused"));
        assert!(refused.contains("no longer lines up"), "{what}: {refused}");
    }

    // Through the apply: the page is untouched and the refusal is said.
    let before = runs_by_object(&app);
    let mut broken = too_few_lines_of_text;
    broken.buffer = good.buffer.replace("first", "FIRST");
    app.apply_one_edit(broken);
    assert_eq!(runs_by_object(&app), before, "a refused apply changed the page");
    assert!(said(&app).contains("no longer lines up"), "{}", said(&app));
}

/// **A block of one text object and a line drawn as shapes is a paragraph.**
/// One object, two lines: counting objects alone sent it down the
/// single-run path, which writes the buffer's later lines as *new* lines
/// below the first — extra text the page never had.
#[test]
fn a_block_of_one_text_object_and_a_frozen_line_is_applied_as_a_paragraph() {
    let mut app = open_page("one-and-frozen", &[(100.0, 700.0, "one lonely line")], "");
    pick_object(&mut app, 0);
    let mut edit = app.tab().editing_run.as_ref().expect("picked").clone();
    assert_eq!(edit.lines.len(), 1, "setup: a single run");
    let rect = edit.lines[0].1;
    edit.lines.push((Vec::new(), Rect { top: rect.bottom, bottom: rect.bottom + 12.0, ..rect }));
    edit.frozen = vec![false, true];
    edit.original = format!("{}\n", edit.original);
    edit.buffer = "one LONELY line\ntyped over the drawn line".to_string();

    let before = runs_by_object(&app);
    app.apply_one_edit(edit);
    let after = runs_by_object(&app);
    assert_eq!(after.len(), before.len(), "a new line was written below: {after:?}");
    assert_eq!(after[&0].0.trim(), "one LONELY line");
    assert!(
        said(&app).contains("1 line drawn as shapes was left as it is"),
        "the apply did not say what it left alone: {}",
        said(&app)
    );
}

/// **A lone frozen line with one text piece is never rewritten either.**
/// One line, one object: nothing but the frozen flag sends it down the
/// paragraph path, which leaves it as it is.
#[test]
fn a_lone_frozen_line_is_never_rewritten() {
    let mut app = open_page("lone-frozen", &[(100.0, 700.0, "one lonely line")], "");
    pick_object(&mut app, 0);
    let mut edit = app.tab().editing_run.as_ref().expect("picked").clone();
    edit.frozen = vec![true];
    edit.buffer = "one LONELY line".to_string();

    let before = runs_by_object(&app);
    app.apply_one_edit(edit);
    assert_eq!(runs_by_object(&app), before, "a frozen line was rewritten");
    assert!(
        said(&app).contains("unchanged; 1 line drawn as shapes was left as it is."),
        "the apply did not say what it left alone: {}",
        said(&app)
    );
}

/// Every way of picking gives one frozen flag per line, none of them set.
#[test]
fn every_way_of_picking_gives_one_frozen_flag_per_line_and_none_set() {
    let mut app = open_page("flags-single", &[(100.0, 700.0, "one lonely line")], "");
    pick_object(&mut app, 0);
    let single = app.tab().editing_run.as_ref().expect("picked").clone();
    assert_eq!((single.lines.len(), single.frozen), (1, vec![false]));

    let mut app = open_page("flags-paragraph", &FOUR, "");
    pick_object(&mut app, 0);
    let paragraph = app.tab().editing_run.as_ref().expect("picked").clone();
    assert_eq!(paragraph.lines.len(), 4);
    assert_eq!(paragraph.frozen, vec![false; 4]);
}

/// Every way of picking stamps the document's command-history generation.
#[test]
fn every_way_of_picking_stamps_the_documents_generation() {
    let mut app = open_page("stamp-single", &[(100.0, 700.0, "one lonely line")], "");
    pick_object(&mut app, 0);
    let single = app.tab().editing_run.as_ref().expect("picked").clone();
    assert_eq!(single.doc_generation, app.doc_generation());

    // After the history has moved, a fresh pick carries the new number.
    app.tab_mut().editing_run.as_mut().expect("editing").buffer = "one LONELY line".to_string();
    app.apply_editing_page();
    assert_ne!(app.doc_generation(), single.doc_generation, "setup: an apply moves the generation");
    let mut app2 = open_page("stamp-paragraph", &FOUR, "");
    pick_object(&mut app2, 0);
    let paragraph = app2.tab().editing_run.as_ref().expect("picked").clone();
    assert_eq!(paragraph.lines.len(), 4, "setup: a paragraph, built by `build_editor_from_lines`");
    assert_eq!(paragraph.doc_generation, app2.doc_generation());
    pick_object(&mut app, 0);
    assert_eq!(
        app.tab().editing_run.as_ref().expect("picked again").doc_generation,
        app.doc_generation(),
        "a fresh pick must carry the generation as it is now"
    );
}

/// **An edit picked before an undo is refused, not written onto objects
/// that may have been renumbered.** The editor holds page-object numbers; an
/// edit, an undo or a redo between the pick and the apply can renumber them.
#[test]
fn an_edit_picked_before_an_undo_is_refused_rather_than_written_onto_other_objects() {
    let mut app = open_page("stale-undo", &FOUR, "");
    // Some history to undo: retype the first line. This also shows a fresh
    // pick still applies.
    pick_object(&mut app, 0);
    let retyped = app.tab().editing_run.as_ref().expect("picked").buffer.replace("first", "FIRST");
    app.tab_mut().editing_run.as_mut().expect("editing").buffer = retyped;
    app.apply_editing_page();
    assert!(runs_by_object(&app)[&0].0.contains("FIRST"), "setup: the first edit applied");

    // Pick again, then move the document under the open editor.
    pick_object(&mut app, 0);
    let picked_at = app.tab().editing_run.as_ref().expect("picked").doc_generation;
    let (undone, _) = app.tab().doc.as_ref().expect("open").session.undo().expect("undo");
    assert!(undone, "setup: there was something to undo");
    assert_ne!(app.doc_generation(), picked_at, "setup: the undo moved the generation");
    let after_undo = runs_by_object(&app);

    let typed = app.tab().editing_run.as_ref().expect("still open").buffer.replace("second", "SECOND");
    app.tab_mut().editing_run.as_mut().expect("editing").buffer = typed;
    app.apply_editing_page();
    assert_eq!(runs_by_object(&app), after_undo, "an edit picked before the undo was written");
    assert!(said(&app).contains(STALE_EDITOR_MESSAGE), "{}", said(&app));
    assert!(app.tab().editing_run.is_none(), "the editor should be closed");
}

/// A stale editor with nothing typed in it just closes, as any other does.
#[test]
fn a_stale_edit_with_nothing_typed_just_closes() {
    let mut app = open_page("stale-nothing", &FOUR, "");
    pick_object(&mut app, 0);
    let retyped = app.tab().editing_run.as_ref().expect("picked").buffer.replace("first", "FIRST");
    app.tab_mut().editing_run.as_mut().expect("editing").buffer = retyped;
    app.apply_editing_page();

    pick_object(&mut app, 0);
    let (undone, _) = app.tab().doc.as_ref().expect("open").session.undo().expect("undo");
    assert!(undone, "setup: there was something to undo");
    let after_undo = runs_by_object(&app);
    app.apply_editing_page();
    assert_eq!(runs_by_object(&app), after_undo);
    assert!(app.tab().editing_run.is_none());
    assert!(!said(&app).contains("not applied"), "a no-op was refused as if it were a write: {}", said(&app));
}

/// **On the real file: retyping a line made of several pieces replaces
/// those pieces and writes nothing else.** The CAMINO paragraph's first line
/// is three pieces ("The COB in", "HSI ", "products sup"); retyped, it is its
/// first piece holding the new words, the other two come off the page, and
/// every other object on the page is exactly as it was. (Every apply used to
/// collapse all of the paragraph's lines, then to paint the other pieces in the
/// page's colour.) Skipped, with a note, where the file is not on this machine.
#[test]
fn retyping_a_multi_piece_line_of_the_real_paragraph_replaces_its_pieces_and_writes_nothing_else() {
    let path = r"C:\Users\hsili\Downloads\CAMINO elitee-plus 3.0.pdf";
    let mut app = PagifyApp::new(Some(path));
    if app.tab().doc.is_none() {
        eprintln!("skipping: CAMINO not present on this machine");
        return;
    }
    let seed = app
        .tab()
        .doc
        .as_ref()
        .expect("open")
        .session
        .text_runs(0)
        .expect("runs")
        .into_iter()
        .find(|r| r.text.contains("manufacturers"))
        .expect("the COB paragraph's own run was not found — has the file changed?");
    app.submit("edittext");
    app.pick_text_run(
        0,
        AppPoint {
            x: ((seed.rect.left + seed.rect.right) / 2.0) as f64,
            y: ((seed.rect.top + seed.rect.bottom) / 2.0) as f64,
        },
    )
    .expect("picked");
    let edit = app.tab().editing_run.as_ref().expect("picked").clone();
    let lines_text: Vec<String> = edit.buffer.split('\n').map(str::to_string).collect();
    assert!(edit.lines.len() >= 4, "setup: need a first, a middle and a last line");
    assert!(
        edit.lines.iter().any(|(objects, _)| objects.len() > 1),
        "setup: the paragraph has a line made of several pieces, or nothing here could show a collapse"
    );

    // A middle line made of plain words only — no control code — retyped by
    // repeating a word it already uses, so no new letter is needed. Made of
    // several pieces, so that taking the others off the page is something the
    // apply has to do (and this test can see it done).
    //
    // **A line may end in the hyphen where the page breaks a word**: the thin
    // hyphen glyphs are members of their lines now (they used to be left out of
    // the buffer), and every multi-piece line of this paragraph ends in one. The
    // retyped line drops it — a hyphen written comes back from PDFium as a
    // control code, which would make the words differ for a reason that is not
    // what is under test — and keeps the plain words before it.
    let plain = |text: &str| {
        text.trim_end().trim_end_matches('-').chars().all(|c| c.is_alphabetic() || c == ' ' || c == '.' || c == ',')
    };
    let target = (0..lines_text.len() - 1)
        .find(|&i| edit.lines[i].0.len() > 1 && plain(&lines_text[i]))
        .expect("a line of plain words made of several pieces");
    let word = lines_text[target]
        .split_whitespace()
        .find(|w| w.chars().count() > 2 && w.chars().all(char::is_alphabetic))
        .expect("a plain word");
    let mut retyped = lines_text.clone();
    retyped[target] = format!("{} {word}", lines_text[target].trim_end().trim_end_matches('-'));

    let before = runs_in_order(&app);
    app.tab_mut().editing_run.as_mut().expect("editing").buffer = retyped.join("\n");
    app.apply_editing_page();
    let after = runs_in_order(&app);

    // The retyped line is its first piece with the new words; its other
    // pieces are gone from the page; every other object — every piece of
    // every other line — is exactly as it was.
    let (first, rest) = edit.lines[target].0.split_first().expect("the line has objects");
    assert_page_after(
        "one middle line of the CAMINO paragraph",
        &before,
        &after,
        &[(*first, &retyped[target])],
        rest,
    );
}

// -- replacing pieces: what the page looks like, and undo ----------------

/// **ONE undo puts the page back EXACTLY, and redo does it again.** Not
/// "the same words": every run's words, colour, box, origin and size, and the
/// picture itself pixel for pixel. Re-typing the old words (what undo used to
/// do) loses the kerning of a `TJ` and the producer's own spacing; the
/// replace undoes by a snapshot of the page instead.
#[test]
fn undo_of_an_apply_puts_the_page_back_exactly_and_redo_does_it_again() {
    let (mut app, before, _edit, _new_third) = five_lines_with_a_word_retyped("undo-exact");
    let picture_before = picture(&app);
    app.apply_editing_page();
    let applied = runs_in_order(&app);
    let picture_applied = picture(&app);
    assert!(
        !same_picture(&picture_before, &picture_applied),
        "setup: the edit changed nothing that is drawn"
    );
    assert_eq!(applied.len(), before.len() - 1, "setup: one piece came off the page");

    let (undone, _) = app.tab().doc.as_ref().expect("open").session.undo().expect("undo");
    assert!(undone, "the apply did not undo in one step");
    let restored = runs_in_order(&app);
    assert_eq!(restored.len(), before.len(), "undo did not bring the removed piece back");
    for (was, now) in before.iter().zip(&restored) {
        assert_eq!(was.object, now.object, "undo renumbered the page");
        assert!(same_run(was, now), "undo did not restore a run exactly: {was:?} now {now:?}");
    }
    assert!(same_picture(&picture(&app), &picture_before), "the page is not pixel-identical after undo");

    let (redone, _) = app.tab().doc.as_ref().expect("open").session.redo().expect("redo");
    assert!(redone, "nothing to redo");
    let again = runs_in_order(&app);
    assert_eq!(again.len(), applied.len());
    for (was, now) in applied.iter().zip(&again) {
        assert!(same_run(was, now), "redo did not reapply exactly: {was:?} now {now:?}");
    }
    assert!(same_picture(&picture(&app), &picture_applied), "the page is not pixel-identical after redo");
}

/// **A later pick of the edited paragraph shows exactly the edited lines.**
/// With the old pieces painted over instead of removed, the detector found
/// them again and the buffer read each retyped line's old words a second
/// time.
#[test]
fn a_later_pick_of_the_edited_paragraph_shows_exactly_the_edited_lines() {
    let (mut app, _before, edit, _new_third) = five_lines_with_a_word_retyped("re-pick");
    let expected: Vec<String> = edit.buffer.replace("labore", "LABORE").split('\n').map(squeeze).collect();
    app.apply_editing_page();

    app.tab_mut().editing_run = None;
    pick_object(&mut app, 0);
    let again = app.tab().editing_run.as_ref().expect("picked again").clone();
    let lines: Vec<String> = again.buffer.split('\n').map(squeeze).collect();
    assert_eq!(lines, expected, "a later pick does not read the page's own lines");
    assert_eq!(again.lines.len(), 5);
    assert_eq!(
        again.lines.iter().map(|(objects, _)| objects.len()).collect::<Vec<_>>(),
        [2, 2, 1, 2, 2],
        "the retyped line is one piece now and the others are as they were"
    );
    assert_eq!(again.buffer.matches("dolore").count(), 1, "a word of the retyped line is there twice");
}

/// **A first piece that becomes LONGER than the old one is not painted over
/// by the line's other piece.** The old piece sat after the first by its own
/// `Td`, painted in the page's colour *after* the new words; under a longer
/// first piece it erased part of them (23% of one datasheet line's ink).
/// Compared against a page on which the same words are simply written in one
/// piece: the two pictures must be identical.
#[test]
fn a_longer_first_piece_is_not_painted_over_by_the_pieces_that_were_after_it() {
    const SECOND: (&str, &str) = ("sit amet ", "consectetur elit");
    let long = "Lorem ipsum dolor sit amet consectetur adipiscing elit sed do";
    let mut app = two_piece_page("overpaint", &[("Lorem ", "ipsum dolor"), SECOND]);
    pick_object(&mut app, 0);
    let edit = app.tab().editing_run.as_ref().expect("picked").clone();
    assert_eq!(edit.lines.len(), 2, "setup: two lines: {:?}", edit.buffer);
    let first_line_old = edit.buffer.split('\n').next().expect("a first line").to_string();
    assert!(squeeze(&first_line_old).len() < long.len(), "setup: the new line is longer");
    let typed = edit.buffer.replacen(&first_line_old, long, 1);
    app.tab_mut().editing_run.as_mut().expect("editing").buffer = typed;
    app.apply_editing_page();

    // The same words, one piece, same place; the second row as it was.
    let reference = two_piece_page("overpaint-reference", &[(long, ""), SECOND]);
    assert!(
        same_picture(&picture(&app), &picture(&reference)),
        "the retyped line is not drawn as the same words written in one piece would be — \
         a piece left on the page paints over part of it, or the words are not where they were"
    );
}

/// A line the person deleted comes off the page whole: every one of its
/// pieces, and nothing else.
#[test]
fn deleting_a_line_removes_every_one_of_its_pieces() {
    let mut app = two_piece_page("delete-line", &ROWS);
    pick_object(&mut app, 4);
    let edit = app.tab().editing_run.as_ref().expect("picked").clone();
    assert_eq!(edit.lines.len(), 5, "setup: five lines");
    let before = runs_in_order(&app);
    let mut lines: Vec<&str> = edit.buffer.split('\n').collect();
    lines.remove(2);
    app.tab_mut().editing_run.as_mut().expect("editing").buffer = lines.join("\n");
    app.apply_editing_page();
    let after = runs_in_order(&app);
    assert_page_after("a deleted line", &before, &after, &[], &edit.lines[2].0);

    // And it comes back, exactly, with one undo.
    let (undone, _) = app.tab().doc.as_ref().expect("open").session.undo().expect("undo");
    assert!(undone);
    let restored = runs_in_order(&app);
    assert_eq!(restored.len(), before.len());
    assert!(before.iter().zip(&restored).all(|(a, b)| same_run(a, b)), "undo did not restore the deleted line");
}

/// **A new colour for the paragraph reaches every piece that stays**, and
/// it undoes together with the replace in ONE step — the colour is written
/// first, while the object numbers are still good, and undone last.
#[test]
fn a_new_colour_reaches_every_piece_that_stays_and_undoes_with_the_replace_in_one_step() {
    let red = Color { r: 200, g: 0, b: 0, a: 255 };
    let (mut app, before, edit, new_third) = five_lines_with_a_word_retyped("recolour");
    assert!(before.iter().all(|r| r.color != red), "setup: nothing is red yet");
    app.tab_mut().editing_run.as_mut().expect("editing").style.color = Some(red);
    let picture_before = picture(&app);
    app.apply_editing_page();
    let after = runs_in_order(&app);

    // The page lost the one piece; every run that is left is red, and the
    // retyped one holds the new words.
    let (first, second) = (edit.lines[2].0[0], edit.lines[2].0[1]);
    let recoloured: Vec<TextRun> =
        before.iter().map(|r| TextRun { color: red, ..r.clone() }).collect();
    assert_page_after("a new colour and a retyped line", &recoloured, &after, &[(first, &new_third)], &[second]);
    assert!(after.iter().all(|r| r.color == red), "a piece that stays was not recoloured: {after:?}");

    let (undone, _) = app.tab().doc.as_ref().expect("open").session.undo().expect("undo");
    assert!(undone, "the colour and the replace did not undo as one step");
    let restored = runs_in_order(&app);
    assert_eq!(restored.len(), before.len());
    assert!(before.iter().zip(&restored).all(|(a, b)| same_run(a, b)), "undo left a colour or a piece behind");
    assert!(same_picture(&picture(&app), &picture_before), "not pixel-identical after one undo");
}

/// A colour with no word changed is just the colour: every piece of every
/// line, no piece removed.
#[test]
fn a_new_colour_alone_recolours_the_whole_paragraph_and_removes_nothing() {
    let red = Color { r: 200, g: 0, b: 0, a: 255 };
    let mut app = two_piece_page("recolour-only", &ROWS);
    pick_object(&mut app, 4);
    let before = runs_in_order(&app);
    app.tab_mut().editing_run.as_mut().expect("editing").style.color = Some(red);
    app.apply_editing_page();
    let after = runs_in_order(&app);
    let recoloured: Vec<TextRun> = before.iter().map(|r| TextRun { color: red, ..r.clone() }).collect();
    assert_page_after("a colour and nothing else", &recoloured, &after, &[], &[]);
    assert!(said(&app).contains("paragraph changed"), "{}", said(&app));
}

/// **A position asked for is said, never silently dropped**: the panel
/// offers one for a paragraph, a paragraph has no single place to move to,
/// and the retyped line stays where it was.
#[test]
fn a_position_asked_for_is_said_not_silently_dropped() {
    let (mut app, before, edit, new_third) = five_lines_with_a_word_retyped("position");
    app.tab_mut().editing_run.as_mut().expect("editing").style.at = Some((300.0, 300.0));
    app.apply_editing_page();
    let (first, second) = (edit.lines[2].0[0], edit.lines[2].0[1]);
    assert_page_after("a position asked for", &before, &runs_in_order(&app), &[(first, &new_third)], &[second]);
    assert!(
        said(&app).contains("paragraph changed; the position was not changed"),
        "the apply did not say the position was not applied: {}",
        said(&app)
    );
}

/// **An engine refusal leaves the page exactly as it was, and is said.** The
/// replace is atomic: here a line is made to name an object that is not text
/// at all, which the engine refuses, and not one thing changes — **not even
/// the new line typed below the paragraph**, which is written only once the
/// replace has gone through.
#[test]
fn an_engine_refusal_leaves_the_page_as_it_was_and_is_said() {
    let mut app = open_page("refusal", &FOUR, "0 0 50 50 re f\n");
    pick_object(&mut app, 0);
    let mut edit = app.tab().editing_run.as_ref().expect("picked").clone();
    assert_eq!(edit.lines.len(), 4, "setup: four lines");
    // The fifth object is the rectangle.
    edit.lines[1].0 = vec![4];
    edit.buffer = format!("{}\nan extra line", edit.buffer.replacen("second", "SECOND", 1));
    let (runs_before, picture_before) = (runs_in_order(&app), picture(&app));
    let typed = edit.buffer.clone();
    app.tab_mut().editing_run = Some(edit);
    assert!(app.apply_editing_page(), "the refusal was not reported as one");
    assert_eq!(runs_in_order(&app).len(), runs_before.len());
    assert!(runs_in_order(&app).iter().zip(&runs_before).all(|(a, b)| same_run(a, b)), "the page changed");
    assert!(same_picture(&picture(&app), &picture_before), "the page's picture changed");
    let last = app.cmd.history().last().expect("something was said").clone();
    assert!(matches!(last.kind, Kind::Error), "the refusal was not said as an error: {last:?}");
    assert!(
        last.text.ends_with(" — nothing was changed."),
        "the refusal did not say that nothing was changed: {last:?}"
    );
    // **Changed with the reason**: the editor used to be closed here, with the
    // person's typing thrown away with the refusal. It stays now, with the words
    // in it and the refusal remembered (shown beside Apply).
    let kept = app.tab().editing_run.as_ref().expect("a refused edit keeps its editor");
    assert_eq!(kept.buffer, typed, "the typing was not kept");
    assert_eq!(kept.refusal.as_ref().map(|r| r.reason.as_str()), Some(last.text.as_str()));
}

/// **A refusal after the colour has been written puts the colour back too.**
/// The colour and the replace are one batch, and a batch is all or nothing:
/// the replace is refused (a line made to name an object that is not text), and
/// the colour that went first must not be left on the page.
#[test]
fn a_refusal_after_a_new_colour_leaves_the_page_exactly_as_it_was() {
    let red = Color { r: 200, g: 0, b: 0, a: 255 };
    let mut app = open_page("refusal-colour", &FOUR, "0 0 50 50 re f\n");
    pick_object(&mut app, 0);
    let mut edit = app.tab().editing_run.as_ref().expect("picked").clone();
    edit.lines[1].0 = vec![1, 4]; // the line's own piece, and the rectangle as a second "piece": not text
    edit.buffer = edit.buffer.replacen("second", "SECOND", 1);
    edit.style.color = Some(red);
    let (runs_before, picture_before) = (runs_in_order(&app), picture(&app));
    app.tab_mut().editing_run = Some(edit);
    app.apply_editing_page();
    let last = app.cmd.history().last().expect("something was said").clone();
    assert!(matches!(last.kind, Kind::Error), "the refusal was not said as an error: {last:?}");
    let runs_after = runs_in_order(&app);
    assert_eq!(runs_after.len(), runs_before.len());
    assert!(
        runs_before.iter().zip(&runs_after).all(|(a, b)| same_run(a, b)),
        "the colour was left on the page after the refusal: {runs_after:?}"
    );
    assert!(same_picture(&picture(&app), &picture_before), "the page's picture changed");
}

/// **New lines typed past the paragraph's end are written below it, after
/// the replace** — by position, since the replace renumbers the page — and
/// the whole thing still undoes exactly (the new line first, then the replace).
#[test]
fn new_lines_typed_past_the_end_are_written_below_the_paragraph_after_the_replace() {
    let (mut app, before, edit, new_third) = five_lines_with_a_word_retyped("surplus");
    let typed = format!("{}\na brand new last line", app.tab().editing_run.as_ref().unwrap().buffer);
    app.tab_mut().editing_run.as_mut().expect("editing").buffer = typed;
    app.apply_editing_page();
    let after = runs_in_order(&app);
    let (first, second) = (edit.lines[2].0[0], edit.lines[2].0[1]);

    // The page as it was, minus the one piece, is the first `before.len() - 1`
    // objects; what follows is the new line, which a text mark writes as one
    // object per letter, left to right on one baseline.
    let kept = before.len() - 1;
    assert!(after.len() > kept, "the new line is not on the page: {after:?}");
    assert_page_after("the rest of the page", &before, &after[..kept], &[(first, &new_third)], &[second]);
    let mut letters: Vec<&TextRun> = after[kept..].iter().collect();
    letters.sort_by(|a, b| a.rect.left.total_cmp(&b.rect.left));
    let written: String = letters.iter().map(|r| r.text.trim()).collect();
    assert_eq!(written, "abrandnewlastline", "the new line's letters: {letters:?}");
    let last_row_bottom = before.iter().map(|r| r.rect.bottom).fold(f32::MIN, f32::max);
    assert!(
        letters.iter().all(|r| r.rect.top >= last_row_bottom - 1.0),
        "the new line is not below the paragraph (its last row ends at {last_row_bottom}): {letters:?}"
    );

    // Undo takes the new line off first, then restores the page.
    for step in ["the new line", "the replace"] {
        let (undone, _) = app.tab().doc.as_ref().expect("open").session.undo().expect("undo");
        assert!(undone, "{step} did not undo");
    }
    let restored = runs_in_order(&app);
    assert_eq!(restored.len(), before.len());
    assert!(before.iter().zip(&restored).all(|(a, b)| same_run(a, b)), "two undos did not restore the page");
}

/// **A join names objects by number, and a replace renumbers them**: pieces
/// that came off move every later object down, so a join left alone would
/// name other objects and open the wrong paragraph. Forgotten when pieces
/// came off the page; kept when nothing did.
#[test]
fn a_join_is_forgotten_when_pieces_came_off_the_page_and_kept_when_none_did() {
    let (mut app, ..) = five_lines_with_a_word_retyped("join-forgotten");
    assert!(app.declare_group(0, vec![0, 2, 4]), "setup: the join is declared");
    app.apply_editing_page();
    assert!(app.tab().joined_groups.is_empty(), "a join survived the renumbering: {:?}", app.tab().joined_groups);

    // One piece a line, so nothing comes off and nothing is renumbered.
    let mut app = open_page("join-kept", &FOUR, "");
    pick_object(&mut app, 0);
    assert!(app.declare_group(0, vec![0, 1, 2, 3]), "setup: the join is declared");
    let retyped = app.tab().editing_run.as_ref().expect("picked").buffer.replace("second", "SECOND");
    app.tab_mut().editing_run.as_mut().expect("editing").buffer = retyped;
    app.apply_editing_page();
    let kept: Vec<(usize, Vec<usize>)> =
        app.tab().joined_groups.iter().map(|group| (group.page, group.objects.clone())).collect();
    assert_eq!(kept, [(0, vec![0, 1, 2, 3])], "a join was forgotten for no reason");
}

/// **A font picked for a paragraph is written on every line, in one step.** The
/// engine refuses a batch in which two lines each need a new font embedded; a
/// font picked for the whole block is one font for all of them.
#[test]
fn a_font_picked_for_a_paragraph_is_written_on_every_line_in_one_step() {
    let mut app = open_page("font-all-lines", &FOUR, "");
    let picked = app
        .writing_faces()
        .into_iter()
        .find(|n| n.to_ascii_lowercase().contains("montserrat"))
        .expect("Montserrat is bundled");
    pick_object(&mut app, 0);
    let before = runs_in_order(&app);
    let picture_before = picture(&app);
    app.tab_mut().editing_run.as_mut().expect("editing").style.face = Some(picked.clone());
    app.apply_editing_page();
    assert!(said(&app).contains("paragraph changed"), "the font pick was refused: {}", said(&app));
    let after = runs_in_order(&app);
    assert_eq!(after.len(), before.len());
    assert!(
        before.iter().zip(&after).all(|(a, b)| squeeze(&a.text) == squeeze(&b.text)),
        "a line's words changed with the font: {after:?}"
    );
    let (undone, _) = app.tab().doc.as_ref().expect("open").session.undo().expect("undo");
    assert!(undone, "the font pick did not undo");
    assert!(same_picture(&picture(&app), &picture_before), "one undo did not restore the page");
}

/// **A new colour for a real paragraph changes the colour and nothing else.**
/// The datasheet's 13-line paragraph and CAMINO's COB paragraph have lines of
/// three to eight pieces, several ending in a hyphen glyph that the extracted
/// text leaves out, and the datasheet's has two lines with a word the page
/// draws as shapes (left as they are, so not recoloured). A colour edit that
/// sent a piece's words back without its hyphen would drop it; every other
/// run — words, box, origin, size — must read exactly as it did, and one
/// undo restores the picture. Skipped where a file is not on this machine.
#[test]
fn a_new_colour_for_a_real_paragraph_changes_the_colour_and_nothing_else() {
    let _turn = tests_support::one_at_a_time();
    let red = Color { r: 200, g: 0, b: 0, a: 255 };
    let mut any = false;
    for (path, seed) in [
        (r"C:\Users\hsili\Desktop\Datasheets - Editors market - Marina mall.pdf", None),
        (r"C:\Users\hsili\Downloads\CAMINO elitee-plus 3.0.pdf", Some("manufacturers")),
    ] {
        if !std::path::Path::new(path).is_file() {
            eprintln!("skipping: {path} is not on this machine");
            continue;
        }
        any = true;
        let mut app = PagifyApp::new(Some(path));
        let before = runs_in_order(&app);
        let seed = match seed {
            Some(text) => before.iter().find(|r| r.text.contains(text)),
            None => before.iter().find(|r| r.object == 986),
        }
        .expect("the paragraph's own run was not found — has the file changed?");
        app.submit("edittext");
        app.pick_text_run(
            0,
            AppPoint {
                x: ((seed.rect.left + seed.rect.right) / 2.0) as f64,
                y: ((seed.rect.top + seed.rect.bottom) / 2.0) as f64,
            },
        )
        .expect("picked");
        let edit = app.tab().editing_run.as_ref().expect("an editor opened").clone();
        assert!(edit.lines.len() >= 5, "setup: {path}: a real paragraph");
        let picture_before = picture(&app);
        app.tab_mut().editing_run.as_mut().expect("editing").style.color = Some(red);
        app.apply_editing_page();
        assert!(said(&app).contains("paragraph changed"), "{path}: the colour was refused: {}", said(&app));

        // Every piece of every line that is not drawn as shapes is red, and
        // nothing else changed; no object came off the page.
        let in_a_frozen_line = |object: usize| {
            edit.lines.iter().zip(&edit.frozen).any(|((objects, _), frozen)| *frozen && objects.contains(&object))
        };
        let in_a_line = |object: usize| edit.lines.iter().any(|(objects, _)| objects.contains(&object));
        let expected: Vec<TextRun> = before
            .iter()
            .map(|r| {
                if in_a_line(r.object) && !in_a_frozen_line(r.object) {
                    TextRun { color: red, ..r.clone() }
                } else {
                    r.clone()
                }
            })
            .collect();
        let after = runs_in_order(&app);
        tests_support::assert_page_after(&format!("{path}: a new colour"), &expected, &after, &[], &[]);
        assert!(!same_picture(&picture(&app), &picture_before), "{path}: the colour changed nothing that is drawn");

        let (undone, _) = app.tab().doc.as_ref().expect("open").session.undo().expect("undo");
        assert!(undone, "{path}: the colour did not undo");
        assert!(same_picture(&picture(&app), &picture_before), "{path}: one undo did not restore the page");
    }
    assert!(any, "neither real file is on this machine");
}

/// **A size set for a paragraph reaches every line** — every piece of every
/// line, which is what a size change has to rewrite — and undoes in one step.
#[test]
fn a_size_set_for_a_paragraph_reaches_every_line_and_undoes_in_one_step() {
    let mut app = two_piece_page("size-all-lines", &ROWS);
    pick_object(&mut app, 4);
    let before = runs_in_order(&app);
    let picture_before = picture(&app);
    app.tab_mut().editing_run.as_mut().expect("editing").style.size = Some(14.0);
    app.apply_editing_page();
    assert!(said(&app).contains("paragraph changed"), "the size was refused: {}", said(&app));
    let after = runs_in_order(&app);
    assert_eq!(after.len(), ROWS.len(), "every line is its first piece now, one object each");
    assert!(after.iter().all(|r| (r.size - 14.0).abs() < 0.05), "a line was left at the old size: {after:?}");
    let (undone, _) = app.tab().doc.as_ref().expect("open").session.undo().expect("undo");
    assert!(undone, "the size change did not undo");
    let restored = runs_in_order(&app);
    assert!(before.iter().zip(&restored).all(|(a, b)| same_run(a, b)), "undo did not restore the lines");
    assert!(same_picture(&picture(&app), &picture_before), "one undo did not restore the page");
}

// -- a retyped line keeps the width it had (justification) ---------------
//
// The app asks the engine to keep a retyped line of a justified paragraph as
// wide as it was (`TextLineEdit::Retype`'s `justify_to`, see
// `paragraph_lines::STRETCH_JUSTIFIED_LINES`). The engine honours the field —
// the left edge stays, the width is the right edge minus the left of the
// original line's box, exact to a thousandth of a point — so the switch is on
// and the first two tests below, the ones that need it, run. (They were ignored
// while the engine refused any width; they failed exactly as the report said:
// "9.51 pt short", "a click opened 2 of the paragraph's 13 lines".)

/// A real justified paragraph opened for editing: the datasheet's 13-line
/// paragraph (click on object 986 of page 1) or CAMINO's COB paragraph (the
/// run that says "manufacturers"). `None` where the file is not on this
/// machine.
fn open_a_justified_paragraph(path: &str, seed_text: Option<&str>) -> Option<(PagifyApp, EditingRun)> {
    if !std::path::Path::new(path).is_file() {
        eprintln!("skipping: {path} is not on this machine");
        return None;
    }
    let mut app = PagifyApp::new(Some(path));
    let runs = runs_in_order(&app);
    let run = match seed_text {
        Some(text) => runs.iter().find(|r| r.text.contains(text)),
        None => runs.iter().find(|r| r.object == 986),
    }
    .expect("the paragraph's own run was not found — has the file changed?");
    app.submit("edittext");
    app.pick_text_run(
        0,
        AppPoint {
            x: ((run.rect.left + run.rect.right) / 2.0) as f64,
            y: ((run.rect.top + run.rect.bottom) / 2.0) as f64,
        },
    )
    .expect("picked");
    let edit = app.tab().editing_run.as_ref().expect("an editor opened").clone();
    assert_eq!(edit.lines.len(), 13, "setup: the 13-line justified paragraph");
    assert!(paragraph_lines::ends_at_one_margin(&edit.lines), "setup: its lines end at one margin");
    Some((app, edit))
}

const MARINA_PARAGRAPH: (&str, Option<&str>) =
    (r"C:\Users\hsili\Desktop\Datasheets - Editors market - Marina mall.pdf", None);
const CAMINO_PARAGRAPH: (&str, Option<&str>) =
    (r"C:\Users\hsili\Downloads\CAMINO elitee-plus 3.0.pdf", Some("manufacturers"));

/// A plain middle line of the paragraph — words only, at least three — and
/// the buffer with that line's second word dropped: the line comes out
/// narrower than it was, and every letter it needs is on the page already.
fn drop_a_word_from_a_middle_line(edit: &EditingRun) -> (usize, String) {
    let lines: Vec<&str> = edit.buffer.split('\n').collect();
    let target = (1..lines.len() - 1)
        .find(|&i| {
            !edit.frozen[i]
                && lines[i].split_whitespace().count() >= 3
                && lines[i].chars().all(|c| c.is_alphabetic() || c == ' ')
        })
        .expect("a plain middle line of at least three words");
    let mut words: Vec<&str> = lines[target].split_whitespace().collect();
    words.remove(1);
    let mut retyped: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    retyped[target] = words.join(" ");
    (target, retyped.join("\n"))
}

/// Where a line's first piece is after the pieces `removed` came off the
/// page: its object number, less one for every removed piece numbered below it.
fn renumbered(first: usize, removed: &[usize]) -> usize {
    first - removed.iter().filter(|object| **object < first).count()
}

/// **A retyped line of a justified paragraph ends where it ended** — within
/// half a point — instead of a few points short of the margin. The retype
/// writes plain words and loses the producer's inter-word spacing; asked for
/// the width the line had, the engine stretches the spaces back.
#[test]
fn a_retyped_line_of_a_justified_paragraph_ends_where_it_ended() {
    let _turn = tests_support::one_at_a_time();
    let mut any = false;
    for (path, seed) in [MARINA_PARAGRAPH, CAMINO_PARAGRAPH] {
        let Some((mut app, edit)) = open_a_justified_paragraph(path, seed) else { continue };
        any = true;
        let (target, retyped) = drop_a_word_from_a_middle_line(&edit);
        let original = edit.lines[target].1;
        let (first, rest) = edit.lines[target].0.split_first().expect("the line has objects");
        app.tab_mut().editing_run.as_mut().expect("editing").buffer = retyped;
        app.apply_editing_page();
        let after = runs_in_order(&app);
        let now = after
            .iter()
            .find(|r| r.object == renumbered(*first, rest))
            .unwrap_or_else(|| panic!("{path}: the retyped line's first piece was not found"));
        assert!(
            (now.rect.right - original.right).abs() <= 0.5,
            "{path}: line {target} ends at {:.2}; it ended at {:.2} ({:.2} pt short)",
            now.rect.right,
            original.right,
            original.right - now.rect.right
        );
        assert!(
            (now.rect.left - original.left).abs() <= 0.5,
            "{path}: line {target} moved sideways: {:.2} then {:.2}",
            original.left,
            now.rect.left
        );
    }
    assert!(any, "neither real file is on this machine");
}

/// **After editing a line of a justified paragraph a click on it opens the
/// WHOLE paragraph again.** The retyped line used to end short; the detector
/// reads a short line as the paragraph's last and stopped there, so the next
/// click opened only the lines above it (CAMINO: 4 of 13).
#[test]
fn after_editing_one_line_a_click_opens_every_line_of_the_paragraph_again() {
    let _turn = tests_support::one_at_a_time();
    let mut any = false;
    for (path, seed) in [MARINA_PARAGRAPH, CAMINO_PARAGRAPH] {
        let Some((mut app, edit)) = open_a_justified_paragraph(path, seed) else { continue };
        any = true;
        let (target, retyped) = drop_a_word_from_a_middle_line(&edit);
        let (first, rest) = edit.lines[target].0.split_first().expect("the line has objects");
        app.tab_mut().editing_run.as_mut().expect("editing").buffer = retyped.clone();
        app.apply_editing_page();

        // Click the retyped line itself, and a line below it.
        let after = runs_in_order(&app);
        for (what, object) in [
            ("the retyped line", renumbered(*first, rest)),
            ("the line after it", renumbered(edit.lines[target + 1].0[0], rest)),
        ] {
            app.tab_mut().editing_run = None;
            let run = after.iter().find(|r| r.object == object).expect("a piece");
            app.pick_text_run(
                0,
                AppPoint {
                    x: ((run.rect.left + run.rect.right) / 2.0) as f64,
                    y: ((run.rect.top + run.rect.bottom) / 2.0) as f64,
                },
            )
            .expect("picked again");
            let again = app.tab().editing_run.as_ref().expect("an editor opened").clone();
            assert_eq!(
                again.lines.len(),
                edit.lines.len(),
                "{path}: a click on {what} opened {} of the paragraph's {} lines",
                again.lines.len(),
                edit.lines.len()
            );
            assert_eq!(
                again.buffer.split('\n').map(squeeze).collect::<Vec<_>>(),
                retyped.split('\n').map(squeeze).collect::<Vec<_>>(),
                "{path}: a click on {what} does not read the paragraph as it was edited"
            );
        }
    }
    assert!(any, "neither real file is on this machine");
}

/// **The paragraph's last line is not stretched**: a short closing line stays
/// as short as its words make it, and nothing else on the page changes.
#[test]
fn the_last_line_of_a_justified_paragraph_is_not_stretched() {
    let _turn = tests_support::one_at_a_time();
    let mut any = false;
    for (path, seed) in [MARINA_PARAGRAPH, CAMINO_PARAGRAPH] {
        let Some((mut app, edit)) = open_a_justified_paragraph(path, seed) else { continue };
        any = true;
        let last = edit.lines.len() - 1;
        let mut lines: Vec<String> = edit.buffer.split('\n').map(str::to_string).collect();
        // The last line cut to its first two thirds, a letter at a time — a
        // short closing line may be a single word — so it is narrower and
        // needs no letter the page does not already have.
        let cut = (lines[last].chars().count() * 2 / 3).max(1);
        lines[last] = lines[last].chars().take(cut).collect();
        let original = edit.lines[last].1;
        let (first, rest) = edit.lines[last].0.split_first().expect("the line has objects");
        let before = runs_in_order(&app);
        app.tab_mut().editing_run.as_mut().expect("editing").buffer = lines.join("\n");
        app.apply_editing_page();
        let after = runs_in_order(&app);
        let new_text = lines[last].clone();
        tests_support::assert_page_after(
            &format!("{path}: the last line retyped"),
            &before,
            &after,
            &[(*first, &new_text)],
            rest,
        );
        let now = after.iter().find(|r| r.object == renumbered(*first, rest)).expect("the last line's first piece");
        assert!(
            now.rect.right < original.right - 3.0,
            "{path}: the last line was stretched: it ends at {:.2}, it ended at {:.2}",
            now.rect.right,
            original.right
        );
    }
    assert!(any, "neither real file is on this machine");
}

/// **A ragged (left-aligned) paragraph's lines are not stretched either**: a
/// word deleted from a line that did not reach the margin leaves it shorter —
/// it is not spaced out to fill the room it left. (Left-aligned text starts at
/// one margin like a justified paragraph does; where the lines *end* is what
/// tells them apart.)
#[test]
fn a_ragged_paragraphs_retyped_line_is_not_stretched() {
    let lines: [(f32, f32, &str); 5] = [
        (100.0, 700.0, "alpha beta gamma delta epsilon zeta eta theta"),
        (100.0, 688.0, "iota kappa lambda mu"),
        (100.0, 676.0, "nu xi omicron pi rho sigma tau upsilon phi"),
        (100.0, 664.0, "chi psi omega alpha"),
        (100.0, 652.0, "beta gamma delta epsilon zeta"),
    ];
    let mut app = open_page("ragged", &lines, "");
    pick_object(&mut app, 0);
    let edit = app.tab().editing_run.as_ref().expect("picked").clone();
    assert_eq!(edit.lines.len(), 5, "setup: five lines: {:?}", edit.buffer);
    assert!(
        !paragraph_lines::ends_at_one_margin(&edit.lines),
        "setup: the lines end all over the place: {:?}",
        edit.lines.iter().map(|(_, r)| r.right).collect::<Vec<_>>()
    );
    let original = edit.lines[0].1;
    let mut typed: Vec<String> = edit.buffer.split('\n').map(str::to_string).collect();
    typed[0] = "alpha beta gamma delta epsilon theta".to_string();
    app.tab_mut().editing_run.as_mut().expect("editing").buffer = typed.join("\n");
    app.apply_editing_page();
    let now = runs_in_order(&app).into_iter().next().expect("the first line");
    assert!(
        now.rect.right < original.right - 10.0,
        "a ragged line was stretched back to its old width: ends at {:.2}, it ended at {:.2}",
        now.rect.right,
        original.right
    );
}
