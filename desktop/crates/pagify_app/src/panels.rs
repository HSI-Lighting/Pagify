//! The modal and floating-window panels `doc_tab/panels.rs` names: dialogs
//! that ask something (update, password, extract range, article box, web
//! link) and lists that are browsed and acted on (snippets, signatures,
//! bookmarks) — plus the shape-properties strip and the zoomed-in detail
//! overlay, which the review groups here too.

use crate::overlay::PageView;
use crate::{
    compact_page_spec, page_to_image, paint_signature, short, view_height, Awaiting, DetailTile, FindReplaceMode,
    ListAction, RenderJob, SignatureList, SignaturePad, SpellCheck, Tool,
};
use crate::spelling;
use crate::theme;
use pagify_shell::page_space::AppPoint;
use pagify_shell::verbs::{PageTarget, Verb};
use pdf_core::Rotation;

impl crate::PagifyApp {
    /// "A newer build is sitting in Dropbox — install it?" Modelled on
    /// `ask_about_unsaved`'s modal: a local decision set inside the closure,
    /// acted on once it returns.
    pub(crate) fn draw_update_prompt(&mut self, ctx: &egui::Context) {
        let Some((version, source)) = self.update_state.update_available.clone() else { return };
        // `Some(true)` = update now, `Some(false)` = later — just the two
        // outcomes this dialog has, not `Decision`'s three (that one is
        // `ask_about_unsaved`'s own, for a different question).
        let mut decision: Option<bool> = None;
        egui::Modal::new(egui::Id::new("update_available")).show(ctx, |ui| {
            ui.set_width(340.0);
            ui.heading("Update available");
            ui.add_space(6.0);
            ui.label(format!(
                "Pagify {version} is available — you have {}.",
                pagify_shell::VERSION
            ));
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.button("Update now").clicked() {
                    decision = Some(true);
                }
                if ui.button("Later").clicked() {
                    decision = Some(false);
                }
            });
        });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            decision = Some(false);
        }
        match decision {
            None => {}
            Some(false) => self.update_state.update_available = None,
            Some(true) => {
                // The same walk a plain `quit` makes — an update is a quit
                // that relaunches a newer build, not a second "is anything
                // unsaved" check of its own. `exit_program` is where it actually
                // happens, once every tab has agreed it is safe to close.
                self.update_state.update_available = None;
                self.pending_update = Some(source);
                self.act(Verb::Quit { force: false });
            }
        }
    }

    /// The Search & Replace panel — a search bar, a replacement bar, and a
    /// mode to pick what the two buttons below them do.
    ///
    /// **An ordinary floating window, not a modal.** Reported from use: the
    /// panel should not "hide the rest of the page" — a modal dims and
    /// blocks everything behind it, which is exactly wrong for a tool whose
    /// whole point is watching matches highlight on the page while it stays
    /// open. `egui::Window` neither dims nor blocks, and can be dragged
    /// clear of whatever it would otherwise sit over.
    ///
    /// **Reported from use: "an option to just search and replace one by
    /// one. and search without replacing."** The dropdown is what picks
    /// between the three: `Find` calls the same `find`/`find_step` the
    /// command box always could, `ReplaceOne` calls `replace_current` one
    /// match at a time, and `ReplaceAll` is the original single-button
    /// behaviour. A word's properties are kept by `replace_all`/
    /// `replace_current` themselves, not by anything drawn here.
    pub(crate) fn draw_find_replace(&mut self, ctx: &egui::Context) {
        let Some(mut panel) = self.tab_mut().panels.find_replace.take() else { return };

        let mut open = true;
        let mut done = false;
        let mut step: Option<bool> = None;
        let mut replace_one = false;
        let mut replace_all = false;

        egui::Window::new("Search & Replace")
            .id(egui::Id::new("find-replace-window"))
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .default_pos(egui::pos2(80.0, 80.0))
            .show(ctx, |ui| {
                ui.set_width(320.0);

                egui::ComboBox::from_id_salt("find-replace-mode")
                    .selected_text(match panel.mode {
                        FindReplaceMode::Find => "Find",
                        FindReplaceMode::ReplaceOne => "Replace one at a time",
                        FindReplaceMode::ReplaceAll => "Replace all",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut panel.mode, FindReplaceMode::Find, "Find");
                        ui.selectable_value(
                            &mut panel.mode,
                            FindReplaceMode::ReplaceOne,
                            "Replace one at a time",
                        );
                        ui.selectable_value(
                            &mut panel.mode,
                            FindReplaceMode::ReplaceAll,
                            "Replace all",
                        );
                    });
                ui.add_space(10.0);

                ui.label("Find:");
                let find_field = ui.add(
                    egui::TextEdit::singleline(&mut panel.find)
                        .desired_width(f32::INFINITY)
                        .hint_text("word to search for"),
                );
                // A single-line field lets go of the keyboard on Enter, so a
                // second Enter would reach nothing: it is handed back at once,
                // which is what lets Enter step through the matches.
                let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                let mut entered = find_field.lost_focus() && enter;
                if entered {
                    find_field.request_focus();
                }

                if panel.mode != FindReplaceMode::Find {
                    ui.add_space(6.0);
                    ui.label("Replace with:");
                    let replace_field = ui.add(
                        egui::TextEdit::singleline(&mut panel.replace)
                            .desired_width(f32::INFINITY)
                            .hint_text("its replacement"),
                    );
                    if replace_field.lost_focus() && enter {
                        entered = true;
                        replace_field.request_focus();
                    }
                }

                ui.add_space(10.0);
                let something = !panel.find.trim().is_empty();
                ui.horizontal(|ui| match panel.mode {
                    FindReplaceMode::Find => {
                        // Find with the word already searched for goes on to
                        // the next match, like Find Next; a different word is a
                        // new search, from the first. Reported from use: it
                        // went back to the first match on every press.
                        if ui.add_enabled(something, egui::Button::new("Find")).clicked()
                            || (entered && something)
                        {
                            step = Some(true);
                        }
                        if ui.button("Previous").clicked() {
                            step = Some(false);
                        }
                        if ui.button("Next").clicked() {
                            step = Some(true);
                        }
                    }
                    FindReplaceMode::ReplaceOne => {
                        if ui.button("Find Next").clicked() {
                            step = Some(true);
                        }
                        if ui.add_enabled(something, egui::Button::new("Replace")).clicked()
                            || (entered && something)
                        {
                            replace_one = true;
                        }
                    }
                    FindReplaceMode::ReplaceAll => {
                        if ui.add_enabled(something, egui::Button::new("Replace All")).clicked()
                            || (entered && something)
                        {
                            replace_all = true;
                        }
                    }
                });
                ui.add_space(4.0);
                if ui.button("Close").clicked() {
                    done = true;
                }
            });

        if !open || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            done = true;
        }

        if let Some(forward) = step {
            if self.tab_mut().panels.find_hits.is_empty() || self.tab_mut().panels.find_needle != panel.find {
                self.find(&panel.find);
            } else {
                self.find_step(forward);
            }
        }
        if replace_one {
            match self.replace_current(&panel.find, &panel.replace) {
                Ok(said) => self.say_info(said),
                Err(e) => self.say_error(e),
            }
        }
        if replace_all {
            match self.replace_all(&panel.find, &panel.replace) {
                Ok(said) => self.say_info(said),
                Err(e) => self.say_error(e),
            }
        }

        if done {
            return;
        }
        self.tab_mut().panels.find_replace = Some(panel);
    }

    /// The Check Spelling panel — one word at a time, its own suggestions,
    /// and the choice to change it, change every occurrence, or leave it
    /// alone.
    ///
    /// An ordinary floating window, not a modal — the same reasoning
    /// `draw_find_replace` documents: a spelling pass is watched against
    /// the page as it goes, not from behind a dimmed overlay of it.
    pub(crate) fn draw_spell_check(&mut self, ctx: &egui::Context) {
        let Some(mut panel) = self.tab_mut().panels.spelling.take() else { return };

        let mut open = true;
        let mut done = false;
        let mut change = false;
        let mut change_all = false;
        let mut ignore = false;
        let mut ignore_all = false;
        let mut add_to_dictionary = false;
        let notice = panel.notice.clone();
        let note = panel.skipped_note();
        let chinese = panel.chinese_note();

        egui::Window::new("Check Spelling")
            .id(egui::Id::new("spell-check-window"))
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .default_pos(egui::pos2(80.0, 80.0))
            .show(ctx, |ui| {
                ui.set_width(320.0);

                // What the panel has to say beyond the word in front of it: a
                // Change that was refused, and pages it could not check. Under
                // the buttons, and under "No misspelled words found." too — a
                // page nothing looked at must not read as a clean one.
                let footer = |ui: &mut egui::Ui| {
                    if let Some(notice) = &notice {
                        ui.add_space(6.0);
                        ui.colored_label(ui.visuals().error_fg_color, notice);
                    }
                    if let Some(note) = &note {
                        ui.add_space(6.0);
                        ui.weak(note);
                    }
                    if let Some(chinese) = chinese {
                        ui.add_space(6.0);
                        ui.weak(chinese);
                    }
                };

                // Still reading the pages, on the scan's own thread: say so, and
                // how far it has got, rather than leave a window that looks hung.
                if let Some((checked, pages)) = panel.scanning {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(format!("Checking spelling\u{2026} page {checked} of {pages}"));
                    });
                    ui.add_space(8.0);
                    if ui.button("Cancel").clicked() {
                        done = true;
                    }
                    return;
                }

                let Some(current) = panel.found.first().cloned() else {
                    ui.label("No misspelled words found.");
                    footer(ui);
                    return;
                };
                ui.label(format!(
                    "word {} of {} — page {}",
                    panel.total_found - panel.found.len() + 1,
                    panel.total_found,
                    current.page + 1,
                ));
                ui.add_space(4.0);
                ui.heading(&current.word);
                ui.add_space(8.0);

                let suggestions = panel.suggestions_for(&current.word).to_vec();
                if suggestions.is_empty() {
                    ui.label("No suggestions.");
                } else {
                    ui.horizontal_wrapped(|ui| {
                        for word in &suggestions {
                            if ui.button(word).clicked() {
                                panel.replacement = word.clone();
                            }
                        }
                    });
                }
                ui.add_space(8.0);

                ui.label("Change to:");
                ui.add(
                    egui::TextEdit::singleline(&mut panel.replacement)
                        .desired_width(f32::INFINITY),
                );

                ui.add_space(10.0);
                let something = !panel.replacement.trim().is_empty();
                ui.horizontal(|ui| {
                    if ui.add_enabled(something, egui::Button::new("Change")).clicked() {
                        change = true;
                    }
                    if ui.add_enabled(something, egui::Button::new("Change All")).clicked() {
                        change_all = true;
                    }
                });
                ui.horizontal(|ui| {
                    if ui.button("Ignore").clicked() {
                        ignore = true;
                    }
                    if ui.button("Ignore All").clicked() {
                        ignore_all = true;
                    }
                });
                // Ignore All is for this check only; this is for good.
                if ui
                    .button("Add to Dictionary")
                    .on_hover_text("Never flag this word again, in any document.")
                    .clicked()
                {
                    add_to_dictionary = true;
                }
                footer(ui);
            });

        if !open || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            done = true;
        }

        // **One action at a time, on `found[0]`, then the best suggestion
        // for whatever is now first.** Removing every entry for the same
        // word (`change_all`/`ignore_all`) rather than tracking an index
        // elsewhere in the list is what keeps this simple: the word in front
        // is always the one being decided about.
        //
        // **A Change the engine refused leaves the word where it is.**
        // Reported from use: the refusal went to the history bar and the word
        // left the list as if it had been fixed, so a page kept its typo and
        // the panel moved on without a word. Now the reason is shown in the
        // panel and the word stays to be changed some other way or ignored.
        self.apply_spell_check_actions(
            &mut panel, add_to_dictionary, change, change_all, ignore, ignore_all,
        );
        if done {
            return;
        }

        self.tab_mut().panels.spelling = Some(panel);
    }

    /// The action the spell dialog's buttons asked for, applied to the word in
    /// front: dictionary, change, change-all, ignore, ignore-all. A Change the
    /// engine refuses leaves the word in place with `panel.notice` set — see
    /// the comment this moved with in `draw_spell_check`.
    fn apply_spell_check_actions(
        &mut self,
        panel: &mut SpellCheck,
        add_to_dictionary: bool,
        change: bool,
        change_all: bool,
        ignore: bool,
        ignore_all: bool,
    ) {
        if add_to_dictionary {
            panel.notice = None;
            if let Some(current) = panel.found.first().cloned() {
                // Known from now on, to every later check in every window. A
                // file that could not be written still leaves the word added
                // for this session, and says so.
                match spelling::add_word(&current.word) {
                    Ok(_) => self.say_info(format!(
                        "\"{}\" added to your dictionary.",
                        current.word.to_lowercase()
                    )),
                    Err(e) => {
                        self.say_error(e.clone());
                        panel.notice = Some(e);
                    }
                }
                let word = current.word.to_lowercase();
                panel.found.retain(|m| m.word.to_lowercase() != word);
                panel.reset_replacement();
            }
        }
        if change || change_all || ignore || ignore_all {
            panel.notice = None;
            if let Some(current) = panel.found.first().cloned() {
                let mut refused = None;
                if change {
                    refused = self
                        .apply_spelling_change(
                            current.page,
                            current.object,
                            &current.word,
                            &panel.replacement,
                        )
                        .err();
                } else if change_all {
                    // `replace_all` leaves out a run it could not rewrite and
                    // answers "not found" when that was every one, so what says
                    // whether anything changed is the document's own history.
                    let history = |app: &Self| app.tab().doc.as_ref().map(|d| d.session.undo_generation());
                    let before = history(self);
                    refused = match self.replace_all(&current.word, &panel.replacement) {
                        Err(e) => Some(e),
                        Ok(_) if history(self) == before => Some(
                            "none of the places it appears could be rewritten, so the \
                             document was left as it was."
                                .to_string(),
                        ),
                        Ok(_) => None,
                    };
                }
                if let Some(why) = refused {
                    let said = format!("could not change \"{}\": {why}", current.word);
                    self.say_error(said.clone());
                    panel.notice = Some(said);
                } else {
                    let word = current.word.to_lowercase();
                    if change_all || ignore_all {
                        panel.found.retain(|m| m.word.to_lowercase() != word);
                    } else {
                        panel.found.remove(0);
                    }
                    panel.reset_replacement();
                }
            }
        }

    }


    /// The Bookmarks panel — every bookmark, click to jump.
    pub(crate) fn draw_bookmark_panel(&mut self, ctx: &egui::Context) {
        let Some(panel) = self.tab_mut().panels.bookmark_panel.take() else { return };
        let mut open = true;
        let mut go_to_page: Option<usize> = None;

        egui::Window::new("Bookmarks")
            .id(egui::Id::new("bookmark-panel"))
            .open(&mut open)
            .default_size(egui::vec2(240.0, 300.0))
            .resizable(true)
            .collapsible(false)
            .show(ctx, |ui| {
                if panel.entries.is_empty() {
                    ui.label("No bookmarks yet — click Bookmark on any page to add one.");
                    return;
                }
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for (title, page) in &panel.entries {
                        if ui.selectable_label(false, format!("{title}   —   p.{}", page + 1)).clicked()
                        {
                            go_to_page = Some(*page);
                        }
                    }
                });
            });

        if let Some(page) = go_to_page {
            self.go_to(PageTarget::Number(page + 1));
        }
        if !open {
            return;
        }
        self.tab_mut().panels.bookmark_panel = Some(panel);
    }

    pub(crate) fn draw_link_prompt(&mut self, ctx: &egui::Context) {
        let Some(mut pending) = self.tab_mut().panels.pending_link.take() else { return };

        let mut open = true;
        let mut done = false;
        let mut add = false;

        egui::Window::new("Add Web Link")
            .id(egui::Id::new("web-link-window"))
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .default_pos(egui::pos2(80.0, 80.0))
            .show(ctx, |ui| {
                ui.set_width(320.0);
                ui.label(format!(
                    "{} line{} selected.",
                    pending.rects.len(),
                    if pending.rects.len() == 1 { "" } else { "s" }
                ));
                ui.add_space(6.0);
                ui.label("Link to:");
                let field = ui.add(
                    egui::TextEdit::singleline(&mut pending.url)
                        .desired_width(f32::INFINITY)
                        .hint_text("https://…"),
                );
                let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));

                ui.add_space(10.0);
                let something = !pending.url.trim().is_empty();
                ui.horizontal(|ui| {
                    if ui.add_enabled(something, egui::Button::new("Add Link")).clicked()
                        || (entered && something)
                    {
                        add = true;
                    }
                    if ui.button("Cancel").clicked() {
                        done = true;
                    }
                });
            });

        if !open || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            done = true;
        }
        if add {
            match self.apply_web_link(&pending) {
                Ok(said) => self.say_info(said),
                Err(e) => self.say_error(e),
            }
            done = true;
        }
        if done {
            return;
        }
        self.tab_mut().panels.pending_link = Some(pending);
    }

    pub(crate) fn draw_article_box_prompt(&mut self, ctx: &egui::Context) {
        let Some(mut pending) = self.tab_mut().panels.pending_article_box.take() else { return };

        let mut open = true;
        let mut done = false;
        let mut add = false;

        egui::Window::new("Add Article Box")
            .id(egui::Id::new("article-box-window"))
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .default_pos(egui::pos2(80.0, 80.0))
            .show(ctx, |ui| {
                ui.set_width(300.0);
                ui.label("Title (optional):");
                let field = ui.add(
                    egui::TextEdit::singleline(&mut pending.title)
                        .desired_width(f32::INFINITY)
                        .hint_text("Article"),
                );
                let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));

                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("Add").clicked() || entered {
                        add = true;
                    }
                    if ui.button("Cancel").clicked() {
                        done = true;
                    }
                });
            });

        if !open || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            done = true;
        }
        if add {
            match self.apply_article_box(&pending) {
                Ok(said) => self.say_info(said),
                Err(e) => self.say_error(e),
            }
            done = true;
        }
        if done {
            return;
        }
        self.tab_mut().panels.pending_article_box = Some(pending);
    }

    /// Ask for a password, whatever it is for, in one window.
    ///
    /// **Every password in this program is asked for the same way.** There were
    /// three ways at one point — a window for opening, a window for locking, and
    /// the command box for `secure` — which meant three sets of wording, three
    /// places to look, and a rule that applied to one of them. Reported from use
    /// as wanting it uniform, and rightly.
    ///
    /// The one distinction that survives is real: a password being **chosen**
    /// has to be strong and is typed twice, because there is nothing to check it
    /// against and a slip cannot be discovered later. A password being **used**
    /// gets neither, because the document already knows the answer.
    /// The words somebody keeps for writing again.
    ///
    /// Shown in full rather than as labels: these *are* the words, and a list
    /// that named them would be a list of names for things that are already
    /// their own name.
    pub(crate) fn draw_snippet_list(&mut self, ctx: &egui::Context) {
        let Some(mut panel) = self.library_state.snippets.take() else { return };

        let mut done = false;
        let mut chosen: Option<String> = None;
        let mut doomed: Option<String> = None;
        let mut add = false;
        let kept: Vec<String> = self.predefined.entries().to_vec();

        egui::Modal::new(egui::Id::new("snippet-list")).show(ctx, |ui| {
            ui.set_width(520.0);
            ui.heading("Predefined text");
            ui.add_space(6.0);
            ui.label(
                "Words kept for filling forms in, on this computer. Only what you \
                 put here is kept — nothing you type onto a page reaches this list.",
            );
            ui.add_space(12.0);

            if kept.is_empty() {
                ui.label("Nothing kept yet.");
                ui.add_space(8.0);
            }

            egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
                for entry in &kept {
                    egui::Frame::new()
                        .fill(theme::panel())
                        .stroke(egui::Stroke::new(1.0, theme::line()))
                        .corner_radius(egui::CornerRadius::ZERO)
                        .inner_margin(8.0)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.set_width(340.0);
                                    // Every line of it: an address is why this
                                    // exists, and showing the first line only
                                    // would hide what is about to be written.
                                    for line in entry.lines() {
                                        ui.label(line);
                                    }
                                });
                                if ui.button("Write").clicked() {
                                    chosen = Some(entry.clone());
                                }
                                if ui.button("Forget").clicked() {
                                    doomed = Some(entry.clone());
                                }
                            });
                        });
                    ui.add_space(6.0);
                }
            });

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(8.0);
            ui.label("Keep something new");
            let field = ui.add(
                egui::TextEdit::multiline(&mut panel.adding)
                    .desired_width(f32::INFINITY)
                    .desired_rows(2)
                    .hint_text("Jane Smith"),
            );
            // Enter finishes it, since a snippet is usually one line — and
            // Shift-Enter is still how a second line is typed.
            let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let something = !panel.adding.trim().is_empty();
                if ui.add_enabled(something, egui::Button::new("Keep it")).clicked() || entered {
                    add = true;
                }
                if ui.button("Done").clicked() {
                    done = true;
                }
            });
        });

        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            done = true;
        }

        if add {
            let typed = panel.adding.trim().to_string();
            if !typed.is_empty() {
                self.predefined.remember(&typed);
                match self.keep_snippets() {
                    Ok(()) => {
                        panel.adding.clear();
                        self.say_info(format!("kept \"{}\".", short(&typed)));
                    }
                    Err(e) => {
                        self.predefined.forget(&typed);
                        self.say_error(e);
                    }
                }
            }
        }
        if let Some(text) = doomed {
            if self.predefined.forget(&text) {
                match self.keep_snippets() {
                    Ok(()) => self.say_info(format!("forgot \"{}\".", short(&text))),
                    Err(e) => self.say_error(e),
                }
            }
        }
        if let Some(text) = chosen {
            // The panel closes: what comes next is a click on the page, and a
            // window over the page would be in the way of it.
            self.use_snippet(&text);
            return;
        }
        if done {
            return;
        }
        self.library_state.snippets = Some(panel);
    }

    /// The list of signatures somebody has drawn.
    ///
    /// Each is drawn rather than named, because a list of names says nothing
    /// about which scrawl is which — and it is drawn by the same code that
    /// places one on a page, so the preview is the thing itself rather than an
    /// impression of it.
    pub(crate) fn draw_signature_list(&mut self, ctx: &egui::Context) {
        let Some(mut panel) = self.library_state.signature_list.take() else { return };

        let mut done = false;
        let mut action: Option<ListAction> = None;
        let current = self.signatures.current().map(|s| s.name.clone());
        let entries = self.signatures.entries().to_vec();

        egui::Modal::new(egui::Id::new("signature-list")).show(ctx, |ui| {
            ui.set_width(560.0);
            ui.heading("Your signatures");
            ui.add_space(6.0);
            // Says which list this is: the drawings, not the marks on a page.
            ui.label(
                "The signatures you have drawn, kept on this computer. A signature \
                 already placed in a document is part of that document.",
            );
            ui.add_space(12.0);

            if entries.is_empty() {
                ui.label("Nothing drawn yet.");
                ui.add_space(10.0);
            }

            for entry in &entries {
                let is_current = current.as_deref() == Some(entry.name.as_str());
                egui::Frame::new()
                    .fill(if is_current { theme::raised() } else { theme::panel() })
                    .stroke(egui::Stroke::new(
                        1.0,
                        if is_current { theme::violet() } else { theme::line() },
                    ))
                    .corner_radius(egui::CornerRadius::ZERO)
                    .inner_margin(10.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let texture = entry.image.as_ref().map(|image| {
                                self.signature_textures
                                    .entry(entry.name.clone())
                                    .or_insert_with(|| {
                                        ctx.load_texture(
                                            format!("signature-{}", entry.name),
                                            egui::ColorImage::from_rgba_unmultiplied(
                                                [image.width as usize, image.height as usize],
                                                &image.rgba,
                                            ),
                                            egui::TextureOptions::LINEAR,
                                        )
                                    })
                                    .clone()
                            });
                            let (response, painter) = ui.allocate_painter(
                                egui::vec2(220.0, 64.0),
                                egui::Sense::hover(),
                            );
                            let area = response.rect;
                            painter.rect_filled(
                                area,
                                4.0,
                                egui::Color32::from_rgb(0xFA, 0xFA, 0xFC),
                            );
                            paint_signature(&painter, area, entry, texture.as_ref());

                            ui.vertical(|ui| {
                                match &mut panel.renaming {
                                    Some((which, typed)) if which == &entry.name => {
                                        let field = ui.add(
                                            egui::TextEdit::singleline(typed)
                                                .desired_width(180.0),
                                        );
                                        let entered = field.lost_focus()
                                            && ui.input(|i| i.key_pressed(egui::Key::Enter));
                                        ui.horizontal(|ui| {
                                            if ui.button("Save").clicked() || entered {
                                                action = Some(ListAction::Rename(
                                                    entry.name.clone(),
                                                    typed.clone(),
                                                ));
                                            }
                                            if ui.button("Cancel").clicked() {
                                                action = Some(ListAction::Rename(
                                                    entry.name.clone(),
                                                    entry.name.clone(),
                                                ));
                                            }
                                        });
                                    }
                                    _ => {
                                        ui.label(egui::RichText::new(&entry.name).strong());
                                        if is_current {
                                            ui.colored_label(theme::violet(), "a click places this");
                                        }
                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            if ui
                                                .add_enabled(
                                                    !is_current,
                                                    egui::Button::new("Use"),
                                                )
                                                .clicked()
                                            {
                                                action = Some(ListAction::Use(entry.name.clone()));
                                            }
                                            if ui.button("Rename").clicked() {
                                                panel.renaming = Some((
                                                    entry.name.clone(),
                                                    entry.name.clone(),
                                                ));
                                            }
                                            // Twice, because it does not come
                                            // back — and the second press says
                                            // what it is about to do.
                                            if panel.doomed.as_deref() == Some(&entry.name) {
                                                if ui
                                                    .add(
                                                        egui::Button::new("Delete for good")
                                                            .fill(theme::danger()),
                                                    )
                                                    .clicked()
                                                {
                                                    action = Some(ListAction::Forget(
                                                        entry.name.clone(),
                                                    ));
                                                }
                                            } else if ui.button("Delete").clicked() {
                                                panel.doomed = Some(entry.name.clone());
                                            }
                                        });
                                    }
                                }
                            });
                        });
                    });
                ui.add_space(8.0);
            }

            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.button("Draw a new one").clicked() {
                    action = Some(ListAction::Draw);
                }
                if ui.button("Done").clicked() {
                    done = true;
                }
            });
        });

        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            done = true;
        }

        if self.apply_signature_list_action(&mut panel, action) {
            return;
        }

        if done {
            return;
        }
        self.library_state.signature_list = Some(panel);
    }

    /// What the signature list asked for once its dialog closed: use, forget,
    /// rename or draw. `true` means the caller must return before restoring
    /// the panel — the Draw arm opens the pad instead. Moved out of
    /// `draw_signature_list`.
    fn apply_signature_list_action(&mut self, panel: &mut SignatureList, action: Option<ListAction>) -> bool {
        match action {
            Some(ListAction::Use(name)) => {
                if self.signatures.choose(&name) {
                    match self.keep_signatures() {
                        Ok(()) => self.say_info(format!("`signature` now places \"{name}\".")),
                        Err(e) => self.say_error(e),
                    }
                }
            }
            Some(ListAction::Forget(name)) => {
                panel.doomed = None;
                if self.signatures.remove(&name) {
                    match self.keep_signatures() {
                        Ok(()) => self.say_info(format!(
                            "\"{name}\" is gone — a drawing is not something undo reaches."
                        )),
                        Err(e) => self.say_error(e),
                    }
                }
            }
            Some(ListAction::Rename(from, to)) => {
                panel.renaming = None;
                if !from.eq_ignore_ascii_case(&to) {
                    match self.signatures.rename(&from, &to) {
                        Ok(()) => match self.keep_signatures() {
                            Ok(()) => self.say_info(format!("\"{from}\" is now \"{to}\".")),
                            Err(e) => self.say_error(e),
                        },
                        Err(why) => self.say_error(why.describe()),
                    }
                }
            }
            Some(ListAction::Draw) => {
                self.library_state.pad = Some(SignaturePad {
                    name: format!("Signature {}", self.signatures.entries().len() + 1),
                    then_place: false,
                    ..SignaturePad::default()
                });
                return true;
            }
            None => {}
        }
        false
    }


    /// The pad a signature is drawn on.
    ///
    /// A white sheet with a line across it, because that is what a person is
    /// used to signing and because the line is what the placed signature will
    /// sit on — drawn here so the shape somebody sees is the shape they get.
    pub(crate) fn draw_signature_pad(&mut self, ctx: &egui::Context) {
        let Some(mut pad) = self.library_state.pad.take() else { return };

        let mut keep = false;
        let mut gave_up = false;
        egui::Modal::new(egui::Id::new("signature-pad")).show(ctx, |ui| {
            ui.set_width(560.0);
            ui.heading("Draw your signature");
            ui.add_space(6.0);
            // Said where it cannot be missed, not in a manual. Somebody who
            // believes this is the cryptographic kind is worse off than
            // somebody with no signature at all.
            ui.label(
                "Drag to write. This is ink — it shows a name, it does not prove \
                 one, and `certify` is what signs with a certificate.",
            );
            ui.add_space(10.0);

            let (response, painter) =
                ui.allocate_painter(egui::vec2(520.0, 200.0), egui::Sense::drag());
            let area = response.rect;
            painter.rect_filled(area, 6.0, egui::Color32::from_rgb(0xFA, 0xFA, 0xFC));
            // The line, a third up from the bottom — where a signature sits on
            // a form.
            let line = area.bottom() - area.height() * 0.28;
            painter.line_segment(
                [
                    egui::pos2(area.left() + 24.0, line),
                    egui::pos2(area.right() - 24.0, line),
                ],
                egui::Stroke::new(1.0, egui::Color32::from_rgb(0xC8, 0xC8, 0xD4)),
            );

            if response.drag_started() {
                pad.strokes.push(Vec::new());
            }
            if let Some(at) = response.interact_pointer_pos() {
                if let Some(stroke) = pad.strokes.last_mut() {
                    let point = (at.x - area.left(), at.y - area.top());
                    // Repeated points are the pointer resting, not writing —
                    // and thousands of them would be stored as the signature.
                    if stroke.last().is_none_or(|last: &(f32, f32)| {
                        (last.0 - point.0).hypot(last.1 - point.1) > 0.75
                    }) {
                        stroke.push(point);
                    }
                }
            }

            let ink = egui::Color32::from_rgb(0x14, 0x2B, 0x63);
            for stroke in &pad.strokes {
                if stroke.len() < 2 {
                    continue;
                }
                let points: Vec<egui::Pos2> = stroke
                    .iter()
                    .map(|(x, y)| egui::pos2(area.left() + x, area.top() + y))
                    .collect();
                painter.add(egui::Shape::line(points, egui::Stroke::new(2.2, ink)));
            }

            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.label("Called");
                ui.add(
                    egui::TextEdit::singleline(&mut pad.name)
                        .desired_width(200.0)
                        .hint_text("Signature"),
                );
            });

            ui.add_space(12.0);
            ui.horizontal(|ui| {
                // Disabled rather than hidden while there is nothing to keep,
                // so the button that will work is the one in the same place.
                let drawn = pad.strokes.iter().any(|s| s.len() >= 2);
                if ui.add_enabled(drawn, egui::Button::new("Keep it")).clicked() {
                    keep = true;
                }
                if ui.button("Clear").clicked() {
                    pad.strokes.clear();
                }
                if ui.button("Cancel").clicked() {
                    gave_up = true;
                }
            });
        });

        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            gave_up = true;
        }

        if keep {
            let (name, strokes) = (pad.name.clone(), pad.strokes.clone());
            match self.save_drawn_signature(&name, &strokes) {
                Ok(said) => {
                    self.say_info(said);
                    // Straight on to the click, for somebody who reached for
                    // the tool wanting to sign rather than to draw.
                    if pad.then_place && self.tab_mut().doc.is_some() {
                        let page = self.tab_mut().view_state.page;
                        self.arm_tool(Tool::Signature, page);
                    }
                }
                Err(e) => {
                    self.say_error(e);
                    // Kept open with the strokes intact: throwing away what
                    // somebody drew because it was too small is the wrong way
                    // round.
                    self.library_state.pad = Some(pad);
                }
            }
            return;
        }
        if gave_up {
            self.say_info("nothing was kept.");
            return;
        }
        self.library_state.pad = Some(pad);
    }

    pub(crate) fn draw_passcode_dialog(&mut self, ctx: &egui::Context) {
        use pagify_shell::passphrase;

        let Some(waiting) = self.tab_mut().secure_state.awaiting_password.clone() else { return };

        // What is being asked, in three questions.
        let confirming =
            matches!(waiting, Awaiting::LockAgain { .. } | Awaiting::SecureAgain { .. });
        let using = matches!(
            waiting,
            Awaiting::Open(_) | Awaiting::Unlock | Awaiting::UnlockItem(_)
        ) || matches!(waiting, Awaiting::Lock { .. } | Awaiting::LockPages(_) | Awaiting::LockImage { .. })
            && self.lock_exists();
        let choosing = !using && !confirming
            || matches!(waiting, Awaiting::LockAgain { .. } | Awaiting::SecureAgain { .. });

        // A rule applies only where one is being chosen.
        let ruled = !using;

        let (heading, explains, act) = passcode_wording(&waiting, using);

        let mut submitted = false;
        let mut gave_up = false;
        egui::Modal::new(egui::Id::new("passcode")).show(ctx, |ui| {
            ui.set_width(440.0);
            ui.heading(&heading);
            ui.add_space(6.0);
            ui.label(&explains);
            ui.add_space(10.0);

            let field = ui.add(
                egui::TextEdit::singleline(&mut *self.tab_mut().secure_state.password_typed)
                    .password(true)
                    .desired_width(f32::INFINITY)
                    .hint_text("password"),
            );
            if !self.tab_mut().secure_state.password_field_focused {
                field.request_focus();
                self.tab_mut().secure_state.password_field_focused = true;
            }
            if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                submitted = true;
            }

            // What is still needed, ticked off as it arrives — shown while one
            // is being chosen, which is the only time it can be acted on.
            if ruled && !confirming {
                ui.add_space(8.0);
                let missing = passphrase::unmet(&self.tab_mut().secure_state.password_typed);
                let here = self.tab_mut().secure_state.password_typed.chars().count();
                for (wanted, said) in [
                    (
                        passphrase::Unmet::TooShort { need: passphrase::LEAST, have: here },
                        format!("{} characters or more", passphrase::LEAST),
                    ),
                    (passphrase::Unmet::NoUppercase, "an upper-case letter".into()),
                    (passphrase::Unmet::NoLowercase, "a lower-case letter".into()),
                    (passphrase::Unmet::NoDigit, "a number".into()),
                    (passphrase::Unmet::NoSymbol, "a symbol".into()),
                ] {
                    let met = !missing
                        .iter()
                        .any(|m| std::mem::discriminant(m) == std::mem::discriminant(&wanted));
                    ui.horizontal(|ui| {
                        ui.colored_label(
                            if met { theme::snap() } else { theme::ink_faint() },
                            if met { "\u{2713}" } else { "\u{2022}" },
                        );
                        ui.colored_label(if met { theme::ink() } else { theme::ink_dim() }, said);
                    });
                }
            }

            // Which handler writes it. Offered only where a password is being
            // chosen for the *file* — locking has no such choice, and a
            // password being used has already been decided.
            let mut restrictions_would_be_lost = false;
            if let Awaiting::Secure(options) = &waiting {
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.tab_mut().secure_state.password_plus, false, "Secure");
                    ui.selectable_value(&mut self.tab_mut().secure_state.password_plus, true, "Secure Plus");
                });
                ui.add_space(4.0);
                if self.tab_mut().secure_state.password_plus {
                    // Said before it is chosen, not discovered afterwards.
                    ui.colored_label(
                        theme::danger(),
                        "Nothing else will open this file. Not Preview, not Acrobat, \
                         not a browser — only Pagify, with this password. There is no \
                         way back without it.",
                    );
                    // **Secure Plus carries no permissions.** A `secure readonly`
                    // under it used to store "everything permitted" and say
                    // nothing — the restriction quietly dropped. Found by audit.
                    // Refused here, where the choice is being made.
                    if *options != pagify_shell::verbs::SecureOptions::default() {
                        restrictions_would_be_lost = true;
                        ui.add_space(4.0);
                        ui.colored_label(
                            theme::danger(),
                            format!(
                                "Secure Plus keeps no permissions, so \"{}\" cannot be set \
                                 under it. Choose Secure to keep the restriction, or run \
                                 `secure` without one.",
                                options.describe()
                            ),
                        );
                    }
                } else {
                    ui.colored_label(
                        theme::ink_dim(),
                        "Standard PDF encryption. Any reader will ask for this password.",
                    );
                }
            }

            if let Some(said) = &self.tab_mut().secure_state.password_problem {
                ui.add_space(6.0);
                ui.colored_label(theme::danger(), said);
            }

            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let ready = !self.tab_mut().secure_state.password_typed.is_empty()
                    && !restrictions_would_be_lost
                    && (!ruled || confirming || passphrase::is_strong_enough(&self.tab_mut().secure_state.password_typed));
                if ui.add_enabled(ready, egui::Button::new(act)).clicked() {
                    submitted = true;
                }
                if ui.button("Cancel").clicked() {
                    gave_up = true;
                }
            });
        });

        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            gave_up = true;
        }
        if gave_up {
            self.tab_mut().secure_state.awaiting_password = None;
            // `Zeroizing` wipes on drop; taking the value drops it.
            drop(std::mem::take(&mut self.tab_mut().secure_state.password_typed));
            self.tab_mut().secure_state.password_problem = None;
            self.tab_mut().secure_state.password_field_focused = false;
            self.say_info(match waiting {
                Awaiting::Open(_) => "left it unopened.",
                Awaiting::Unlock | Awaiting::UnlockItem(_) => "nothing was unlocked.",
                Awaiting::Secure(_)
                | Awaiting::SecureAgain { .. }
                | Awaiting::SecureCurrent(_) => "no password was set.",
                _ => "nothing was locked.",
            });
            return;
        }
        if !submitted || self.tab_mut().secure_state.password_typed.is_empty() {
            return;
        }

        let typed = std::mem::take(&mut self.tab_mut().secure_state.password_typed);
        let _ = choosing;
        self.answer_passcode(&typed);
    }

    pub(crate) fn draw_extract_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut ask) = self.tab_mut().panels.extract_ask.take() else { return };
        let Some(count) = self.tab().doc.as_ref().map(|d| d.page_count) else { return };
        let current = self.tab().view_state.page;
        let selected = self.tab().organize.organize_selected.clone();

        let mut go = false;
        let mut cancel = false;
        egui::Modal::new(egui::Id::new("extract-pages")).show(ctx, |ui| {
            ui.set_width(420.0);
            ui.heading("Extract pages");
            ui.add_space(6.0);
            ui.label(format!(
                "Copies the pages you name into a new PDF. This document has {count} page{}.",
                if count == 1 { "" } else { "s" }
            ));
            ui.add_space(10.0);
            ui.label("Pages:");
            let field = ui.add(
                egui::TextEdit::singleline(&mut ask.pages)
                    .desired_width(f32::INFINITY)
                    .hint_text("e.g. 1-3,7"),
            );
            if !ask.focused {
                field.request_focus();
                ask.focused = true;
            }
            if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                go = true;
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.small_button("This page").clicked() {
                    ask.pages = (current + 1).to_string();
                    ask.problem = None;
                }
                if ui.add_enabled(!selected.is_empty(), egui::Button::new("Selected pages").small()).clicked() {
                    ask.pages = compact_page_spec(&selected);
                    ask.problem = None;
                }
                if ui.small_button("All pages").clicked() {
                    ask.pages = "all".into();
                    ask.problem = None;
                }
            });
            if let Some(problem) = &ask.problem {
                ui.add_space(6.0);
                ui.colored_label(theme::danger(), problem);
            }
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.add_enabled(!ask.pages.trim().is_empty(), egui::Button::new("Extract\u{2026}")).clicked() {
                    go = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });

        if cancel || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.say_info("nothing was extracted.");
            return;
        }
        if go && !ask.pages.trim().is_empty() {
            match pagify_shell::organize::parse_range(ask.pages.trim(), count) {
                Err(e) => ask.problem = Some(e),
                Ok(pages) => match self.pick_extract_destination(&pages) {
                    Some(dest) if self.is_open_document(&dest) => {
                        ask.problem = Some(
                            "That is the document you have open. Choose another file name.".into(),
                        );
                    }
                    Some(dest) => {
                        self.extract(ask.pages.trim(), &dest);
                        return;
                    }
                    // The Save box was closed without a file: the dialog stays,
                    // so the pages typed are not lost.
                    None => {}
                },
            }
        }
        self.tab_mut().panels.extract_ask = Some(ask);
    }

    /// Layers a sharper render of whatever's actually on screen over the
    /// whole-page texture [`Self::texture_for`] already drew, for the one
    /// case that texture's own render ceiling leaves it blurrier than the
    /// zoom actually asked for: a large-format page (A1, A0) zoomed into a
    /// small detail. See `texture_for`'s own doc for the full story.
    ///
    /// `page_rect` is the page's own on-screen rect this frame, unclipped —
    /// this needs that to know how much of the page is actually visible, not
    /// just where it would sit if the whole thing were.
    pub(crate) fn draw_detail_overlay(
        &mut self,
        ctx: &egui::Context,
        ui: &egui::Ui,
        page: usize,
        view: PageView,
        page_rect: egui::Rect,
        device_scale: f32,
    ) {
        // The detail is rendered from the page upright, in page coordinates; laid
        // over a turned view it would show a different part of the page from the
        // one under it.
        if !matches!(self.tab().view_state.rotation, Rotation::None) {
            return;
        }
        let (w, h) = self.tab_mut()
            .doc
            .as_ref()
            .and_then(|d| d.strip.size_of(page))
            .unwrap_or((612.0, 792.0));

        // The whole-page texture is already as sharp as what was asked for —
        // nothing for this to add. The 2% slop is for float roundoff between
        // the two independent computations of the same ceiling, not a real
        // tolerance for blur.
        if device_scale <= Self::whole_page_scale_ceiling(ctx, w, h) * 1.02 {
            return;
        }

        let visible_screen = page_rect.intersect(ui.clip_rect());
        if visible_screen.width() < 1.0 || visible_screen.height() < 1.0 {
            return;
        }

        let visible_crop = Self::page_rect_from_screen(view, visible_screen, w, h);
        let zoom_step = pdf_core::render::cache::quantise_zoom(device_scale);

        // Reuse the existing tile when it already covers what's visible now
        // at this zoom — a pan or scroll inside its own margin is then a
        // plain redraw, no re-render, the same way a whole-page texture
        // serves every frame between zoom changes.
        let reusable = self
            .tab_mut()
            .doc
            .as_ref()
            .and_then(|d| d.caches.detail.as_ref())
            .is_some_and(|tile| {
                Self::detail_tile_covers(tile.page, tile.zoom_step, tile.crop, page, zoom_step, visible_crop)
            });

        // Not while the zoom is still moving: a detail made for a step it has
        // already left is a stall for nothing, and the one held — if there is
        // one — is drawn meanwhile, softer, where it was.
        let moving = self.render_state.async_render && self.zoom_is_moving(ctx);
        if !reusable && !moving {
            let margin = egui::vec2(
                visible_screen.width() * Self::DETAIL_MARGIN_FRAC,
                visible_screen.height() * Self::DETAIL_MARGIN_FRAC,
            );
            let crop = Self::page_rect_from_screen(view, visible_screen.expand2(margin), w, h);
            let crop_w = (crop.right - crop.left).max(1.0);
            let crop_h = (crop.bottom - crop.top).max(1.0);
            // The crop's own ceiling, not the whole page's — a small crop
            // affords a far higher scale from the same pixel budget, which
            // is the entire point of rendering one instead of the whole page.
            let region_scale = device_scale.min(Self::whole_page_scale_ceiling(ctx, crop_w, crop_h));
            let region_step = pdf_core::render::cache::quantise_zoom(region_scale);
            let rendered_scale =
                (region_step as f32 * pdf_core::render::cache::ZOOM_QUANTUM).min(region_scale);

            if self.render_state.async_render {
                // Off this thread, like the whole page: the visible part of a
                // zoomed-in page is millions of pixels, and a render of it in the
                // middle of a frame was a stall of its own. Not asked for again
                // while one is already being made that will cover what is on
                // screen — a pan would otherwise ask for a new one every frame.
                let Some(doc) = self.tab_mut().doc.as_ref() else { return };
                let (doc_id, epoch, session) = (doc.id, doc.render_epoch, doc.session.clone());
                let pending_covers = self
                    .render_state.renders
                    .as_ref()
                    .and_then(|w| w.in_flight.get(&(doc_id, page, true)))
                    .is_some_and(|(step, _, pending)| {
                        pending.is_some_and(|c| Self::detail_tile_covers(page, *step, c, page, zoom_step, visible_crop))
                    });
                if !pending_covers {
                    self.ask_for_render(ctx, RenderJob {
                        doc: doc_id,
                        epoch,
                        page,
                        step: zoom_step,
                        rotation: Rotation::None,
                        scale: rendered_scale,
                        crop: Some(crop),
                        session,
                    });
                }
            } else {
                let Some(doc) = self.tab_mut().doc.as_mut() else { return };
                let Ok(raster) = doc.session.render_page_region(page, crop, rendered_scale) else { return };
                let texture = ctx.load_texture(
                    format!("detail{page}"),
                    page_to_image(&raster),
                    egui::TextureOptions::LINEAR,
                );
                doc.caches.detail = Some(DetailTile { page, zoom_step, crop, texture });
                self.render_state.render_stats.detail_on_ui_thread += 1;
            }
        }

        let Some(doc) = self.tab_mut().doc.as_ref() else { return };
        let Some(tile) = doc.caches.detail.as_ref().filter(|t| t.page == page) else { return };
        let screen_rect = egui::Rect::from_min_max(
            view.to_screen(AppPoint::new(tile.crop.left as f64, tile.crop.top as f64)),
            view.to_screen(AppPoint::new(tile.crop.right as f64, tile.crop.bottom as f64)),
        );
        ui.painter().image(
            tile.texture.id(),
            screen_rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
    }

    /// The colour and fill of whatever is selected on the markup layer —
    /// one or more drawn shapes. Applies a change to every selected shape
    /// at once, the same way dragging one moves the whole selection.
    pub(crate) fn draw_shape_properties(&mut self, ui: &mut egui::Ui, page: usize) {
        let height = view_height(self, page);
        let selection: Vec<usize> =
            self.tab_mut().markup.page(page, height).selection().iter().copied().collect();
        if selection.is_empty() {
            return;
        }

        ui.label(format!(
            "{} shape{} selected",
            selection.len(),
            if selection.len() == 1 { "" } else { "s" }
        ));
        ui.add_space(6.0);

        let (r, g, b) = selection
            .first()
            .and_then(|&i| self.tab_mut().markup.page(page, height).resolved_color(i))
            .unwrap_or((0, 0, 0));
        let mut rgb = [r, g, b];
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Colour").color(theme::ink_dim()));
            if ui.color_edit_button_srgb(&mut rgb).changed() {
                let layer = self.tab_mut().markup.page(page, height);
                for &index in &selection {
                    layer.set_color(index, (rgb[0], rgb[1], rgb[2]));
                }
                self.say_info(format!(
                    "{} shape{} recoloured.",
                    selection.len(),
                    if selection.len() == 1 { "" } else { "s" }
                ));
            }
        });

        // Only a closed shape has an inside to fill — a line or an open
        // polyline has nothing `Layer::set_filled` could pair a hatch to
        // that would ever be visible, so the choice is not offered for one.
        let fillable: Vec<usize> = {
            let layer = self.tab_mut().markup.page(page, height);
            selection
                .iter()
                .copied()
                .filter(|&index| {
                    layer.objects().get(index).is_some_and(|o| {
                        matches!(&o.geom, cad_kernel::Geom::Circle(_))
                            || matches!(&o.geom, cad_kernel::Geom::Polyline(p) if p.closed)
                    })
                })
                .collect()
        };
        if !fillable.is_empty() {
            let mut filled = {
                let layer = self.tab_mut().markup.page(page, height);
                fillable.iter().all(|&index| layer.is_filled(index))
            };
            if ui.checkbox(&mut filled, "Filled").changed() {
                let layer = self.tab_mut().markup.page(page, height);
                for &index in &fillable {
                    layer.set_filled(index, filled);
                }
                self.say_info(if filled { "filled." } else { "fill removed." });
            }
        }

        // Every drawn shape has an outline, so thickness applies to the
        // whole selection — not just the closed ones `fillable` picked out.
        let mut mm = selection
            .first()
            .and_then(|&i| self.tab_mut().markup.page(page, height).resolved_lineweight_mm(i))
            .unwrap_or(0.25);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Thickness").color(theme::ink_dim()));
            if ui.add(egui::Slider::new(&mut mm, 0.05..=3.0).suffix(" mm")).changed() {
                let layer = self.tab_mut().markup.page(page, height);
                for &index in &selection {
                    layer.set_lineweight_mm(index, mm);
                }
            }
        });

        // An arrowhead only ever means something on a straight line — see
        // `Layer::arrow_ends` — so this is offered only when the selection
        // has one, exactly like `fillable` gates the Filled checkbox above.
        let lines: Vec<usize> = {
            let layer = self.tab_mut().markup.page(page, height);
            selection
                .iter()
                .copied()
                .filter(|&index| {
                    layer.objects().get(index).is_some_and(|o| matches!(&o.geom, cad_kernel::Geom::Line(_)))
                })
                .collect()
        };
        if !lines.is_empty() {
            let (mut start, mut end) = {
                let layer = self.tab_mut().markup.page(page, height);
                (
                    lines.iter().all(|&i| layer.arrow_ends(i).0),
                    lines.iter().all(|&i| layer.arrow_ends(i).1),
                )
            };
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Arrowhead").color(theme::ink_dim()));
                let mut changed = false;
                changed |= ui.checkbox(&mut start, "Start").changed();
                changed |= ui.checkbox(&mut end, "End").changed();
                if changed {
                    let layer = self.tab_mut().markup.page(page, height);
                    for &index in &lines {
                        layer.set_arrow_ends(index, start, end);
                    }
                    self.say_info("arrowheads updated.");
                }
            });
        }
    }
}

/// The three pieces of wording the passcode dialog needs for whatever it is
/// asking: the heading, the line under it, and the confirm button's label.
/// Pure over what is being asked and whether the passcode is being *used* on
/// an already-locked document (`using` at the call site) — it used to be
/// three long matches inline in `draw_passcode_dialog`.
fn passcode_wording(waiting: &Awaiting, using: bool) -> (String, String, &'static str) {
        let heading = match &waiting {
            Awaiting::Open(_) => "This document needs a password".to_string(),
            Awaiting::Unlock | Awaiting::UnlockItem(_) => "Unlock".to_string(),
            Awaiting::LockAgain { .. } | Awaiting::SecureAgain { .. } => {
                "Type it again".to_string()
            }
            Awaiting::Certificate(_) => "Sign this document".to_string(),
            Awaiting::SecureCurrent(_) => "Change this document's password".to_string(),
            Awaiting::Secure(_) => "Choose a password for this file".to_string(),
            _ if using => "Unlock to add to this document".to_string(),
            _ => "Choose a passcode for this document".to_string(),
        };
        let explains = match &waiting {
            Awaiting::Open(path) => std::path::Path::new(path)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.clone()),
            Awaiting::Unlock | Awaiting::UnlockItem(_) => {
                "The passcode this document was locked with.".into()
            }
            Awaiting::LockAgain { .. } | Awaiting::SecureAgain { .. } => {
                "The same one, so a slip cannot lock you out.".into()
            }
            Awaiting::Certificate(path) => format!(
                "The password on {}. The signature covers the file as it is now.",
                std::path::Path::new(path)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string())
            ),
            Awaiting::SecureCurrent(_) => {
                "Type the password it has now. The new one comes next.".into()
            }
            Awaiting::Secure(_) => {
                "It encrypts the whole file. Nobody can open it without this — \
                 in Pagify or anywhere else."
                    .into()
            }
            _ if using => "The passcode this document is already locked with.".into(),
            _ => "It locks the text and the pictures alike, and it is the only way \
                  back to what is hidden."
                .into(),
        };
        let act = match &waiting {
            Awaiting::Open(_) => "Open",
            Awaiting::Unlock | Awaiting::UnlockItem(_) => "Unlock document",
            Awaiting::Certificate(_) => "Sign",
            Awaiting::SecureCurrent(_) => "Continue",
            Awaiting::Secure(_) | Awaiting::SecureAgain { .. } => "Set password",
            _ => "Lock document",
        };
    (heading, explains, act)
}
