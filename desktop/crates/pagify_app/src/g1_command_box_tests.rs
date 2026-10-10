#[allow(unused_imports)]
use super::ui_tests::{fixture, harness, harness_from, harness_60fps, click, drag};
use super::*;
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;

fn box_id() -> egui::Id {
    egui::Id::new(COMMAND_INPUT)
}

fn focus_box(h: &mut Harness<'static, PagifyApp>) {
    h.ctx.memory_mut(|m| m.request_focus(box_id()));
    h.run_steps(2);
}

fn box_has_focus(h: &Harness<'static, PagifyApp>) -> bool {
    h.ctx.memory(|m| m.has_focus(box_id()))
}

fn key_event(key: egui::Key, pressed: bool) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: Default::default(),
    }
}

fn press(h: &mut Harness<'static, PagifyApp>, key: egui::Key) {
    h.input_mut().events.push(key_event(key, true));
    h.input_mut().events.push(key_event(key, false));
    h.run_steps(2);
}

/// Typing as a keyboard delivers it. A Space is **both** a key press and
/// a text event (egui-winit 0.36.1 pushes the two for one press), and the
/// filter this guards removed both — a test that sent only the text
/// would never have met it.
fn type_text(h: &mut Harness<'static, PagifyApp>, text: &str) {
    for ch in text.chars() {
        if ch == ' ' {
            h.input_mut().events.push(key_event(egui::Key::Space, true));
        }
        h.input_mut().events.push(egui::Event::Text(ch.to_string()));
        h.run_steps(1);
        if ch == ' ' {
            h.input_mut().events.push(key_event(egui::Key::Space, false));
            h.run_steps(1);
        }
    }
}

fn errors(h: &Harness<'static, PagifyApp>) -> Vec<String> {
    h.state()
        .cmd
        .history()
        .iter()
        .filter(|e| e.kind == pagify_shell::command::Kind::Error)
        .map(|e| e.text.clone())
        .collect()
}

fn echoes(h: &Harness<'static, PagifyApp>) -> Vec<String> {
    h.state()
        .cmd
        .history()
        .iter()
        .filter(|e| e.kind == pagify_shell::command::Kind::Echo)
        .map(|e| e.text.clone())
        .collect()
}

fn click_label(h: &mut Harness<'static, PagifyApp>, label: &str) {
    h.get_by_label(label).click();
    h.run_steps(3);
}

fn on_organize(name: &str) -> Harness<'static, PagifyApp> {
    let mut h = harness(name);
    h.state_mut().tab_mut().ribbon = Tab::Organize;
    h.run_steps(3);
    h
}

/// `extract` and then a Space used to run `extract` there and then — the
/// usage error, and the word the user typed gone — so no line of more
/// than one word could be typed at all.
#[test]
fn a_space_typed_into_the_command_box_is_a_space_and_runs_nothing() {
    let mut h = harness("pages-ladder.pdf");
    focus_box(&mut h);

    type_text(&mut h, "extract 1-3");

    assert_eq!(h.state().cmd.input(), "extract 1-3", "the Space did not type a space");
    assert!(echoes(&h).is_empty(), "something was run while typing: {:?}", echoes(&h));
    assert!(errors(&h).is_empty(), "typing a Space raised an error: {:?}", errors(&h));
}

/// The whole of it end to end: a typed line with spaces runs on Enter, and
/// is still recorded — only the bare dialog-or-prefill lines are not.
#[test]
fn a_typed_line_with_spaces_runs_on_enter_and_is_still_recorded() {
    let out = std::env::temp_dir().join(format!("pagify-g1-extract-{}.pdf", std::process::id()));
    let _ = std::fs::remove_file(&out);

    let mut h = harness("pages-ladder.pdf");
    h.state_mut().submit("record g1test");
    assert!(h.state().recorder.is_recording());
    focus_box(&mut h);

    type_text(&mut h, &format!("extract 1-2 {}", out.display()));
    press(&mut h, egui::Key::Enter);

    let wrote = out.is_file();
    let _ = std::fs::remove_file(&out);
    assert!(wrote, "the typed extract did not write its file; errors: {:?}", errors(&h));
    assert_eq!(h.state().recorder.steps(), 1, "the fully specified line was not recorded");
}

/// Ctrl+F puts `find ` in the box. A search for two words could not be
/// typed, because the first Space ran `find two`.
#[test]
fn a_search_of_two_words_can_be_typed_after_ctrl_f() {
    let mut h = harness("pages-ladder.pdf");
    focus_box(&mut h);
    // An earlier command: it leaves egui's stored caret at 0, which is
    // what a prefill that only sets the text then types in front of.
    type_text(&mut h, "fit");
    press(&mut h, egui::Key::Enter);

    // egui reads the held modifiers from `ModifiersChanged`, not from the
    // modifiers a key event carries.
    h.input_mut().events.push(egui::Event::ModifiersChanged(egui::Modifiers::COMMAND));
    h.input_mut().events.push(egui::Event::Key {
        key: egui::Key::F,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::COMMAND,
    });
    h.run_steps(2);
    h.input_mut().events.push(egui::Event::ModifiersChanged(Default::default()));
    h.run_steps(1);
    assert_eq!(h.state().cmd.input(), "find ");

    type_text(&mut h, "two words");
    assert_eq!(h.state().cmd.input(), "find two words", "typed text did not land after the prefill");
}

/// Swap needs two numbers, so after the Space fix its prefill is the way
/// to give them. Typed after an earlier command, they landed *in front of*
/// the prefilled word: `1 2swappages `.
#[test]
fn swap_takes_its_two_pages_typed_after_the_prefill() {
    let mut h = on_organize("pages-ladder.pdf");
    focus_box(&mut h);
    type_text(&mut h, "fit");
    press(&mut h, egui::Key::Enter);

    click_label(&mut h, "Swap");
    assert_eq!(h.state().cmd.input(), "swappages ");
    assert!(box_has_focus(&h), "the box did not take focus");

    type_text(&mut h, "1 2");
    assert_eq!(h.state().cmd.input(), "swappages 1 2");
}

/// Up recalls an earlier line into the box; what is typed next belongs at
/// the end of it, as in any shell.
#[test]
fn typing_after_a_recalled_line_goes_on_its_end() {
    let mut h = harness("pages-ladder.pdf");
    focus_box(&mut h);
    type_text(&mut h, "zoom fit");
    press(&mut h, egui::Key::Enter);
    press(&mut h, egui::Key::ArrowUp);
    assert_eq!(h.state().cmd.input(), "zoom fit", "Up did not recall the line");

    type_text(&mut h, "x");
    assert_eq!(h.state().cmd.input(), "zoom fitx");
}

/// Delete, Extract and Move ran their bare command and got a red usage
/// error and an empty box. Now Delete and Move wait in the box for their
/// pages, and Extract opens a dialog for its pages and its file.
#[test]
fn an_extract_button_asks_in_a_dialog_and_leaves_the_box_alone() {
    let mut h = on_organize("pages-ladder.pdf");
    focus_box(&mut h);
    type_text(&mut h, "fit");
    press(&mut h, egui::Key::Enter);
    let before = echoes(&h);

    click_label(&mut h, "Extract");

    assert_eq!(echoes(&h), before, "Extract ran a command");
    assert!(errors(&h).is_empty(), "Extract raised an error: {:?}", errors(&h));
    assert_eq!(h.state().cmd.input(), "", "Extract left words in the box");
    assert!(h.state().tab().panels.extract_ask.is_some(), "the dialog did not open");
    assert_eq!(h.state().tab().doc.as_ref().map(|d| d.page_count), Some(5), "Extract changed the document");
}

#[test]
fn a_button_whose_bare_command_cannot_succeed_waits_for_its_arguments() {
    for (label, verb) in [("Delete", "deletepage"), ("Move", "movepage")] {
        let mut h = on_organize("pages-ladder.pdf");
        // Something typed before, so the stored caret is not the end.
        focus_box(&mut h);
        type_text(&mut h, "fit");
        press(&mut h, egui::Key::Enter);
        let before = echoes(&h);

        click_label(&mut h, label);

        assert_eq!(echoes(&h), before, "{label} ran a command");
        assert!(errors(&h).is_empty(), "{label} raised an error: {:?}", errors(&h));
        assert_eq!(h.state().cmd.input(), format!("{verb} "), "{label} did not leave its verb in the box");
        assert!(box_has_focus(&h), "{label} did not focus the box");
        let last = h.state().cmd.history().last().expect("a usage line");
        assert_eq!(last.kind, pagify_shell::command::Kind::Info, "{label}: the usage is not an info line");
        assert!(last.text.starts_with(&format!("usage: {verb}")), "{label}: {:?}", last.text);
        assert_eq!(h.state().tab().doc.as_ref().map(|d| d.page_count), Some(5), "{label} changed the document");

        // And the pages typed next follow the verb.
        type_text(&mut h, "2");
        assert_eq!(h.state().cmd.input(), format!("{verb} 2"), "{label}: typing did not land at the end");
    }
}

/// A click that only fills the box is not a command: a recording does not
/// take it, and a tool armed for a pick is not cancelled by it.
#[test]
fn a_click_that_only_fills_the_box_is_not_recorded_and_keeps_the_armed_tool() {
    let mut h = on_organize("pages-ladder.pdf");
    h.state_mut().submit("record g1test");
    h.state_mut().submit("measure");
    assert!(h.state().tab().tool.is_some(), "measure did not arm");
    let steps = h.state().recorder.steps();

    click_label(&mut h, "Delete");

    assert_eq!(h.state().recorder.steps(), steps, "the fill was recorded");
    assert!(h.state().tab().tool.is_some(), "the fill cancelled the armed tool");
    assert!(
        !h.state().cmd.history().iter().any(|e| e.text.contains("cancelled")),
        "the fill said it cancelled a pick"
    );
}

/// The start screen's Tool Wizard cards send `extract` and `import` bare
/// through the same handler — the first Extract a tester meets.
#[test]
fn the_extract_card_with_nothing_open_says_to_open_a_pdf_first() {
    let mut app = PagifyApp::new(None);
    app.outlined_fonts = Default::default();
    let mut h = harness_from(app);
    assert!(h.state().tab().doc.is_none());

    // Merge is the card that would otherwise open a native picker, which
    // nothing in this suite may do: with nothing open it must not.
    for blurb in ["Pull a range of pages", "Bring pages in from another document"] {
        h.get_by_label_contains(blurb).click();
        h.run_steps(3);

        assert!(errors(&h).is_empty(), "{blurb}: raised an error: {:?}", errors(&h));
        assert!(echoes(&h).is_empty(), "{blurb}: ran a command: {:?}", echoes(&h));
        let last = h.state().cmd.history().last().expect("a message");
        assert_eq!(last.kind, pagify_shell::command::Kind::Info);
        assert!(last.text.contains("open a PDF first"), "{blurb}: {:?}", last.text);
    }
}

/// The decision itself, with no window — and the one branch the UI tests
/// cannot reach, because Merge's bare `import` with a document open
/// raises a blocking native picker and nothing in this suite may click it.
#[test]
fn a_click_runs_fills_or_asks_for_a_file_by_what_the_bare_command_would_get() {
    // Runs as it always did.
    for command in ["insertpage", "reversepages", "thumbnails", "rotatepages", "line", "open", "saveas", "extracttext"] {
        assert_eq!(ribbon_click(command), RibbonClick::Run, "{command}");
    }
    // The table's own fill-and-wait buttons keep waiting, with no usage line added.
    for command in ["swappages ", "croppages all ", "note ", "calibrate ", "replay "] {
        assert_eq!(ribbon_click(command), RibbonClick::Fill(None), "{command}");
    }
    // A bare command the parser refuses fills the box instead, usage line in hand.
    for command in ["deletepage", "movepage"] {
        match ribbon_click(command) {
            RibbonClick::Fill(Some(usage)) => {
                assert!(usage.starts_with(&format!("usage: {command}")), "{command}: {usage}")
            }
            other => panic!("{command}: {other:?}"),
        }
    }
    // A fully specified line is a command and runs, as does a typed one with its path.
    for command in ["extract 1-3 out.pdf", "deletepage 2", "movepage 5 2", "import other.pdf 1-2"] {
        assert_eq!(ribbon_click(command), RibbonClick::Run, "{command}");
    }
    // A verb that takes a file asks for one.
    assert_eq!(ribbon_click("import"), RibbonClick::PickFile);
    // Extract needs pages and a file: it gets a dialog, not a usage line.
    assert_eq!(ribbon_click("extract"), RibbonClick::Extract);
}

/// The survey behind the rule: of every ribbon button, only these three
/// run a bare command the parser refuses. Pinned so a button added later
/// that needs words is a decision somebody made, not a red error a tester
/// finds — the rule above catches it either way.
#[test]
fn only_delete_extract_and_move_run_a_bare_command_the_parser_refuses() {
    let mut asked: Vec<&str> = Vec::new();
    for tab in Tab::ALL {
        for (_glyph, _label, command) in tab.leading().iter().chain(tab.buttons()) {
            if matches!(
                ribbon_click(command.text()),
                RibbonClick::Fill(Some(_)) | RibbonClick::PickFile | RibbonClick::Extract
            ) {
                asked.push(command.text());
            }
        }
    }
    asked.sort();
    asked.dedup();
    assert_eq!(asked, ["deletepage", "extract", "movepage"]);
}

/// **Reported from use (report 10): "Extract lacks a UI".** The card used to
/// leave `extract ` in the box for the pages and a path to be typed. It now
/// opens a dialog that asks for the pages, so nothing is run and the box is
/// left alone.
#[test]
fn the_extract_card_with_a_document_open_asks_for_its_pages_in_a_dialog() {
    let mut h = harness("pages-ladder.pdf");
    h.state_mut().tab_mut().ribbon = Tab::File;
    h.run_steps(3);

    h.get_by_label_contains("Pull a range of pages").click();
    h.run_steps(3);

    assert!(errors(&h).is_empty(), "raised an error: {:?}", errors(&h));
    assert!(echoes(&h).is_empty(), "ran a command: {:?}", echoes(&h));
    assert_eq!(h.state().cmd.input(), "", "the box was filled instead of a dialog opening");
    let ask = h.state().tab().panels.extract_ask.clone().expect("the dialog did not open");
    assert_eq!(ask.pages, "1", "with nothing selected it offers the page on screen");
    h.get_by_label("Extract pages");
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert!(h.state().tab().panels.extract_ask.is_none(), "Cancel left the dialog open");
}

#[test]
fn the_extract_dialog_offers_the_selected_pages_and_says_what_is_wrong_with_a_range() {
    let mut h = harness("pages-ladder.pdf");
    h.state_mut().tab_mut().organize_selected = vec![0, 1, 2, 4];
    h.state_mut().open_extract_dialog();
    h.run_steps(3);
    assert_eq!(h.state().tab().panels.extract_ask.clone().unwrap().pages, "1-3,5");

    // A range outside the document is refused in the dialog, which stays
    // open with the reason — the Save box is never reached.
    h.state_mut().tab_mut().panels.extract_ask.as_mut().unwrap().pages = "999".into();
    h.get_by_label("Extract\u{2026}").click();
    h.run_steps(3);
    let ask = h.state().tab().panels.extract_ask.clone().expect("the dialog closed on a bad range");
    assert!(ask.problem.is_some(), "no reason shown");
    assert_eq!(ask.pages, "999", "what was typed was thrown away");
}

/// Writing the pages over the document being read would replace the file
/// under the live session. Refused for the typed command as well as the box.
#[test]
fn extracting_over_the_open_document_is_refused_and_leaves_it_alone() {
    // A copy, so a regression here damages a scratch file and not a fixture.
    let dir = std::env::temp_dir().join(format!("pagify-extract-over-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let open = dir.join("open.pdf");
    std::fs::copy(crate::ui_tests::fixture("pages-ladder.pdf"), &open).unwrap();
    let mut h = crate::ui_tests::harness_at(open.to_str().unwrap()).expect("the copy did not open");
    let before = std::fs::read(&open).unwrap();
    assert!(h.state().is_open_document(&open));
    h.state_mut().extract("1", &open);
    h.run_steps(2);
    assert_eq!(std::fs::read(&open).unwrap(), before, "the open file was written");
    assert!(!errors(&h).is_empty(), "no refusal was shown");
}

#[test]
fn a_page_list_is_written_as_ranges() {
    assert_eq!(compact_page_spec(&[0, 1, 2, 6]), "1-3,7");
    assert_eq!(compact_page_spec(&[4, 2, 3, 3]), "3-5");
    assert_eq!(compact_page_spec(&[0]), "1");
    assert_eq!(compact_page_spec(&[]), "");
}
