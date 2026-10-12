//! **What the editor shows is what the page has.**
//!
//! Reported with two screenshots: the page after two lines had been retyped with
//! spaces typed in to line a value up, and the paragraph editor opened over the
//! same two lines. The editor showed the words spread across the box, and a gap
//! gone; the page had its gap where it had been typed. Three things made them
//! differ, and each is held here:
//!
//! 1. PDFium's text layer folds a run of space glyphs into one space, so the
//!    buffer lacked the page's own spaces (and an apply from it would have taken
//!    the line's gap off the page, or left it, depending on which line was
//!    touched);
//! 2. the editor stretched any block whose lines start at one margin, where the
//!    apply asked for a width only if they also ended at one — and a line whose
//!    box runs on past its last ink is padded, not justified;
//! 3. the single run took the leading spaces off before writing.
//!
//! The datasheet is not in the repository; without it these skip.

use super::*;
use pdf_core::document::{Rect, TextRun};

const DATASHEET: &str = r"C:\Users\hsili\Desktop\test pdf for pgify\TECHNICAL DATASHEET Q-2075-REV.pdf";

fn centre(rect: &Rect) -> AppPoint {
    AppPoint { x: ((rect.left + rect.right) / 2.0) as f64, y: ((rect.top + rect.bottom) / 2.0) as f64 }
}

fn runs(app: &PagifyApp) -> Vec<TextRun> {
    app.tab().doc.as_ref().expect("open").session.text_runs(0).expect("runs")
}

fn run_with(app: &PagifyApp, needle: &str) -> TextRun {
    runs(app).into_iter().find(|r| r.text.contains(needle)).unwrap_or_else(|| panic!("{needle:?} is not on the page"))
}

/// The page as the user had it: both lines retyped on their own, each with spaces
/// typed in to put the value well to the right.
fn the_users_page() -> Option<PagifyApp> {
    if !std::path::Path::new(DATASHEET).exists() {
        eprintln!("skipped: the datasheet is not at {DATASHEET}");
        return None;
    }
    let mut app = PagifyApp::new(Some(DATASHEET));
    for (needle, typed) in [
        ("Driver Efficiency", format!("Driver Efficiency:{}>95%", " ".repeat(21))),
        ("Driver Power Factor", format!("Driver Power Factor: >9{}0", " ".repeat(10))),
    ] {
        let run = run_with(&app, needle);
        app.pick_text_run(0, centre(&run.rect)).expect("picked");
        app.tab_mut().edit.editing_run.as_mut().expect("editing").buffer = typed;
        app.apply_editing_page();
    }
    Some(app)
}

fn reopened_block(app: &mut PagifyApp) -> EditingRun {
    let run = run_with(app, "Driver Efficiency");
    // Clicked on the value, to the right of the words: where the report's click fell.
    app.pick_text_run(0, AppPoint { x: 171.4, y: 328.1 }).or_else(|_| app.pick_text_run(0, centre(&run.rect))).expect("picked");
    app.tab().edit.editing_run.as_ref().expect("an editor opened").clone()
}

/// **The buffer holds the page's own spaces**, and PDFium's text — which is what the
/// box held before — did not.
#[test]
fn the_box_holds_the_spaces_the_page_has() {
    let Some(mut app) = the_users_page() else { return };
    let line = run_with(&app, "Driver Efficiency");
    assert!(!line.text.contains("  "), "PDFium now keeps the spaces, and this test no longer proves anything: {:?}", line.text);

    let t = std::time::Instant::now();
    let edit = reopened_block(&mut app);
    println!("the pick, exact text read included, took {:?}", t.elapsed());
    assert_eq!(edit.lines.len(), 2, "the two lines are one paragraph, as in the report");
    let typed = format!("Driver Efficiency:{}>95%", " ".repeat(21));
    let lines: Vec<&str> = edit.buffer.split('\n').collect();
    assert_eq!(lines[0].trim_end(), typed, "line 1 lost its gap: {:?}", edit.buffer);
    assert_eq!(lines[1].trim_end(), format!("Driver Power Factor: >9{}0", " ".repeat(10)), "line 2 lost its gap");
    assert_eq!(edit.original, edit.buffer, "what it said and what is typed start the same");
}

/// **A padded line is not justified**, and neither the preview nor the apply
/// stretches it: the line's box runs on past its ink.
#[test]
fn a_padded_pair_of_lines_is_not_drawn_justified() {
    let Some(mut app) = the_users_page() else { return };
    let edit = reopened_block(&mut app);
    assert!(
        pagify_shell::paragraph_lines::paragraph_should_justify(&edit.lines)
            && pagify_shell::paragraph_lines::ends_at_one_margin(&edit.lines),
        "the geometry alone would have called it justified — that is the trap"
    );
    assert!(!edit.justify, "a block padded with spaces was drawn stretched to its box");
}

/// **A block that really is justified still is**: the datasheet's first paragraph,
/// whose lines end on a letter at one margin.
#[test]
fn a_justified_paragraph_still_is() {
    if !std::path::Path::new(DATASHEET).exists() {
        return;
    }
    let mut app = PagifyApp::new(Some(DATASHEET));
    let (pb, _) = app.page_blocks(0).expect("blocks");
    let block = pb
        .blocks
        .iter()
        .position(|b| b.lines.len() >= 4 && b.lines.iter().all(|l| l.outlined.is_empty() && l.objects.len() == 1))
        .expect("a written paragraph of four lines or more on page 1");
    let object = pb.blocks[block].lines[0].objects[0];
    let rect = pb.runs[&object].rect;
    app.pick_text_run(0, centre(&rect)).expect("picked");
    let edit = app.tab().edit.editing_run.as_ref().expect("an editor opened").clone();
    let geometry = pagify_shell::paragraph_lines::paragraph_should_justify(&edit.lines)
        && pagify_shell::paragraph_lines::ends_at_one_margin(&edit.lines);
    println!("{} lines, geometry says justified: {geometry}, editor says: {}", edit.lines.len(), edit.justify);
    // Whatever the geometry says of this paragraph, the ink gate may only ever take a
    // stretch away from a padded line — never from one whose lines end on their ink.
    assert_eq!(edit.justify, geometry, "an ordinary paragraph lost its justification");
}

/// **Applying the paragraph as it stands changes nothing**, and applying a change to
/// one line leaves the other as it was — the gap included.
#[test]
fn an_edit_to_one_line_keeps_the_other_and_its_own_gap() {
    let Some(mut app) = the_users_page() else { return };
    let before = runs(&app);
    let (first, second) = (run_with(&app, "Driver Efficiency"), run_with(&app, "Driver Power Factor"));

    // As it stands: nothing is written.
    reopened_block(&mut app);
    app.apply_editing_page();
    for run in &before {
        let now = runs(&app).into_iter().find(|r| r.object == run.object).expect("still there");
        assert_eq!((now.text.as_str(), now.rect.right), (run.text.as_str(), run.rect.right), "object {}", run.object);
    }

    // One word of line 1 changed: line 1 is written with its gap, line 2 is not touched.
    let mut edit = reopened_block(&mut app);
    edit.buffer = edit.buffer.replace(">95%", ">99%");
    app.tab_mut().edit.editing_run = Some(edit);
    app.apply_editing_page();
    let (one, two) = (run_with(&app, "Driver Efficiency"), run_with(&app, "Driver Power Factor"));
    assert!(one.text.contains(">99%"), "{:?}", one.text);
    assert_eq!(two.rect.right, second.rect.right, "line 2 was written though nothing in it changed");
    // Its right edge alone would not say: a stretched line ends where it ended with the gap
    // in the wrong place. The stream says where the gap is.
    let exact = app.tab().doc.as_ref().unwrap().session.run_exact_texts(0, &[one.object]).expect("exact");
    assert_eq!(
        exact[&one.object].trim_end(),
        format!("Driver Efficiency:{}>99%", " ".repeat(21)),
        "line 1 was rewritten without its typed gap"
    );
    assert!(
        // A 9 is a hair wider than a 5; the gap itself is 46 pt.
        (one.rect.right - first.rect.right).abs() < 3.0,
        "line 1 moved from {:.1} to {:.1}: its gap did not come through the retype",
        first.rect.right,
        one.rect.right
    );
}

/// **The preview shows what the apply would write.** The same text, laid out by
/// the page's own measure, is what `Written` carries: nothing but the typed words.
#[test]
fn the_apply_writes_the_spaces_that_were_shown() {
    let Some(mut app) = the_users_page() else { return };
    let mut edit = reopened_block(&mut app);
    let shown: Vec<String> = edit.buffer.split('\n').map(str::to_string).collect();
    edit.buffer = edit.buffer.replace(">95%", ">99%");
    let commands = app.paragraph_edit_commands(&edit, &edit.buffer).expect("commands").commands;
    let written: Vec<String> = commands
        .iter()
        .flat_map(|command| match command {
            pdf_core::command::Command::ReplaceTextLines { edits, .. } => edits
                .iter()
                .filter_map(|e| match e {
                    pdf_core::document::TextLineEdit::Retype { text, justify_to, .. } => {
                        assert!(justify_to.is_none(), "a padded line was asked to be stretched");
                        Some(text.clone())
                    }
                    _ => None,
                })
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        })
        .collect();
    assert_eq!(written.len(), 1, "only the changed line is written: {written:?}");
    assert_eq!(written[0], shown[0].replace(">95%", ">99%"), "what is written is what was shown");
}

/// **A label over a value set in by spaces is not a paragraph.** The reported pair —
/// "Tunable option controller" over an indented "remote RF driver, DMX" — was drawn
/// with its first line spread across the box: the two lines' boxes start at one
/// margin (the value's own begins with the spaces that indent it) and the first ends
/// on its ink, so the geometry called it justified. Where each line starts to *show*
/// says otherwise.
#[test]
fn a_label_over_an_indented_value_is_not_drawn_justified() {
    if !std::path::Path::new(DATASHEET).exists() {
        eprintln!("skipped: the datasheet is not at {DATASHEET}");
        return;
    }
    let mut app = PagifyApp::new(Some(DATASHEET));
    let run = run_with(&app, "Tunable option");
    app.pick_text_run(0, centre(&run.rect)).expect("picked");
    let edit = app.tab().edit.editing_run.as_ref().expect("an editor opened").clone();
    assert_eq!(edit.lines.len(), 2, "the label and its value are one block");
    let value = edit.buffer.lines().nth(1).expect("a second line");
    assert!(value.starts_with("     "), "the value's own indent is in the box: {value:?}");
    assert!(
        pagify_shell::paragraph_lines::paragraph_should_justify(&edit.lines)
            && pagify_shell::paragraph_lines::ends_at_one_margin(&edit.lines),
        "the geometry alone would have called it justified — that is the trap"
    );
    assert!(!edit.justify, "the label was stretched across the box");
}
