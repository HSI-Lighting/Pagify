//! The Text Style panel and the new text box: the font is always named, and (from
//! phase 1) the box's character styles follow the selection.

use super::ui_tests::{click, harness};
use super::*;
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;

fn centre(r: &pdf_core::document::Rect) -> AppPoint {
    AppPoint { x: ((r.left + r.right) / 2.0) as f64, y: ((r.top + r.bottom) / 2.0) as f64 }
}

/// A box dragged out on page 1, with a couple of frames for the panel to appear.
fn open_box(h: &mut Harness<'static, PagifyApp>) {
    h.state_mut().begin_text_box(0, AppPoint { x: 50.0, y: 500.0 }, AppPoint { x: 250.0, y: 540.0 }).expect("box opened");
    h.run_steps(2);
}

fn box_face(h: &Harness<'static, PagifyApp>) -> Option<String> {
    Some(h.state().tab().edit.new_text_box.as_ref().expect("a box is open").rich.look_at(0).face.clone())
}

/// Words put into the open box, and a couple of frames for the box to take them in.
fn type_into_box(h: &mut Harness<'static, PagifyApp>, text: &str) {
    h.state_mut().tab_mut().edit.new_text_box.as_mut().expect("a box is open").buffer = text.into();
    h.run_steps(2);
}

/// The box's selection, as the box's own text field keeps it — and the focus in the
/// box, as a drag across its words gives it.
fn select(h: &mut Harness<'static, PagifyApp>, range: std::ops::Range<usize>) {
    let id = egui::Id::new(("new-text-box", 0usize));
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

fn rich(h: &Harness<'static, PagifyApp>) -> pagify_shell::rich_text::RichText {
    h.state().tab().edit.new_text_box.as_ref().expect("a box is open").rich.clone()
}

fn flagged(h: &Harness<'static, PagifyApp>, which: fn(&pagify_shell::rich_text::CharLook) -> bool) -> Vec<(std::ops::Range<usize>, bool)> {
    let r = rich(h);
    r.spans(0..r.char_count()).into_iter().map(|(range, look)| (range, which(look))).collect()
}

fn underlined(h: &Harness<'static, PagifyApp>) -> Vec<(std::ops::Range<usize>, bool)> {
    flagged(h, |look| look.underline)
}

#[test]
fn underline_on_a_selection_underlines_only_those_characters() {
    let mut h = harness("two-column.pdf");
    open_box(&mut h);
    type_into_box(&mut h, "Hello there");
    select(&mut h, 6..11);
    press(&mut h, "Underline");
    assert_eq!(underlined(&h), vec![(0..6, false), (6..11, true)]);
}

/// **Clicking the panel takes the focus from the box** — the selection it acts on is
/// the one the box had.
#[test]
fn the_panel_acts_on_the_selection_the_box_had_before_the_click() {
    let mut h = harness("two-column.pdf");
    open_box(&mut h);
    type_into_box(&mut h, "Hello there");
    select(&mut h, 6..11);
    press(&mut h, "Underline");
    press(&mut h, "Strikethrough");
    assert_eq!(flagged(&h, |look| look.strike), vec![(0..6, false), (6..11, true)]);
}

#[test]
fn with_nothing_selected_a_style_changes_every_character() {
    let mut h = harness("two-column.pdf");
    open_box(&mut h);
    type_into_box(&mut h, "Hello there");
    press(&mut h, "Underline");
    assert_eq!(underlined(&h), vec![(0..11, true)]);
}

#[test]
fn typing_after_a_styled_word_continues_its_look() {
    let mut h = harness("two-column.pdf");
    open_box(&mut h);
    type_into_box(&mut h, "Hello there");
    select(&mut h, 6..11);
    press(&mut h, "Underline");
    h.state_mut().tab_mut().edit.new_text_box.as_mut().unwrap().buffer.push('!');
    h.run_steps(2);
    assert_eq!(underlined(&h), vec![(0..6, false), (6..12, true)]);
}

#[test]
fn a_selection_that_varies_shows_mixed_and_a_click_sets_it_for_all() {
    let mut h = harness("two-column.pdf");
    open_box(&mut h);
    type_into_box(&mut h, "Hello there");
    select(&mut h, 6..11);
    press(&mut h, "Underline");
    select(&mut h, 0..11);
    press(&mut h, "Underline");
    assert_eq!(underlined(&h), vec![(0..11, true)], "a mixed Underline should turn it on for all");
}

#[test]
fn the_box_draws_each_run_in_its_face_size_and_colour() {
    use pagify_shell::rich_text::{CharChange, CharLook, RichText, Script};
    let mut rich = RichText::new(CharLook::plain("Montserrat-Regular", 10.0, [0, 0, 0, 255]));
    rich.edit("ab");
    rich.set_char(1..2, &CharChange { size: Some(20.0), color: Some([255, 0, 0, 255]), underline: Some(true), ..Default::default() });
    let ready: std::collections::BTreeSet<String> = ["Montserrat-Regular".to_string()].into();
    let family = egui::FontFamily::Name(rich_preview::face_family("Montserrat-Regular").into());

    let job = rich_preview::box_job(&rich, "ab", 2.0, &ready, 500.0);
    let b = &job.sections[1].format;
    assert_eq!(b.font_id, egui::FontId::new(40.0, family.clone()));
    assert_eq!(b.color, egui::Color32::from_rgb(255, 0, 0));
    assert!(b.underline.width > 0.0, "an underlined run has no underline");
    assert!(job.sections[0].format.underline.width == 0.0, "a plain run is underlined");

    rich.set_char(0..1, &CharChange { script: Some(Script::Super), ..Default::default() });
    let job = rich_preview::box_job(&rich, "ab", 2.0, &ready, 500.0);
    assert_eq!(job.sections[0].format.valign, egui::Align::TOP);
    assert!((job.sections[0].format.font_id.size - 10.0 * 2.0 * 0.58).abs() < 1e-3);

    let job = rich_preview::box_job(&rich, "ab", 2.0, &Default::default(), 500.0);
    assert_eq!(job.sections[1].format.font_id.family, egui::FontFamily::Proportional, "a face not yet installed is drawn in");
}

/// **A width other than 100% is laid out without shaping every letter on its own** —
/// the box's layouter runs on every frame it is drawn. Timed against shaping all of
/// the words once, so the machine's speed cancels out.
#[test]
fn a_long_box_at_another_width_is_shaped_once_not_per_letter() {
    use pagify_shell::rich_text::{CharChange, CharLook, RichText};
    let face = "Montserrat-Regular";
    if !pdf_core::text::is_registered(face) {
        pdf_core::text::register(face, BUNDLED_OUTLINED_FONTS[0].to_vec()).expect("registered");
    }
    let text = "The quick brown fox jumps over the lazy dog. ".repeat(70);
    let mut rich = RichText::new(CharLook::plain(face, 12.0, [0, 0, 0, 255]));
    rich.edit(&text);
    rich.set_char(0..0, &CharChange { width_pct: Some(125.0), ..Default::default() });

    let started = std::time::Instant::now();
    pdf_core::text::shape(face, &text).expect("shaped");
    let once = started.elapsed();
    let started = std::time::Instant::now();
    let job = rich_preview::box_job(&rich, &text, 1.5, &Default::default(), 400.0);
    let laid_out = started.elapsed();
    eprintln!("shaping {} letters once {once:?}; the box's layout {laid_out:?}", text.chars().count());
    assert_eq!(job.sections.len(), text.chars().count(), "test assumption: a section per letter");
    assert!(laid_out < once * 10, "laid out in {laid_out:?}, shaping it all once takes {once:?}");
}

/// **A face is drawn only from the frame after it is installed** — drawing in a family
/// egui has not bound yet panics.
#[test]
fn a_face_is_used_from_the_frame_after_it_is_installed() {
    use pagify_shell::rich_text::CharChange;
    let mut h = harness("two-column.pdf");
    h.state_mut().faces_state.system_fonts = Some(Vec::new());
    open_box(&mut h);
    type_into_box(&mut h, "Hello");
    h.state_mut()
        .tab_mut()
        .edit
        .new_text_box
        .as_mut()
        .unwrap()
        .rich
        .set_char(0..5, &CharChange { face: Some("Montserrat-Bold".into()), ..Default::default() });
    h.run_steps(1);
    assert!(!h.state().faces_state.box_faces_ready.contains("Montserrat-Bold"), "usable on the frame it was installed");
    h.run_steps(1);
    assert!(h.state().faces_state.box_faces_ready.contains("Montserrat-Bold"), "not usable on the frame after");
    h.run_steps(3);
}

/// Installing a box's faces replaces egui's fonts wholesale: the Edit Text box's face
/// must come with them, or the next editor draws in a family nothing is bound to.
#[test]
fn a_new_boxs_faces_do_not_unbind_the_edit_text_face() {
    use pagify_shell::rich_text::CharChange;
    let mut h = harness("text-lines.pdf");
    h.state_mut().faces_state.system_fonts = Some(Vec::new());
    h.state_mut().submit("edittext");
    let run = h.state().tab().doc.as_ref().unwrap().session.text_runs(0).expect("runs").into_iter().next().expect("a run");
    h.state_mut().pick_text_run(0, centre(&run.rect)).expect("picked");
    h.run_steps(4);
    assert!(h.state().faces_state.editor_face_ready, "test assumption: the run's face is installed");

    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    open_box(&mut h);
    type_into_box(&mut h, "Hello");
    h.state_mut()
        .tab_mut()
        .edit
        .new_text_box
        .as_mut()
        .unwrap()
        .rich
        .set_char(0..5, &CharChange { face: Some("Montserrat-Bold".into()), ..Default::default() });
    h.run_steps(4);
    h.key_press(egui::Key::Escape);
    h.run_steps(2);

    h.state_mut().submit("edittext");
    h.state_mut().pick_text_run(0, centre(&run.rect)).expect("picked again");
    h.run_steps(4);
    assert!(h.state().tab().edit.editing_run.is_some(), "the editor did not open again");
}

/// **Reported from use: "(automatic)" is confusing — show the font's name.** A new box
/// starts in a font with a name: Arial where it is installed (Windows lists it by its
/// PostScript name, `ArialMT`).
#[test]
fn a_new_box_starts_in_a_named_font() {
    let installed = system_fonts::list();
    let expected = ["ArialMT", "Arial"]
        .into_iter()
        .find(|name| installed.iter().any(|f| f.name == *name))
        .unwrap_or("Montserrat-Regular");
    let mut h = harness("two-column.pdf");
    open_box(&mut h);
    assert_eq!(box_face(&h).as_deref(), Some(expected));
    assert!(h.query_by_label(&format!("Font {expected}")).is_some(), "the font field does not name {expected}");
}

/// Where Arial is not installed, the box starts in the face Pagify carries with it.
#[test]
fn a_new_box_without_arial_starts_in_the_bundled_face() {
    let mut h = harness("two-column.pdf");
    h.state_mut().faces_state.system_fonts = Some(Vec::new());
    open_box(&mut h);
    assert_eq!(box_face(&h).as_deref(), Some("Montserrat-Regular"));
}

/// Neither the field nor the picker says "(automatic)", for a new box or for the page's
/// own words; the Edit Text picker's first entry names the page's own font.
#[test]
fn nothing_in_the_panel_or_the_picker_says_automatic() {
    let mut h = harness("two-column.pdf");
    open_box(&mut h);
    let face = box_face(&h).expect("a named face");
    let button = h.get_by_label(&format!("Font {face}")).rect().center();
    click(&mut h, button);
    h.run_steps(2);
    assert!(h.state().faces_state.font_picker_open, "test assumption: the picker opened");
    assert!(h.query_by_label_contains("(automatic)").is_none(), "a new box's panel or picker says (automatic)");

    let mut h = harness("text-lines.pdf");
    h.state_mut().submit("edittext");
    let run = h.state().tab().doc.as_ref().unwrap().session.text_runs(0).expect("runs").into_iter().next().expect("a run");
    h.state_mut().pick_text_run(0, centre(&run.rect)).expect("picked");
    h.run_steps(2);
    let name = h.state().tab().edit.editing_run.as_ref().expect("an editor").current_face.clone()
        .expect("test assumption: the run's font name is read");
    let button = h.get_by_label(&format!("Font {name}")).rect().center();
    click(&mut h, button);
    h.run_steps(2);
    assert!(h.query_by_label_contains("(automatic)").is_none(), "the Edit Text panel or picker says (automatic)");
    assert!(
        h.query_by_label(&format!("{name} (the page's own font)")).is_some(),
        "the picker's first entry does not name the page's own font"
    );
}

/// On a page too heavy to read, the font's name is not read (it is a further open of
/// the page, which a click there is built to avoid), and the field says what it is in
/// words — not "(automatic)".
#[test]
fn a_word_on_a_page_too_heavy_to_read_is_not_called_automatic() {
    let mut shapes = String::new();
    for _ in 0..HEAVY_PAGE_OBJECTS + 100 {
        shapes.push_str("0 0 1 1 re f\n");
    }
    let app = super::wrap_hyphen_tests::open_page("heavy-font-name", &[(100.0, 700.0, "words on a heavy page")], &shapes);
    let mut h = super::ui_tests::harness_from(app);
    h.state_mut().submit("edittext");
    let run = h.state().tab().doc.as_ref().unwrap().session.text_runs(0).expect("runs").into_iter().next().expect("a run");
    h.state_mut().pick_text_run(0, centre(&run.rect)).expect("picked");
    h.run_steps(2);
    let edit = h.state().tab().edit.editing_run.as_ref().expect("an editor");
    assert!(edit.current_face.is_none(), "test assumption: a heavy page's font name is not read");
    assert!(h.query_by_label("Font The page's own font").is_some(), "the field does not say whose font it is");
    assert!(h.query_by_label_contains("(automatic)").is_none(), "the field says (automatic)");
}

// ---- Add to Page writes the styled box (phase 1) ----

fn session<'a>(h: &'a Harness<'static, PagifyApp>) -> &'a pagify_shell::Session {
    &h.state().tab().doc.as_ref().expect("a document").session
}

fn said(h: &Harness<'static, PagifyApp>) -> String {
    h.state().cmd.history().iter().map(|e| e.text.as_str()).collect::<Vec<_>>().join("\n")
}

/// A change to the box's characters `range`, and the frames for the box to draw it —
/// in its faces, once they are installed.
fn style(h: &mut Harness<'static, PagifyApp>, range: std::ops::Range<usize>, change: pagify_shell::rich_text::CharChange) {
    h.state_mut().tab_mut().edit.new_text_box.as_mut().expect("a box is open").rich.set_char(range, &change);
    h.run_steps(3);
}

/// A box in the face Pagify carries with it (no installed fonts), with `text` typed in.
fn box_with(text: &str) -> Harness<'static, PagifyApp> {
    let mut h = harness("two-column.pdf");
    h.state_mut().faces_state.system_fonts = Some(Vec::new());
    open_box(&mut h);
    type_into_box(&mut h, text);
    h
}

/// Where the box's own text field drew character `index` of `text` — the left end of
/// its baseline, in page points — read from what it painted.
fn shown_at(h: &Harness<'static, PagifyApp>, text: &str, index: usize) -> (f32, f32) {
    let view = h.state().tab().view_state.last_view.expect("the page was drawn");
    for clipped in &h.output().shapes {
        let egui::Shape::Text(shape) = &clipped.shape else { continue };
        if shape.galley.text() != text {
            continue;
        }
        let mut at = 0;
        for placed in &shape.galley.rows {
            let count = placed.row.glyphs.len();
            if index < at + count {
                let glyph = &placed.row.glyphs[index - at];
                let p = view.to_page(shape.pos + placed.pos.to_vec2() + glyph.pos.to_vec2());
                return (p.x as f32, p.y as f32);
            }
            at += count + placed.ends_with_newline as usize;
        }
    }
    panic!("the box did not draw {text:?}");
}

fn shapes_on_page(h: &Harness<'static, PagifyApp>) -> Vec<pdf_core::document::DrawnObject> {
    session(h).drawn_objects(0).expect("drawn").into_iter().filter(|d| d.kind == pdf_core::document::DrawnKind::Shape).collect()
}

/// Add to Page pressed: the runs and lines it put on the page.
fn add_to_page(h: &mut Harness<'static, PagifyApp>) -> (Vec<pdf_core::document::TextRun>, Vec<pdf_core::document::DrawnObject>) {
    let runs: std::collections::HashSet<usize> = session(h).text_runs(0).expect("runs").iter().map(|r| r.object).collect();
    let lines: std::collections::HashSet<usize> = shapes_on_page(h).iter().map(|d| d.object).collect();
    h.get_by_label_contains("Add to Page").click();
    h.run_steps(2);
    let mut added: Vec<_> = session(h).text_runs(0).expect("runs").into_iter().filter(|r| !runs.contains(&r.object)).collect();
    added.sort_by(|a, b| a.origin.x.total_cmp(&b.origin.x));
    let drawn = shapes_on_page(h).into_iter().filter(|d| !lines.contains(&d.object)).collect();
    (added, drawn)
}

fn run<'a>(runs: &'a [pdf_core::document::TextRun], ch: &str) -> &'a pdf_core::document::TextRun {
    runs.iter().find(|r| r.text.trim() == ch).unwrap_or_else(|| panic!("no {ch:?} among {:?}", runs.iter().map(|r| &r.text).collect::<Vec<_>>()))
}

fn em_width(face: &str, text: &str) -> f32 {
    pdf_core::text::shape(face, text).expect("shapes").width()
}

#[test]
fn add_to_page_writes_each_look_where_the_box_showed_it() {
    use pagify_shell::rich_text::CharChange;
    let mut h = box_with("Hello there");
    style(&mut h, 6..11, CharChange { size: Some(20.0), underline: Some(true), ..Default::default() });
    let (shown_h, shown_t) = (shown_at(&h, "Hello there", 0), shown_at(&h, "Hello there", 6));

    let (runs, lines) = add_to_page(&mut h);
    let (first, t) = (run(&runs, "H"), run(&runs, "t"));
    assert!((first.size - 14.0).abs() < 0.3, "Hello is {} pt", first.size);
    assert!((t.size - 20.0).abs() < 0.3, "there is {} pt", t.size);
    let after_hello = first.origin.x + 14.0 * em_width("Montserrat-Regular", "Hello ");
    assert!((t.origin.x - after_hello).abs() <= 0.5, "t at {}, Hello ends at {after_hello}", t.origin.x);
    for (written, shown, what) in [(first, shown_h, "H"), (t, shown_t, "t")] {
        assert!((written.origin.x - shown.0).abs() <= 1.0, "{what} written at x {}, the box drew it at {}", written.origin.x, shown.0);
    }
    // The row is on its largest letters' baseline, and the smaller words share it — the
    // box sits them a little low (egui lines up the bottoms), the page does not.
    assert!((t.origin.y - shown_t.1).abs() <= 1.0, "t written at y {}, the box drew it at {}", t.origin.y, shown_t.1);
    assert!((first.origin.y - t.origin.y).abs() < 0.1, "Hello on {}, there on {}", first.origin.y, t.origin.y);

    assert_eq!(lines.len(), 1, "one line under there: {:?}", lines.iter().map(|l| l.rect).collect::<Vec<_>>());
    let line = lines[0].rect;
    let end = t.origin.x + 20.0 * em_width("Montserrat-Regular", "there");
    assert!((line.left - t.origin.x).abs() <= 1.0 && (line.right - end).abs() <= 1.0, "{line:?} for {}..{end}", t.origin.x);
    assert!(line.top > t.origin.y, "the line {line:?} is not below the baseline {}", t.origin.y);
}

/// **The page keeps the words in the order they are read.** Every glyph is an object of
/// its own, and the page lists them in the order they were written — the order a
/// selection, a copy or a screen reader takes them in.
#[test]
fn a_styled_box_is_written_in_reading_order() {
    use pagify_shell::rich_text::CharChange;
    let mut h = box_with("Hello there\nmore words");
    style(&mut h, 6..11, CharChange { size: Some(20.0), ..Default::default() });
    let before: std::collections::HashSet<usize> = session(&h).text_runs(0).expect("runs").iter().map(|r| r.object).collect();
    h.get_by_label_contains("Add to Page").click();
    h.run_steps(2);
    let read: String = session(&h)
        .text_runs(0)
        .expect("runs")
        .iter()
        .filter(|r| !before.contains(&r.object))
        .map(|r| r.text.trim().to_string())
        .collect();
    assert_eq!(read, "Hellotheremorewords");
}

#[test]
fn superscript_lands_smaller_and_raised() {
    use pagify_shell::rich_text::{CharChange, Script};
    let mut h = box_with("x2");
    style(&mut h, 1..2, CharChange { script: Some(Script::Super), ..Default::default() });
    let (runs, _) = add_to_page(&mut h);
    let (x, two) = (run(&runs, "x"), run(&runs, "2"));
    assert!((two.size - 0.58 * 14.0).abs() < 0.2, "the 2 is {} pt", two.size);
    let raised = x.origin.y - two.origin.y;
    assert!((raised - 0.33 * 14.0).abs() < 0.2, "the 2 is raised {raised} pt");
}

#[test]
fn letter_spacing_width_and_baseline_shift_reach_the_page() {
    use pagify_shell::rich_text::CharChange;
    // The first word as it is, the second changed: the steps from one W to the next.
    let steps = |change: CharChange| {
        let mut h = box_with("WWWW WWWW");
        style(&mut h, 5..9, change);
        let (runs, _) = add_to_page(&mut h);
        let ws: Vec<_> = runs.iter().filter(|r| r.text.trim() == "W").collect();
        assert_eq!(ws.len(), 8, "{:?}", runs.iter().map(|r| &r.text).collect::<Vec<_>>());
        let step = |i: usize| ws[i + 1].origin.x - ws[i].origin.x;
        (step(0), step(4), ws.iter().map(|w| w.origin.y).collect::<Vec<_>>(), ws.iter().map(|w| w.rect.right - w.rect.left).collect::<Vec<_>>())
    };

    let (plain, spaced, _, _) = steps(CharChange { letter_spacing_pt: Some(5.0), ..Default::default() });
    assert!((spaced - plain - 5.0).abs() < 0.2, "letter spacing 5 pt moved each W {} pt", spaced - plain);

    let (plain, narrow, _, widths) = steps(CharChange { width_pct: Some(50.0), ..Default::default() });
    assert!((narrow / plain - 0.5).abs() < 0.02, "at 50% each W steps {narrow}, at 100% {plain}");
    assert!((widths[4] / widths[0] - 0.5).abs() < 0.03, "a W at 50% is {} wide, at 100% {}", widths[4], widths[0]);

    let (_, _, baselines, _) = steps(CharChange { baseline_pt: Some(3.0), ..Default::default() });
    assert!((baselines[0] - baselines[4] - 3.0).abs() < 0.2, "a 3 pt baseline shift moved it {} pt", baselines[0] - baselines[4]);
}

#[test]
fn one_undo_takes_the_whole_box_off_and_redo_puts_it_back() {
    use pagify_shell::rich_text::CharChange;
    let mut h = box_with("Hello there");
    style(&mut h, 6..11, CharChange { size: Some(20.0), underline: Some(true), ..Default::default() });
    let (runs_before, lines_before) = (session(&h).text_runs(0).expect("runs").len(), shapes_on_page(&h).len());
    let (runs, lines) = add_to_page(&mut h);
    assert!(!runs.is_empty() && lines.len() == 1, "test assumption: the box and its line are written");
    assert!(!said(&h).contains("times"), "the box is one step: {}", said(&h));

    h.state_mut().submit("undo");
    h.run_steps(1);
    assert_eq!(session(&h).text_runs(0).expect("runs").len(), runs_before, "a character was left behind");
    assert_eq!(shapes_on_page(&h).len(), lines_before, "the line was left behind");

    h.state_mut().submit("redo");
    h.run_steps(1);
    assert_eq!(session(&h).text_runs(0).expect("runs").len(), runs_before + runs.len());
    assert_eq!(shapes_on_page(&h).len(), lines_before + 1);
}

/// **Never writes boxes.** No face Pagify carries draws 日, so the letters are put in a
/// face that cannot draw ordinary ones either: Montserrat with its `cmap` hidden.
#[test]
fn a_letter_the_face_cannot_draw_is_written_in_one_that_can_and_said() {
    use pagify_shell::rich_text::CharChange;
    let name = "test-box-montserrat-without-cmap";
    if !pdf_core::text::is_registered(name) {
        let mut font = BUNDLED_OUTLINED_FONTS[0].to_vec();
        let count = u16::from_be_bytes([font[4], font[5]]) as usize;
        let at = (0..count).map(|i| 12 + 16 * i).find(|&at| &font[at..at + 4] == b"cmap").expect("a cmap");
        font[at..at + 4].copy_from_slice(b"cmaq");
        pdf_core::text::register(name, font).expect("registered");
    }
    let mut h = box_with("Lumen here");
    let boxes = |face: &str| pdf_core::text::shape(face, "Lumen").expect("shapes").glyphs.iter().filter(|g| g.id == 0).count();
    assert!(boxes(name) > 0 && boxes("Montserrat-Regular") == 0, "test assumption: only the test face draws boxes");
    style(&mut h, 0..5, CharChange { face: Some(name.into()), ..Default::default() });

    // The glyphs the mark carries: the test face keeps Montserrat's own name inside it,
    // so the page's font name cannot tell the two apart.
    let ctx = h.ctx.clone();
    let open = h.state_mut().tab_mut().edit.new_text_box.take().expect("a box");
    let (command, _) = h.state_mut().new_box_command(&ctx, &open).expect("a mark");
    h.state_mut().tab_mut().edit.new_text_box = Some(open);
    let pdf_core::command::Command::AddAnnotation { annotation: pdf_core::document::Annotation::Text { font, glyphs, more, .. }, .. } = command
    else {
        panic!("not one text mark")
    };
    for (face, glyphs) in std::iter::once((font, glyphs)).chain(more.into_iter().map(|p| (p.font, p.glyphs))) {
        assert_ne!(face, name, "written in the face that cannot draw the letters");
        assert!(glyphs.iter().all(|g| g.id != 0 || g.ch.trim().is_empty()), "a box was written in {face}");
    }

    add_to_page(&mut h);
    assert!(said(&h).contains("Written in Montserrat-Regular"), "the box did not say which face: {}", said(&h));
}

/// Words that wrap are written on the rows the box showed: each row's first letter
/// lands where the box drew it.
#[test]
fn a_box_that_wraps_is_written_on_the_rows_it_showed() {
    let text = "one two three four five six seven eight nine ten eleven twelve";
    let mut h = box_with(text);
    let view = h.state().tab().view_state.last_view.expect("drawn");
    let firsts: Vec<(char, AppPoint)> = h
        .output()
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Text(shape) if shape.galley.text() == text => Some(shape),
            _ => None,
        })
        .flat_map(|shape| {
            shape.galley.rows.iter().filter_map(move |placed| {
                let glyph = placed.row.glyphs.first()?;
                Some((glyph.chr, view.to_page(shape.pos + placed.pos.to_vec2() + glyph.pos.to_vec2())))
            })
        })
        .collect();
    assert!(firsts.len() >= 3, "test assumption: the words wrap to three rows or more, not {}", firsts.len());

    let (runs, _) = add_to_page(&mut h);
    for (letter, at) in firsts {
        assert!(
            runs.iter().any(|r| r.text.trim() == letter.to_string()
                && (r.origin.x - at.x as f32).abs() <= 1.0
                && (r.origin.y - at.y as f32).abs() <= 1.0),
            "no {letter:?} written where the box drew it, at {at:?}"
        );
    }
}

/// The box keeps the width its text field wrapped the words at — its own, inside its
/// margins, however wide it is dragged — for Add to Page to lay them out again at.
#[test]
fn a_wide_box_wraps_its_words_at_its_own_width() {
    let mut h = harness("two-column.pdf");
    h.state_mut().begin_text_box(0, AppPoint { x: 40.0, y: 500.0 }, AppPoint { x: 560.0, y: 540.0 }).expect("box opened");
    h.run_steps(2);
    let scale = h.state().tab().view_state.last_view.expect("drawn").scale;
    let wrap = h.state().tab().edit.new_text_box.as_ref().expect("a box").wrap_px;
    let inside = 520.0 * scale - 2.0 * BOX_MARGIN_PX as f32;
    assert!((wrap - inside).abs() < 1.0, "a box {inside} wide inside wraps at {wrap}");
}

/// **A space the face has no glyph for leaves a gap, not a box.** A tab pasted into a
/// box shapes to glyph 0 in most faces, and glyph 0 draws an empty rectangle.
#[test]
fn a_tab_the_face_cannot_draw_leaves_a_gap_not_a_box() {
    let mut h = box_with("a\tb");
    let face = box_face(&h).expect("a face");
    assert!(
        pdf_core::text::shape(&face, "\t").expect("shapes").glyphs.iter().any(|g| g.id == 0),
        "test assumption: {face} has no glyph for a tab"
    );
    let ctx = h.ctx.clone();
    let open = h.state_mut().tab_mut().edit.new_text_box.take().expect("a box");
    let (command, _) = h.state_mut().new_box_command(&ctx, &open).expect("a mark");
    let pdf_core::command::Command::AddAnnotation { annotation: pdf_core::document::Annotation::Text { glyphs, more, .. }, .. } = command
    else {
        panic!("not one text mark")
    };
    let written: Vec<_> = glyphs.into_iter().chain(more.into_iter().flat_map(|p| p.glyphs)).collect();
    let boxes: Vec<&str> = written.iter().filter(|g| g.id == 0).map(|g| g.ch.as_str()).collect();
    assert!(boxes.is_empty(), "glyph 0 written for {boxes:?}");
    let (a, b) = (written.iter().find(|g| g.ch == "a").expect("a"), written.iter().find(|g| g.ch == "b").expect("b"));
    assert!(b.x - a.x > 0.5 * 14.0, "the tab's gap went missing: a at {}, b at {}", a.x, b.x);
}

/// A word the page draws as shapes has no font of its own: the field names the face its
/// replacement will be written in.
#[test]
fn a_drawn_word_names_the_face_its_replacement_is_written_in() {
    let mut h = harness("outlined-montserrat.pdf");
    h.state_mut().submit("edittext");
    let word = h
        .state_mut()
        .drawn_words_on(0)
        .iter()
        .filter(|w| !w.text.trim().is_empty())
        .max_by_key(|w| w.text.trim().chars().count())
        .cloned()
        .expect("a recognised word");
    h.state_mut().pick_text_run(0, centre(&word.rect)).expect("picked");
    h.run_steps(2);
    let expected = h
        .state()
        .writing_faces()
        .into_iter()
        .find(|name| pdf_core::text::covers(name, &word.text))
        .unwrap_or_else(|| "Helvetica".into());
    let shown = h.state().tab().edit.editing_run.as_ref().expect("an editor").current_face.clone();
    assert_eq!(shown.as_deref(), Some(expected.as_str()));
    assert!(h.query_by_label(&format!("Font {expected}")).is_some(), "the font field does not name {expected}");
}
