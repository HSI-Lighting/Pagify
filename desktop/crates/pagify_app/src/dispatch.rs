//! Saying things to the command bar (`say_info`/`say_error`), and the two
//! dispatch switchboards: `run` (a parsed [`Dispatch`], Pagify's own verbs
//! split out from the kernel's) and `act` (every [`Verb`]) — the review's
//! `dispatch.rs`.

use crate::hub;
use crate::{
    signature_is_a_warning, signature_line, Awaiting, Closing, DrawKind, FindReplace, SignatureList,
    SignaturePad, SnippetList, Tab, Tool,
};
use pagify_shell::command_plan::{CommandEnv, Effect};
use pagify_shell::command::{Dispatch, Kind};
use pagify_shell::verbs::{self, SignatureAction, Verb};

impl crate::PagifyApp {
    pub(crate) fn say_info(&mut self, text: impl Into<String>) {
        let text = text.into();
        self.recording_state.session_log.record("info", &text);
        self.cmd.say(Kind::Info, text);
    }

    pub(crate) fn say_error(&mut self, text: impl Into<String>) {
        // **Does not open the box.** It used to — "a command that failed
        // silently because its explanation was collapsed out of view is worse
        // than one that never ran" — and was refused from use: an error
        // throwing the whole history panel open over the page is a worse
        // interruption than the one it was reporting. The explanation is not
        // out of view when the box is shut: the collapsed bar prints the last
        // thing said, in red for an error, on the line under the buttons; and
        // the arrow beside the input opens the rest whenever it is wanted.
        let text = text.into();
        self.ui_state.errors_said += 1;
        self.recording_state.session_log.record("error", &text);
        self.cmd.say(Kind::Error, text);
    }

    pub(crate) fn run(&mut self, dispatch: Dispatch) {
        match dispatch {
            Dispatch::Pagify(Verb::Help(topic)) => {
                for l in verbs::help_text(topic.as_deref()) {
                    self.say_info(l);
                }
            }
            Dispatch::Pagify(Verb::Planned { verb, phase }) => {
                self.say_info(format!("`{verb}` is planned for {phase}, and not built yet."));
            }
            Dispatch::Pagify(verb) => self.act(verb),
            Dispatch::Kernel(command) => self.draw(*command),
            ref other => self.cmd.report(other),
        }
    }

    pub(crate) fn act(&mut self, verb: Verb) {
        // Phase 4b is complete: every verb that reaches `act` is planned in
        // the shell (DESIGN_REVIEW.md §3.4) — `run` answers `Help` and
        // `Planned` before this point. A verb with no plan is a bug, not a
        // fall-through, and says so.
        let effects = pagify_shell::command_plan::plan(&verb, &self.command_env())
            .unwrap_or_else(|| unreachable!("unplanned verb reached act: {verb:?}"));
        for effect in effects {
            self.apply_effect(effect);
        }
    }


    /// The small snapshot the shell's `plan` needs to decide a verb's
    /// effects: guards, current page and its height, and whether a pick is
    /// waiting.
    fn command_env(&self) -> CommandEnv {
        let tab = self.tab();
        let page = tab.view_state.page;
        CommandEnv {
            has_doc: tab.doc.is_some(),
            page,
            page_height: tab
                .doc
                .as_ref()
                .and_then(|d| d.strip.size_of(page))
                .map(|(_, h)| h as f64)
                .unwrap_or(792.0),
            tool_armed: tab.tool.is_some(),
            signature_count: self.library_state.signatures.entries().len(),
        }
    }

    /// Apply one planned [`Effect`] — the doing half of the shell's planning
    /// (`command_plan::plan`). The bodies are the ones `act_document` ran.
    fn apply_effect(&mut self, effect: Effect) {
        match effect {
            Effect::OpenPath(path) => self.open(&path.to_string_lossy()),
            Effect::OpenFileDialog => self.open_dialog(),
            Effect::CloseDocument { force } => {
                if !force && self.would_lose_work() {
                    self.tab_mut().closing = Some(Closing::Document);
                    return;
                }
                if self.tab_mut().doc.take().is_some() {
                    self.tab_mut().markup.clear();
                    self.forget_passcode();
                    self.cmd.prompt_mut().document = None;
                    self.tab_mut().ribbon = Tab::File;
                    self.say_info("closed.");
                } else {
                    self.say_error("nothing open.");
                }
            }
            Effect::Quit { force } => {
                if !force {
                    if let Some(index) = self.tab_with_unsaved_work() {
                        self.active_tab = index;
                        self.tab_mut().closing = Some(Closing::Program);
                        // The whole program, not just this window.
                        self.hub_state.win.closing_leaves = hub::Leaving::Program;
                        return;
                    }
                }
                self.leave(if force { hub::Leaving::Now } else { hub::Leaving::Program });
            }
            Effect::GoTo(target) => self.go_to(target),
            Effect::Zoom(target) => self.set_zoom(target),
            Effect::RotatePage(degrees) => self.rotate_view(degrees),
            Effect::ReportTextLayer => self.report_text_layer(true),
            Effect::Say(text) => self.say_info(text),
            Effect::CheckUpdate => self.spawn_update_check(true),
            Effect::ToggleLayers => {
                if self.tab_mut().doc.is_none() {
                    self.say_error("nothing open.");
                    return;
                }
                self.ui_state.show_layers = !self.ui_state.show_layers;
                if self.ui_state.show_layers {
                    self.forget_layers();
                    let page = self.tab().view_state.page;
                    let count = self.layers_on(page).len();
                    self.say_info(format!(
                        "layers: page {} draws {count} thing{}, topmost first. \
                         Pick one, then `bringtofront` or `sendtoback`.",
                        page + 1,
                        if count == 1 { "" } else { "s" }
                    ));
                } else {
                    self.say_info("layers: closed.");
                }
            }
            Effect::RepairLocks => {
                if self.tab_mut().doc.is_none() {
                    self.say_error("nothing open.");
                    return;
                }
                self.repair_locks(true);
            }
            Effect::SetOpacity(percent) => {
                let page = self.tab_mut().view_state.page;
                let target = self.tab_mut()
                    .selection
                    .selected
                    .as_ref()
                    .filter(|s| s.page == page)
                    .map(|s| s.object)
                    .or_else(|| {
                        self.tab_mut().selection.picked_layer.and_then(|at| self.layers_on(page).get(at).map(|d| d.object))
                    });
                match target {
                    Some(object) => match self.set_opacity_of(page, object, percent / 100.0) {
                        Ok(said) => self.say_info(said),
                        Err(e) => self.say_error(e),
                    },
                    None => self.say_info("select something first — Edit Object, or a row in the layer list."),
                }
            }
            Effect::Restack(stacking) => self.restack_picked(stacking),
            Effect::ArmLockArea => {
                if self.tab_mut().doc.is_none() {
                    self.say_error("nothing open.");
                } else {
                    let page = self.tab_mut().view_state.page;
                    self.arm_tool(Tool::Lock, page);
                }
            }
            Effect::LockPages(spec) => {
                let Some(doc) = &self.tab_mut().doc else {
                    self.say_error("nothing open.");
                    return;
                };
                match pagify_shell::organize::parse_range(&spec, doc.page_count) {
                    Ok(pages) => {
                        self.ask_or_reuse_passcode(
                            Awaiting::LockPages(pages),
                            "type a passcode to lock these pages with, or Escape to give up.",
                        );
                    }
                    Err(why) => self.say_error(why),
                }
            }
            Effect::Unlock => {
                if self.tab_mut().doc.is_none() {
                    self.say_error("nothing open.");
                } else if self.tab_mut().doc.as_ref().is_some_and(|d| d.session.locked_pages().is_empty()) {
                    self.say_error("nothing in this document is locked.");
                } else {
                    // Unlike locking, this always asks even when a passcode is
                    // held — see `a_held_passcode_does_not_unlock_anything`.
                    self.tab_mut().secure_state.awaiting_password = Some(Awaiting::Unlock);
                    self.say_info("type the passcode this was locked with, or Escape to give up.");
                }
            }
            Effect::ArmCalibrate { distance, unit } => {
                let page = self.tab_mut().view_state.page;
                self.arm_tool(Tool::Calibrate { distance, unit }, page);
            }
            Effect::ReportScale => {
                let d = self.tab_mut().calibration.describe();
                self.say_info(d);
            }
            Effect::ArmMeasure(kind) => {
                let page = self.tab_mut().view_state.page;
                self.arm_tool(Tool::Measure(kind), page);
            }
            Effect::SayError(text) => self.say_error(text),
            Effect::TakePick(at) => self.take_pick(at),
            Effect::Sensitivity(what) => self.act_sensitivity(what),
            Effect::FillSign(what) => self.act_fill_sign(what),
            Effect::PredefinedText(words) => match words {
                Some(text) => self.use_snippet(&text),
                None => {
                    self.library_state.snippets = Some(SnippetList::default());
                    if self.library_state.predefined.is_empty() {
                        self.say_info(
                            "nothing kept yet — type some words into the window, or \
                             `predefinedtext Jane Smith` to keep and write them at once.",
                        );
                    }
                }
            },
            Effect::TakeUpObjectTool { edit, page } => self.take_up_object_tool(edit, page),
            Effect::ArmSignLine { page } => self.arm_tool(Tool::SignLine, page),
            Effect::ArmSignRectangle { page } => self.arm_tool(Tool::SignRectangle, page),
            Effect::ReportDocumentStatus => {
                for line in self.document_status() {
                    self.say_info(line);
                }
            }
            Effect::ReportSessionLog => match self.recording_state.session_log.path() {
                Some(path) => self.say_info(format!(
                    "recording every command and outcome to {} — send it along with a bug report.",
                    path.display()
                )),
                None => self.say_info(
                    "no session log this run — the config directory could not be written to.",
                ),
            },
            Effect::RunApplySignatures => self.act_apply_signatures(),
            Effect::RunManageSignatures(what) => self.act_manage_signatures(what),
            Effect::OpenSignaturePad { name, then_place } => {
                self.library_state.pad = Some(SignaturePad { name, then_place, ..SignaturePad::default() });
            }
            Effect::UploadSignaturePath(path) => self.upload_signature(&path),
            Effect::UploadSignatureDialog => self.upload_signature_dialog(),
            Effect::ArmSignature { page } => self.arm_tool(Tool::Signature, page),
            Effect::RunValidate => self.act_validate(),
            Effect::RunCertify(path) => self.act_certify(path),
            Effect::ArmWhiteout => {
                if self.tab_mut().doc.is_none() {
                    self.say_error("nothing open.");
                } else {
                    let page = self.tab_mut().view_state.page;
                    self.arm_tool(Tool::Whiteout, page);
                }
            }
            Effect::ArmDrawArrow => {
                if self.tab_mut().doc.is_none() {
                    self.say_error("nothing open.");
                } else {
                    let page = self.tab_mut().view_state.page;
                    self.arm_tool(Tool::Draw(DrawKind::Arrow), page);
                }
            }
            Effect::ToggleFill => {
                self.prefs_state.draw_fill = !self.prefs_state.draw_fill;
                self.say_info(if self.prefs_state.draw_fill {
                    "fill: on — the next rectangle or circle is drawn filled."
                } else {
                    "fill: off — the next rectangle or circle is drawn hollow."
                });
            }
            Effect::ArmRedact => {
                if self.tab_mut().doc.is_none() {
                    self.say_error("nothing open.");
                } else {
                    let page = self.tab_mut().view_state.page;
                    self.arm_tool(Tool::Redact, page);
                }
            }
            Effect::LockWords => {
                if self.tab_mut().doc.is_none() {
                    self.say_error("nothing open.");
                    return;
                }
                // **Selecting the words is the obvious way to say which words.**
                // Asking for two opposite corners of a rectangle is a drawing
                // gesture, and this is not a drawing operation — reported from
                // use as not being what anyone reaches for. A selection already
                // on the page is taken as the answer; otherwise it says to make
                // one, and the rectangle stays available for the cases a
                // selection cannot express, like an area of a scan.
                if self.tab_mut().selection.text_selection.is_some() {
                    self.lock_selection();
                    return;
                }
                self.say_info(
                    "lock: select the words to hide, then `lock` again — or drag a rectangle with `lockarea`.",
                );
            }
            Effect::RunSecure(options) => {
                let Some(doc) = &self.tab_mut().doc else {
                    self.say_error("nothing open.");
                    return;
                };
                // **Refused before a password is typed, not after.**
                //
                // Two passwords cannot both be written — the content would be
                // encrypted twice and open for nobody — and the engine has
                // always refused that. But it refused at *save*, by which time
                // somebody had chosen a password, met the rule and typed it
                // twice for nothing.
                // A document that already has one is *changed*, not refused:
                // the current password first, then the new one.
                if doc.session.already_has_password() {
                    self.tab_mut().secure_state.awaiting_password = Some(Awaiting::SecureCurrent(options));
                    self.say_info("this document has a password — type it to change it.");
                    return;
                }
                if doc.session.is_secured() {
                    self.say_info(
                        "this document already has a password waiting — `unsecure` takes it off.",
                    );
                    return;
                }
                let allowed = options.describe();
                self.tab_mut().secure_state.awaiting_password = Some(Awaiting::Secure(options));
                self.say_info(format!(
                    "type a password for this document, or Escape to give up. \
                     Anyone opening the file will be asked for it — {allowed}."
                ));
            }
            Effect::RunSmartRedact { redact } => self.act_smart_redact(redact),
            Effect::RunHiddenData { clean } => {
                let Some(doc) = &self.tab_mut().doc else {
                    self.say_error("nothing open.");
                    return;
                };
                if clean {
                    // **What is printed is what a second survey of the cleaned
                    // bytes found**, not what the first survey listed. The line
                    // used to print the findings as removals — and for the
                    // attachment and the script, which the clean left alone,
                    // that was a claim with nothing behind it. Found by audit.
                    match doc.session.remove_hidden_data() {
                        Ok(done) => {
                            if let Some(doc) = &mut self.tab_mut().doc {
                                doc.rendered_is_stale();
                            }
                            self.tab_mut().selection.text_selection = None;
                            self.tab_mut().panels.find_hits.clear();
                            let said = done.describe();
                            if done.is_clean() {
                                self.say_info(if done.removed().is_empty() {
                                    format!("{said}.")
                                } else {
                                    format!("{said}. Save to write it out.")
                                });
                            } else {
                                self.say_error(format!("{said}. Save to write out what was removed."));
                            }
                        }
                        Err(e) => self.say_error(e.to_string()),
                    }
                } else {
                    match doc.session.hidden_data() {
                        Ok(found) => self.say_info(if found.is_empty() {
                            found.describe()
                        } else {
                            format!("{} — `hiddendata clean` takes it out.", found.describe())
                        }),
                        Err(e) => self.say_error(e.to_string()),
                    }
                }
            }
            Effect::RunUnsecure => {
                let Some(doc) = &mut self.tab_mut().doc else {
                    self.say_error("nothing open.");
                    return;
                };
                match doc.session.unsecure_document() {
                    Ok(()) => self.say_info("the password is off; save to write it out."),
                    Err(e) => self.say_error(e.to_string()),
                }
            }
            Effect::FinishPick => {
                let closeable = self.tab_mut()
                    .tool
                    .as_ref()
                    .is_some_and(|p| p.kind.ends_on_enter() && p.points.len() >= 2);
                if closeable {
                    self.resolve_tool();
                // No multi-point kind is left with fewer than Enter needs —
                // anything armed at this point is still waiting for more points.
                } else if self.tab_mut().tool.is_some() {
                    self.say_error("not enough points yet.");
                } else {
                    self.say_error("nothing to finish.");
                }
            }
            Effect::SaveFile => {
                self.save(None);
            }
            Effect::SaveFileAs(path) => {
                self.save_as(path);
            }
            Effect::SaveAsDialog => self.save_as_dialog(),
            Effect::SaveShareCopy(path) => self.save_share_copy(path),
            Effect::ShareCopyDialog => self.share_copy_dialog(),
            Effect::RunExtract { pages, dest } => self.extract(&pages, &dest),
            Effect::RunImport { source, pages } => self.import(&source, &pages),
            Effect::RunDeletePages(pages) => self.delete_pages(&pages),
            Effect::RunInsertPage => self.insert_page(),
            Effect::RunMovePages { pages, before } => self.move_pages(&pages, before),
            Effect::RunReflow => self.reflow(),
            Effect::RunFind(needle) => self.find(&needle),
            Effect::RunFindStep { forward } => self.find_step(forward),
            Effect::OpenFindReplace => self.tab_mut().panels.find_replace = Some(FindReplace::default()),
            Effect::OpenSpellCheck => self.open_spell_check(),
            Effect::AddBookmark => self.add_bookmark_here(),
            Effect::ArmArticleBox => {
                if self.tab_mut().doc.is_none() {
                    self.say_error("nothing open.");
                } else {
                    let page = self.tab_mut().view_state.page;
                    self.arm_tool(Tool::ArticleBox, page);
                }
            }
            Effect::BeginWebLink => self.begin_web_link(),
            Effect::BeginJoinText => self.begin_join_text(),
            Effect::BeginMatchProperties => self.begin_match_properties(),
            Effect::RunMarkText(kind) => self.mark_selection(kind),
            Effect::RunListMarks => self.list_marks(),
            Effect::RunRemoveMark(n) => self.remove_mark(n),
            Effect::RunNote(text) => self.add_note(text),
            Effect::UndoRedo { undo } => self.undo_redo(undo),
            Effect::SetPointer(mode) => {
                // Switching tools abandons whatever was half-picked. Leaving a
                // pending operation armed under a new tool is how a click meant
                // for one thing lands in another.
                if let Some(armed) = self.tab_mut().tool.take() {
                    if armed.kind.cancel_drops_checkpoint() {
                        let page = self.tab().view_state.page;
                        if let Some(layer) = self.tab_mut().markup.existing_mut(page) {
                            layer.forget_last_step();
                        }
                    }
                }
                self.tab_mut().pointer = mode;
                self.say_info(match mode {
                    pagify_shell::verbs::PointerMode::Select =>
                        "select — drag over text to select it, or over paper to select marks.",
                    pagify_shell::verbs::PointerMode::Pan => "hand — drag to move the page.",
                });
            }
            Effect::CopyText => {
                if !self.copy_organize_selection() && !self.copy_object_selection() {
                    self.tab_mut().view_state.copy_wanted = true;
                }
            }
            Effect::Paste => {
                // Something copied outside Pagify since its own last copy is
                // what was meant — the system clipboard knows which came last.
                let outside = self.tab().doc.is_some().then(|| (self.clipboard_state.read_outside)()).flatten();
                if let Some(outside) = outside {
                    let content = self.outside_content(outside);
                    self.pick_up(content);
                } else if self.current_page_clipboard().is_some() {
                    self.paste_organize_selection();
                } else {
                    self.paste_object_selection();
                }
            }
            Effect::EditText => self.edit_text(),
            Effect::AddText(text) => self.add_text(text),
            Effect::AddImage(path) => match path {
                Some(path) => self.add_image(&path),
                None => self.add_image_dialog(),
            },
            Effect::SetLayout(layout) => self.set_layout(layout),
            Effect::RunReversePages => self.reverse_pages(),
            Effect::ToggleOrganizeGrid => self.toggle_organize_grid(),
            Effect::ToggleAppearance => self.toggle_appearance(),
            Effect::RunDuplicatePages(spec) => self.duplicate_pages(&spec),
            Effect::RunCropPages { pages, margin } => self.crop_pages(&pages, margin),
            Effect::RunResizePages { pages, width_pt, height_pt } => {
                self.resize_pages(&pages, width_pt, height_pt)
            }
            Effect::RunSwapPages { a, b } => self.swap_pages(a, b),
            Effect::RunRotatePages { pages, quarters } => self.rotate_pages(&pages, quarters),
            Effect::RunExtractText(spec) => self.extract_text(&spec),
            Effect::RunOutlinedFont(action) => self.outlined_font_action(action),
            Effect::ClearRecent => {
                // The list is a record of what somebody has been reading;
                // clearing it removes the file, and says what else is kept
                // beside it so they can decide about those too.
                let where_kept = pagify_shell::state::state_dir()
                    .map(|d| d.display().to_string())
                    .unwrap_or_else(|| "nowhere on this system".into());
                match self.library_state.recent.forget_all() {
                    Ok(()) => self.say_info(format!(
                        "the recent-documents list is gone. Pagify keeps its own files in \
                         {where_kept}: predefined.json (saved texts), signatures.json (drawn \
                         signatures), outlined_fonts.json (glyph shapes learned from documents), \
                         and scripts/ (recordings) — delete any of them there."
                    )),
                    Err(e) => self.say_error(format!("could not remove the list: {e}")),
                }
            }
        }
    }



    /// What `sensitivity` does. Moved out of `act_tools` whole.
    fn act_sensitivity(&mut self, what: Option<String>) {
                use pdf_core::document::sensitivity::Sensitivity;
                let Some(doc) = &self.tab_mut().doc else {
                    self.say_error("nothing open.");
                    return;
                };
                match what {
                    // Reporting.
                    None => {
                        let said = match doc.session.sensitivity() {
                            Some(level) => format!(
                                "marked {} — {}. `sensitivity none` takes it off.",
                                level.stamp(),
                                level.describe()
                            ),
                            None => "not marked. Try `sensitivity confidential`, or \
                                     public, internal or secret."
                                .to_string(),
                        };
                        self.say_info(said);
                    }
                    // Taking it off.
                    Some(word) if word.is_empty() => match doc.session.clear_sensitivity() {
                        Ok(()) => {
                            if let Some(doc) = &mut self.tab_mut().doc {
                                doc.rendered_is_stale();
                            }
                            self.say_info("the marking is off. Save to write it out.");
                        }
                        Err(e) => self.say_error(e.to_string()),
                    },
                    // Marking.
                    Some(word) => {
                        let Some(level) = Sensitivity::parse(&word) else {
                            self.say_error(format!("sensitivity: don't know {word:?}."));
                            return;
                        };
                        match doc.session.set_sensitivity(level) {
                            Ok(()) => {
                                if let Some(doc) = &mut self.tab_mut().doc {
                                    doc.rendered_is_stale();
                                }
                                // Says plainly what a marking is and is not.
                                // Somebody reaching for it may believe it
                                // protects the document; it does not.
                                self.say_info(format!(
                                    "marked {} on every page — {}. This says what you \
                                     intend, it does not enforce it: `secure` is what \
                                     withholds a document. Save to write it out.",
                                    level.stamp(),
                                    level.describe()
                                ));
                            }
                            Err(e) => self.say_error(e.to_string()),
                        }
                    }
                }
    }

    /// What `fillsign`/`signcheck`/`signcross`/`signdot` do. Moved out of
    /// `act_tools` whole.
    fn act_fill_sign(&mut self, what: Option<String>) {
                if self.tab_mut().doc.is_none() {
                    self.say_error("nothing open.");
                    return;
                }
                let page = self.tab_mut().view_state.page;
                match what.as_deref().and_then(pdf_core::document::FillMark::parse) {
                    // A tick, a cross or a dot: one click each.
                    Some(mark) => self.arm_tool(Tool::Fill(mark), page),
                    // Bare `fillsign` types where you click, which is the other
                    // half of filling a form in by hand.
                    None => {
                        self.arm_tool(Tool::PickText, page);
                        self.say_info(
                            "fill: click a line of text to change it, or use \
                             `addtext <words>` to write somewhere new — and \
                             `fillsign tick`, `cross` or `dot` for a box.",
                        );
                    }
                }
    }



    /// Everything `applysignatures` does: run the confirmation pass over every
    /// page, place the saved signatures and say what happened. Moved out of
    /// `act_signing` whole — it is a page walk, not a dispatch arm.
    fn act_apply_signatures(&mut self) {
                let Some(doc) = &self.tab_mut().doc else {
                    self.say_error("nothing open.");
                    return;
                };
                let pages = doc.page_count;
                let mut applied = 0usize;
                let mut touched = Vec::new();
                for page in 0..pages {
                    match doc.session.apply_signatures(page) {
                        Ok(0) => {}
                        Ok(count) => {
                            applied += count;
                            touched.push(page + 1);
                        }
                        Err(e) => {
                            self.say_error(format!("page {}: {e}", page + 1));
                            return;
                        }
                    }
                }

                if applied == 0 {
                    // Not a failure: a document with no signatures placed on it
                    // is the ordinary case for this verb being pressed by
                    // mistake.
                    self.say_info(
                        "no signatures are placed on this document. `signature` places one.",
                    );
                    return;
                }
                if let Some(doc) = &mut self.tab_mut().doc {
                    doc.rendered_is_stale();
                }
                // Applied signatures are page content now, not annotations —
                // a selection naming one by its old annotation index would
                // be pointing at nothing, or worse, at whatever else now
                // sits at that index.
                self.tab_mut().selection.signature_selected = None;
                self.tab_mut().selection.signature_grab = None;
                // Any other annotation on the page — including a selected
                // placed picture — just had its own index shift under it,
                // for the same reason.
                self.tab_mut().selection.placed_image_selected = None;
                self.tab_mut().selection.placed_image_grab = None;
                // **Says the two things that matter and are not obvious**: that
                // they can no longer be picked up, and that the way back is to
                // close without saving rather than to press undo.
                self.say_info(format!(
                    "{applied} signature{} on page{} {} {} part of the page now — \
                     nothing can select or delete {} any more. Undo does not reach \
                     this; closing without saving does.",
                    if applied == 1 { "" } else { "s" },
                    if touched.len() == 1 { "" } else { "s" },
                    touched.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", "),
                    if applied == 1 { "is" } else { "are" },
                    if applied == 1 { "it" } else { "them" },
                ));
    }

    /// Everything `managesignatures` does, in its several sub-actions. Moved
    /// out of `act_signing` whole for the same reason as its sibling above.
    fn act_manage_signatures(&mut self, what: pagify_shell::verbs::Signatures) {
                use pagify_shell::verbs::Signatures as What;
                match what {
                    What::Open => {
                        self.library_state.signature_list = Some(SignatureList::default());
                        if self.library_state.signatures.is_empty() {
                            self.say_info("no signatures yet — the window has a button to draw one.");
                        }
                    }
                    What::List => {
                        let said = self.signature_list_lines();
                        self.say_info(said);
                    }
                    What::Use(name) => {
                        if self.library_state.signatures.choose(&name) {
                            match self.keep_signatures() {
                                Ok(()) => {
                                    let chosen = self.library_state
                                        .signatures
                                        .current()
                                        .map(|s| s.name.clone())
                                        .unwrap_or(name);
                                    self.say_info(format!("`signature` now places \"{chosen}\"."));
                                }
                                Err(e) => self.say_error(e),
                            }
                        } else {
                            // Says what there is, rather than choosing for them.
                            self.say_error(format!(
                                "no signature called \"{name}\". There is: {}",
                                self.signature_list_lines()
                            ));
                        }
                    }
                    What::Forget(name) => {
                        if self.library_state.signatures.remove(&name) {
                            match self.keep_signatures() {
                                // Said plainly: this one does not come back.
                                Ok(()) => self.say_info(format!(
                                    "\"{name}\" is gone — a drawing is not something undo \
                                     reaches. {}",
                                    self.signature_list_lines()
                                )),
                                Err(e) => self.say_error(e),
                            }
                        } else {
                            self.say_error(format!(
                                "no signature called \"{name}\". There is: {}",
                                self.signature_list_lines()
                            ));
                        }
                    }
                    What::Rename(to) => {
                        let Some(from) = self.library_state.signatures.current().map(|s| s.name.clone()) else {
                            self.say_error(
                                "no signatures drawn yet — `signature draw` makes one.",
                            );
                            return;
                        };
                        match self.library_state.signatures.rename(&from, &to) {
                            Ok(()) => match self.keep_signatures() {
                                Ok(()) => self.say_info(format!("\"{from}\" is now \"{to}\".")),
                                Err(e) => self.say_error(e),
                            },
                            Err(why) => self.say_error(format!(
                                "{} — {}",
                                why.describe(),
                                self.signature_list_lines()
                            )),
                        }
                    }
                }
    }




    /// What `validate` does: run the validation pass and report what it found.
    /// Moved out of `act_security` whole — it is a document walk, not an arm.
    fn act_validate(&mut self) {
                let Some(doc) = &self.tab_mut().doc else {
                    self.say_error("nothing open.");
                    return;
                };
                match doc.session.validate_signatures() {
                    Ok(found) if found.is_empty() => {
                        // Not a failure, and not phrased as one.
                        self.say_info("this document carries no signatures.");
                    }
                    Ok(found) => {
                        let said: Vec<String> =
                            found.iter().map(|s| signature_line("signature", s)).collect();
                        let all_well = !found.iter().any(signature_is_a_warning);
                        let line = said.join(" | ");
                        if all_well {
                            self.say_info(line);
                        } else {
                            self.say_error(line);
                        }
                    }
                    Err(e) => self.say_error(e.to_string()),
                }
    }

    /// What `certify`/`sign` does: write the certification signature at
    /// `path`, or ask for one. Moved out of `act_security` whole.
    fn act_certify(&mut self, path: Option<std::path::PathBuf>) {
                let Some(doc) = &self.tab_mut().doc else {
                    self.say_error("nothing open.");
                    return;
                };
                match path {
                    // Reporting.
                    None => {
                        let count = doc.session.signature_count();
                        self.say_info(if count == 0 {
                            "not signed. `certify <certificate.p12>` signs it.".to_string()
                        } else {
                            format!(
                                "{count} signature{}. Anything written after a \
                                 signature breaks it.",
                                if count == 1 { "" } else { "s" }
                            )
                        });
                    }
                    Some(path) => {
                        if !path.is_file() {
                            self.say_error(format!("{}: no such file.", path.display()));
                            return;
                        }
                        self.tab_mut().secure_state.awaiting_password = Some(Awaiting::Certificate(path));
                        self.say_info("type the certificate's password, or Escape to give up.");
                    }
                }
    }


    /// The whole Smart Redact pass, moved out of `act_security`: it is a page
    /// walk with its own confirmation and result message, not a dispatch arm.
    fn act_smart_redact(&mut self, redact: bool) {
                let Some(doc) = &self.tab_mut().doc else {
                    self.say_error("nothing open.");
                    return;
                };
                // Every page, because the thing somebody is looking for is
                // rarely on the one they happen to be reading.
                let mut found = Vec::new();
                for page in 0..doc.page_count {
                    match doc.session.sensitive_on(page) {
                        Ok(on_page) => found.extend(on_page),
                        Err(e) => {
                            self.say_error(format!("page {}: {e}", page + 1));
                            return;
                        }
                    }
                }
                if found.is_empty() {
                    self.say_info(
                        "nothing found that can be checked — addresses, card numbers, \
                         account numbers and telephone numbers are what this looks for.",
                    );
                    return;
                }

                if !redact {
                    // Named, with their pages, so a person can look before
                    // anything happens to them.
                    let mut said: Vec<String> = found
                        .iter()
                        .take(12)
                        .map(|f| {
                            format!("p{} {}: {}", f.page_index + 1, f.kind.describe(), f.text)
                        })
                        .collect();
                    if found.len() > said.len() {
                        said.push(format!("and {} more", found.len() - said.len()));
                    }
                    self.say_info(format!(
                        "{} found — {}. `smartredact redact` blacks them out.",
                        found.len(),
                        said.join("; ")
                    ));
                    return;
                }

                // Destroyed, not hidden: this is redaction, and there is no
                // passcode to bring any of it back.
                //
                // **"Gone for good" is only said of what is proven gone.** Each
                // area is surveyed first, and anything the survey says would
                // survive — nested content, an image, type drawn as curves —
                // is reported against that item, not folded into a count of
                // successes. Found by audit: a card number drawn through a
                // form was reported "redacted — gone for good" while the
                // number was still extractable from the saved file.
                let mut gone = 0usize;
                let mut partly: Vec<String> = Vec::new();
                let mut left: Vec<String> = Vec::new();
                let faces = self.outlined_font_bytes();
                let borrowed: Vec<&[u8]> = faces.iter().map(Vec::as_slice).collect();
                let Some(doc) = &self.tab_mut().doc else { return };
                for item in &found {
                    let survives: Vec<String> = doc
                        .session
                        .preview_redaction(item.page_index, item.area, &borrowed)
                        .map(|report| report.blockers().iter().map(|b| b.describe()).collect())
                        .unwrap_or_default();
                    // Through the command, so each one lands in the history and
                    // can be undone one at a time — the same as a redaction
                    // somebody drew by hand.
                    let outcome = doc.session.execute(pdf_core::command::Command::Redact {
                        page_index: item.page_index,
                        area: item.area,
                        fill: Some(pdf_core::document::Color { r: 0, g: 0, b: 0, a: 255 }),
                        // These were found *by* their text, so there is text to
                        // clear; an image crossing the edge must not stop the
                        // rest of the page being done — it is reported instead.
                        // The engine still refuses to paint over words it
                        // removed none of, whatever this says.
                        allow_incomplete: true,
                        outlined_fonts: faces.clone(),
                    });
                    let page = item.page_index + 1;
                    match outcome {
                        Ok(_) if survives.is_empty() => gone += 1,
                        Ok(_) => partly.push(format!("p{page} {}: {}", item.text, survives.join("; "))),
                        Err(e) => left.push(format!("p{page} {}: {e}", item.text)),
                    }
                }
                if let Some(doc) = &mut self.tab_mut().doc {
                    doc.rendered_is_stale();
                }
                self.tab_mut().selection.text_selection = None;
                self.tab_mut().panels.find_hits.clear();
                if partly.is_empty() && left.is_empty() {
                    self.say_info(format!("{gone} redacted — gone for good. Save to write it out."));
                } else {
                    let mut said = vec![format!("{gone} gone for good")];
                    if !partly.is_empty() {
                        said.push(format!(
                            "{} NOT fully cleared — words removed, but something under the \
                             area may still hold them ({})",
                            partly.len(),
                            partly.join(", ")
                        ));
                    }
                    if !left.is_empty() {
                        said.push(format!(
                            "{} left untouched, still in the document ({})",
                            left.len(),
                            left.join(", ")
                        ));
                    }
                    self.say_error(format!("{}. Save to write out what was removed.", said.join("; ")));
                }
    }






}
