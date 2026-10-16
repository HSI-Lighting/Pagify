//! Character styles in the Edit Text box over a page's own words: the box opens with
//! the looks the page already has, the panel and the preview act on the selection,
//! and Apply writes the looks back into the page's own text.

use super::wrap_hyphen_tests::open_page;
use super::*;
use pagify_shell::rich_text::{Mixed, Script};

/// The runs of page 1, as the page says them.
fn runs_of(app: &PagifyApp) -> Vec<pdf_core::document::TextRun> {
    app.tab().doc.as_ref().expect("open").session.text_runs(0).expect("runs")
}

/// A click on the middle of a box.
fn middle_of(rect: &pdf_core::document::Rect) -> AppPoint {
    AppPoint { x: ((rect.left + rect.right) / 2.0) as f64, y: ((rect.top + rect.bottom) / 2.0) as f64 }
}

/// A hermetic page drawing `lines` (`(x, y, words)`) and then `extra` raw operators,
/// opened in Edit Text by a click on the run that says `words`.
fn picked(name: &str, lines: &[(f32, f32, &str)], extra: &str, words: &str) -> (PagifyApp, EditingRun) {
    let mut app = open_page(name, lines, extra);
    let run = runs_of(&app).into_iter().find(|r| r.text.contains(words)).unwrap_or_else(|| panic!("no run says {words:?}"));
    app.submit("edittext");
    app.pick_text_run(0, middle_of(&run.rect)).expect("picked");
    let edit = app.tab().edit.editing_run.as_ref().expect("an editor opened").clone();
    (app, edit)
}

/// One line, two pieces: the second drawn after `change`.
fn two_pieces(change: &str) -> String {
    format!("BT /F1 12 Tf 100 700 Td (Hello ) Tj {change} (there) Tj ET")
}

#[test]
fn a_word_the_page_sets_larger_opens_larger() {
    let (_app, edit) = picked("larger", &[], &two_pieces("/F1 20 Tf"), "Hello");
    assert_eq!(edit.buffer, "Hello there");
    assert_eq!(edit.rich.look_of(6..11).size, Mixed::One(20.0));
    assert_eq!(edit.rich.look_of(0..6).size, Mixed::One(12.0));
    assert!(edit.styled == false, "opening is not styling");
}

#[test]
fn a_piece_in_another_font_opens_in_it() {
    let (_app, edit) = picked("other-font", &[], &two_pieces("/F2 12 Tf"), "Hello");
    let (plain, bold) = (edit.rich.look_at(1).clone(), edit.rich.look_at(11).clone());
    assert!(plain.page_font.is_some() && bold.page_font.is_some() && plain.page_font != bold.page_font, "{plain:?} {bold:?}");
    assert!(bold.face.contains("Bold"), "{:?}", bold.face);
    // Each page font in the box is remembered by a piece drawn in it.
    assert_eq!(edit.font_objects.len(), 2, "{:?}", edit.font_objects);
    assert_eq!(edit.line_base.len(), 1);
    assert_eq!(edit.line_base[0].page_font, plain.page_font, "the line is written like its first piece");
}

#[test]
fn a_rise_and_letter_spacing_open_as_the_page_has_them() {
    // On the second piece of a line.
    let (_app, edit) = picked("rise-piece", &[], &two_pieces("2 Tc 3 Ts"), "Hello");
    let there = edit.rich.look_of(6..11);
    assert_eq!((there.letter_spacing_pt, there.baseline_pt), (Mixed::One(2.0), Mixed::One(3.0)), "{there:?}");
    let hello = edit.rich.look_of(0..6);
    assert_eq!((hello.letter_spacing_pt, hello.baseline_pt), (Mixed::One(0.0), Mixed::One(0.0)), "{hello:?}");
    // And on a run that is alone.
    let (_app, edit) = picked("rise-alone", &[], "BT /F1 12 Tf 2 Tc 50 Tz 3 Ts 100 700 Td (Hello) Tj ET", "Hello");
    let look = edit.rich.look_of(0..5);
    assert_eq!(
        (look.letter_spacing_pt, look.width_pct, look.baseline_pt),
        (Mixed::One(1.0), Mixed::One(50.0), Mixed::One(3.0)),
        "spacing counted at the scaling it is drawn at: {look:?}"
    );
}

#[test]
fn a_superscript_the_page_has_opens_as_one() {
    let content = "BT /F1 12 Tf 100 700 Td (m) Tj /F1 6.96 Tf 3.96 Ts (2) Tj ET";
    let (_app, edit) = picked("super", &[], content, "m");
    let two = edit.rich.look_at(2);
    assert_eq!((two.script, two.size), (Script::Super, 12.0), "{two:?}");
    assert_eq!(edit.rich.look_at(1).script, Script::Normal);
}

#[test]
fn an_underline_the_page_has_opens_underlined() {
    // A rule of ours under "there" only: "Hello " spans 100 to 130.7, "there" to 158.
    let rule = "/PagifyRule <</Kind 1>> BDC q 1 0 0 1 0 0 cm 0 0 0 rg 131 697.5 27 0.9 re f Q EMC";
    let (_app, edit) = picked("underlined", &[], &format!("{}\n{rule}", two_pieces("")), "Hello");
    assert_eq!(edit.rich.look_of(6..11).underline, Mixed::One(true));
    assert_eq!(edit.rich.look_of(0..6).underline, Mixed::One(false));
    assert_eq!(edit.rich.look_of(0..11).strike, Mixed::One(false));
}

#[test]
fn a_paragraph_of_one_look_opens_unstyled() {
    let lines = [(100.0, 700.0, "alpha beta gam"), (100.0, 688.0, "ma delta epsi"), (100.0, 676.0, "lon zeta eta")];
    let (_app, edit) = picked("one-look", &lines, "", "ma delta");
    assert_eq!(edit.lines.len(), 3, "setup: the whole paragraph opened");
    assert!(!edit.styled);
    assert_eq!(edit.rich, edit.picked);
    assert_eq!(edit.rich.runs().len(), 1, "{:?}", edit.rich.runs());
    assert_eq!(edit.rich.text(), edit.buffer);
    assert_eq!(edit.line_base.len(), 3);
    assert_eq!(edit.box_chars.len(), edit.buffer.chars().count());
    // The newlines belong to no piece; every other character has one.
    let newlines: Vec<usize> = edit.buffer.chars().enumerate().filter(|(_, c)| *c == '\n').map(|(i, _)| i).collect();
    assert_eq!(newlines.len(), 2);
    for (i, source) in edit.box_chars.iter().enumerate() {
        assert_eq!(source.is_none(), newlines.contains(&i), "character {i}: {source:?}");
    }
    // Each piece's words are kept to write it again with.
    assert_eq!(edit.piece_texts.len(), 3);
}

/// **The mended hyphen keeps the piece it came from.** The page's own hyphen glyph
/// reads back as the control code U+0002 and is "-" in the box; the box's character
/// finds the piece and the place in it that draws it, and a hyphen the box added
/// because the page draws one after a line finds none.
#[test]
fn a_box_char_maps_to_its_piece_through_a_mended_hyphen() {
    use pdf_core::document::{Color, Point, Rect, TextRun};
    let mut app = open_page("mended", &[], "");
    let run = |object: usize, text: &str, top: f32| TextRun {
        object,
        text: text.to_string(),
        rect: Rect { left: 100.0, top, right: 100.0 + 5.0 * text.len() as f32, bottom: top + 10.0 },
        origin: Point { x: 100.0, y: top + 8.0 },
        size: 8.0,
        color: Color { r: 0, g: 0, b: 0, a: 255 },
    };
    let runs = vec![run(40, "sup\u{2}", 100.0), run(41, "port", 112.0)];
    let names: HashMap<usize, String> = runs.iter().map(|r| (r.object, "Face".to_string())).collect();
    let lines: Vec<ParagraphLine> =
        runs.iter().map(|r| ParagraphLine { objects: vec![r.object], rect: r.rect, frozen: false, follows_drawn: false }).collect();
    let (edit, _) = app.build_editor_from_lines(0, lines, &runs, &names, &HashMap::new(), &[], None).expect("built");
    assert_eq!(edit.buffer, "sup-\nport");
    assert_eq!(
        edit.box_chars,
        [Some((40, 0)), Some((40, 1)), Some((40, 2)), Some((40, 3)), None, Some((41, 0)), Some((41, 1)), Some((41, 2)), Some((41, 3))]
    );
    assert_eq!(edit.piece_texts[&40], "sup\u{2}", "the page's own spelling is kept to write it again with");
}

// ---- Task 10: the panel and the preview act on the selection -------------------------

use super::text_style_panel as tsp;
use super::ui_tests::{click, harness_from};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use pagify_shell::rich_text::{CharChange, CharLook, RichText};

/// A hermetic page opened in Edit Text on its first run, in a window, with the panel
/// drawn.
fn editor(name: &str, content: &str) -> Harness<'static, PagifyApp> {
    let mut app = open_page(name, &[], content);
    let run = runs_of(&app).into_iter().find(|r| r.text.contains("Hello")).expect("the first run");
    app.submit("edittext");
    app.pick_text_run(0, middle_of(&run.rect)).expect("picked");
    let mut h = harness_from(app);
    h.run_steps(3);
    h
}

fn editing<'a>(h: &'a Harness<'static, PagifyApp>) -> &'a EditingRun {
    h.state().tab().edit.editing_run.as_ref().expect("an editor is open")
}

/// The box's selection, as its text field keeps it — and the focus in the box.
fn select(h: &mut Harness<'static, PagifyApp>, range: std::ops::Range<usize>) {
    let id = egui::Id::new(("run-editor", 0usize, editing(h).object));
    h.ctx.memory_mut(|m| m.request_focus(id));
    let mut state = egui::text_edit::TextEditState::load(&h.ctx, id).unwrap_or_default();
    state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
        egui::text::CCursor::new(range.start),
        egui::text::CCursor::new(range.end),
    )));
    state.store(&h.ctx, id);
    h.run_steps(1);
}

/// A real click on a panel control, which takes the focus away from the box.
fn press(h: &mut Harness<'static, PagifyApp>, label: &str) {
    let at = h.get_by_label(label).rect().center();
    click(h, at);
    h.run_steps(1);
}

fn flags(h: &Harness<'static, PagifyApp>, which: fn(&CharLook) -> bool) -> Vec<(std::ops::Range<usize>, bool)> {
    let r = &editing(h).rich;
    r.spans(0..r.char_count()).into_iter().map(|(range, look)| (range, which(look))).collect()
}

#[test]
fn underline_on_a_selected_word_of_the_box_underlines_only_it() {
    let mut h = editor("style-underline", &two_pieces(""));
    select(&mut h, 6..11);
    assert_eq!(editing(&h).selection, 6..11, "the box keeps the selection while it has the focus");
    press(&mut h, "Underline");
    assert_eq!(flags(&h, |l| l.underline), vec![(0..6, false), (6..11, true)]);
    assert!(editing(&h).styled, "a character style is a restyle");
    assert_eq!(editing(&h).picked.look_of(0..11).underline, Mixed::One(false), "what was opened is kept");
}

#[test]
fn the_edit_text_panel_shows_mixed_for_a_varied_selection() {
    let mut h = editor("style-mixed", &two_pieces("/F1 20 Tf"));
    let look = edit_text_look(editing(&h), 0..11);
    assert_eq!(look.size, Mixed::Mixed, "12 pt and 20 pt in one selection");
    assert_eq!(edit_text_look(editing(&h), 6..11).size, Mixed::One(20.0));
    assert_eq!(edit_text_look(editing(&h), 0..0).size, Mixed::Mixed, "an empty selection is every character");
    // And the panel itself, drawn from the selection the box had.
    select(&mut h, 6..11);
    h.run_steps(1);
    assert!(h.query_by_label("Underline").is_some(), "the panel is drawn");
}

#[test]
fn with_nothing_selected_underline_takes_every_character() {
    let mut h = editor("style-all", &two_pieces(""));
    press(&mut h, "Underline");
    assert_eq!(flags(&h, |l| l.underline), vec![(0..11, true)]);
    assert!(editing(&h).styled);
}

#[test]
fn a_whole_box_size_change_is_still_the_boxs_style() {
    let mut h = editor("style-whole-size", &two_pieces(""));
    h.state_mut().restyle_selection(
        StyleTarget::EditText,
        0..0,
        &tsp::Changes { size: Some(16.0), ..Default::default() },
    );
    let edit = editing(&h);
    assert_eq!(edit.style.size, Some(16.0), "the box's own size, as it always was");
    assert!(!edit.styled, "the whole box restyled is not a character style");
    assert_eq!(edit.rich.look_of(0..11).size, Mixed::One(16.0), "and every character shows it");
    // A size given to part of it is a character style.
    h.state_mut().restyle_selection(
        StyleTarget::EditText,
        6..11,
        &tsp::Changes { size: Some(30.0), ..Default::default() },
    );
    assert!(editing(&h).styled);
    assert_eq!(editing(&h).style.size, Some(16.0), "the box's own size is untouched by a part of it");
}

#[test]
fn bold_without_a_bold_face_says_so_and_changes_nothing() {
    let mut h = editor("style-no-bold", &two_pieces(""));
    h.state_mut().faces_state.system_fonts = Some(Vec::new());
    let (before, said) = (editing(&h).rich.clone(), h.state().ui_state.errors_said);
    press(&mut h, "Bold");
    assert!(h.state().ui_state.errors_said > said, "it was not said");
    assert_eq!(editing(&h).rich, before, "the looks were changed anyway");
    assert!(!editing(&h).styled);
    assert_eq!(editing(&h).style.face, None);
}

#[test]
fn the_preview_draws_an_underlined_word_underlined() {
    let mut rich = RichText::new(CharLook::plain("Helvetica", 12.0, [0, 0, 0, 255]));
    rich.edit("Hello there");
    rich.set_char(6..11, &CharChange { underline: Some(true), size: Some(20.0), ..Default::default() });
    let doc = egui::TextFormat { font_id: egui::FontId::proportional(24.0), color: egui::Color32::BLACK, ..Default::default() };
    let looks = EditorLooks {
        rich,
        base_size: 12.0,
        base_px: 24.0,
        doc: doc.clone(),
        fallback: doc.clone(),
        ready: Default::default(),
    };
    let job = editor_layout_job_with_looks("Hello there", false, 400.0, &doc, &doc, &|_| true, &mut |_| 0.0, &looks);
    let words = |s: &egui::text::LayoutSection| job.text[s.byte_range.start.0..s.byte_range.end.0].to_string();
    let under: String =
        job.sections.iter().filter(|s| s.format.underline.width > 0.0).map(|s| words(s)).collect();
    assert_eq!(under, "there");
    let sizes: Vec<(String, f32)> = job.sections.iter().map(|s| (words(s), s.format.font_id.size)).collect();
    assert!(sizes.iter().any(|(w, size)| w == "there" && (*size - 40.0).abs() < 1e-3), "20 pt in a 12 pt box at 24 px: {sizes:?}");
    assert!(sizes.iter().any(|(w, size)| w.starts_with("Hello") && (*size - 24.0).abs() < 1e-3), "{sizes:?}");
    // The text of the job is the buffer's, untouched, so a cursor lands where it is typed.
    assert_eq!(job.text, "Hello there");
}

#[test]
fn the_character_styles_are_open_and_the_paragraph_styles_say_next() {
    let next = Some("Comes with paragraph styles, the next stage.".to_string());
    let open = edit_text_gates(false);
    for (name, gate) in [
        ("bold", &open.bold),
        ("italic", &open.italic),
        ("underline", &open.underline),
        ("strike", &open.strike),
        ("script", &open.script),
        ("letter spacing", &open.letter_spacing),
        ("width", &open.width),
        ("baseline", &open.baseline),
    ] {
        assert_eq!(gate, &None, "{name} should work");
    }
    for (name, gate) in [
        ("justify", &open.justify),
        ("indent", &open.indent),
        ("lists", &open.lists),
        ("line spacing", &open.line_spacing),
        ("paragraph spacing", &open.paragraph_spacing),
    ] {
        assert_eq!(gate, &next, "{name} should say what comes next");
    }
    // Drawn lettering is replaced in one look: its character styles stay closed.
    let drawn = edit_text_gates(true);
    let why = Some("Drawn lettering is replaced in one look.".to_string());
    assert_eq!(
        (&drawn.underline, &drawn.strike, &drawn.script, &drawn.letter_spacing, &drawn.width, &drawn.baseline),
        (&why, &why, &why, &why, &why, &why)
    );
}

// ---- Task 11: Apply writes the looks ---------------------------------------------------

/// The runs of page 1 as the open document now has them.
fn page_runs(h: &Harness<'static, PagifyApp>) -> Vec<pdf_core::document::TextRun> {
    runs_of(h.state())
}

/// The shapes page 1 draws: with no art on these pages, only our lines.
fn rules(h: &Harness<'static, PagifyApp>) -> Vec<pdf_core::document::DrawnObject> {
    let session = &h.state().tab().doc.as_ref().expect("open").session;
    session
        .drawn_objects(0)
        .expect("drawn")
        .into_iter()
        .filter(|d| d.kind == pdf_core::document::DrawnKind::Shape)
        .collect()
}

fn run_saying<'a>(runs: &'a [pdf_core::document::TextRun], words: &str) -> &'a pdf_core::document::TextRun {
    runs.iter()
        .find(|r| r.text.trim() == words)
        .unwrap_or_else(|| panic!("no run says {words:?}: {:?}", runs.iter().map(|r| &r.text).collect::<Vec<_>>()))
}

/// Page 1's content stream, decoded, as the document would save it.
fn page_stream_bytes(h: &Harness<'static, PagifyApp>) -> Vec<u8> {
    use pdf_core::pdf::{content, File, Object};
    let path = std::env::temp_dir().join(format!("pagify-style-stream-{}-{:?}.pdf", std::process::id(), std::thread::current().id()));
    h.state().tab().doc.as_ref().expect("open").session.save_to(&path, false).expect("save");
    let bytes = std::fs::read(&path).expect("read back");
    let _ = std::fs::remove_file(&path);
    let file = File::parse(&bytes).expect("parse");
    let root = file.resolve(file.trailer().get(b"Root").expect("root")).expect("catalogue");
    let pages = file.resolve(root.as_dict().and_then(|d| d.get(b"Pages")).expect("pages")).expect("tree");
    let kids = match pages.as_dict().and_then(|d| d.get(b"Kids")).map(|k| file.resolve(k)) {
        Some(Ok(Object::Array(kids))) => kids,
        _ => panic!("no page tree"),
    };
    let page = file.resolve(&kids[0]).expect("page");
    let contents = page.as_dict().and_then(|d| d.get(b"Contents")).expect("contents");
    let mut out = Vec::new();
    let mut take = |object: Object| {
        if let Object::Stream(dict, range) = object {
            out.extend_from_slice(&content::decode(&dict, &bytes[range]).expect("decode"));
            out.push(b'\n');
        }
    };
    match file.resolve(contents).expect("contents") {
        Object::Array(parts) => parts.iter().filter_map(|p| file.resolve(p).ok()).for_each(&mut take),
        stream => take(stream),
    }
    out
}

/// The panel's Apply, pressed.
fn apply(h: &mut Harness<'static, PagifyApp>) {
    press(h, "Apply");
    h.run_steps(2);
}

#[test]
fn underlining_a_word_writes_a_rule_under_exactly_it() {
    let mut h = editor("apply-underline", &two_pieces(""));
    select(&mut h, 6..11);
    press(&mut h, "Underline");
    apply(&mut h);
    assert!(h.state().tab().edit.editing_run.is_none(), "the edit was refused: {:?}", h.state().cmd.history().iter().map(|e| &e.text).collect::<Vec<_>>());
    let (runs, lines) = (page_runs(&h), rules(&h));
    assert_eq!(lines.len(), 1, "one line, under one word: {lines:?}");
    let there = run_saying(&runs, "there");
    assert!((lines[0].rect.left - there.rect.left).abs() < 0.7 && (lines[0].rect.right - there.rect.right).abs() < 0.7, "{:?} vs {:?}", lines[0].rect, there.rect);
    assert!(lines[0].rect.top > there.origin.y && lines[0].rect.bottom < there.origin.y + 0.4 * there.size, "under the baseline: {:?} {:?}", lines[0].rect, there.origin);
    assert_eq!(run_saying(&runs, "Hello").text, "Hello ", "the words are the page's own");
}

#[test]
fn a_superscript_word_lands_smaller_and_raised() {
    let mut h = editor("apply-super", &two_pieces(""));
    let before = page_runs(&h);
    let baseline = run_saying(&before, "Hello").origin.y;
    select(&mut h, 6..11);
    press(&mut h, "Superscript");
    apply(&mut h);
    let runs = page_runs(&h);
    let there = run_saying(&runs, "there");
    assert!((there.size - 0.58 * 12.0).abs() < 0.05, "58% of 12 pt: {}", there.size);
    assert!((baseline - there.origin.y - 0.33 * 12.0).abs() < 0.1, "raised a third of an em: {} from {baseline}", there.origin.y);
    assert!((run_saying(&runs, "Hello").size - 12.0).abs() < 0.01);
}

#[test]
fn colouring_one_word_colours_only_it() {
    let mut h = editor("apply-colour", &two_pieces(""));
    select(&mut h, 6..11);
    h.state_mut().restyle_selection(
        StyleTarget::EditText,
        6..11,
        &tsp::Changes { color: Some([200, 0, 0]), ..Default::default() },
    );
    apply(&mut h);
    let runs = page_runs(&h);
    let (red, black) = (run_saying(&runs, "there").color, run_saying(&runs, "Hello").color);
    assert_eq!((red.r, red.g, red.b), (200, 0, 0));
    assert_eq!((black.r, black.g, black.b), (0, 0, 0));
    assert!(rules(&h).is_empty(), "a colour draws no line");
}

#[test]
fn letter_spacing_one_word_moves_only_the_words_after_it() {
    let content = "BT /F1 12 Tf 100 700 Td (Hello ) Tj (there ) Tj (friend) Tj ET";
    let mut h = editor("apply-spacing", content);
    let before = page_runs(&h);
    h.state_mut().restyle_selection(
        StyleTarget::EditText,
        6..11,
        &tsp::Changes { letter_spacing: Some(2.0), ..Default::default() },
    );
    apply(&mut h);
    let after = page_runs(&h);
    assert!((run_saying(&after, "Hello").rect.right - run_saying(&before, "Hello").rect.right).abs() < 0.05, "before the word: unmoved");
    let moved = run_saying(&after, "friend").rect.left - run_saying(&before, "friend").rect.left;
    assert!((moved - 10.0).abs() < 0.1, "5 letters × 2 pt: {moved}");
    assert!((run_saying(&after, "friend").rect.right - run_saying(&before, "friend").rect.right - 10.0).abs() < 0.1, "and by no more");
}

#[test]
fn a_selection_across_two_pieces_styles_both() {
    let mut h = editor("apply-two-pieces", &two_pieces(""));
    select(&mut h, 3..8);
    press(&mut h, "Underline");
    apply(&mut h);
    let (runs, lines) = (page_runs(&h), rules(&h));
    let said: String = runs.iter().map(|r| r.text.as_str()).collect();
    assert_eq!(said, "Hello there", "the words are the page's own, in the same order");
    assert_eq!(lines.len(), 2, "a line under each piece's part: {lines:?}");
    let (left, right) = (lines.iter().map(|l| l.rect.left).fold(f32::MAX, f32::min), lines.iter().map(|l| l.rect.right).fold(f32::MIN, f32::max));
    let all = page_runs(&h);
    assert!(left > all.iter().map(|r| r.rect.left).fold(f32::MAX, f32::min) + 5.0, "not under the first letters: {left}");
    assert!(right < all.iter().map(|r| r.rect.right).fold(f32::MIN, f32::max) - 1.0, "not under the last: {right}");
}

#[test]
fn untouched_lines_are_left_byte_for_byte() {
    let lines = [(100.0, 700.0, "alpha beta gam"), (100.0, 688.0, "ma delta epsi"), (100.0, 676.0, "lon zeta eta")];
    let mut app = open_page("apply-untouched", &lines, "");
    let run = runs_of(&app).into_iter().find(|r| r.text.contains("ma delta")).expect("the middle line");
    app.submit("edittext");
    app.pick_text_run(0, middle_of(&run.rect)).expect("picked");
    let mut h = harness_from(app);
    h.run_steps(3);
    assert_eq!(editing(&h).lines.len(), 3, "setup: the whole paragraph");
    let before = page_stream_bytes(&h);
    let (first, last) = (b"BT /F1 10 Tf 100 700 Td (alpha beta gam) Tj ET".as_slice(), b"BT /F1 10 Tf 100 676 Td (lon zeta eta) Tj ET".as_slice());
    let has = |haystack: &[u8], needle: &[u8]| haystack.windows(needle.len()).any(|w| w == needle);
    assert!(has(&before, first) && has(&before, last), "test assumption: the stream is as written");
    // "delta" is characters 3..8 of the middle line, which starts at 15.
    select(&mut h, 18..23);
    press(&mut h, "Underline");
    apply(&mut h);
    let after = page_stream_bytes(&h);
    assert!(has(&after, first) && has(&after, last), "an operator of a line nobody touched changed:\n{}", String::from_utf8_lossy(&after));
    assert_eq!(rules(&h).len(), 1);
}

/// Three lines of three pieces each, set to one margin on both sides: a justified paragraph.
fn justified_paragraph() -> String {
    (0..3)
        .map(|n| {
            let y = 700 - 12 * n;
            format!("BT /F1 10 Tf 1 0 0 1 100 {y} Tm (alpha ) Tj 1 0 0 1 127.24 {y} Tm (beta ) Tj 1 0 0 1 149.48 {y} Tm (gamma) Tj ET\n")
        })
        .collect()
}

#[test]
fn underlining_a_word_of_a_justified_paragraph_moves_nothing() {
    let mut app = open_page("apply-justified", &[], &justified_paragraph());
    let first = runs_of(&app).into_iter().find(|r| r.text.contains("alpha")).expect("a line");
    app.submit("edittext");
    app.pick_text_run(0, middle_of(&first.rect)).expect("picked");
    let mut h = harness_from(app);
    h.run_steps(3);
    assert_eq!(editing(&h).lines.len(), 3, "setup: the whole paragraph");
    assert!(editing(&h).justify, "setup: it is drawn justified");
    let characters = |h: &Harness<'static, PagifyApp>| h.state().tab().doc.as_ref().expect("open").session.characters(0).expect("characters");
    let (before, chars_before) = (page_runs(&h), characters(&h));
    // "beta" on the middle line: lines are "alpha beta gamma" (16 characters).
    select(&mut h, 23..27);
    press(&mut h, "Underline");
    apply(&mut h);
    let (after, chars_after) = (page_runs(&h), characters(&h));
    assert_eq!(before.len(), after.len(), "no piece came or went: {:?}", after.iter().map(|r| &r.text).collect::<Vec<_>>());
    for (was, now) in before.iter().zip(&after) {
        assert_eq!(was.text, now.text);
        assert!((was.origin.y - now.origin.y).abs() < 0.01, "{:?} moved: {:?} -> {:?}", was.text, was.origin, now.origin);
        // Where the ink of its words starts and ends, in the place the piece was: a
        // trailing space is not ink, and the box a space has is not a glyph that moved.
        let area = (was.rect.left - 1.0, was.rect.top, was.rect.right + 1.0, was.rect.bottom);
        for (a, b) in [
            (chars_before.ink_left_in(area), chars_after.ink_left_in(area)),
            (chars_before.ink_right_in(area), chars_after.ink_right_in(area)),
        ] {
            let (a, b) = (a.expect("ink before"), b.expect("ink after"));
            assert!((a - b).abs() < 0.01, "{:?} moved: {a} -> {b}", was.text);
        }
    }
    assert_eq!(rules(&h).len(), 1);
}

#[test]
fn a_styled_word_survives_retyping_its_line() {
    let mut h = editor("apply-survives", &two_pieces(""));
    select(&mut h, 6..11);
    press(&mut h, "Underline");
    apply(&mut h);
    assert_eq!(rules(&h).len(), 1, "setup: the word is underlined");
    // Reopened: the underline is the page's now.
    let run = page_runs(&h).into_iter().find(|r| r.text.contains("Hello")).expect("the line");
    h.state_mut().submit("edittext");
    h.state_mut().pick_text_run(0, middle_of(&run.rect)).expect("picked again");
    h.run_steps(3);
    assert_eq!(editing(&h).rich.look_of(6..11).underline, Mixed::One(true), "it opens underlined");
    assert_eq!(editing(&h).rich.look_of(0..6).underline, Mixed::One(false));
    // Another word of the line is retyped (as wide or narrower: a wider line would wrap).
    h.state_mut().tab_mut().edit.editing_run.as_mut().expect("editing").buffer = "Jello there".into();
    h.run_steps(2);
    apply(&mut h);
    let (runs, lines) = (page_runs(&h), rules(&h));
    let said: String = runs.iter().map(|r| r.text.as_str()).collect();
    assert_eq!(said, "Jello there");
    assert_eq!(lines.len(), 1, "one line, not two and not none: {lines:?}");
    let there = run_saying(&runs, "there");
    assert!((lines[0].rect.left - there.rect.left).abs() < 0.7 && (lines[0].rect.right - there.rect.right).abs() < 0.7, "under its new place: {:?} vs {:?}", lines[0].rect, there.rect);
}

#[test]
fn one_undo_puts_the_paragraph_back() {
    let mut h = editor("apply-undo", &two_pieces(""));
    let (stream, runs) = (page_stream_bytes(&h), page_runs(&h));
    select(&mut h, 6..11);
    press(&mut h, "Underline");
    apply(&mut h);
    assert_eq!(rules(&h).len(), 1, "setup");
    let (undone, _) = h.state().tab().doc.as_ref().expect("open").session.undo().expect("undo");
    assert!(undone, "one undo");
    assert!(rules(&h).is_empty(), "the line went with the words' restyle");
    assert_eq!(page_stream_bytes(&h), stream, "the page is as it was, exactly");
    assert_eq!(page_runs(&h).len(), runs.len());
}

#[test]
fn a_single_run_takes_styles_too() {
    let mut h = editor("apply-single", "BT /F1 12 Tf 100 700 Td (Hello there) Tj ET");
    assert_eq!(editing(&h).lines.len(), 1);
    select(&mut h, 6..11);
    press(&mut h, "Underline");
    apply(&mut h);
    let lines = rules(&h);
    assert_eq!(lines.len(), 1, "{lines:?}");
    let runs = page_runs(&h);
    let there = run_saying(&runs, "there");
    assert!((lines[0].rect.left - there.rect.left).abs() < 0.7 && (lines[0].rect.right - there.rect.right).abs() < 0.7, "{:?} vs {:?}", lines[0].rect, there.rect);
    let said: String = runs.iter().map(|r| r.text.as_str()).collect();
    assert_eq!(said, "Hello there");
}

/// **An edit of one look sends what it sent before**: a box nobody styled one character
/// at a time, on a paragraph whose lines each have one look, builds the very commands it
/// always built — no spans — whether it is a word retyped or a size for the whole box.
#[test]
fn an_edit_of_one_look_sends_what_it_sent_before() {
    use pdf_core::command::Command;
    use pdf_core::document::TextLineEdit;
    let lines = [(100.0, 700.0, "alpha beta gam"), (100.0, 688.0, "ma delta epsi"), (100.0, 676.0, "lon zeta eta")];
    let (mut app, mut edit) = picked("apply-plain", &lines, "", "ma delta");
    assert!(!edit.uses_looks(), "test assumption: one look to a line");
    let edits_of = |app: &mut PagifyApp, edit: &EditingRun, typed: &str| -> Vec<TextLineEdit> {
        let planned = app.paragraph_edit_commands(edit, typed).expect("planned");
        planned
            .commands
            .into_iter()
            .flat_map(|c| match c {
                Command::ReplaceTextLines { edits, .. } => edits,
                _ => Vec::new(),
            })
            .collect()
    };
    // A word retyped: one retype of that line's piece, nothing styled.
    let typed = edit.buffer.replace("delta", "DELTA");
    let sent = edits_of(&mut app, &edit, &typed);
    assert_eq!(sent.len(), 1, "{sent:?}");
    let TextLineEdit::Retype { text, spans, remove, style, .. } = &sent[0] else { panic!("a retype: {sent:?}") };
    assert_eq!((text.as_str(), spans.is_empty(), remove.is_empty()), ("ma DELTA epsi", true, true));
    assert_eq!((style.size, style.face.clone()), (None, None));
    // A size for the whole box: every line written, in that size, with no spans.
    edit.style.size = Some(14.0);
    let sent = edits_of(&mut app, &edit, &edit.buffer.clone());
    assert_eq!(sent.len(), 3, "{sent:?}");
    for each in &sent {
        let TextLineEdit::Retype { style, spans, .. } = each else { panic!("a retype: {each:?}") };
        assert_eq!((style.size, spans.is_empty()), (Some(14.0), true));
    }
}

/// **On the owner's own file**, with 2-byte CID fonts (Identity-H subsets) that no
/// hermetic page has: underline one word of VIVO's first paragraph and Apply. That word's
/// line is written; every glyph of the page stays where it was, the words are the
/// page's, and exactly one line is drawn, within the word. Skipped where the file is not.
#[test]
fn underlining_a_word_of_vivo_changes_that_word_only() {
    let path = r"C:\Users\hsili\Desktop\VIVO mini.pdf";
    if !std::path::Path::new(path).exists() {
        eprintln!("skipped: VIVO mini.pdf is not at {path}");
        return;
    }
    let mut app = PagifyApp::new(Some(path));
    let click = runs_of(&app).into_iter().find(|r| r.text.starts_with("ing solution")).expect("the second line");
    app.submit("edittext");
    app.pick_text_run(0, middle_of(&click.rect)).expect("picked");
    let mut h = harness_from(app);
    h.run_steps(3);
    let word = editing(&h).buffer.find("ambient").map(|at| editing(&h).buffer[..at].chars().count()).expect("the word");
    let characters = |h: &Harness<'static, PagifyApp>| h.state().tab().doc.as_ref().expect("open").session.characters(0).expect("characters");
    let (before, chars_before, shapes_before) = (page_runs(&h), characters(&h), rules(&h).len());
    select(&mut h, word..word + "ambient".chars().count());
    press(&mut h, "Underline");
    apply(&mut h);
    assert!(h.state().tab().edit.editing_run.is_none(), "refused: {:?}", h.state().cmd.history().iter().rev().take(2).map(|e| &e.text).collect::<Vec<_>>());
    let (after, chars_after) = (page_runs(&h), characters(&h));

    let said = |runs: &[pdf_core::document::TextRun]| runs.iter().map(|r| r.text.split_whitespace().collect::<String>()).collect::<String>();
    assert_eq!(said(&before), said(&after), "the page's own words");
    // Every glyph where it was, in the place of every piece the page had.
    for was in &before {
        let area = (was.rect.left - 1.0, was.rect.top, was.rect.right + 1.0, was.rect.bottom);
        for (a, b) in [
            (chars_before.ink_left_in(area), chars_after.ink_left_in(area)),
            (chars_before.ink_right_in(area), chars_after.ink_right_in(area)),
        ] {
            if let (Some(a), Some(b)) = (a, b) {
                assert!((a - b).abs() < 0.02, "{:?} moved: {a} -> {b}", was.text);
            }
        }
    }
    let lines = rules(&h);
    assert_eq!(lines.len(), shapes_before + 1, "one line, under one word");
    let drawn = lines.iter().map(|l| l.rect).find(|r| (r.right - r.left) < 60.0 && r.left > click.rect.left && r.right < click.rect.right).expect("the line");
    assert!(drawn.right - drawn.left > 15.0, "wide enough for seven letters at 8 pt: {drawn:?}");
    assert!(drawn.top > click.origin.y && drawn.top < click.origin.y + 4.0, "under the baseline: {drawn:?} {:?}", click.origin);
}

// ---- Task 11, the refusal path: the words are not the price of a look ------------------

/// What the app has said, all of it, oldest first.
fn said_so_far(h: &Harness<'static, PagifyApp>) -> String {
    h.state().cmd.history().iter().map(|e| e.text.as_str()).collect::<Vec<_>>().join(" | ")
}

/// Two pieces, the second spaced out (so the box opens with two looks and nobody has styled
/// it), with a face the engine has no way to write with put on the second word — the one
/// thing the page's own looks cannot say, and the engine refuses.
fn refused_looks(name: &str) -> Harness<'static, PagifyApp> {
    let mut h = editor(name, &two_pieces("2 Tc"));
    assert!(editing(&h).uses_looks() && !editing(&h).styled, "setup: several looks, nobody styled");
    let unavailable = CharChange { face: Some("Zzz Not Available".into()), ..CharChange::default() };
    h.state_mut().tab_mut().edit.editing_run.as_mut().expect("editing").rich.set_char(6..11, &unavailable);
    h
}

/// **A refusal of the looks must not cost the words.** Nobody styled this box; it only
/// opened with two looks to its line. Fixing a typo used to work (the line was written in
/// one look) and must still work when the engine will not write the looks.
#[test]
fn a_typo_is_fixed_when_the_engine_will_not_write_the_looks() {
    let mut h = refused_looks("refusal-typo");
    h.state_mut().tab_mut().edit.editing_run.as_mut().expect("editing").buffer = "Jello there".into();
    h.run_steps(2);
    apply(&mut h);
    let said: String = page_runs(&h).iter().map(|r| r.text.as_str()).collect();
    assert_eq!(said, "Jello there", "the words are fixed; said: {}", said_so_far(&h));
    assert!(said_so_far(&h).contains("each line is in one look"), "and the looks lost are said: {}", said_so_far(&h));
}

/// **But a look somebody asked for is not quietly dropped.** The same refusal on a box the
/// person styled: nothing is flattened and nothing changes.
#[test]
fn a_look_asked_for_and_refused_changes_nothing() {
    let mut h = refused_looks("refusal-asked");
    let (stream, runs) = (page_stream_bytes(&h), page_runs(&h));
    h.state_mut().tab_mut().edit.editing_run.as_mut().expect("editing").styled = true;
    h.state_mut().tab_mut().edit.editing_run.as_mut().expect("editing").buffer = "Jello there".into();
    h.run_steps(2);
    apply(&mut h);
    assert!(said_so_far(&h).contains("nothing was changed"), "refused and said so: {}", said_so_far(&h));
    assert_eq!(page_stream_bytes(&h), stream, "the page is as it was, exactly");
    assert_eq!(page_runs(&h).len(), runs.len());
}
