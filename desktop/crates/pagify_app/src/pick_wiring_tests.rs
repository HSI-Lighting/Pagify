use super::*;
use pdf_core::document::{Rect, TextRun};

const MARINA: &str = r"C:\Users\hsili\Desktop\test pdf for pgify\Datasheets - Editors market - Marina mall.pdf";
const CAMINO: &str = r"C:\Users\hsili\Downloads\CAMINO elitee-plus 3.0.pdf";
const DATASHEET: &str = r"C:\Users\hsili\Desktop\test pdf for pgify\TECHNICAL DATASHEET Q-2075-REV.pdf";
const RING600: &str = r"D:\Dropbox\Datasheets\Data sheet 2024\RING 600.pdf";

fn fixture(name: &str) -> String {
    format!("{}/../../../rust/pdf_core/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))
}

use super::tests_support::one_at_a_time;

fn app(name: &str) -> PagifyApp {
    let app = PagifyApp::new(Some(&fixture(name)));
    assert!(app.tab().doc.is_some(), "{name} did not open");
    app
}

/// The middle of a box, as a click on it.
fn centre(rect: &Rect) -> AppPoint {
    AppPoint { x: ((rect.left + rect.right) / 2.0) as f64, y: ((rect.top + rect.bottom) / 2.0) as f64 }
}

fn text_runs(app: &PagifyApp, page: usize) -> Vec<TextRun> {
    app.tab().doc.as_ref().expect("open").session.text_runs(page).expect("runs")
}

/// The session log of `app` redirected to a scratch folder, and where to
/// read it back from. The folder is the caller's to remove.
fn log_into(app: &mut PagifyApp, tag: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("pagify-test-pick-{tag}-{}", std::process::id()));
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

// -- the user's paragraph ------------------------------------------------

/// The objects of one block, grouped into the visual lines a person sees —
/// by baseline, top to bottom, each line left to right. **Worked out from
/// the page's own words and positions, not from the detector**, so it is a
/// second opinion on what the detector says.
fn expected_lines(runs: &HashMap<usize, TextRun>, ids: &[usize]) -> Vec<Vec<usize>> {
    let mut ids = ids.to_vec();
    ids.sort_by(|a, b| runs[a].origin.y.total_cmp(&runs[b].origin.y).then(a.cmp(b)));
    let mut lines: Vec<Vec<usize>> = Vec::new();
    let mut baseline = f32::NAN;
    for id in ids {
        if lines.is_empty() || (runs[&id].origin.y - baseline).abs() > 3.0 {
            baseline = runs[&id].origin.y;
            lines.push(vec![id]);
        } else {
            lines.last_mut().expect("a line").push(id);
        }
    }
    for line in &mut lines {
        line.sort_by(|a, b| runs[a].rect.left.total_cmp(&runs[b].rect.left));
    }
    lines
}

/// **The user's own paragraph, on the user's own datasheet.** Page 1 has,
/// one under the other: a five-line paragraph (objects 951 to 979), a
/// heading, the 13-line paragraph the user marked in red (985 to 1043 —
/// 57 text objects, plus two words the page draws as shapes, 1026 and
/// 1035, which are not text objects at all), and another heading. Clicking
/// *any* word of any of them has to open exactly that block, whole: the old
/// geometric walk managed it for none of these 95 words.
///
/// Every one of the 95 words is clicked in turn and each time the editor
/// must hold: the block's own objects in the block's own lines (worked
/// out here from the page, see [`expected_lines`]); one buffer line per
/// line; the words exactly as the page has them, with a hyphen only where
/// the page has one; the two lines with a drawn word in them frozen and
/// no others; and the look of the body text — not of the three-letter
/// "HSI" in another weight inside the first line, and not of the headings.
///
/// Skipped, with a note, where the file is not on this machine.
#[test]
fn clicking_any_word_of_the_datasheets_blocks_opens_exactly_that_block() {
    if !std::path::Path::new(MARINA).is_file() {
        eprintln!("skipping: the Marina datasheet is not on this machine");
        return;
    }
    let _turn = one_at_a_time();
    let mut app = PagifyApp::new(Some(MARINA));
    assert!(app.tab().doc.is_some(), "the datasheet is on this machine but would not open");
    let runs: HashMap<usize, TextRun> = text_runs(&app, 0).into_iter().map(|r| (r.object, r)).collect();
    let (page, _) = app.page_blocks(0).expect("the page's blocks");
    let font_of = |object: usize| page.styles[&object].font;
    fn face_key(app: &PagifyApp, object: usize) -> u64 {
        let bytes = app.tab().doc.as_ref().unwrap().session.run_font_data(0, object).unwrap().expect("a font");
        font_key(&bytes)
    }

    // (what it is, its objects, which of its lines draw a word as shapes,
    //  an object that is in the body font / the heading font of the block)
    let blocks: [(&str, std::ops::RangeInclusive<usize>, &[usize], usize); 4] = [
        ("the five-line paragraph", 951..=979, &[], 951),
        ("the heading 'The Light Source - COB'", 980..=984, &[], 981),
        ("the 13-line paragraph marked in red", 985..=1043, &[8, 10], 986),
        ("the heading under it", 1044..=1047, &[], 1045),
    ];
    // The two fonts the test leans on must really be different fonts, or
    // "the body font, not the heading's" would pass for any answer.
    assert_ne!(font_of(986), font_of(981), "setup: the body and the heading are one font");
    assert_ne!(font_of(986), font_of(987), "setup: the 'HSI ' scrap is the body's own font");
    assert_ne!(face_key(&app, 986), face_key(&app, 981), "setup: the body and heading font programs are one");
    assert_ne!(face_key(&app, 986), face_key(&app, 987), "setup: the scrap's font program is the body's");

    let mut ran = 0usize;
    let mut alone = 0usize;
    let mut failures: Vec<String> = Vec::new();
    let mut times: Vec<std::time::Duration> = Vec::new();
    for (name, range, frozen_at, look_of) in blocks {
        let ids: Vec<usize> = range.filter(|o| runs.contains_key(o)).collect();
        let lines = expected_lines(&runs, &ids);
        let frozen: Vec<bool> = (0..lines.len()).map(|i| frozen_at.contains(&i)).collect();
        let texts: Vec<String> = lines
            .iter()
            .map(|line| line.iter().map(|o| runs[o].text.replace(['\r', '\n'], " ")).collect::<String>())
            .collect();
        let want_buffer = fix_extracted_text(&texts.join("\n"));
        let want_font = font_of(look_of);
        let want_face = face_key(&app, look_of);
        eprintln!("{name}: {} words in {} lines", ids.len(), lines.len());

        for &seed in &ids {
            ran += 1;
            app.tab_mut().edit.editing_run = None;
            let started = std::time::Instant::now();
            let outcome = app.pick_text_run(0, centre(&runs[&seed].rect));
            times.push(started.elapsed());
            let who = format!("{name}, clicking object {seed} ({:?})", runs[&seed].text);
            let message = match outcome {
                Ok(message) => message,
                Err(why) => {
                    failures.push(format!("{who}: refused: {why}"));
                    continue;
                }
            };
            let Some(edit) = app.tab().edit.editing_run.as_ref() else {
                failures.push(format!("{who}: no editor opened"));
                continue;
            };
            // **A word on a line the page draws part of as shapes opens that
            // word alone**, not the paragraph: in the paragraph its line is
            // frozen — never written — so a box over it could not change the
            // words that were clicked. The paragraph is still one click away on
            // any of its written lines (all the rest of the words, below).
            let own_line = lines.iter().position(|line| line.contains(&seed)).expect("the seed is on a line");
            if frozen_at.contains(&own_line) {
                alone += 1;
                if edit.lines != vec![(vec![seed], runs[&seed].rect)] || edit.frozen != [false] {
                    failures.push(format!("{who}: on a frozen line it should open the word alone: {:?}", edit.lines));
                } else if edit.buffer != fix_extracted_text(&runs[&seed].text) {
                    failures.push(format!("{who}: the word alone reads {:?}", edit.buffer));
                } else if !message.starts_with("opened this word alone: its line is partly drawn as shapes") {
                    failures.push(format!("{who}: the message does not say why it opened alone: {message}"));
                }
                continue;
            }
            // **The paragraph is the written lines around the click**: a line with
            // a drawn word in it cuts the block, and nothing drawn is ever in the
            // box. Worked out here from which lines the page draws part of as
            // shapes, not from the cut the editor makes.
            let (mut from, mut to) = (own_line, own_line + 1);
            while from > 0 && !frozen[from - 1] {
                from -= 1;
            }
            while to < lines.len() && !frozen[to] {
                to += 1;
            }
            let want_lines = &lines[from..to];
            let want_buffer = fix_extracted_text(&texts[from..to].join("\n"));
            let got: Vec<Vec<usize>> = edit.lines.iter().map(|(objects, _)| objects.clone()).collect();
            if got != want_lines {
                let flat = |ls: &[Vec<usize>]| ls.iter().flatten().copied().collect::<std::collections::BTreeSet<_>>();
                failures.push(format!(
                    "{who}: opened {} lines / {} objects, wanted {} lines / {} objects (missing {:?}, extra {:?})",
                    got.len(),
                    flat(&got).len(),
                    want_lines.len(),
                    flat(want_lines).len(),
                    flat(want_lines).difference(&flat(&got)).collect::<Vec<_>>(),
                    flat(&got).difference(&flat(want_lines)).collect::<Vec<_>>()
                ));
                continue;
            }
            if edit.buffer.split('\n').count() != edit.lines.len() {
                failures.push(format!("{who}: {} buffer lines for {} lines", edit.buffer.split('\n').count(), edit.lines.len()));
            }
            if edit.original != edit.buffer {
                failures.push(format!("{who}: the buffer is not what was picked"));
            }
            if edit.buffer != want_buffer {
                failures.push(format!("{who}: the words differ from the page's own:\n  got  {:?}\n  want {want_buffer:?}", edit.buffer));
            }
            if edit.frozen.iter().any(|f| *f) {
                failures.push(format!("{who}: a drawn line is in the box: frozen {:?}", edit.frozen));
            }
            if edit.object != want_lines[0][0] {
                failures.push(format!("{who}: the editor sits on object {}, not the first, {}", edit.object, want_lines[0][0]));
            }
            if font_of(edit.look_object) != want_font || !ids.contains(&edit.look_object) {
                failures.push(format!(
                    "{who}: the look comes from object {} (font {}), wanted font {want_font}",
                    edit.look_object,
                    font_of(edit.look_object)
                ));
            }
            if app.editor_face != Some(want_face) {
                failures.push(format!("{who}: the editor asked for another font program than the block's own"));
            }
            // No drawn line is in the box, so the pick has none to talk about.
            if message.contains("drawn as shapes") {
                failures.push(format!("{who}: the message talks of drawn words that are not in the box: {message}"));
            }
        }
    }

    times.sort();
    eprintln!(
        "datasheet page 1: {ran} picks — first {:?}, median {:?}, slowest {:?}",
        times.first().copied().unwrap_or_default(),
        times[times.len() / 2],
        times.last().copied().unwrap_or_default()
    );
    assert_eq!(ran, 95, "29 + 5 + 57 + 4 words: has the file changed?");
    assert_eq!(alone, 9, "the words on the two lines with a drawn word in them (4 + 5): has the file changed?");
    assert!(
        failures.is_empty(),
        "{} of {ran} clicks did not open the block they were in:\n{}",
        failures.len(),
        failures.iter().take(12).cloned().collect::<Vec<_>>().join("\n")
    );
}

/// **A hyphen before a drawn line stays — in the buffer, and on the page once
/// the line is retyped.** The datasheet's first paragraph (objects 8 to 16,
/// "VEGA series is powerful lighting solutions...") has a line the page draws
/// as shapes between its first and its fourth lines, and the line before it
/// ends "driv" + the page's hyphen code: the census found the hyphen dropped
/// from the buffer — so retyping the line would have taken it off the page.
///
/// Checked on the real page: every line of the buffer is the page's own words
/// with the hyphen code as "-" (including that one); a word of line 1 is
/// retyped and applied; the retyped piece ends in a real "-" that PDFium reads
/// back, the apply says it went through, and a click on the paragraph again
/// opens the same lines with the hyphen still there once, not twice.
///
/// **The nine-line block is cut at its drawn lines now**, so what opens is the
/// two written lines above the first of them — the second being the one that
/// ends in the hyphen — and the drawn line after it is not in the box at all.
///
/// Skipped, with a note, where the file is not on this machine.
#[test]
fn the_hyphen_before_a_drawn_line_stays_in_the_buffer_and_on_the_page_when_the_line_is_retyped() {
    if !std::path::Path::new(MARINA).is_file() {
        eprintln!("skipping: the Marina datasheet is not on this machine");
        return;
    }
    let _turn = one_at_a_time();
    let mut app = PagifyApp::new(Some(MARINA));
    let runs: HashMap<usize, TextRun> = text_runs(&app, 0).into_iter().map(|r| (r.object, r)).collect();
    assert!(runs[&9].text.ends_with('\u{2}'), "setup: object 9 ends in the hyphen code: {:?}", runs[&9].text);
    app.pick_text_run(0, centre(&runs[&8].rect)).expect("picked");
    let edit = app.tab().edit.editing_run.as_ref().expect("an editor opened").clone();
    assert_eq!(edit.lines.len(), 2, "setup: the two written lines above the drawn one: {:?}", edit.buffer);
    assert_eq!(edit.frozen, [false, false]);

    // The page's own words, the hyphen code as "-" wherever there is one.
    let wanted: Vec<String> = edit
        .lines
        .iter()
        .map(|(objects, _)| {
            if objects.is_empty() {
                block_input::OUTLINED_PLACEHOLDER.to_string()
            } else {
                objects.iter().map(|o| runs[o].text.replace('\u{2}', "-")).collect()
            }
        })
        .collect();
    let lines: Vec<&str> = edit.buffer.split('\n').collect();
    assert_eq!(lines, wanted, "the buffer is not the page's own words");
    assert!(lines[1].ends_with("COB, driv-"), "the hyphen before the drawn line was dropped: {:?}", lines[1]);

    // Retype a word of that line.
    let mut typed: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    typed[1] = typed[1].replacen("technology", "technologies", 1);
    assert_ne!(typed[1], lines[1], "setup: the word is on the line");
    app.tab_mut().edit.editing_run.as_mut().expect("editing").buffer = typed.join("\n");
    app.apply_editing_page();
    let said = app.cmd.history().iter().map(|e| e.text.as_str()).collect::<Vec<_>>().join("\n");
    assert!(said.contains("paragraph changed"), "the apply did not go through:\n{said}");

    // On the page: the retyped piece is the typed words, and the typed "-" is
    // written as the font's own hyphen glyph — the same code the page used
    // before, which this document's broken text mapping reads back as the
    // control code again. So the hyphen is still drawn, as the same glyph, in
    // the line's own font (nothing was substituted).
    let after: HashMap<usize, TextRun> = text_runs(&app, 0).into_iter().map(|r| (r.object, r)).collect();
    assert_eq!(
        after[&9].text,
        typed[1].replace('-', "\u{2}"),
        "what the page now says of that line: the hyphen is no longer the page's own glyph"
    );
    assert!(after[&9].text.ends_with("driv\u{2}"));
    let substituted = app.tab().doc.as_ref().expect("open").session.substituted_face();
    assert_eq!(substituted, None, "the line was written in another font than its own");

    // And the detector still reads it as a hyphenated line end: the same nine
    // lines, the same words, the hyphen once.
    app.tab_mut().edit.editing_run = None;
    app.pick_text_run(0, centre(&after[&8].rect)).expect("picked again");
    let again = app.tab().edit.editing_run.as_ref().expect("an editor opened").clone();
    assert_eq!(again.lines.len(), 2, "the paragraph reopens as the same two lines: {:?}", again.buffer);
    let lines_again: Vec<&str> = again.buffer.split('\n').collect();
    assert_eq!(lines_again[1], typed[1], "line 1 reopens as it was typed");
    assert!(!again.buffer.contains("--"), "the hyphen was doubled: {:?}", again.buffer);
}

/// **The box takes its look from the font most of the text is set in, by
/// the page's own identity for the font — not by its name.** The real
/// datasheet names five different weights "Montserrat-Thin", so a vote on
/// names sees one font and falls back to the first line's: a paragraph
/// that starts with a bold scrap would open in bold. Here every object has
/// the same name and size; only the font ids differ.
#[test]
fn the_look_is_the_font_most_of_the_text_is_set_in_even_when_every_font_has_the_same_name() {
    let mut app = app("text-lines.pdf");
    let run = |object: usize, text: &str, top: f32| TextRun {
        object,
        text: text.to_string(),
        rect: Rect { left: 100.0, top, right: 100.0 + 5.0 * text.len() as f32, bottom: top + 10.0 },
        origin: pdf_core::document::Point { x: 100.0, y: top + 8.0 },
        size: 8.0,
        color: pdf_core::document::Color { r: 0, g: 0, b: 0, a: 255 },
    };
    let runs = vec![run(0, "Head", 100.0), run(1, "the body of the paragraph", 112.0), run(2, "more body", 124.0)];
    let names: HashMap<usize, String> = (0..3).map(|o| (o, "Montserrat-Thin".to_string())).collect();
    let lines = || -> Vec<ParagraphLine> {
        runs.iter().map(|r| ParagraphLine { objects: vec![r.object], rect: r.rect, frozen: false, follows_drawn: false }).collect()
    };
    let style = |font: u32| pdf_core::document::RunStyle { font, stem_milli_em: Some(51), axis: (1.0, 0.0) };

    // Object 0 alone in one font, 1 and 2 in another: the second has most of the ink.
    let styles: HashMap<usize, pdf_core::document::RunStyle> =
        [(0, style(3)), (1, style(5)), (2, style(5))].into_iter().collect();
    let (edit, look) = app.build_editor_from_lines(0, lines(), &runs, &names, &styles, &[], None).expect("built");
    assert_eq!(look, 1, "the body's first object, not the first line's");
    assert_eq!(edit.look_object, 1);

    // One font throughout: nothing to outvote, the first object it is.
    let one: HashMap<usize, pdf_core::document::RunStyle> =
        [(0, style(5)), (1, style(5)), (2, style(5))].into_iter().collect();
    let (edit, look) = app.build_editor_from_lines(0, lines(), &runs, &names, &one, &[], None).expect("built");
    assert_eq!((look, edit.look_object), (0, 0));
}

/// **The whole of what the user asked for, on the user's own paragraph:
/// click a word of it, retype one line, apply — and only that line
/// changes.** The 13-line paragraph of page 1 (objects 985 to 1043) has
/// lines of three to eight pieces, two lines with a word the page draws
/// as shapes, and a hyphen glyph at the end of five of them. One middle
/// line made of plain words is retyped by repeating a word it already
/// has (so no letter the font lacks is needed); every other line's objects
/// must hold exactly the words and colour they held, the retyped line's
/// other pieces are removed from the page, and nothing else is.
#[test]
fn retyping_one_line_of_the_datasheets_paragraph_writes_only_that_line() {
    if !std::path::Path::new(MARINA).is_file() {
        eprintln!("skipping: the Marina datasheet is not on this machine");
        return;
    }
    let _turn = one_at_a_time();
    let mut app = PagifyApp::new(Some(MARINA));
    assert!(app.tab().doc.is_some(), "the datasheet is on this machine but would not open");
    let runs: HashMap<usize, TextRun> = text_runs(&app, 0).into_iter().map(|r| (r.object, r)).collect();
    app.pick_text_run(0, centre(&runs[&986].rect)).expect("picked");
    let edit = app.tab().edit.editing_run.as_ref().expect("an editor opened").clone();
    // The 13 lines are cut at the two with a drawn word in them (lines 8 and 10); a click on its first line
    // opens the eight above the first of them.
    assert_eq!(edit.lines.len(), 8, "setup: the user's paragraph, cut above its first drawn line");

    // Line 3, "lighting industry conforming": plain words, several pieces.
    let target = 3;
    let typed_lines: Vec<String> = edit.buffer.split('\n').map(str::to_string).collect();
    assert!(typed_lines[target].starts_with("lighting industry"), "setup: {:?}", typed_lines[target]);
    assert!(edit.lines[target].0.len() > 1, "setup: the line is made of several pieces");
    let mut retyped = typed_lines.clone();
    retyped[target] = format!("{} industry", typed_lines[target].trim_end());

    let before = tests_support::runs_in_order(&app);
    app.tab_mut().edit.editing_run.as_mut().expect("editing").buffer = retyped.join("\n");
    let started = std::time::Instant::now();
    app.apply_editing_page();
    eprintln!("timing: retyping one line of the 13-line datasheet paragraph took {:?}", started.elapsed());
    let said = app.cmd.history().iter().map(|e| e.text.as_str()).collect::<Vec<_>>().join("\n");
    assert!(
        said.contains("paragraph changed"),
        "the apply did not go through — the engine must be able to write every object of the line:\n{said}"
    );
    let after = tests_support::runs_in_order(&app);

    // The retyped line is its first piece holding the new words and its
    // other pieces are gone from the page (they were painted in the page's
    // colour, which left their old words in the file); every other object
    // — every piece of every other line, and everything outside the
    // paragraph — holds exactly the words, colour and place it held.
    let (first, rest) = edit.lines[target].0.split_first().expect("the line has objects");
    tests_support::assert_page_after(
        "one line of the datasheet's paragraph",
        &before,
        &after,
        &[(*first, &retyped[target])],
        rest,
    );
}

// -- the log -------------------------------------------------------------

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

/// `key=value` out of a pick line (values hold no spaces).
fn field<'a>(line: &'a str, key: &str) -> &'a str {
    line.split(' ')
        .find_map(|pair| pair.strip_prefix(key).and_then(|rest| rest.strip_prefix('=')))
        .unwrap_or_else(|| panic!("no `{key}=` in {line:?}"))
}

/// **Every click leaves exactly one `pick` line in the session log, on
/// whichever way it goes** — a paragraph, one run, a joined group, a word
/// drawn as outlines, rotated text refused, nothing there — and the line
/// holds ids, counts, geometry and times, never a word of the page.
#[test]
fn every_click_writes_exactly_one_content_free_pick_line_to_the_session_log() {
    let _turn = one_at_a_time();
    // -- a paragraph, then the same click again -------------------------
    let mut app = app("two-column.pdf");
    let (dir, log) = log_into(&mut app, "paths");
    let runs = text_runs(&app, 0);
    let mut left: Vec<TextRun> = runs.iter().filter(|r| r.rect.left < 300.0).cloned().collect();
    left.sort_by(|a, b| a.rect.top.total_cmp(&b.rect.top));
    let middle = left[left.len() / 2].clone();
    let at = centre(&middle.rect);

    let (outcome, line) = click_logged(&mut app, &log, 0, at);
    assert!(outcome.is_ok(), "{outcome:?}");
    assert_eq!(field(&line, "path"), "block", "{line}");
    assert_eq!(field(&line, "seed"), middle.object.to_string(), "{line}");
    assert_eq!(field(&line, "rule"), "exact", "{line}");
    assert_eq!((field(&line, "lines"), field(&line, "objects"), field(&line, "frozen")), ("8", "8", "[]"), "{line}");
    assert_eq!(field(&line, "cache"), "miss", "the first click on a page reads it: {line}");
    assert!(line.starts_with("page 1 click=("), "{line}");
    assert!(field(&line, "frags").parse::<usize>().unwrap() >= 8, "{line}");
    assert!(field(&line, "total_ms").parse::<f32>().unwrap() >= 0.0, "{line}");
    // Content-free: not one word of what was clicked on.
    for run in &left {
        for word in run.text.split_whitespace().filter(|w| w.chars().count() >= 4) {
            assert!(!line.contains(word), "the log line holds the page's own word {word:?}: {line}");
        }
    }
    let block = field(&line, "block").to_string();

    let (_, again) = click_logged(&mut app, &log, 0, at);
    assert_eq!(field(&again, "cache"), "hit", "nothing changed since the page was read: {again}");
    assert_eq!(field(&again, "block"), block, "{again}");

    // -- a joined group ---------------------------------------------------
    let (a, b) = (
        runs.iter().min_by(|x, y| x.rect.top.total_cmp(&y.rect.top)).unwrap().clone(),
        runs.iter().max_by(|x, y| x.rect.top.total_cmp(&y.rect.top)).unwrap().clone(),
    );
    let total = app.characters(0).expect("characters").len();
    app.tab_mut().selection.text_selection = Some(0..total);
    app.tab_mut().organize.selection_page = 0;
    app.join_selected_text().expect("join");
    app.tab_mut().edit.editing_run = None;
    let (outcome, line) = click_logged(&mut app, &log, 0, centre(&b.rect));
    assert!(outcome.is_ok(), "{outcome:?}");
    assert_eq!(field(&line, "path"), "joined", "{line}");
    assert_eq!(field(&line, "seed"), b.object.to_string(), "{line}");
    let _ = a;
    let _ = std::fs::remove_dir_all(&dir);

    // -- one run alone, and bare paper -------------------------------------
    let (mut single, (dir, log)) = app_with_log("text-lines.pdf", "single");
    let only = text_runs(&single, 0)[0].clone();
    let (outcome, line) = click_logged(&mut single, &log, 0, centre(&only.rect));
    assert!(outcome.expect("picked").starts_with("edit the words on the page"), "{line}");
    assert_eq!(field(&line, "path"), "single", "{line}");
    assert_eq!((field(&line, "lines"), field(&line, "objects")), ("1", "1"), "a block of one object: {line}");

    let (outcome, line) = click_logged(&mut single, &log, 0, AppPoint { x: 3.0, y: 3.0 });
    assert!(outcome.expect_err("bare paper").contains("no text there"), "{line}");
    assert_eq!(field(&line, "path"), "none", "{line}");
    assert_eq!((field(&line, "seed"), field(&line, "block")), ("none", "none"), "{line}");
    let _ = std::fs::remove_dir_all(&dir);

    // -- a word drawn as outlines -------------------------------------------
    let (mut outlined, (dir, log)) = app_with_log("outlined-montserrat.pdf", "drawn");
    let word = outlined
        .drawn_words_on(0)
        .iter()
        .filter(|w| !w.text.trim().is_empty())
        .max_by_key(|w| w.text.trim().chars().count())
        .cloned()
        .expect("a recognised word");
    let (outcome, line) = click_logged(&mut outlined, &log, 0, centre(&word.rect));
    assert!(outcome.expect("picked").contains("drawn, not written"), "{line}");
    assert_eq!(field(&line, "path"), "drawn", "{line}");
    assert_eq!(
        (field(&line, "frags"), field(&line, "blocks"), field(&line, "seed")),
        ("0", "0", "none"),
        "no text objects on this page: {line}"
    );
    let _ = std::fs::remove_dir_all(&dir);

    // -- rotated text, refused (needs the real file) ---------------------------
    if !std::path::Path::new(CAMINO).is_file() {
        eprintln!("skipping the rotated-text case: CAMINO is not on this machine");
        return;
    }
    let mut camino = PagifyApp::new(Some(CAMINO));
    let (dir, log) = log_into(&mut camino, "rotated");
    let pages = camino.tab().doc.as_ref().expect("open").page_count;
    let rotated = (0..pages).find_map(|p| {
        text_runs(&camino, p)
            .into_iter()
            .find(|r| looks_rotated(&r.rect, r.text.trim().chars().count()))
            .map(|r| (p, r))
    });
    let Some((page, run)) = rotated else {
        eprintln!("skipping the rotated-text case: CAMINO has no rotated label");
        return;
    };
    let (outcome, line) = click_logged(&mut camino, &log, page, centre(&run.rect));
    assert!(outcome.expect_err("refused").contains("rotated"), "{line}");
    assert_eq!(field(&line, "path"), "refused-rotated", "{line}");
    let _ = std::fs::remove_dir_all(&dir);
}

fn app_with_log(name: &str, tag: &str) -> (PagifyApp, (std::path::PathBuf, std::path::PathBuf)) {
    let mut app = app(name);
    let log = log_into(&mut app, tag);
    (app, log)
}

// -- a page that moved, and a page that cannot be read --------------------

/// **A page read before an edit, an undo or a redo is never used after
/// it.** The reading is cached per page and stamped with the document's
/// render epoch and the session's undo generation; both are checked, so a
/// page changed behind the app's back — straight through the session, as
/// the tests (and any path that forgets to say so) do — is still read
/// afresh. A stale reading would show the words as they were, and
/// object numbers that no longer mean what they did.
#[test]
fn a_page_read_before_an_edit_or_an_undo_is_not_used_after_it() {
    let (mut app, (dir, log)) = app_with_log("text-lines.pdf", "stale");
    let first = text_runs(&app, 0)[0].clone();
    // Just inside the left end of the run: still inside it after its words change.
    let at = AppPoint { x: (first.rect.left + 3.0) as f64, y: ((first.rect.top + first.rect.bottom) / 2.0) as f64 };
    let session = app.tab().doc.as_ref().expect("open").session.clone();

    let (_, line) = click_logged(&mut app, &log, 0, at);
    assert_eq!(field(&line, "cache"), "miss", "{line}");
    assert_eq!(app.tab().edit.editing_run.as_ref().expect("picked").buffer.trim(), first.text.trim());
    let (_, line) = click_logged(&mut app, &log, 0, at);
    assert_eq!(field(&line, "cache"), "hit", "nothing moved: {line}");

    // Changed behind the app's back: no `rendered_is_stale`, no epoch move.
    session
        .execute(pdf_core::command::Command::SetTextRun {
            page_index: 0,
            object: first.object,
            text: "CHANGED UNDERNEATH THE CACHE".to_string(),
            style: Default::default(),
        })
        .expect("edit through the session");
    let (_, line) = click_logged(&mut app, &log, 0, at);
    assert_eq!(field(&line, "cache"), "miss", "the history moved, so the page is read again: {line}");
    assert_eq!(
        app.tab().edit.editing_run.as_ref().expect("picked").buffer.trim(),
        "CHANGED UNDERNEATH THE CACHE",
        "a stale reading would still offer the old words"
    );

    // And back, the same way.
    let (undone, _) = session.undo().expect("undo");
    assert!(undone);
    let (_, line) = click_logged(&mut app, &log, 0, at);
    assert_eq!(field(&line, "cache"), "miss", "{line}");
    assert_eq!(app.tab().edit.editing_run.as_ref().expect("picked").buffer.trim(), first.text.trim());
    let (_, line) = click_logged(&mut app, &log, 0, at);
    assert_eq!(field(&line, "cache"), "hit", "{line}");

    // Through the app's own paths: an apply, then the app's own undo.
    app.tab_mut().edit.editing_run.as_mut().expect("editing").buffer = "TYPED IN THE BOX".to_string();
    app.apply_editing_page();
    let (_, line) = click_logged(&mut app, &log, 0, at);
    assert_eq!(field(&line, "cache"), "miss", "an apply makes the next click read the page again: {line}");
    assert_eq!(app.tab().edit.editing_run.as_ref().expect("picked").buffer.trim(), "TYPED IN THE BOX");
    app.tab_mut().edit.editing_run = None;
    app.submit("undo");
    let (_, line) = click_logged(&mut app, &log, 0, at);
    assert_eq!(field(&line, "cache"), "miss", "{line}");
    assert_eq!(app.tab().edit.editing_run.as_ref().expect("picked").buffer.trim(), first.text.trim());
    let _ = std::fs::remove_dir_all(&dir);
}

/// The other half of the cache's key, and of its clearing. Splitting a run
/// into its letters is the one page change that does not go through the
/// history (it moves no undo generation); what makes the next click read
/// the page again is [`Doc::rendered_is_stale`], which the app calls right
/// after — the render epoch it moves, and the cache it empties.
#[test]
fn a_page_changed_outside_the_history_is_read_again_once_it_is_declared_stale() {
    let (mut app, (dir, log)) = app_with_log("text-lines.pdf", "split");
    let first = text_runs(&app, 0)[0].clone();
    let at = AppPoint { x: (first.rect.left + 2.0) as f64, y: ((first.rect.top + first.rect.bottom) / 2.0) as f64 };
    let (_, line) = click_logged(&mut app, &log, 0, at);
    assert_eq!((field(&line, "objects"), field(&line, "cache")), ("1", "miss"), "{line}");

    let session = app.tab().doc.as_ref().expect("open").session.clone();
    let generation = session.undo_generation();
    session.split_run_into_characters(0, first.object).expect("split");
    assert_eq!(session.undo_generation(), generation, "setup: a split does not move the history");
    app.tab_mut().doc.as_mut().expect("open").rendered_is_stale();

    let (_, line) = click_logged(&mut app, &log, 0, at);
    assert_eq!(field(&line, "cache"), "miss", "{line}");
    assert!(
        field(&line, "objects").parse::<usize>().unwrap() > 1,
        "the letters of the split run are the page now: {line}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// **Edit Text never stops working.** If the page's text cannot be read in
/// one pass the click falls back to what the tool always did: the one run
/// under the pointer, found from the boxes alone — no paragraph, no
/// detector. The log says it happened.
#[test]
fn edit_text_still_opens_the_run_when_the_pages_text_cannot_be_read_in_one_pass() {
    struct Restore;
    impl Drop for Restore {
        fn drop(&mut self) {
            tests_support::SNAPSHOT_FAILS.with(|fails| fails.set(false));
        }
    }
    let _restore = Restore;

    let (mut app, (dir, log)) = app_with_log("two-column.pdf", "fallback");
    let runs = text_runs(&app, 0);
    let mut left: Vec<TextRun> = runs.iter().filter(|r| r.rect.left < 300.0).cloned().collect();
    left.sort_by(|a, b| a.rect.top.total_cmp(&b.rect.top));
    let middle = left[left.len() / 2].clone();

    tests_support::SNAPSHOT_FAILS.with(|fails| fails.set(true));
    let (outcome, line) = click_logged(&mut app, &log, 0, centre(&middle.rect));
    assert!(outcome.expect("still picked").starts_with("edit the words on the page"), "{line}");
    let edit = app.tab().edit.editing_run.as_ref().expect("an editor opened");
    assert_eq!(edit.lines, vec![(vec![middle.object], middle.rect)], "the run alone, not its paragraph");
    assert_eq!(edit.buffer.trim(), middle.text.trim());
    assert_eq!(field(&line, "path"), "single", "{line}");
    assert_eq!((field(&line, "frags"), field(&line, "blocks"), field(&line, "block")), ("0", "0", "none"), "{line}");
    assert_eq!(field(&line, "seed"), middle.object.to_string(), "{line}");
    let notes = logged(&log, "pick-note");
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].contains("picking the run alone"), "{notes:?}");

    // Bare paper is still bare paper.
    let (outcome, line) = click_logged(&mut app, &log, 0, AppPoint { x: 3.0, y: 3.0 });
    assert!(outcome.expect_err("nothing there").contains("no text there"), "{line}");
    assert_eq!(field(&line, "path"), "none", "{line}");

    // And the moment it can be read again, the paragraph is back.
    tests_support::SNAPSHOT_FAILS.with(|fails| fails.set(false));
    app.tab_mut().edit.editing_run = None;
    let (_, line) = click_logged(&mut app, &log, 0, centre(&middle.rect));
    assert_eq!((field(&line, "path"), field(&line, "lines")), ("block", "8"), "{line}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// **The small picture a paragraph's background colour is sampled from is
/// made once per state of the page, not once per click.** A render is the
/// bulk of what a click costs once the page has been read, and every click
/// on one page wants the same picture. It is kept under the same key as the
/// page's reading — page, render epoch, undo generation — so an edit, an
/// undo or a declared change makes the next click render the page as it is
/// now, and a hide colour is never sampled from a picture of what was.
#[test]
fn the_background_is_sampled_from_one_picture_per_state_of_the_page() {
    let mut app = app("two-column.pdf");
    let kept = |app: &PagifyApp| {
        app.tab().doc.as_ref().expect("open").caches.sampling.borrow().as_ref().map(|(_, picture)| picture.clone())
    };
    let mut left: Vec<TextRun> = text_runs(&app, 0).into_iter().filter(|r| r.rect.left < 300.0).collect();
    left.sort_by(|a, b| a.rect.top.total_cmp(&b.rect.top));
    assert!(kept(&app).is_none(), "nothing is rendered before the first click");

    app.pick_text_run(0, centre(&left[1].rect)).expect("picked");
    let first = kept(&app).expect("a picture was kept");
    app.pick_text_run(0, centre(&left[5].rect)).expect("picked");
    assert!(
        std::rc::Rc::ptr_eq(&first, &kept(&app).expect("still kept")),
        "a second click on the same page made a picture of its own"
    );

    // The page moves: the next click makes a new picture.
    app.tab_mut().edit.editing_run.as_mut().expect("editing").buffer = "ONE\nTWO\nTHREE\nFOUR\nFIVE\nSIX\nSEVEN\nEIGHT".to_string();
    app.apply_editing_page();
    assert!(kept(&app).is_none(), "an apply leaves the old picture behind");
    // At the left end of a line: the words there are short now.
    let at_left = |r: &Rect| AppPoint { x: (r.left + 2.0) as f64, y: ((r.top + r.bottom) / 2.0) as f64 };
    app.pick_text_run(0, at_left(&left[1].rect)).expect("picked");
    let second = kept(&app).expect("a picture was kept");
    assert!(!std::rc::Rc::ptr_eq(&first, &second), "the picture is of the page before the edit");

    // Behind the app's back, as the stale-cache test does: only the key can tell.
    let session = app.tab().doc.as_ref().expect("open").session.clone();
    let (undone, _) = session.undo().expect("undo");
    assert!(undone);
    app.pick_text_run(0, at_left(&left[1].rect)).expect("picked");
    assert!(
        !std::rc::Rc::ptr_eq(&second, &kept(&app).expect("a picture was kept")),
        "an undo made behind the app's back left the old picture in use"
    );
}

/// **A page with no text objects at all is not an error.** Its reading is
/// empty (no shapes either: a page of outlines has tens of thousands, and
/// there is no text to bridge between them), and a click on one of its
/// drawn words still reaches [`PagifyApp::drawn_word_at`] — the only way to
/// edit words that are artwork — while bare paper still says there is no
/// text there.
#[test]
fn a_page_with_no_text_objects_still_reaches_the_drawn_words() {
    let _turn = one_at_a_time();
    let (mut app, (dir, log)) = app_with_log("outlined-montserrat.pdf", "no-text");
    let (page, _) = app.page_blocks(0).expect("an empty page reads fine");
    assert!(
        page.runs.is_empty() && page.frags.is_empty() && page.blocks.is_empty() && page.shapes.is_empty(),
        "setup: this page has no text objects, so nothing is read from it"
    );
    let word = app
        .drawn_words_on(0)
        .iter()
        .filter(|w| !w.text.trim().is_empty())
        .max_by_key(|w| w.text.trim().chars().count())
        .cloned()
        .expect("a recognised word");

    let (outcome, line) = click_logged(&mut app, &log, 0, centre(&word.rect));
    assert!(outcome.expect("a drawn word").contains("drawn, not written"), "{line}");
    assert!(app.tab().edit.editing_run.as_ref().expect("an editor opened").drawn);
    assert_eq!(field(&line, "path"), "drawn", "{line}");

    app.tab_mut().edit.editing_run = None;
    let (outcome, line) = click_logged(&mut app, &log, 0, AppPoint { x: 2.0, y: 2.0 });
    let said = outcome.expect_err("bare paper");
    assert!(said.contains("no text there"), "{said}");
    assert_eq!(field(&line, "path"), "none", "{line}");
    let _ = std::fs::remove_dir_all(&dir);
}

// -- lines drawn as shapes -------------------------------------------------

/// **A paragraph with lines drawn as shapes is cut at them: what opens is the
/// written lines around the click, and no drawn line is ever in the box.**
/// Page 1 of the datasheet has a 19-line, 96-object block whose lines 4, 14
/// and 18 (counting from 0) are drawn as outlines — ligature-heavy body lines
/// the producer converted to paths. It used to open whole, a `[drawn text]`
/// placeholder in the middle of the box standing for each, so one click gave a
/// box that was part editable and part not (reported from use). The detector's
/// block is unchanged — the hand labelings count the drawn words in it — and
/// the cut is made where the editor opens: lines 0-3, 5-13 and 15-17 are
/// three paragraphs, each its own box.
#[test]
fn a_block_with_lines_drawn_as_shapes_opens_as_the_written_lines_around_the_click() {
    if !std::path::Path::new(MARINA).is_file() {
        eprintln!("skipping: the Marina datasheet is not on this machine");
        return;
    }
    let _turn = one_at_a_time();
    let mut app = PagifyApp::new(Some(MARINA));
    assert!(app.tab().doc.is_some(), "the datasheet is on this machine but would not open");
    let (page, _) = app.page_blocks(0).expect("the page's blocks");
    // **Changed for what the detector and the adapter read now, with the reason**:
    // the thin glyphs of the page (a hyphen 2 points wide, an "l" 0.4 point wide)
    // are members of their lines — they used to be left out of the buffer — so
    // the line the page ends in a hyphen after a drawn word (line 14) is an
    // outlined word *and* the hyphen glyph, a text object: the line is frozen
    // but is no longer without an object. Lines 4 and 18 are still drawn with
    // nothing written in them. What the test protects is unchanged: the whole
    // block opens, with a placeholder for each line that has no text.
    let (index, block) = page
        .blocks
        .iter()
        .enumerate()
        .find(|(_, b)| b.lines.len() == 19 && b.lines.iter().filter(|l| !l.outlined.is_empty()).count() == 3)
        .expect("the 19-line block with three lines drawn as shapes is gone from page 1 — has the detector or the file changed?");
    let drawn_lines: Vec<usize> =
        block.lines.iter().enumerate().filter(|(_, l)| l.objects.is_empty()).map(|(i, _)| i).collect();
    let frozen_lines: Vec<usize> =
        block.lines.iter().enumerate().filter(|(_, l)| !l.outlined.is_empty()).map(|(i, _)| i).collect();
    assert_eq!(drawn_lines, [4, 18], "setup: block {index}: the lines with no text object");
    assert_eq!(frozen_lines, [4, 14, 18], "setup: block {index}: the lines with drawn words");

    // The paragraphs, worked out here from the drawn lines alone: the maximal runs of lines between them.
    let written: Vec<std::ops::Range<usize>> = vec![0..4, 5..14, 15..18];
    for range in &written {
        assert!(range.clone().all(|i| !frozen_lines.contains(&i)), "setup: {range:?} has a drawn line in it");
    }

    // Clicked from every one of its words, so every line is tried as the seed.
    let (mut clicked, mut alone) = (0, 0);
    for (line_index, line) in block.lines.iter().enumerate() {
        for &seed in &line.objects {
            app.tab_mut().edit.editing_run = None;
            let said = app.pick_text_run(0, centre(&page.runs[&seed].rect)).expect("picked");
            let edit = app.tab().edit.editing_run.as_ref().expect("an editor opened");
            clicked += 1;
            // A word on a line that is partly drawn opens alone — see
            // `clicking_any_word_of_the_datasheets_blocks_opens_exactly_that_block`.
            if frozen_lines.contains(&line_index) {
                alone += 1;
                assert_eq!(edit.lines.len(), 1, "clicking object {seed} on a drawn line: {said}");
                assert!(said.starts_with("opened this word alone: its line is partly drawn"), "{said}");
                continue;
            }
            // The paragraph is the written lines around the click, and nothing else.
            let own = written.iter().find(|r| r.contains(&line_index)).expect("a written line is in a written run");
            let expected: Vec<Vec<usize>> = own.clone().map(|i| block.lines[i].objects.clone()).collect();
            let opened: Vec<Vec<usize>> = edit.lines.iter().map(|(objects, _)| objects.clone()).collect();
            assert_eq!(opened, expected, "clicking object {seed} on line {line_index}: {said}");
            assert!(edit.frozen.iter().all(|f| !f), "a drawn line is in the editor: clicking object {seed}");
            assert!(
                !edit.buffer.contains(block_input::OUTLINED_PLACEHOLDER),
                "the placeholder is in the box: clicking object {seed}: {:?}",
                edit.buffer
            );
            assert_eq!(edit.buffer.split('\n').count(), own.len(), "one buffer line for every line of the paragraph");
            assert!(said.contains(&format!("editing a paragraph of {} lines", own.len())), "{said}");
            assert!(!said.contains("drawn as shapes"), "the pick still talks about drawn lines it no longer holds: {said}");
        }
    }
    assert_eq!(clicked, 99, "every word of the block was clicked");
    assert_eq!(alone, 1, "the one text object on a drawn line: the hyphen glyph");
}

/// **An edit beside lines drawn as shapes touches only the words retyped.** A
/// four-line block of page 1 — a line of text, two lines drawn entirely as
/// outlines, and a last line of text (`given project.`) — has one text object
/// on each of its text lines. The drawn lines cut the block, so the first
/// line opens as the one run it is; retyped, only it is written, and every
/// other object — and every shape on the page — is exactly as it was.
#[test]
fn retyping_beside_lines_drawn_as_shapes_writes_only_the_retyped_line() {
    if !std::path::Path::new(MARINA).is_file() {
        eprintln!("skipping: the Marina datasheet is not on this machine");
        return;
    }
    let _turn = one_at_a_time();
    let mut app = PagifyApp::new(Some(MARINA));
    assert!(app.tab().doc.is_some(), "the datasheet is on this machine but would not open");
    let (page, _) = app.page_blocks(0).expect("the page's blocks");
    let block = page
        .blocks
        .iter()
        // **Selected by which lines are drawn, not by which have no text object**
        // (changed with the reason): the hyphen glyph the page ends the first drawn
        // line with is a member of its line now, so that line has one text object
        // (a "-", frozen like the rest of the line) where it had none.
        .find(|b| {
            b.lines.len() == 4
                && b.lines[0].objects.len() == 1
                && b.lines[0].outlined.is_empty()
                && !b.lines[1].outlined.is_empty()
                && !b.lines[2].outlined.is_empty()
                && b.lines[2].objects.is_empty()
                && b.lines[3].outlined.is_empty()
                && b.lines[3].objects.len() == 1
        })
        .expect("the text / drawn / drawn / text block is gone from page 1 — has the detector or the file changed?");
    let (first, last) = (block.lines[0].objects[0], block.lines[3].objects[0]);
    let shapes_before = app.tab().doc.as_ref().unwrap().session.drawn_objects(0).expect("shapes").len();

    // **The drawn lines are not in the box at all.** This used to open the four lines whole, the two drawn
    // ones frozen in the middle of the buffer — one of them the hyphen glyph the page ends it with, the
    // other a `[drawn text]` placeholder — and apply had to be taught to leave them alone. A line of text
    // with nothing but drawn lines under it is now what it looks like: one run.
    let said = app.pick_text_run(0, centre(&page.runs[&first].rect)).expect("picked");
    assert!(!said.contains("drawn as shapes"), "{said}");
    let edit = app.tab().edit.editing_run.as_ref().expect("an editor opened").clone();
    assert_eq!(edit.lines.len(), 1, "the drawn lines came along: {said}");
    assert_eq!(edit.frozen, [false]);
    assert!(!edit.buffer.contains(block_input::OUTLINED_PLACEHOLDER), "{:?}", edit.buffer);
    assert!(!edit.buffer.contains('\n'), "{:?}", edit.buffer);

    let words_before: HashMap<usize, (String, pdf_core::document::Color)> =
        text_runs(&app, 0).into_iter().map(|r| (r.object, (r.text, r.color))).collect();
    app.tab_mut().edit.editing_run.as_mut().expect("editing").buffer = format!("{} RETYPED", edit.buffer.trim_end());
    app.apply_editing_page();
    let words_after: HashMap<usize, (String, pdf_core::document::Color)> =
        text_runs(&app, 0).into_iter().map(|r| (r.object, (r.text, r.color))).collect();
    assert_eq!(words_after.len(), words_before.len(), "objects were added or lost");
    assert!(words_after[&first].0.contains("RETYPED"), "the retyped line was not written: {:?}", words_after[&first].0);
    assert_eq!(words_after[&last], words_before[&last], "the line after the drawn ones was touched");
    for (object, was) in &words_before {
        if *object != first {
            assert_eq!(&words_after[object], was, "object {object}, which is not on the retyped line, changed");
        }
    }
    let shapes_after = app.tab().doc.as_ref().unwrap().session.drawn_objects(0).expect("shapes").len();
    assert_eq!(shapes_after, shapes_before, "a shape was added or taken off the page");
}

/// **A guard, run on demand: retype one line in every paragraph of a file,
/// and nothing else on its page may change.**
///
/// For every block of more than one text object on the first
/// `PAGIFY_SWEEP_PAGES` pages (default 1) of `PAGIFY_SWEEP_PDF` (default
/// the datasheet) — in a freshly opened copy of the file each time, so one
/// block's edit cannot show in the next — a word of it is clicked and its
/// first line of plain words is retyped by repeating a word it already has
/// (so no letter the font lacks is needed), then applied. The retyped line
/// must be its first piece holding the new words, its other pieces must be
/// gone from the page, and every other object on the page — every piece of
/// every other line, words, colour, box, origin and size — must be exactly
/// as it was, the page having lost exactly the pieces removed. The engine refusing an edit — safely and whole, for
/// text it cannot map to the operators that draw it — is counted and said,
/// not failed on; an edit that goes through and leaves anything else
/// different is the failure.
///
/// `cargo test -p pagify_app --release apply_sweep -- --ignored --nocapture`
#[test]
#[ignore = "a sweep that takes a minute or more: run with --ignored --nocapture"]
fn apply_sweep() {
    let path = std::env::var("PAGIFY_SWEEP_PDF").unwrap_or_else(|_| MARINA.to_string());
    if !std::path::Path::new(&path).is_file() {
        eprintln!("sweep: skipping, {path} is not on this machine");
        return;
    }
    let pages: usize = std::env::var("PAGIFY_SWEEP_PAGES").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
    let mut app = PagifyApp::new(Some(&path));
    let page_total = app.tab().doc.as_ref().expect("open").page_count;
    let words = |text: &str| text.split_whitespace().find(|w| w.chars().count() > 2 && w.chars().all(char::is_alphabetic)).map(str::to_string);
    let mut tally: std::collections::BTreeMap<String, usize> = Default::default();
    let mut damaged: Vec<String> = Vec::new();
    let mut refused: Vec<String> = Vec::new();
    for page in 0..pages.min(page_total) {
        let (page_blocks, _) = app.page_blocks(page).expect("the page's blocks");
        for (index, block) in page_blocks.blocks.iter().enumerate().filter(|(_, b)| b.objects().len() > 1) {
            app = PagifyApp::new(Some(&path));
            if app.pick_text_run(page, centre(&page_blocks.runs[&block.objects()[0]].rect)).is_err() {
                *tally.entry("the click was refused".into()).or_default() += 1;
                continue;
            }
            let edit = app.tab().edit.editing_run.as_ref().expect("an editor opened").clone();
            let typed: Vec<String> = edit.buffer.split('\n').map(str::to_string).collect();
            let Some(target) = (0..typed.len()).find(|&i| !edit.frozen[i] && !edit.lines[i].0.is_empty() && words(&typed[i]).is_some()) else {
                *tally.entry("no line of plain words to retype".into()).or_default() += 1;
                continue;
            };
            let mut retyped = typed.clone();
            retyped[target] = format!("{} {}", typed[target].trim_end(), words(&typed[target]).expect("a word"));
            let before = tests_support::runs_on_page(&app, page);
            app.tab_mut().edit.editing_run.as_mut().expect("editing").buffer = retyped.join("\n");
            let history = app.cmd.history().len();
            app.apply_editing_page();
            let said: Vec<String> = app.cmd.history().iter().skip(history).map(|e| e.text.clone()).collect();
            if !said.iter().any(|s| s.contains("paragraph changed")) {
                let why: String = said.last().map_or(String::new(), |s| s.chars().take(100).collect());
                *tally.entry(format!("refused by the engine: {why}")).or_default() += 1;
                refused.push(format!("page {} block {index} line {target}: {why}", page + 1));
                continue;
            }
            // The retyped line is its first piece holding the new words,
            // its other pieces are gone from the page, and every other
            // object — every piece of every other line, and everything
            // outside the paragraph — is as it was.
            let after = tests_support::runs_on_page(&app, page);
            let (first, rest) = edit.lines[target].0.split_first().expect("the line has objects");
            match tests_support::page_after_problem(&before, &after, &[(*first, &retyped[target])], rest) {
                None => *tally.entry("ok".into()).or_default() += 1,
                Some(wrong) => {
                    *tally.entry("DAMAGED".into()).or_default() += 1;
                    damaged.push(format!("page {} block {index} line {target}: {wrong}", page + 1));
                }
            }
        }
    }
    for (what, n) in &tally {
        eprintln!("sweep: {n:4} {what}");
    }
    assert!(tally.get("ok").copied().unwrap_or(0) > 0, "the sweep retyped nothing: {tally:?}");
    assert!(damaged.is_empty(), "{} edits changed more than the line retyped:\n{}", damaged.len(), damaged.join("\n"));
    // On the datasheet itself — the file this was built for — not one paragraph may be refused.
    if std::env::var("PAGIFY_SWEEP_PDF").is_err() {
        assert!(refused.is_empty(), "{} paragraphs of the datasheet could not be retyped:\n{}", refused.len(), refused.join("\n"));
    }
}

// -- what a click costs ------------------------------------------------------

/// **A measurement, not a check: what a click costs.** Per page of the
/// datasheet and of CAMINO: arming Edit Text, the first click on the page
/// (which reads it), the clicks after it (which do not), and one render of
/// the page at the sampling scale that every paragraph pick pays for. Run
/// it alone — `cargo test -p pagify_app --release pick_timings -- --ignored
/// --nocapture` — since the PDFium lock is one for the whole process and
/// any other test running beside it shows up as a wait.
#[test]
#[ignore = "a measurement: run alone with --ignored --nocapture"]
fn pick_timings() {
    for (name, path) in [("datasheet", MARINA), ("CAMINO", CAMINO)] {
        if !std::path::Path::new(path).is_file() {
            eprintln!("timing: skipping the {name}: it is not on this machine");
            continue;
        }
        let mut app = PagifyApp::new(Some(path));
        let (dir, log) = log_into(&mut app, "timings");
        let started = std::time::Instant::now();
        app.submit("edittext");
        eprintln!("timing: {name}: arming Edit Text took {:?}", started.elapsed());
        let pages = app.tab().doc.as_ref().expect("open").page_count.min(3);
        let mut after = 0u64;
        for page in 0..pages {
            let runs = text_runs(&app, page);
            // A spread of paragraphs: every 25th run that has words in it.
            let targets: Vec<&TextRun> =
                runs.iter().filter(|r| r.text.trim().chars().count() > 4).step_by(25).take(24).collect();
            let mut later: Vec<std::time::Duration> = Vec::new();
            for (i, run) in targets.iter().enumerate() {
                app.tab_mut().edit.editing_run = None;
                let started = std::time::Instant::now();
                let _ = app.pick_text_run(page, centre(&run.rect));
                let took = started.elapsed();
                let line = newest_pick_line(&log, &mut after);
                if i == 0 {
                    eprintln!("timing: {name} page {}: first click, which reads the page: {took:?}  [{line}]", page + 1);
                } else {
                    later.push(took);
                }
            }
            later.sort();
            if !later.is_empty() {
                eprintln!(
                    "timing: {name} page {}: {} clicks after it: median {:?}, slowest {:?}",
                    page + 1,
                    later.len(),
                    later[later.len() / 2],
                    later[later.len() - 1]
                );
            }
            let started = std::time::Instant::now();
            let _ = app.tab().doc.as_ref().expect("open").session.render_page(page, PagifyApp::BACKGROUND_SAMPLE_SCALE);
            eprintln!("timing: {name} page {}: one render at the sampling scale {:?}", page + 1, started.elapsed());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// **A measurement, not a check: what an apply costs on the user's
/// paragraph.** The 13-line paragraph of page 1 (57 objects) — one line
/// retyped, then every line retyped (each reversed, so no letter the font
/// lacks is needed, as the budget tests do), then both taken back with
/// undo. Run alone: `cargo test -p pagify_app --release apply_timings --
/// --ignored --nocapture`.
#[test]
#[ignore = "a measurement: run alone with --ignored --nocapture"]
fn apply_timings() {
    if !std::path::Path::new(MARINA).is_file() {
        eprintln!("timing: skipping: the Marina datasheet is not on this machine");
        return;
    }
    let mut app = PagifyApp::new(Some(MARINA));
    let runs: HashMap<usize, TextRun> = text_runs(&app, 0).into_iter().map(|r| (r.object, r)).collect();
    let session = app.tab().doc.as_ref().expect("open").session.clone();
    for (what, every_line) in [("one line", false), ("every line", true)] {
        app.tab_mut().edit.editing_run = None;
        app.pick_text_run(0, centre(&runs[&986].rect)).expect("picked");
        let edit = app.tab().edit.editing_run.as_ref().expect("an editor opened").clone();
        let objects: usize = edit.lines.iter().map(|(o, _)| o.len()).sum();
        let typed: Vec<String> = edit
            .buffer
            .split('\n')
            .enumerate()
            .map(|(i, line)| {
                if (every_line && !edit.frozen[i]) || i == 3 {
                    line.chars().rev().collect::<String>()
                } else {
                    line.to_string()
                }
            })
            .collect();
        app.tab_mut().edit.editing_run.as_mut().expect("editing").buffer = typed.join("\n");
        let started = std::time::Instant::now();
        app.apply_editing_page();
        let applied = started.elapsed();
        let said = app.cmd.history().iter().last().map(|e| e.text.clone()).unwrap_or_default();
        let started = std::time::Instant::now();
        let (undone, _) = session.undo().expect("undo");
        eprintln!(
            "timing: retyping {what} of a 13-line, {objects}-object paragraph: apply {applied:?} ({said}); undo {:?} (undone: {undone})",
            started.elapsed()
        );
        app.tab_mut().doc.as_mut().expect("open").rendered_is_stale();
    }
}

// -- the census ------------------------------------------------------------

/// What a click wrote to the log after byte `after`; moves `after` on.
fn newest_pick_line(log: &std::path::Path, after: &mut u64) -> String {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(log).expect("the log file exists");
    file.seek(SeekFrom::Start(*after)).expect("seek");
    let mut tail = String::new();
    file.read_to_string(&mut tail).expect("read");
    *after += tail.len() as u64;
    tail.lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).expect("valid json"))
        .filter(|l| l["kind"] == "pick")
        .map(|l| l["text"].as_str().unwrap_or_default().to_string())
        .last()
        .expect("the click wrote no pick line")
}

/// **A measurement, not a check: what clicking every word of a file opens.**
///
/// Run on demand against any document —
/// `PAGIFY_CENSUS_PDF=<file> PAGIFY_CENSUS_OUT=<file>.jsonl cargo test -p
/// pagify_app --release paragraph_census -- --ignored --nocapture` — it
/// clicks the middle of **every text object with area of every page** and
/// writes one JSON line per click: the page (1-based), the object clicked
/// and its words, the object the click actually resolved to (the smallest
/// box under that point can be another), the way the click went
/// (`block`, `single`, `joined`, `drawn`, `refused-rotated`, `none`),
/// whether it opened, the message, the editor's lines as arrays of object
/// ids with their frozen flags, the buffer, the object the box takes its
/// look from, the time the click took, and the log line it wrote. Scoring
/// the lines against hand-made labels is a separate job; this only reads
/// the file the way a person with a mouse would.
#[test]
#[ignore = "a measurement: set PAGIFY_CENSUS_PDF and PAGIFY_CENSUS_OUT, then run with --ignored"]
fn paragraph_census() {
    use std::io::Write;
    let pdf = std::env::var("PAGIFY_CENSUS_PDF").expect("PAGIFY_CENSUS_PDF: the file to click through");
    let out = std::env::var("PAGIFY_CENSUS_OUT").expect("PAGIFY_CENSUS_OUT: where to write the lines");
    let mut app = PagifyApp::new(Some(&pdf));
    assert!(app.tab().doc.is_some(), "{pdf} did not open");
    let (dir, log) = log_into(&mut app, "census");
    let mut sink = std::io::BufWriter::new(std::fs::File::create(&out).expect("PAGIFY_CENSUS_OUT is writable"));
    // All the pages, unless PAGIFY_CENSUS_PAGES names how many to do (from the first).
    let pages = app.tab().doc.as_ref().expect("open").page_count;
    let pages = std::env::var("PAGIFY_CENSUS_PAGES").ok().and_then(|n| n.parse().ok()).map_or(pages, |n: usize| pages.min(n));
    let mut after = 0u64;
    let mut total = 0usize;
    for page in 0..pages {
        // The seeds come from a reading made on the side, so the first click
        // on each page still pays for (and logs) the app's own: the cache
        // is not warmed for it.
        let snapshot = match app.tab().doc.as_ref().expect("open").session.page_text_snapshot(page) {
            Ok(snapshot) => snapshot,
            Err(why) => {
                eprintln!("page {} could not be read: {why}", page + 1);
                continue;
            }
        };
        let page_blocks = block_input::build_page_blocks(page, 0, 0, snapshot);
        let mut seeds: Vec<usize> = page_blocks.runs.keys().copied().collect();
        seeds.sort_unstable();
        for seed in seeds {
            let run = &page_blocks.runs[&seed];
            let at = centre(&run.rect);
            let resolved = block_input::pick_seed(&page_blocks, at.x as f32, at.y as f32, HIT_TOLERANCE_PT as f32)
                .map(|(object, _)| object);
            app.tab_mut().edit.editing_run = None;
            let started = std::time::Instant::now();
            let outcome = app.pick_text_run(page, at);
            let pick_ms = started.elapsed().as_secs_f32() * 1000.0;
            let line = newest_pick_line(&log, &mut after);
            let edit = app.tab().edit.editing_run.as_ref();
            let record = serde_json::json!({
                "page": page + 1,
                "seed": seed,
                "text": run.text,
                "resolved_seed": resolved,
                "path": field(&line, "path"),
                "ok": outcome.is_ok(),
                "message": outcome.as_ref().unwrap_or_else(|e| e),
                "lines": edit.map(|e| e.lines.iter().map(|(objects, _)| objects.clone()).collect::<Vec<_>>()),
                "frozen": edit.map(|e| e.frozen.clone()),
                "buffer": edit.map(|e| e.buffer.clone()),
                "look_object": edit.map(|e| e.look_object),
                "pick_ms": pick_ms,
                "log": line,
            });
            writeln!(sink, "{record}").expect("write a census line");
            total += 1;
        }
        eprintln!("census: page {} done ({total} clicks so far)", page + 1);
    }
    sink.flush().expect("flush");
    // Every block the editor's guard refused, and every page that could not
    // be read in one pass, said in words (content-free).
    let notes = logged(&log, "pick-note");
    for note in notes.iter().take(20) {
        eprintln!("census: note: {note}");
    }
    eprintln!("census: {} notes (blocks refused by the guard, pages not readable in one pass)", notes.len());
    let _ = std::fs::remove_dir_all(&dir);
    eprintln!("census: {total} clicks over {pages} pages, written to {out}");
}

/// **Reported from use: a pasted copy of a word wasn't the same word when
/// read back** — specifically, a letter narrow enough in this font (a
/// capital "I", 0.41pt wide at 8pt) silently vanished from
/// `session.text_runs()` after being pasted, though it was drawn correctly.
/// Root cause: `write_text` gives a paste one PDF text object per glyph, and
/// `text_runs()`'s own "nothing to have clicked on" filter dropped any
/// object under 0.5pt wide or tall — meant for genuinely empty objects, but
/// a real letter that thin got caught by the same net. Fixed by lowering the
/// filter to `HAS_AREA_PT` (`pdf_core::document::pdfium_doc`), which still
/// excludes zero-area objects without excluding real ink.
#[test]
fn a_pasted_words_own_narrow_letters_read_back_the_same_as_the_original() {
    if !std::path::Path::new(MARINA).is_file() {
        eprintln!("skipping: the Marina datasheet is not on this machine");
        return;
    }
    let mut app = PagifyApp::new(Some(MARINA));
    app.submit("editobject");
    app.tab_mut().page = 1;
    let before: std::collections::HashSet<usize> = text_runs(&app, 1).iter().map(|r| r.object).collect();
    let src = text_runs(&app, 1).into_iter().find(|r| r.object == 2844).expect("the Power Input: label");
    assert_eq!(src.text, "Power Input:", "setup: the fixture's own label changed under this test");

    app.tab_mut().selected = Some(Selected { page: 1, object: src.object, rect: src.rect, what: "the words" });
    assert!(app.copy_object_selection(), "copy should succeed");
    assert!(app.start_paste_ghost(None), "should have a paste in hand");
    // A blank margin, away from any other text on the page, so nothing else
    // could be mistaken for part of the pasted word.
    app.place_paste_ghost(1, AppPoint { x: 400.0, y: 15.0 });

    let after = text_runs(&app, 1);
    let mut new_runs: Vec<TextRun> = after.into_iter().filter(|r| !before.contains(&r.object)).collect();
    new_runs.sort_by(|a, b| a.rect.left.total_cmp(&b.rect.left));
    let read_back: String = new_runs.iter().map(|r| r.text.as_str()).collect();
    assert_eq!(
        read_back.replace(' ', ""),
        src.text.replace(' ', ""),
        "the pasted word did not read back the same letters it was copied with: {read_back:?} vs {:?}",
        src.text
    );
}

/// **Reported from use: a pasted word looked "scrambled," letters raised
/// above their neighbours, even alone on a blank page with nothing to
/// overlap.** PDFium splits "Description:" into five adjacent word-runs
/// ("D", "es", "cr", "ipti", "on:" — see `content_for_selected`'s own "the
/// words" branch) that all share one baseline in the original. The group
/// branch's vertical anchor used to be `rect.top`, and a box's top sits
/// above its baseline by that run's own ascent — not the same for every
/// fragment of one word: "ipti" (an ascender-letter run: i, t) has a
/// noticeably higher box top than "es" or "on:" (x-height only), even
/// though all five sit on the same line in the source. Anchoring on
/// `rect.top` took that ink-height difference as a real vertical offset
/// between fragments, raising "ipt" above its neighbours in the pasted
/// copy. Fixed by anchoring on each run's own baseline (`origin.y`)
/// instead, which — unlike `rect.top` — is provably the same for every
/// fragment of one shared line.
#[test]
fn a_split_words_own_fragments_share_one_baseline_not_each_fragments_own_ascent() {
    if !std::path::Path::new(MARINA).is_file() {
        eprintln!("skipping: the Marina datasheet is not on this machine");
        return;
    }
    let mut app = PagifyApp::new(Some(MARINA));
    app.submit("editobject");
    let runs = text_runs(&app, 0);
    let by_obj: std::collections::HashMap<usize, TextRun> = runs.iter().map(|r| (r.object, r.clone())).collect();
    // "D", "es", "cr", "ipti", "on:" — confirmed by inspection to share one
    // baseline (`origin.y`) while their `rect.top` genuinely differs, since
    // "ipti" alone carries ascender letters the other fragments do not.
    let objs = [3usize, 4, 5, 6, 7];
    let tops: Vec<f32> = objs.iter().map(|&o| by_obj[&o].rect.top).collect();
    assert!(
        tops.iter().any(|&t| (t - tops[0]).abs() > 1.0),
        "setup: these fragments' own box tops should genuinely differ (ascender letters vs. x-height only): {tops:?}"
    );
    let origins: Vec<f32> = objs.iter().map(|&o| by_obj[&o].origin.y).collect();
    assert!(
        origins.iter().all(|&y| (y - origins[0]).abs() < 0.01),
        "setup: these fragments should share one real baseline: {origins:?}"
    );

    app.tab_mut().group = objs
        .iter()
        .map(|&o| {
            let r = &by_obj[&o];
            Selected { page: 0, object: o, rect: r.rect, what: "the words" }
        })
        .collect();
    assert!(app.copy_object_selection());
    let Some(ObjectClipboard::Group(items)) = app.object_clipboard.clone() else {
        panic!("expected a Group on the clipboard");
    };
    let dys: Vec<f32> = items.iter().map(|(_, _, dy)| *dy).collect();
    assert!(
        dys.iter().all(|&dy| (dy - dys[0]).abs() < 0.01),
        "every fragment of one split word must share the same vertical offset \
         (one baseline), not each fragment's own box top: {dys:?}"
    );
}

/// **Reported from use: a pasted word came back visibly *wider* than the
/// original, letters spaced apart that sat close together in the source —
/// seen even alone on a blank page, with nothing to overlap and the
/// baseline bug above already fixed.** Measured directly: this font's own
/// GPOS carries no kerning at all and the run's text matrix shows no `Tz`
/// either, yet reshaping this exact fragment in its own font at its own
/// size consistently came out noticeably wider than the rect the source
/// actually drew — bespoke, per-instance positioning baked into the page's
/// own content stream, which no font file can carry back out through a
/// fresh shape. Fixed with a measured correction
/// (`ObjectClipboard::Text::track`) applied to every glyph's advance.
///
/// **A single fragment, not the whole group**, because the group's own
/// fragment-to-fragment positions are pinned to each fragment's *original*
/// rect (the baseline-anchor fix above) independent of `track` — pasting
/// the whole word can still measure close to the original's overall width
/// even without this fix, since neighbouring fragments' pinned positions
/// absorb some of the stretch. "ipti" alone has nothing to absorb it into.
#[test]
fn a_pasted_fragments_own_width_matches_the_original_not_a_bare_reshape() {
    if !std::path::Path::new(MARINA).is_file() {
        eprintln!("skipping: the Marina datasheet is not on this machine");
        return;
    }
    let mut app = PagifyApp::new(Some(MARINA));
    app.submit("editobject");
    let runs = text_runs(&app, 0);
    // "ipti" — four characters, no kerning in this font's GPOS, no `Tz`,
    // and measured at 12% wider than its own rect from a bare reshape.
    let run = runs.iter().find(|r| r.object == 6).cloned().expect("the ipti fragment");
    let original_width = run.rect.right - run.rect.left;

    app.tab_mut().selected = Some(Selected { page: 0, object: run.object, rect: run.rect, what: "the words" });
    assert!(app.copy_object_selection());
    assert!(app.start_paste_ghost(None));
    app.submit("insertpage");
    let before: std::collections::HashSet<usize> = text_runs(&app, 0).iter().map(|r| r.object).collect();
    app.place_paste_ghost(0, AppPoint { x: 150.0, y: 150.0 });

    let after = text_runs(&app, 0);
    let new_runs: Vec<&TextRun> = after.iter().filter(|r| !before.contains(&r.object)).collect();
    assert!(!new_runs.is_empty(), "nothing was pasted");
    let pasted_width = new_runs.iter().map(|r| r.rect.right).fold(f32::MIN, f32::max)
        - new_runs.iter().map(|r| r.rect.left).fold(f32::MAX, f32::min);

    // A bare reshape (no `track`) measured about 12% too wide on this exact
    // fragment; the fix should land within a fraction of a point.
    assert!(
        (pasted_width - original_width).abs() < 1.0,
        "pasted width {pasted_width:.2} does not match the original {original_width:.2} \
         (ratio {:.4}) — the track correction is not being applied",
        pasted_width / original_width
    );
}

/// **Reported from use: deleting a dense selection froze the app for
/// minutes, and "select all" on the same page looked like a crash.**
/// `delete_group` used to call `Command::RemoveObject` once per member, and
/// each one of those re-saves and re-parses the *whole* document just to
/// locate and splice out one object — about 400ms on this real 49-page
/// file, measured directly. A few hundred objects at that rate is minutes,
/// which is what reached the user as a freeze and then a force-close.
/// `Command::RemoveObjects` pays the save/parse cost once for the whole
/// batch, so removing 40 objects through `delete_group` should cost close
/// to what removing *one* object costs, not forty times that.
#[test]
fn deleting_a_dense_selection_does_not_cost_once_per_object() {
    if !std::path::Path::new(DATASHEET).is_file() {
        eprintln!("skipping: the technical datasheet is not on this machine");
        return;
    }
    let mut app = PagifyApp::new(Some(DATASHEET));
    app.submit("editobject");

    let page = 5; // page 6, the dense specs-table page reported from use.
    let drawn = {
        let doc = app.tab_mut().doc.as_ref().expect("open");
        doc.session.drawn_objects(page).expect("objects")
    };
    assert!(drawn.len() >= 40, "expected the dense page to still have plenty of objects: {}", drawn.len());

    // Time removing ONE object the old, single-object way, for comparison.
    let one = drawn[0].object;
    let t0 = std::time::Instant::now();
    {
        let doc = app.tab_mut().doc.as_ref().expect("open");
        doc.session
            .execute(pdf_core::command::Command::RemoveObject { page_index: page, object: one })
            .expect("single remove");
    }
    let single_ms = t0.elapsed().as_secs_f64() * 1000.0;
    {
        let doc = app.tab_mut().doc.as_ref().expect("open");
        let (undone, _) = doc.session.undo().expect("undo");
        assert!(undone, "the single removal should be undoable");
    }

    // Now batch-delete 40 objects through the real `delete_group` path.
    let batch: Vec<&pdf_core::document::DrawnObject> = drawn.iter().skip(1).take(40).collect();
    app.tab_mut().group =
        batch.iter().map(|d| Selected { page, object: d.object, rect: d.rect, what: "the thing" }).collect();
    let batch_len = batch.len();

    let before = {
        let doc = app.tab_mut().doc.as_ref().expect("open");
        doc.session.drawn_objects(page).expect("objects").len()
    };
    let t1 = std::time::Instant::now();
    app.delete_group();
    let batch_ms = t1.elapsed().as_secs_f64() * 1000.0;
    let after = {
        let doc = app.tab_mut().doc.as_ref().expect("open");
        doc.session.drawn_objects(page).expect("objects").len()
    };

    eprintln!("timing: single remove {single_ms:.1}ms, batch of {batch_len} {batch_ms:.1}ms");
    assert_eq!(before - after, batch_len, "every member of the batch should have been removed, in one pass");
    assert!(
        batch_ms < single_ms * 5.0 + 500.0,
        "batching {batch_len} objects took {batch_ms:.1}ms against a single object's {single_ms:.1}ms — \
         this should cost roughly one save+parse, not one per object",
    );
}

fn said(app: &PagifyApp) -> String {
    app.cmd.history().iter().map(|e| e.text.as_str()).collect::<Vec<_>>().join("\n")
}

/// How many dark pixels each of `slices` equal-width columns of `rect` holds,
/// rendered at 8x. A glyph with no outline leaves its column blank.
fn ink_per_column(app: &PagifyApp, rect: Rect, slices: usize) -> Vec<usize> {
    let raster = app
        .tab()
        .doc
        .as_ref()
        .expect("open")
        .session
        .render_page_region(0, Rect { left: rect.left, top: rect.top, right: rect.right, bottom: rect.bottom }, 8.0)
        .expect("render");
    let (w, h) = (raster.width as usize, raster.height as usize);
    (0..slices)
        .map(|slice| {
            let (from, to) = (slice * w / slices, (slice + 1) * w / slices);
            (0..h)
                .flat_map(|y| (from..to).map(move |x| (x, y)))
                .filter(|&(x, y)| raster.pixels[(y * w + x) * 4] < 140)
                .count()
        })
        .collect()
}

/// **Reported from use: retyping the `600` of a `600mm` dimension to `500`
/// drew an empty box where the `5` should be.** The label's font is a subset
/// of `space zero six m` (and a few more letters) that still *declares* every
/// digit, so the edit wrote code `0x35` into it and nothing in the way said
/// that the font had no outline for it. It is now written in a font on the
/// page that has — and says so — and **every glyph has ink**: the first fix
/// that borrowed a font on the page picked one with the digits' codes and no
/// digits, and the number vanished.
#[test]
fn retyping_a_digit_the_labels_font_never_drew_is_written_in_a_font_that_can_draw_it() {
    if !std::path::Path::new(RING600).is_file() {
        eprintln!("skipping: the RING 600 datasheet is not on this machine");
        return;
    }
    let mut app = PagifyApp::new(Some(RING600));
    app.pick_text_run(0, AppPoint { x: 440.0, y: 168.5 }).expect("the dimension is clickable");
    app.tab_mut().edit.editing_run.as_mut().expect("editing").buffer = "500mm".to_string();
    assert!(!app.apply_editing_page(), "the edit was refused: {}", said(&app));

    let said = said(&app);
    assert!(said.contains("Written in"), "the swap was not announced: {said}");
    assert!(!said.contains("MyriadPro"), "it claims the label's own font drew a 5: {said}");

    let runs = text_runs(&app, 0);
    let label = runs.iter().find(|r| r.text.trim() == "500mm").expect("the new words are on the page");
    let ink = ink_per_column(&app, label.rect, 5);
    assert!(ink.iter().all(|&n| n > 15), "a glyph has no outline — per-column ink {ink:?}");
}

/// Words made only of what the label's font did draw stay in it, untouched.
#[test]
fn retyping_the_dimension_with_letters_its_font_kept_changes_no_font() {
    if !std::path::Path::new(RING600).is_file() {
        eprintln!("skipping: the RING 600 datasheet is not on this machine");
        return;
    }
    let mut app = PagifyApp::new(Some(RING600));
    app.pick_text_run(0, AppPoint { x: 440.0, y: 168.5 }).expect("the dimension is clickable");
    app.tab_mut().edit.editing_run.as_mut().expect("editing").buffer = "660mm".to_string();
    assert!(!app.apply_editing_page(), "the edit was refused: {}", said(&app));
    assert!(!said(&app).contains("Written in"), "a font was swapped for words it could draw: {}", said(&app));
}

/// **Why a paste "worked" where a retype did not.** The label's font is a bare
/// CFF program, which the writing registry cannot read, so a paste silently
/// fell back to Helvetica — a font with every digit in it, and not the
/// label's. It is now said at the copy.
#[test]
fn copying_words_whose_font_cannot_be_reused_says_what_a_paste_will_use() {
    if !std::path::Path::new(RING600).is_file() {
        eprintln!("skipping: the RING 600 datasheet is not on this machine");
        return;
    }
    let mut app = PagifyApp::new(Some(RING600));
    app.submit("editobject");
    let runs = text_runs(&app, 0);
    let label = runs
        .iter()
        .find(|r| r.text == "6" && r.rect.left > 436.0 && r.rect.left < 440.0 && r.rect.top > 160.0 && r.rect.top < 170.0)
        .expect("the dimension's first digit");
    app.tab_mut().selected = Some(Selected { page: 0, object: label.object, rect: label.rect, what: "the words" });
    assert!(app.copy_object_selection());
    assert!(said(&app).contains("Helvetica"), "{}", said(&app));
}

/// **Reported from use, on RING 600: a paragraph with a drawn line in it opened with
/// `[drawn text]` in the middle of the box.** Its third line, "in components like SMD,
/// driver, reflector etc.,", is outlines, so the four-line paragraph is now two: the two
/// written lines above it, and the one below. Neither editor holds the drawn line, and the
/// log says which lines opened and which were left out.
#[test]
fn a_paragraph_with_a_drawn_line_in_it_opens_as_the_two_paragraphs_around_it() {
    if !std::path::Path::new(RING600).is_file() {
        eprintln!("skipping: the RING 600 datasheet is not on this machine");
        return;
    }
    let mut app = PagifyApp::new(Some(RING600));
    let (page, _) = app.page_blocks(0).expect("the page's blocks");
    let block = page
        .blocks
        .iter()
        .find(|b| b.lines.len() == 4 && b.lines[2].objects.is_empty() && !b.lines[2].outlined.is_empty())
        .expect("the four-line paragraph with a drawn third line is gone from page 1 — has the file changed?");

    // Above the drawn line.
    let said = app.pick_text_run(0, centre(&page.runs[&block.lines[0].objects[0]].rect)).expect("picked");
    let edit = app.tab().edit.editing_run.as_ref().expect("an editor opened");
    assert_eq!(edit.lines.len(), 2, "the paragraph above the drawn line is its first two lines: {said}");
    assert!(!edit.buffer.contains(block_input::OUTLINED_PLACEHOLDER), "{:?}", edit.buffer);
    assert!(edit.frozen.iter().all(|f| !f), "{said}");
    assert!(!said.contains("drawn as shapes"), "{said}");

    // Below it.
    app.tab_mut().edit.editing_run = None;
    app.pick_text_run(0, centre(&page.runs[&block.lines[3].objects[0]].rect)).expect("picked");
    let edit = app.tab().edit.editing_run.as_ref().expect("an editor opened");
    assert!(!edit.buffer.contains(block_input::OUTLINED_PLACEHOLDER), "{:?}", edit.buffer);
    assert!(
        edit.lines.iter().flat_map(|(objects, _)| objects).all(|o| block.lines[3].objects.contains(o)),
        "words of the paragraph above came along: {:?}",
        edit.lines
    );
}

/// **The boxes over the page are the paragraphs a click opens.** On RING 600 the
/// four-line paragraph with a drawn third line is boxed twice — above the drawn line and
/// below it — with the drawn line dashed between them, and the box the pointer is over is
/// the one lit. Nothing is boxed unless Edit Text is in hand, and nothing is lit while a
/// box is open.
#[test]
fn edit_text_boxes_the_paragraphs_around_a_drawn_line_and_lights_the_one_under_the_pointer() {
    use crate::canvas::BoxStyle;
    if !std::path::Path::new(RING600).is_file() {
        eprintln!("skipping: the RING 600 datasheet is not on this machine");
        return;
    }
    let mut app = PagifyApp::new(Some(RING600));
    let (page, _) = app.page_blocks(0).expect("the page's blocks");
    let block = page
        .blocks
        .iter()
        .find(|b| b.lines.len() == 4 && b.lines[2].objects.is_empty() && !b.lines[2].outlined.is_empty())
        .expect("the four-line paragraph with a drawn third line is gone from page 1");
    let view = crate::overlay::PageView { origin: egui::Pos2::new(0.0, 0.0), scale: 1.0 };
    let screen = |at: AppPoint| view.to_screen(at);
    let upper = centre(&page.runs[&block.lines[0].objects[0]].rect);
    let lower = centre(&page.runs[&block.lines[3].objects[0]].rect);

    // Not boxed until Edit Text is in hand.
    assert!(app.paragraph_boxes(0, view, screen(upper)).is_empty(), "boxes with no tool armed");

    app.submit("edittext");
    let boxes = app.paragraph_boxes(0, view, screen(upper));
    let drawn: Vec<_> = boxes.iter().filter(|(_, s)| *s == BoxStyle::Drawn).collect();
    let lit: Vec<_> = boxes.iter().filter(|(_, s)| *s == BoxStyle::Lit).collect();
    assert!(!drawn.is_empty(), "the drawn line is not marked: {boxes:?}");
    assert_eq!(lit.len(), 1, "exactly one box is under the pointer: {lit:?}");
    // The lit box is the paragraph above the drawn line: it holds the pointer and stops short of the drawn one.
    let (lit_box, _) = lit[0];
    assert!(lit_box.contains(screen(upper)), "the lit box is not over the pointer");
    assert!(!lit_box.contains(screen(lower)), "the lit box runs on past the drawn line: {lit_box:?}");
    // The drawn box starts where the lit one ends (each is padded by 2pt, so they may overlap by that).
    assert!(
        drawn.iter().any(|(r, _)| (r.top() - lit_box.bottom()).abs() < 8.0),
        "the drawn box does not follow the lit one: {drawn:?} / {lit_box:?}"
    );

    // The paragraph below the drawn line lights on its own.
    let boxes = app.paragraph_boxes(0, view, screen(lower));
    let lit_below: Vec<_> = boxes.iter().filter(|(_, s)| *s == BoxStyle::Lit).collect();
    assert_eq!(lit_below.len(), 1);
    assert!(lit_below[0].0.contains(screen(lower)) && !lit_below[0].0.contains(screen(upper)));

    // Over the drawn line itself nothing is lit: it is not a paragraph.
    let gap = AppPoint { x: ((block.lines[2].left + block.lines[2].right) / 2.0) as f64, y: ((block.lines[2].top + block.lines[2].bottom) / 2.0) as f64 };
    assert!(
        app.paragraph_boxes(0, view, screen(gap)).iter().all(|(r, s)| *s != BoxStyle::Lit || !r.contains(screen(gap))),
        "the drawn line was lit as if it could be opened"
    );

    // With a box open, the rest stay outlined and nothing is lit.
    app.pick_text_run(0, upper).expect("picked");
    let boxes = app.paragraph_boxes(0, view, screen(lower));
    assert!(!boxes.is_empty() && boxes.iter().all(|(_, s)| *s != BoxStyle::Lit), "a box was lit while the editor was open");
}


/// The page rendered at 1x, as RGBA bytes.
fn page_pixels(app: &PagifyApp) -> Vec<u8> {
    app.tab().doc.as_ref().expect("open").session.render_page(0, 1.0).expect("render").pixels
}

fn pixels_that_differ(a: &[u8], b: &[u8]) -> usize {
    assert_eq!(a.len(), b.len(), "the page changed size");
    a.chunks(4).zip(b.chunks(4)).filter(|(x, y)| x != y).count()
}

/// **Reported from use, on RING 600: replacing one drawn word destroyed the page.**
/// The word is part of one path that draws a whole line, so the replacement takes that
/// line off — and it used to do it with `Command::Redact`, which rebuilds the page's whole
/// content stream and, on a page from Illustrator, loses most of it: the picture, the left
/// column and most of the text gone, what was left shifted. Measured: a quarter of the
/// page's pixels changed and 3,031 text runs became 2,985. The area held one path.
///
/// What may change now is the drawn line and the words put in its place — a sliver of the
/// page — and every text run that was there still is.
#[test]
fn replacing_a_drawn_word_takes_off_its_shape_and_nothing_else() {
    if !std::path::Path::new(RING600).is_file() {
        eprintln!("skipping: the RING 600 datasheet is not on this machine");
        return;
    }
    let mut app = PagifyApp::new(Some(RING600));
    app.submit("edittext");
    let words_before = text_runs(&app, 0).len();
    let before = page_pixels(&app);

    app.pick_text_run(0, AppPoint { x: 297.8, y: 93.8 }).expect("a drawn word was not picked");
    assert!(app.tab().edit.editing_run.as_ref().is_some_and(|e| e.drawn), "setup: the click did not pick a drawn word");
    app.tab_mut().edit.editing_run.as_mut().expect("editing").buffer = "S".to_string();
    app.apply_editing_page();
    let told = said(&app);
    assert!(told.contains("replaced the drawn word"), "{told}");

    let words_after = text_runs(&app, 0).len();
    assert_eq!(words_after, words_before + 1, "text was lost or gained besides the new words: {words_before} -> {words_after}");
    let after = page_pixels(&app);
    let changed = pixels_that_differ(&before, &after);
    assert!(
        changed * 100 < before.len() / 4,
        "{changed} of {} pixels changed — more than the one drawn line and its replacement",
        before.len() / 4
    );
}

/// "…so `undo` twice puts the artwork back" — and it is the artwork as it was, not near it.
#[test]
fn undoing_a_drawn_word_replacement_twice_restores_the_page_exactly() {
    if !std::path::Path::new(RING600).is_file() {
        eprintln!("skipping: the RING 600 datasheet is not on this machine");
        return;
    }
    let mut app = PagifyApp::new(Some(RING600));
    app.submit("edittext");
    let before = page_pixels(&app);
    app.pick_text_run(0, AppPoint { x: 297.8, y: 93.8 }).expect("a drawn word was not picked");
    app.tab_mut().edit.editing_run.as_mut().expect("editing").buffer = "S".to_string();
    app.apply_editing_page();
    assert_ne!(pixels_that_differ(&before, &page_pixels(&app)), 0, "setup: the replacement changed nothing");

    app.submit("undo");
    app.submit("undo");
    assert_eq!(pixels_that_differ(&before, &page_pixels(&app)), 0, "two undos did not restore the page");
}

const EXCEL_QUOTE: &str = r"D:\Dropbox\1.QUOTATION\QT 2026\2141-AL FAHIM HQ - CALLIOPE-DALMA MALL-ABUDHABI-HS-2126-AJ-2141 CHIP AND DRIVER REPLACEMENT.pdf";

/// **A line can be added to a paragraph in a PDF printed from Excel.**
///
/// Reported from use, with the log line `a new line could not be added: pdfium
/// error: run-own-font-… could not be cut down: UnknownKind`. The new line is
/// written in the paragraph's own font, which is cut down to the letters it needs.
/// Office writes that font's table directory out of order (`glyf, cmap, head, …`),
/// the subsetter finds a table by binary search, walked past `glyf` and called a
/// TrueType font unknown. Adding a *separate* text box worked all along because it
/// uses one of the app's own fonts, which are in order.
#[test]
fn a_line_can_be_added_to_a_paragraph_in_a_pdf_printed_from_excel() {
    if !std::path::Path::new(EXCEL_QUOTE).is_file() {
        eprintln!("skipping: the Excel quotation is not on this machine");
        return;
    }
    let mut app = PagifyApp::new(Some(EXCEL_QUOTE));
    let run = text_runs(&app, 0)
        .into_iter()
        .find(|r| r.text.trim() == "Chip and driver replacement")
        .expect("the description is on the page");
    app.pick_text_run(0, centre(&run.rect)).expect("the description is clickable");
    let typed = format!("{}\nA third line added by hand", app.tab().edit.editing_run.as_ref().expect("editing").buffer.trim_end());
    app.tab_mut().edit.editing_run.as_mut().expect("editing").buffer = typed;
    assert!(!app.apply_editing_page(), "the edit was refused: {}", said(&app));

    let said = said(&app);
    assert!(!said.contains("could not be added"), "the new line was not added: {said}");
    // Excel's subset of Arial Bold holds only the letters the sheet printed and
    // has no `b`, so the line cannot be written in it: it goes in a face that has
    // every letter, and says so, rather than drawing a box for the `b` of "by".
    assert!(said.contains("The new line is written in"), "the swap was not announced: {said}");

    // The new line is written a glyph at a time, so it comes back as one text
    // object per letter: read it as what is on the line below the paragraph,
    // left to right.
    let below = text_runs(&app, 0)
        .into_iter()
        .filter(|r| r.rect.left > 100.0 && r.rect.left < 330.0 && r.rect.top > 249.0 && r.rect.top < 258.0)
        .collect::<Vec<_>>();
    let mut below = below;
    below.sort_by(|a, b| a.rect.left.total_cmp(&b.rect.left));
    let line: String = below.iter().map(|r| r.text.as_str()).collect::<String>().split_whitespace().collect();
    assert_eq!(line, "Athirdlineaddedbyhand", "the new line is not on the page; the log says: {said}");

    // **And every letter is drawn.** The font here is Excel's subset of the
    // letters the sheet uses, so a letter it never held would be extracted fine
    // and leave a blank box — the same failure as the dimension's missing `5`.
    let blank: Vec<&str> = below
        .iter()
        .filter(|r| !r.text.trim().is_empty())
        .filter(|r| ink_per_column(&app, r.rect, 1)[0] == 0)
        .map(|r| r.text.as_str())
        .collect();
    assert!(blank.is_empty(), "these letters of the new line have no ink: {blank:?}");
    if let Ok(out) = std::env::var("PAGIFY_SAVE_EXCEL_PARAGRAPH_PNG") {
        let area = Rect { left: 110.0, top: 228.0, right: 260.0, bottom: 262.0 };
        let r = app.tab().doc.as_ref().expect("open").session.render_page_region(0, area, 8.0).expect("render");
        image::save_buffer(out, &r.pixels, r.width, r.height, image::ColorType::Rgba8).expect("png");
    }
}

/// **A new line made only of letters the paragraph's font has stays in it**, with
/// nothing said — the swap is for the letters the Excel subset never held, not for
/// every added line.
#[test]
fn a_new_line_of_letters_the_documents_font_has_stays_in_the_documents_font() {
    if !std::path::Path::new(EXCEL_QUOTE).is_file() {
        eprintln!("skipping: the Excel quotation is not on this machine");
        return;
    }
    let mut app = PagifyApp::new(Some(EXCEL_QUOTE));
    let run = text_runs(&app, 0)
        .into_iter()
        .find(|r| r.text.trim() == "Chip and driver replacement")
        .expect("the description is on the page");
    app.pick_text_run(0, centre(&run.rect)).expect("the description is clickable");
    let typed = format!("{}\nChip and driver", app.tab().edit.editing_run.as_ref().expect("editing").buffer.trim_end());
    app.tab_mut().edit.editing_run.as_mut().expect("editing").buffer = typed;
    assert!(!app.apply_editing_page(), "the edit was refused: {}", said(&app));

    let said = said(&app);
    assert!(!said.contains("could not be added"), "the new line was not added: {said}");
    assert!(!said.contains("The new line is written in"), "a swap was announced for letters the font has: {said}");
    let line: String = text_runs(&app, 0)
        .into_iter()
        .filter(|r| r.rect.left > 100.0 && r.rect.left < 330.0 && r.rect.top > 249.0 && r.rect.top < 258.0)
        .map(|r| r.text)
        .collect::<String>()
        .split_whitespace()
        .collect();
    assert_eq!(line, "Chipanddriver", "the new line is not on the page");
}
