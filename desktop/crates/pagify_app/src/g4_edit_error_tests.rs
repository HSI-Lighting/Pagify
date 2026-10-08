#[allow(unused_imports)]
use super::ui_tests::{click, drag, fixture, harness, harness_60fps, harness_from};
use super::*;
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;

fn page_with(content: &[u8], resources: &str, extra: &[Vec<u8>]) -> Vec<u8> {
    let mut objects: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R \
             /Resources << /Font << /F1 5 0 R >> {resources} >> >>"
        )
        .into_bytes(),
        [format!("<< /Length {} >>\nstream\n", content.len()).as_bytes(), content, b"\nendstream"].concat(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
    ];
    objects.extend(extra.iter().cloned());
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref_at = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in &offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF", objects.len() + 1).as_bytes(),
    );
    out
}

fn opened(name: &str, bytes: &[u8]) -> Harness<'static, PagifyApp> {
    let dir = std::env::temp_dir().join("pagify-g4-edit-error-tests");
    std::fs::create_dir_all(&dir).expect("a scratch folder");
    let path = dir.join(name);
    std::fs::write(&path, bytes).expect("a scratch pdf");
    let app = PagifyApp::new(Some(path.to_str().expect("a utf-8 path")));
    assert!(app.tab().doc.is_some(), "{name} did not open");
    harness_from(app)
}

fn a_character_on_screen(h: &mut Harness<'static, PagifyApp>) -> egui::Pos2 {
    let app = h.state_mut();
    let page = app.tab_mut().page;
    let chars = app.characters(page).expect("no characters");
    let r = chars.line_rects(0..1).into_iter().next().expect("no character box");
    let mid = AppPoint { x: ((r.left + r.right) / 2.0) as f64, y: ((r.top + r.bottom) / 2.0) as f64 };
    app.tab_mut().last_view.expect("the page was never drawn").to_screen(mid)
}

fn history(h: &Harness<'static, PagifyApp>) -> Vec<(String, bool)> {
    h.state()
        .cmd
        .history()
        .iter()
        .map(|e| (e.text.clone(), matches!(e.kind, Kind::Error)))
        .collect()
}

/// What was said as an error, oldest first.
fn errors(h: &Harness<'static, PagifyApp>) -> Vec<String> {
    history(h).into_iter().filter(|(_, error)| *error).map(|(text, _)| text).collect()
}

/// One run the engine will not edit: `[500 (Hello)] TJ` moves the text half
/// an em left of where the stream puts the pen, so the stream does not
/// confirm where the object starts and the edit is refused, with the same
/// `Unsupported` a real datasheet's continuation pieces earn. The page
/// `pdf_core`'s own refusal test uses.
const KERNED: &[u8] = b"BT /F1 12 Tf 72 700 Td [500 (Hello)] TJ ET";

/// The editor open on the kerned page's run, `typed` already in its box.
fn editing_the_kerned_page(typed: &str) -> Harness<'static, PagifyApp> {
    let mut h = opened("kerned.pdf", &page_with(KERNED, "", &[]));
    h.state_mut().command_open = false;
    h.state_mut().submit("edittext");
    h.run_steps(1);
    let at = a_character_on_screen(&mut h);
    click(&mut h, at);
    h.run_steps(2);
    h.state_mut().tab_mut().editing_run.as_mut().expect("no editor opened").buffer = typed.into();
    h
}

/// Somewhere well clear of the open editor's own box, on bare paper.
fn clear_of_the_editor(h: &Harness<'static, PagifyApp>) -> egui::Pos2 {
    let run = h.state().tab().editing_run.clone().expect("no editor is open");
    let view = h.state().tab().last_view.expect("the page was never drawn");
    let corner = view.to_screen(AppPoint {
        x: run.rect.left.max(run.rect.right) as f64,
        y: run.rect.top.max(run.rect.bottom) as f64,
    });
    egui::pos2(corner.x + 240.0, corner.y + 180.0)
}

/// Bare paper below every run on `text-lines.pdf`, worked out from the runs
/// themselves — a guessed offset lands inside a paragraph as often as not.
fn bare_paper_below_the_text(h: &Harness<'static, PagifyApp>) -> egui::Pos2 {
    let app = h.state();
    let runs = app.tab().doc.as_ref().expect("open").session.text_runs(0).expect("runs");
    let lowest = runs.iter().map(|r| r.rect.top.max(r.rect.bottom)).fold(0.0f32, f32::max);
    let view = app.tab().last_view.expect("page never drawn");
    view.to_screen(AppPoint { x: 40.0, y: (lowest + 30.0) as f64 })
}

// ------------------------------------------------------------ the wording --

/// **A refusal says what happened and what to do, not that a feature is
/// missing.** Reported from use: "that text is drawn in a way this cannot
/// edit is not implemented yet" — `PdfError::Unsupported` prints
/// "<reason> is not implemented yet", and about twenty deliberate refusals
/// on the text-edit path use it, so a protective refusal read like a crash.
/// Only the refusals are reworded; a genuine gap keeps the engine's own
/// words, and every other kind of error passes through untouched.
#[test]
fn a_deliberate_refusal_is_worded_as_one_and_a_genuine_gap_keeps_its_wording() {
    // The engine's own reasons, verbatim from `pdfium_doc.rs`, and a phrase
    // each explanation must carry.
    let refusals: [(&'static str, &str); 12] = [
        ("that is not a text run", "refuses rather than risk"),
        ("that text is drawn in a way this cannot edit", "refuses rather than risk"),
        ("that run's codes cannot be lined up with its text", "refuses rather than risk"),
        ("that run's characters belong to none of its codes", "refuses rather than risk"),
        ("that text selects no font", "this refuses instead"),
        ("that page declares no fonts", "this refuses instead"),
        ("that font's codes cannot be counted", "this refuses instead"),
        ("that font carries no character map this can read", "this refuses instead"),
        (
            "this run's colour cannot be changed without rewriting the page around it, so it has been left alone",
            "Retyping the words is not affected",
        ),
        (
            "this run cannot be changed without rewriting the page around it, so it has been left alone",
            "this refuses",
        ),
        ("this run's font cannot write those characters", "without a new Position or Colour"),
        (
            "changing how these words look rewrites the rest of the page here, so nothing was changed. \
             Retyping the words themselves does not hit this guard, only their size, colour or position; \
             doing that on this page",
            "Retyping the words on their own does not hit this limit",
        ),
    ];
    for (reason, carries) in refusals {
        let said = explain(&PdfError::Unsupported(reason));
        assert!(!said.contains("not implemented"), "{reason:?} still reads as a missing feature: {said}");
        assert!(said.ends_with('.'), "{said:?} is not a sentence");
        assert!(said.contains(carries), "{reason:?} does not say what to do ({carries:?}): {said}");
    }
    // The one that is a noun phrase, not a clause, gets a sentence of its own.
    let said = explain(&PdfError::Unsupported("editing two runs that one operator draws"));
    assert!(!said.contains("not implemented") && said.contains("refuses rather than"), "{said}");

    // A genuine gap keeps the wording: nothing was refused, it is not built.
    let gap = PdfError::Unsupported("listing what a page draws");
    assert_eq!(explain(&gap), "listing what a page draws is not implemented yet");
    // A reason nobody has written an explanation for is passed through, not guessed at.
    let unknown = PdfError::Unsupported("a content stream written inline");
    assert_eq!(explain(&unknown), unknown.to_string());
    // Everything that is not `Unsupported` is untouched.
    for other in [
        PdfError::InvalidArgument("'\u{3a9}' is not in this text's font. Add one with `outlinedfont add <file.ttf>`.".into()),
        PdfError::Pdfium("the page could not be rewritten".into()),
        PdfError::MalformedDocument,
    ] {
        assert_eq!(explain(&other), other.to_string());
    }
}

/// **The same refusal, end to end, through the real Apply** — and read on
/// the bar under the buttons, where somebody looks, not only in the log.
#[test]
fn a_refused_apply_reads_as_a_refusal_on_screen_not_as_a_missing_feature() {
    let mut h = editing_the_kerned_page("Jello");
    h.state_mut().apply_editing_page();
    h.run_steps(2);

    let said = errors(&h);
    assert_eq!(said.len(), 1, "{said:?}");
    assert!(!said[0].contains("not implemented"), "reads as a missing feature: {}", said[0]);
    assert!(
        said[0].starts_with("that text is drawn in a way this cannot edit"),
        "the engine's own reason should still be in it: {}",
        said[0]
    );
    // No tool is armed after Apply, so the bar shows the last thing said. **Changed
    // with the reason**: the refused edit's box stays open now, and says why beside
    // its Apply button too — so the refusal is on screen twice, not once.
    assert!(h.query_all_by_label_contains("refuses rather than risk").next().is_some(), "not on screen: {said:?}");
}

/// A paragraph — a line the page draws in several pieces — is applied
/// through its own function and prints its own copy of the error. The empty
/// `() Tj` makes no text object, so there is one operator more than there
/// are objects and the batch cannot be counted (`pdf_core`'s
/// `a_page_whose_objects_do_not_match_its_operators_is_not_counted`).
#[test]
fn a_refused_paragraph_apply_reads_as_a_refusal_too() {
    let content = b"BT /F1 12 Tf 72 700 Td () Tj (Same) Tj (Same) Tj (Same) Tj ET";
    let mut h = opened("paragraph.pdf", &page_with(content, "", &[]));
    h.state_mut().command_open = false;
    h.state_mut().submit("edittext");
    h.run_steps(1);
    let at = a_character_on_screen(&mut h);
    click(&mut h, at);
    h.run_steps(2);
    let edit = h.state().tab().editing_run.clone().expect("no editor opened");
    assert!(
        edit.lines.iter().map(|(objects, _)| objects.len()).sum::<usize>() > 1,
        "this is not the paragraph path: {:?}",
        edit.lines
    );
    h.state_mut().tab_mut().editing_run.as_mut().expect("no editor").buffer = "Aaaa Bbbb Cccc".into();
    h.state_mut().apply_editing_page();
    h.run_steps(2);

    let said = errors(&h);
    assert_eq!(said.len(), 1, "{said:?}");
    assert!(!said[0].contains("not implemented"), "reads as a missing feature: {}", said[0]);
    assert!(said[0].contains("refuses"), "{}", said[0]);
    // On screen on the bar and beside Apply (see the single run's test above).
    assert!(h.query_all_by_label_contains("refuses").next().is_some(), "not on screen: {said:?}");
}

/// The page's text now, read fresh.
fn page_text(h: &mut Harness<'static, PagifyApp>) -> String {
    let app = h.state_mut();
    app.tab_mut().doc.as_mut().unwrap().caches.text = None;
    app.characters(0).map(|c| c.text()).unwrap_or_default()
}

/// The editor open on the first thing in `content`, the page's other text left alone.
fn editing_first_thing_of(content: &[u8]) -> Harness<'static, PagifyApp> {
    // A file of its own: tests run side by side, and one shared name races.
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let name = format!("typed-away-{}.pdf", NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    let mut h = opened(&name, &page_with(content, "", &[]));
    h.state_mut().command_open = false;
    h.state_mut().submit("edittext");
    h.run_steps(1);
    let at = a_character_on_screen(&mut h);
    click(&mut h, at);
    h.run_steps(2);
    assert!(h.state().tab().editing_run.is_some(), "no editor opened");
    h
}

/// **Selecting all the words in the box and deleting them deletes the text.**
/// Reported from use: it did nothing, though cutting it down to one character
/// worked — "left as it was" for an empty box.
#[test]
fn typing_a_run_away_deletes_it_and_undo_puts_it_back() {
    let mut h = editing_first_thing_of(
        b"BT /F1 12 Tf 72 700 Td (Hello) Tj ET BT /F1 12 Tf 72 400 Td (Elsewhere) Tj ET",
    );
    assert!(page_text(&mut h).contains("Hello"));
    h.state_mut().tab_mut().editing_run.as_mut().expect("editor").buffer.clear();
    h.state_mut().apply_editing_page();
    h.run_steps(2);

    assert!(errors(&h).is_empty(), "{:?}", errors(&h));
    let after = page_text(&mut h);
    assert!(!after.contains("Hello"), "the words are still on the page: {after:?}");
    assert!(after.contains("Elsewhere"), "something else was deleted too: {after:?}");
    let said = history(&h).last().map(|(text, _)| text.clone()).unwrap_or_default();
    assert!(said.starts_with("deleted"), "{said:?}");

    h.state_mut().submit("undo");
    let back = page_text(&mut h);
    assert!(back.contains("Hello") && back.contains("Elsewhere"), "undo did not put it back: {back:?}");
}

/// A box with only spaces left is as empty as one with nothing in it.
#[test]
fn a_box_left_with_only_spaces_deletes_too() {
    let mut h = editing_first_thing_of(
        b"BT /F1 12 Tf 72 700 Td (Hello) Tj ET BT /F1 12 Tf 72 400 Td (Elsewhere) Tj ET",
    );
    h.state_mut().tab_mut().editing_run.as_mut().expect("editor").buffer = "  \n ".into();
    h.state_mut().apply_editing_page();
    h.run_steps(2);

    let after = page_text(&mut h);
    assert!(!after.contains("Hello") && after.contains("Elsewhere"), "{after:?}");
}

/// Every line of a paragraph goes, as one undo step, and nothing outside it.
#[test]
fn typing_a_whole_paragraph_away_deletes_every_line_in_one_undo() {
    let mut h = editing_first_thing_of(
        b"BT /F1 12 Tf 14 TL 72 700 Td (First line of the words that run on) Tj T* \
          (Second line of the words that run) Tj T* (Third line of the words that go) Tj ET \
          BT /F1 12 Tf 72 400 Td (Elsewhere) Tj ET",
    );
    let lines = h.state().tab().editing_run.as_ref().expect("editor").lines.len();
    assert_eq!(lines, 3, "this page's three lines were not picked as one paragraph");
    h.state_mut().tab_mut().editing_run.as_mut().expect("editor").buffer.clear();
    h.state_mut().apply_editing_page();
    h.run_steps(2);

    assert!(errors(&h).is_empty(), "{:?}", errors(&h));
    let after = page_text(&mut h);
    assert!(!after.contains("First") && !after.contains("Second") && !after.contains("Third"), "a line is left: {after:?}");
    assert!(after.contains("Elsewhere"), "something else was deleted too: {after:?}");

    h.state_mut().submit("undo");
    let back = page_text(&mut h);
    assert!(back.contains("First line") && back.contains("Second line"), "one undo did not bring it all back: {back:?}");
}

/// Leaving the box alone is still no edit — only an emptied box is a deletion.
#[test]
fn a_box_left_as_it_was_is_still_not_an_edit() {
    let mut h = editing_first_thing_of(b"BT /F1 12 Tf 72 700 Td (Hello) Tj ET");
    let generation = h.state().doc_generation();
    h.state_mut().apply_editing_page();
    h.run_steps(2);
    assert_eq!(h.state().doc_generation(), generation, "an untouched box changed the document");
    assert!(page_text(&mut h).contains("Hello"));
}

// ---- ⌘V in Edit Object / Edit Text picks the paste up; a click puts it down ----

/// Where a click puts a paste down, on the page.
fn page_spot(h: &Harness<'static, PagifyApp>, x: f64, y: f64) -> egui::Pos2 {
    h.state().tab().last_view.expect("the page was never drawn").to_screen(AppPoint { x, y })
}

fn occurrences(h: &mut Harness<'static, PagifyApp>, words: &str) -> usize {
    page_text(h).matches(words).count()
}

/// Every galley painted this frame, as `(its text, the alpha of its first letter's colour, where it starts)`.
fn painted_texts(h: &Harness<'static, PagifyApp>) -> Vec<(String, u8, egui::Pos2)> {
    fn walk(shape: &egui::Shape, out: &mut Vec<(String, u8, egui::Pos2)>) {
        match shape {
            egui::Shape::Text(t) => out.push((
                t.galley.job.text.clone(),
                t.galley.job.sections.first().map(|s| s.format.color.a()).unwrap_or(255),
                t.pos,
            )),
            egui::Shape::Vec(inner) => inner.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    h.output().shapes.iter().for_each(|clipped| walk(&clipped.shape, &mut out));
    out
}

/// An Edit Object harness with the page's first run of words picked.
fn words_picked_in_edit_object() -> (Harness<'static, PagifyApp>, pdf_core::document::TextRun) {
    let mut h = harness("text-lines.pdf");
    h.state_mut().submit("editobject");
    h.run_steps(1);
    let run = h
        .state()
        .tab()
        .doc
        .as_ref()
        .unwrap()
        .session
        .text_runs(0)
        .unwrap()
        .into_iter()
        .find(|r| r.text.trim().chars().count() > 4)
        .expect("a run with words in it");
    h.state_mut().tab_mut().selected =
        Some(Selected { page: 0, object: run.object, rect: run.rect, what: "the words" });
    (h, run)
}

/// **Copy words in Edit Object, paste, and the copy follows the pointer —
/// nothing is on the page until the click.** Then undo takes it away.
#[test]
fn pasting_words_picks_them_up_and_a_click_puts_them_down() {
    let (mut h, run) = words_picked_in_edit_object();
    let words = run.text.trim().to_string();
    let before = occurrences(&mut h, &words);

    h.event(egui::Event::Copy);
    h.run_steps(2);
    assert!(
        matches!(&h.state().object_clipboard, Some(ObjectClipboard::Text { lines, .. }) if lines == &[words.clone()]),
        "the words were not copied as words"
    );

    h.event(egui::Event::Paste(COPIED_IN_PAGIFY.into()));
    h.run_steps(2);
    assert!(h.state().paste_ghost.is_some(), "paste did not pick the copy up");
    assert_eq!(occurrences(&mut h, &words), before, "something was put on the page before the click");

    { let spot = page_spot(&h, 100.0, 150.0); click(&mut h, spot); }
    h.run_steps(2);
    assert!(h.state().paste_ghost.is_none(), "the click did not put it down");
    assert_eq!(occurrences(&mut h, &words), before + 1, "the copy is not on the page");
    assert!(errors(&h).is_empty(), "{:?}", errors(&h));

    h.state_mut().submit("undo");
    assert_eq!(occurrences(&mut h, &words), before, "undo did not take the pasted words back");
}

/// While it is picked up it is drawn at half strength under the pointer.
#[test]
fn the_picked_up_paste_is_drawn_at_half_strength_where_the_pointer_is() {
    let (mut h, run) = words_picked_in_edit_object();
    let words = run.text.trim().to_string();
    h.event(egui::Event::Copy);
    h.run_steps(2);
    h.event(egui::Event::Paste(COPIED_IN_PAGIFY.into()));
    h.run_steps(2);

    let pointer = page_spot(&h, 250.0, 300.0);
    h.event(egui::Event::PointerMoved(pointer));
    h.run_steps(2);

    let ghosts: Vec<_> = painted_texts(&h).into_iter().filter(|(text, _, at)| text == &words && (at.x - pointer.x).abs() < 2.0).collect();
    assert!(!ghosts.is_empty(), "no ghost was painted at the pointer {pointer:?}");
    assert!(ghosts.iter().all(|(_, alpha, _)| (100..=140).contains(alpha)), "not half strength: {ghosts:?}");

    // And the pointer's own page position is where it would land.
    h.event(egui::Event::PointerMoved(page_spot(&h, 100.0, 100.0)));
    h.run_steps(2);
    assert!(painted_texts(&h).iter().any(|(text, _, at)| text == &words && (at.x - page_spot(&h, 100.0, 100.0).x).abs() < 2.0));
}

#[test]
fn escape_puts_a_picked_up_paste_down_nowhere() {
    let (mut h, run) = words_picked_in_edit_object();
    let words = run.text.trim().to_string();
    let before = occurrences(&mut h, &words);
    h.event(egui::Event::Copy);
    h.run_steps(2);
    h.event(egui::Event::Paste(COPIED_IN_PAGIFY.into()));
    h.run_steps(2);
    assert!(h.state().paste_ghost.is_some());

    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(h.state().paste_ghost.is_none(), "Escape did not drop it");
    assert_eq!(occurrences(&mut h, &words), before);
}

/// Words copied somewhere else (what `Event::Paste` carries) are pasted the
/// same way in Edit Text.
#[test]
fn text_copied_elsewhere_is_picked_up_in_edit_text_too() {
    let mut h = harness("text-lines.pdf");
    h.state_mut().submit("edittext");
    h.run_steps(1);
    h.event(egui::Event::Paste("Brand new words\nand a second line".into()));
    h.run_steps(2);
    assert!(
        matches!(&h.state().paste_ghost, Some(PasteGhost { content: ObjectClipboard::Text { lines, .. }, .. }) if lines.len() == 2),
        "the pasted text was not picked up"
    );

    { let spot = page_spot(&h, 100.0, 160.0); click(&mut h, spot); }
    h.run_steps(2);
    let page = page_text(&mut h);
    assert!(page.contains("Brand new words") && page.contains("and a second line"), "{page:?}");

    h.state_mut().submit("undo");
    let after = page_text(&mut h);
    assert!(!after.contains("Brand new words") && !after.contains("second line"), "one undo should take both lines back: {after:?}");
}

/// Outside Edit Object / Edit Text nothing changes: ⌘V pastes at once.
#[test]
fn with_no_editing_tool_in_hand_paste_does_not_pick_anything_up() {
    let mut h = harness("text-lines.pdf");
    h.event(egui::Event::Paste("Brand new words".into()));
    h.run_steps(2);
    assert!(h.state().paste_ghost.is_none());
}

/// A picture copied from the page is picked up and put down as a picture.
#[test]
fn a_picture_from_the_page_is_copied_and_pasted_where_clicked() {
    let mut h = harness("pictures.pdf");
    h.state_mut().submit("editobject");
    h.run_steps(1);
    let image = h.state().tab().doc.as_ref().unwrap().session.images_on(0).unwrap().remove(0);
    let middle = page_spot(
        &h,
        ((image.rect.left + image.rect.right) / 2.0) as f64,
        ((image.rect.top + image.rect.bottom) / 2.0) as f64,
    );
    click(&mut h, middle);
    assert!(h.state().tab().selected.is_some(), "the picture was not picked");
    let marks = |h: &Harness<'static, PagifyApp>| h.state().tab().doc.as_ref().unwrap().session.placed_image_marks(0).unwrap().len();
    let before = marks(&h);

    h.event(egui::Event::Copy);
    h.run_steps(2);
    assert!(matches!(h.state().object_clipboard, Some(ObjectClipboard::Image { .. })), "the picture was not copied");
    h.event(egui::Event::Paste(COPIED_IN_PAGIFY.into()));
    h.run_steps(2);
    assert!(h.state().paste_ghost.is_some());
    assert_eq!(marks(&h), before, "it was placed before the click");

    { let spot = page_spot(&h, 150.0, 150.0); click(&mut h, spot); }
    h.run_steps(2);
    assert_eq!(marks(&h), before + 1, "the click did not put the picture down; said {:?}", history(&h));
}

/// Nothing selected inside the box: ⌘C takes the whole run or paragraph.
#[test]
fn copying_in_the_editor_with_nothing_selected_takes_the_whole_text() {
    let mut h = editing_first_thing_of(b"BT /F1 12 Tf 72 700 Td (Hello there) Tj ET");
    h.event(egui::Event::Copy);
    h.run_steps(2);
    assert!(
        matches!(&h.state().object_clipboard, Some(ObjectClipboard::Text { lines, .. }) if lines == &["Hello there".to_string()]),
        "the run was not copied whole"
    );
}

// ---- the run editor's box has handles on its left and right edges ----

/// Where the box's two grips are on screen, left then right.
fn grips(h: &Harness<'static, PagifyApp>) -> Vec<egui::Pos2> {
    fn walk(shape: &egui::Shape, out: &mut Vec<egui::Pos2>) {
        match shape {
            egui::Shape::Rect(r) if r.fill == egui::Color32::WHITE && (r.rect.width() - 9.0).abs() < 0.1 => {
                out.push(r.rect.center())
            }
            egui::Shape::Vec(inner) => inner.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    h.output().shapes.iter().for_each(|clipped| walk(&clipped.shape, &mut out));
    out.sort_by(|a, b| a.x.total_cmp(&b.x));
    out.dedup_by(|a, b| (a.x - b.x).abs() < 0.5 && (a.y - b.y).abs() < 0.5);
    out
}

const WORDS: &[u8] = b"BT /F1 12 Tf 72 700 Td (Hello big wide world there) Tj ET";

/// **Drag the right edge in and the words fold onto more lines; drag it out
/// and they come back onto one** — the same words either way, and Apply
/// writes what the box showed.
#[test]
fn dragging_the_right_edge_wraps_the_words_and_dragging_it_back_unwraps_them() {
    let mut h = editing_first_thing_of(WORDS);
    h.run_steps(2);
    let grips_now = grips(&h);
    assert_eq!(grips_now.len(), 2, "the box has no grips: {grips_now:?}");
    let (left, right) = (grips_now[0], grips_now[1]);
    assert!(!h.state().tab().editing_run.as_ref().unwrap().buffer.contains('\n'));

    drag(&mut h, right, right - egui::vec2((right.x - left.x) * 0.55, 0.0));
    h.run_steps(3);
    let narrow = h.state().tab().editing_run.as_ref().unwrap().clone();
    assert!(narrow.box_resize.width_pt.is_some(), "the drag did not set a width");
    assert!(narrow.buffer.contains('\n'), "narrowing the box did not wrap the words: {:?}", narrow.buffer);
    assert_eq!(narrow.buffer.replace('\n', " "), "Hello big wide world there", "the words changed");

    let now = grips(&h);
    let back_out = now.last().copied().expect("a grip");
    drag(&mut h, back_out, back_out + egui::vec2(400.0, 0.0));
    h.run_steps(3);
    let wide = h.state().tab().editing_run.as_ref().unwrap().clone();
    assert!(!wide.buffer.contains('\n'), "widening the box left the words folded: {:?}", wide.buffer);
    assert_eq!(wide.buffer, "Hello big wide world there");
}

#[test]
fn a_narrowed_box_applies_as_the_lines_it_showed() {
    let mut h = editing_first_thing_of(WORDS);
    h.run_steps(2);
    let grips_now = grips(&h);
    let (left, right) = (grips_now[0], grips_now[1]);
    drag(&mut h, right, right - egui::vec2((right.x - left.x) * 0.6, 0.0));
    h.run_steps(3);
    let lines = h.state().tab().editing_run.as_ref().unwrap().buffer.matches('\n').count() + 1;
    assert!(lines >= 2);

    h.state_mut().apply_editing_page();
    h.run_steps(2);
    assert!(errors(&h).is_empty(), "{:?}", errors(&h));
    let page = page_text(&mut h);
    for word in ["Hello", "big", "wide", "world", "there"] {
        assert_eq!(page.matches(word).count(), 1, "{word} is not on the page exactly once: {page:?}");
    }
}

/// Dragging the left edge moves where the run starts.
#[test]
fn dragging_the_left_edge_moves_the_run_with_it() {
    let mut h = editing_first_thing_of(WORDS);
    h.run_steps(2);
    let before = h.state().tab().doc.as_ref().unwrap().session.text_runs(0).unwrap().remove(0).origin.x;
    let left = grips(&h)[0];
    drag(&mut h, left, left + egui::vec2(30.0, 0.0));
    h.run_steps(3);
    let shift = h.state().tab().editing_run.as_ref().unwrap().box_resize.left_shift_pt;
    assert!(shift > 1.0, "the left grip did not move the box: {shift}");

    h.state_mut().apply_editing_page();
    h.run_steps(2);
    assert!(errors(&h).is_empty(), "{:?}", errors(&h));
    let after = h.state().tab().doc.as_ref().unwrap().session.text_runs(0).unwrap().remove(0).origin.x;
    assert!((after - (before + shift)).abs() < 1.0, "run started at {before}, box moved {shift}, now at {after}");
}

/// A new text box has all eight handles, and each moves its own edges.
#[test]
fn a_new_text_box_is_resized_from_its_eight_handles() {
    let mut h = harness("text-lines.pdf");
    h.state_mut().begin_text_box(0, AppPoint { x: 20.0, y: 100.0 }, AppPoint { x: 120.0, y: 160.0 }).expect("a box");
    h.run_steps(3);
    let all = grips(&h);
    assert_eq!(all.len(), 8, "the box has {} handles: {all:?}", all.len());

    let before = h.state().tab().new_text_box.as_ref().unwrap().rect;
    let corner = all.iter().copied().max_by(|a, b| (a.x + a.y).total_cmp(&(b.x + b.y))).unwrap();
    drag(&mut h, corner, corner + egui::vec2(60.0, 30.0));
    h.run_steps(3);
    let grown = h.state().tab().new_text_box.as_ref().unwrap().rect;
    // The page re-fits while the panel settles, so only the size of the move is
    // checked: a 60 x 30 px drag, at a zoom of about two, is about 30 x 15 points.
    assert!((15.0..45.0).contains(&(grown.right - before.right)), "right edge: {before:?} -> {grown:?}");
    assert!((7.0..23.0).contains(&(grown.bottom - before.bottom)), "bottom edge: {before:?} -> {grown:?}");
    assert_eq!((grown.left, grown.top), (before.left, before.top), "the far corner moved");

    // And the left edge on its own moves only the left edge.
    let left = grips(&h).into_iter().min_by(|a, b| a.x.total_cmp(&b.x)).unwrap();
    drag(&mut h, left, left + egui::vec2(20.0, 0.0));
    h.run_steps(3);
    let narrowed = h.state().tab().new_text_box.as_ref().unwrap().rect;
    assert!(narrowed.left > grown.left + 5.0, "{grown:?} -> {narrowed:?}");
    assert_eq!((narrowed.right, narrowed.top, narrowed.bottom), (grown.right, grown.top, grown.bottom));
}

// ---- the Text Style panel ----

/// The panel has the controls of a text editor and no position row.
#[test]
fn the_style_panel_has_the_text_controls_and_no_position_row() {
    let mut h = editing_first_thing_of(WORDS);
    h.run_steps(2);
    for label in [
        "Bold", "Italic", "Underline", "Strikethrough", "Superscript", "Subscript", "Align left", "Align centre",
        "Align right", "Justify", "Line spacing",
    ] {
        assert!(h.query_all_by_label(label).next().is_some(), "no `{label}` control in the panel");
    }
    for gone in ["Position", "Page Left", "Page Center", "Page Right", "Align in box"] {
        assert!(h.query_all_by_label(gone).next().is_none(), "`{gone}` is still in the panel");
    }
}

/// Bold is the same family's own Bold face — not the font left as it was.
#[test]
fn bold_picks_the_bold_face_of_the_same_family_and_says_when_there_is_none() {
    let mut h = editing_first_thing_of(WORDS);
    h.state_mut().pick_style_face(Some("Montserrat-Regular"), true, false);
    let face = h.state().tab().editing_run.as_ref().unwrap().style.face.clone();
    assert!(face.as_deref().is_some_and(|f| f.to_ascii_lowercase().contains("bold")), "not a bold face: {face:?}");

    let before = face;
    h.state_mut().pick_style_face(Some("Zzyzx Sans"), true, false);
    assert_eq!(h.state().tab().editing_run.as_ref().unwrap().style.face, before, "the face changed with no bold to change to");
    let said = errors(&h);
    assert!(said.last().is_some_and(|e| e.contains("no bold face of zzyzx sans")), "{said:?}");
}

/// A new text box's alignment buttons set its alignment; Justify, which
/// does nothing for a box yet, is there greyed and changes nothing.
#[test]
fn a_new_box_takes_its_alignment_from_the_style_panel() {
    let mut h = harness("text-lines.pdf");
    h.state_mut().begin_text_box(0, AppPoint { x: 20.0, y: 100.0 }, AppPoint { x: 120.0, y: 160.0 }).expect("a box");
    h.run_steps(3);
    assert_eq!(h.state().tab().new_text_box.as_ref().unwrap().align, TextAlign::Left);
    let centre = h.get_by_label("Align centre").rect().center();
    click(&mut h, centre);
    h.run_steps(2);
    assert_eq!(h.state().tab().new_text_box.as_ref().unwrap().align, TextAlign::Center);
    let right = h.get_by_label("Align right").rect().center();
    click(&mut h, right);
    h.run_steps(2);
    assert_eq!(h.state().tab().new_text_box.as_ref().unwrap().align, TextAlign::Right);
    let justify = h.get_by_label("Justify").rect().center();
    click(&mut h, justify);
    h.run_steps(2);
    assert_eq!(h.state().tab().new_text_box.as_ref().unwrap().align, TextAlign::Right);
}

/// A paragraph's lines are the page's own — no grips on it.
#[test]
fn a_paragraph_has_no_resize_grips() {
    let mut h = editing_first_thing_of(
        b"BT /F1 12 Tf 14 TL 72 700 Td (First line of the words that run on) Tj T* \
          (Second line of the words that run) Tj T* (Third line of the words that go) Tj ET",
    );
    h.run_steps(2);
    assert!(grips(&h).is_empty(), "a paragraph got grips: {:?}", grips(&h));
}

#[test]
fn the_paper_round_a_copied_shape_goes_clear_but_white_inside_it_stays() {
    // 5x5: a black ring on white, one white pixel inside the ring.
    let mut rgba = vec![255u8; 5 * 5 * 4];
    for (x, y) in [(1, 1), (2, 1), (3, 1), (1, 2), (3, 2), (1, 3), (2, 3), (3, 3)] {
        let at = (y * 5 + x) * 4;
        rgba[at..at + 3].copy_from_slice(&[0, 0, 0]);
    }
    clear_paper_around(&mut rgba, 5, 5);
    let alpha = |x: usize, y: usize| rgba[(y * 5 + x) * 4 + 3];
    assert_eq!(alpha(0, 0), 0, "the corner paper stayed");
    assert_eq!(alpha(4, 2), 0, "the paper on the edge stayed");
    assert_eq!(alpha(1, 1), 255, "ink was cleared");
    assert_eq!(alpha(2, 2), 255, "the white inside the ring was cleared");
}

/// Search & Replace goes through the same engine call and prints its own
/// copy of the error.
#[test]
fn replace_current_words_a_refusal_the_same_way() {
    let mut h = opened("kerned-replace.pdf", &page_with(KERNED, "", &[]));
    // Replace now shows a match before it replaces it (the Find fixes): the
    // first press shows, the second does the replacing and meets the refusal.
    h.state_mut().replace_current("Hello", "Jello").expect("the first press shows the match");
    let said = h.state_mut().replace_current("Hello", "Jello").expect_err("the kerned run was edited");
    assert!(!said.contains("not implemented"), "reads as a missing feature: {said}");
    assert!(said.contains("refuses rather than risk"), "{said}");
}

/// So does Check Spelling's change, which retypes a run the same way.
#[test]
fn a_spelling_change_words_a_refusal_the_same_way() {
    let mut h = opened("kerned-spelling.pdf", &page_with(KERNED, "", &[]));
    let object = h.state().tab().doc.as_ref().expect("open").session.text_runs(0).expect("runs")[0].object;
    let said = h.state_mut().apply_spelling_change(0, object, "Hello", "Jello").expect_err("the kerned run was edited");
    assert!(!said.contains("not implemented"), "reads as a missing feature: {said}");
    assert!(said.contains("refuses rather than risk"), "{said}");
}

// ------------------------------------------------- the error stays visible --

/// **An error stays on the bar while the tool that caused it is armed.**
/// Reported from use: the red line was replaced in the same frame by the
/// tool's own "click the words to change", so a click that found no text
/// showed nothing at all unless the history happened to be open. The
/// prompt is appended after the error, not dropped — an armed tool still
/// announces itself.
#[test]
fn a_click_on_bare_paper_stays_on_screen_beside_the_prompt_while_the_tool_is_armed() {
    let mut h = harness("text-lines.pdf");
    h.state_mut().command_open = false;
    h.state_mut().submit("edittext");
    h.run_steps(1);

    let below = bare_paper_below_the_text(&h);
    click(&mut h, below);
    h.run_steps(2);

    let last = history(&h).pop().expect("nothing was said");
    assert!(last.1 && last.0.contains("no text there"), "the error was not the last thing said: {last:?}");
    assert!(h.state().tab().tool.is_some(), "the tool should still be in hand");
    assert!(
        h.query_by_label_contains("no text there").is_some(),
        "the error is not on screen — the bar shows only the prompt"
    );
    assert!(
        h.query_by_label_contains("click the words to change").is_some(),
        "an armed tool stopped saying that it is armed"
    );
}

/// The other half: the error is what the bar shows *until the next thing is
/// said*, not for as long as the tool stays armed.
#[test]
fn a_stale_error_does_not_stay_on_the_bar_once_the_tool_is_armed_afresh() {
    let mut h = harness("text-lines.pdf");
    h.state_mut().command_open = false;
    h.state_mut().submit("edittext");
    h.run_steps(1);
    let below = bare_paper_below_the_text(&h);
    click(&mut h, below);
    h.run_steps(2);
    assert!(h.query_by_label_contains("no text there").is_some(), "setup: the error should be showing");

    h.state_mut().submit("edittext");
    h.run_steps(2);
    assert!(
        h.query_by_label_contains("no text there").is_none(),
        "an old error is still on the bar after the tool was armed again"
    );
    assert!(h.query_by_label_contains("click the words to change").is_some());
}

/// **The click that closes the editor picks once.** Reported from use, in
/// the logs: one click away from an open editor logged two identical "no
/// text there" errors a few milliseconds apart — the click-away block
/// re-armed the tool and resolved the pick, and the armed-tool block further
/// down the same frame took the same click again.
#[test]
fn the_click_that_closes_an_editor_picks_once_not_twice() {
    let mut h = harness("text-lines.pdf");
    h.state_mut().command_open = false;
    h.state_mut().submit("edittext");
    h.run_steps(1);
    let at = a_character_on_screen(&mut h);
    click(&mut h, at);
    h.run_steps(2);
    h.state_mut().tab_mut().editing_run.as_mut().expect("no run was picked").buffer = "REPLACED".into();

    let away = clear_of_the_editor(&h);
    click(&mut h, away);
    h.run_steps(2);

    let misses: Vec<String> = errors(&h).into_iter().filter(|e| e.starts_with("no text there")).collect();
    assert_eq!(misses.len(), 1, "one click, but it picked {} times: {misses:?}", misses.len());
    assert!(h.query_by_label_contains("no text there").is_some(), "the miss is not on screen");
    // And the edit it closed still landed — this changes nothing about success.
    let app = h.state_mut();
    app.tab_mut().doc.as_mut().unwrap().caches.text = None;
    let page = app.characters(0).map(|c| c.text()).unwrap_or_default();
    assert!(page.contains("REPLACED"), "clicking away did not apply the change:\n{page}");
}

/// **A click-away apply that is refused shows the refusal**, and does not
/// then pick under the same click: whatever that pick said would replace it.
///
/// **Changed with the reason** (a refused apply must not throw the typing away):
/// the editor used to be closed by the refusal, the tool put back in hand quietly
/// for the next click. It stays now, with its words, and the refusal beside its
/// Apply button; the tool is not re-armed, there being no pick to resolve. The
/// next click away from the same edit lets it go (with the words on the
/// clipboard) and **is** the pick of what is under it — the tool back in hand as
/// it always was. What this protects is unchanged: the refusal is shown, and the
/// click does not pick over it.
#[test]
fn a_refused_click_away_apply_leaves_its_refusal_on_screen_and_the_tool_armed() {
    let mut h = editing_the_kerned_page("Jello");
    let away = clear_of_the_editor(&h);
    click(&mut h, away);
    h.run_steps(2);

    let kept = h.state().tab().editing_run.as_ref().expect("the refused edit's box was closed with its words");
    assert_eq!(kept.buffer, "Jello", "the typing was thrown away");
    let said = errors(&h);
    assert_eq!(said.len(), 1, "one click, one error — the refusal: {said:?}");
    assert!(said[0].starts_with("that text is drawn in a way this cannot edit"), "{said:?}");
    assert!(h.query_all_by_label_contains("refuses rather than risk").next().is_some(), "the refusal is not on screen");
    assert!(h.state().tab().tool.is_none(), "nothing was picked under the click, so no pick is waiting");

    // The second click away from the same refused edit lets it go, and picks.
    click(&mut h, away);
    h.run_steps(2);
    assert!(h.state().tab().editing_run.is_none(), "the same refused edit was kept a second time");
    assert!(h.state().text_to_offer.is_none() || h.state().text_to_offer.as_deref() == Some("Jello"));
    assert!(h.state().tab().tool.is_some(), "the tool should be back in hand for the next click");
    assert!(
        h.query_all_by_label_contains("click the words to change").next().is_some()
            || h.query_all_by_label_contains("no text there").next().is_some(),
        "the click was not answered as a pick"
    );
}

// ------------------------------------------------------- the typeface advice --

/// Rectangles the size of letters in a row, drawn as filled paths: what
/// type converted to outlines looks like to the engine, with no typeface in
/// the world that could read it.
fn drawn_letters() -> String {
    (0..16).map(|i| format!("{} 500 6 9 re f\n", 100 + i * 9)).collect()
}

/// A point on the fourth drawn letter, and one on bare paper, in page points.
const ON_A_LETTER: (f64, f64) = (100.0 + 3.0 * 9.0 + 3.0, 792.0 - 504.5);
const ON_BARE_PAPER: (f64, f64) = (450.0, 120.0);

fn pick(h: &mut Harness<'static, PagifyApp>, at: (f64, f64)) -> Result<String, String> {
    h.state_mut().pick_text_run(0, AppPoint { x: at.0, y: at.1 })
}

fn kind_of(h: &Harness<'static, PagifyApp>) -> pdf_core::document::PageTextKind {
    h.state().tab().doc.as_ref().expect("open").session.classify(0).expect("classify").kind
}

const THE_ADVICE: &str = "outlinedfont add";

/// **A blank spot is not a missing typeface.** Reported from use, in the
/// logs: every click on bare paper of a page that is partly a photograph
/// (classified `Hybrid`) was told the words "look drawn rather than
/// written" and to add a font — twenty-five times, none of them near any
/// drawn word.
#[test]
fn a_blank_spot_on_a_page_with_a_photograph_is_not_blamed_on_a_missing_typeface() {
    let image = b"<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray \
                  /BitsPerComponent 8 /Length 1 >>\nstream\n\x80\nendstream"
        .to_vec();
    let content = b"q 612 0 0 792 0 0 cm /Im1 Do Q\nBT /F1 12 Tf 72 700 Td (Hello there world) Tj ET";
    let mut h = opened("photo.pdf", &page_with(content, "/XObject << /Im1 6 0 R >>", &[image]));
    assert_eq!(kind_of(&h), pdf_core::document::PageTextKind::Hybrid, "the fixture is not a hybrid page");

    let said = pick(&mut h, ON_BARE_PAPER).expect_err("a blank spot picked something");
    assert!(said.starts_with("no text there"), "{said}");
    assert!(!said.contains(THE_ADVICE), "a blank spot was blamed on a missing typeface: {said}");
}

/// The same, where the page really does hold drawn letters — elsewhere on it.
#[test]
fn a_blank_spot_away_from_the_drawn_letters_is_not_blamed_on_a_missing_typeface() {
    // Outlined (no text at all) and Hybrid (text beside the outlines): both
    // kinds used to answer every blank click with the advice.
    for (name, text, kind) in [
        ("outlined-only.pdf", "", pdf_core::document::PageTextKind::Outlined),
        ("outlined-beside-text.pdf", "BT /F1 12 Tf 72 700 Td (Hello there world) Tj ET\n", pdf_core::document::PageTextKind::Hybrid),
    ] {
        let content = format!("{text}{}", drawn_letters());
        let mut h = opened(name, &page_with(content.as_bytes(), "", &[]));
        assert_eq!(kind_of(&h), kind, "{name} is not the kind of page this test is about");

        let said = pick(&mut h, ON_BARE_PAPER).expect_err("a blank spot picked something");
        assert!(said.starts_with("no text there"), "{name}: {said}");
        assert!(!said.contains(THE_ADVICE), "{name}: a blank spot was blamed on a missing typeface: {said}");
    }
}

/// **The advice stays where it is true**: a click on the drawn letters
/// themselves, which no typeface on hand can read, still says what would
/// let them be read.
#[test]
fn a_click_on_the_drawn_letters_themselves_still_gets_the_typeface_advice() {
    for (name, text) in [
        ("outlined-only-click.pdf", ""),
        ("outlined-beside-text-click.pdf", "BT /F1 12 Tf 72 700 Td (Hello there world) Tj ET\n"),
    ] {
        let content = format!("{text}{}", drawn_letters());
        let mut h = opened(name, &page_with(content.as_bytes(), "", &[]));
        let said = pick(&mut h, ON_A_LETTER).expect_err("the rectangles were read as words");
        assert!(said.contains(THE_ADVICE), "{name}: drawn letters were not given the advice: {said}");
    }
}

// ---- the Properties panel keeps the width it is given ----

fn properties_width(h: &Harness<'static, PagifyApp>) -> f32 {
    egui::PanelState::load(&h.ctx, egui::Id::new("properties_panel"))
        .expect("the properties panel")
        .outer_rect
        .width()
}

/// **The panel does not grow by itself.** Reported from use: opening Edit Text
/// made the Properties panel take the whole window, and dragging it narrower
/// put it straight back. A panel is as wide as its content, and the Text
/// Style's first row fills "what is left" after items whose size it assumed:
/// the colour button was 14 pt wider than assumed, so the content outgrew the
/// panel by that much on every frame, for as long as the window had room.
#[test]
fn the_properties_panel_does_not_grow_by_itself() {
    let mut h = editing_first_thing_of(WORDS);
    h.run_steps(3);
    let settled = properties_width(&h);
    h.run_steps(40);
    let later = properties_width(&h);
    assert!((later - settled).abs() < 0.5, "the panel grew from {settled} to {later} in 40 frames with nothing touched");
    assert!(later < 1400.0 * 0.4, "the panel is {later} pt wide in a 1400 pt window");
}

/// **The colour button is on the panel, at the size the row is laid out for.**
/// It was egui's own 40 pt button where 26 was assumed, which is what pushed
/// it past the panel's edge (and past the window's) and widened the panel.
#[test]
fn the_colour_button_is_the_size_the_row_was_laid_out_for_and_inside_the_panel() {
    let mut h = editing_first_thing_of(WORDS);
    h.run_steps(5);
    let panel = egui::PanelState::load(&h.ctx, egui::Id::new("properties_panel")).unwrap().outer_rect;
    let colour = h
        .get_all_by_role(egui::accesskit::Role::ColorWell)
        .map(|node| node.rect())
        .find(|rect| rect.center().x > panel.left())
        .expect("the colour button is in the panel");
    assert!((colour.width() - 26.0).abs() < 0.5, "the colour button is {} pt wide, not the 26 the row is laid out for", colour.width());
    assert!(panel.contains_rect(colour), "the colour button {colour:?} is not inside the panel {panel:?}");
}

/// The same for a new text box, which shows the same Text Style.
#[test]
fn the_new_box_properties_panel_does_not_grow_by_itself_either() {
    let mut h = harness("text-lines.pdf");
    h.state_mut().begin_text_box(0, AppPoint { x: 20.0, y: 100.0 }, AppPoint { x: 120.0, y: 160.0 }).expect("a box");
    h.run_steps(3);
    let settled = properties_width(&h);
    h.run_steps(40);
    assert!((properties_width(&h) - settled).abs() < 0.5, "the panel grew from {settled} to {}", properties_width(&h));
}

/// **A width the person drags it to stays.** Wider, then back to what it was:
/// each is where it was put, and neither is undone a few frames later.
#[test]
fn a_width_dragged_to_stays() {
    let mut h = editing_first_thing_of(WORDS);
    h.run_steps(5);
    let start = properties_width(&h);
    let edge_y = 500.0;
    let edge = |h: &Harness<'static, PagifyApp>| {
        egui::PanelState::load(&h.ctx, egui::Id::new("properties_panel")).unwrap().outer_rect.left()
    };

    let from = egui::pos2(edge(&h), edge_y);
    drag(&mut h, from, from - egui::vec2(100.0, 0.0));
    h.run_steps(30);
    let wider = properties_width(&h);
    assert!((wider - (start + 100.0)).abs() < 3.0, "dragged 100 pt wider from {start}, now {wider}");

    let from = egui::pos2(edge(&h), edge_y);
    drag(&mut h, from, from + egui::vec2(100.0, 0.0));
    h.run_steps(30);
    let back = properties_width(&h);
    assert!((back - start).abs() < 3.0, "dragged back by 100 pt from {wider}, expected about {start}, now {back}");
}

