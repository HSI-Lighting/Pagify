//! `PagifyApp`'s own workspace methods: which tab is showing, opening and
//! closing documents, leaving the window.

use crate::hub;
use crate::{spawn_update_script, Awaiting, Closing, Doc, DocTab, Tab, NEXT_DOC_ID};
use pagify_shell::measure::Calibration;
use pagify_shell::reader::{Strip, PAGE_GAP_PT};
use pagify_shell::Session;
use pdf_core::error::PdfError;

impl crate::PagifyApp {
    /// The document showing right now — see [`Self::active_tab`].
    pub(crate) fn tab(&self) -> &DocTab {
        &self.tabs[self.active_tab]
    }

    /// The document showing right now, mutably — see [`Self::active_tab`].
    pub(crate) fn tab_mut(&mut self) -> &mut DocTab {
        &mut self.tabs[self.active_tab]
    }

    /// Close one tab — the tab strip's × and Ctrl+W. Unlike the `close`
    /// command, which empties a tab's document and leaves the (now
    /// backstage) tab in place, this removes the tab outright: there is no
    /// blank tab to fall back to, so closing the only one left closes the
    /// window — which, if it is the last window, quits the app, matching what
    /// closing today's one and only window does.
    pub(crate) fn close_tab(&mut self, index: usize) {
        if self.would_lose_work_in_tab(index) {
            self.active_tab = index;
            self.tab_mut().closing = Some(Closing::Tab(index));
            return;
        }
        if self.tabs.len() == 1 {
            self.leave(hub::Leaving::Window);
            return;
        }
        self.remove_tab(index);
    }

    /// Put the tab showing among the tabs the strip has room for. `visible` is
    /// how many tabs it drew. A hidden one — chosen from the menu, or left
    /// showing when another was closed — takes the place of the last tab that
    /// is drawn, which goes behind the menu instead.
    ///
    /// Not while a tab is being carried, or an unsaved-changes question is up:
    /// both name their tab by its place in the strip.
    pub(crate) fn bring_active_tab_into_the_strip(&mut self, visible: usize) {
        if visible == 0 || visible >= self.tabs.len() || self.active_tab < visible {
            return;
        }
        if self.carrying_a_tab() || self.tabs.iter().any(|t| t.closing.is_some()) {
            return;
        }
        self.tabs.swap(visible - 1, self.active_tab);
        self.active_tab = visible - 1;
    }

    /// Bookkeeping shared by every way a tab goes away — take it out, then keep
    /// `active_tab` pointing at a tab that still exists. Left unadjusted if
    /// that empties `tabs` entirely: only a window about to be dropped (a tab
    /// dragged out of its last place — see [`hub`]) is ever left like that,
    /// and nothing draws it again.
    pub(crate) fn remove_tab(&mut self, index: usize) -> DocTab {
        let tab = self.tabs.remove(index);
        if self.tabs.is_empty() {
            return tab;
        }
        if self.active_tab >= self.tabs.len() {
            self.active_tab = self.tabs.len() - 1;
        } else if self.active_tab > index {
            self.active_tab -= 1;
        }
        tab
    }

    /// Say that this window is done — the one place `quit`, a closed last tab
    /// and `Closing::Program` all end, rather than exiting directly, so an
    /// update staged by **Update now** is never dropped on the floor by
    /// whichever of them happens to run. What follows is the `hub::Hub`'s
    /// to decide: a window going is only that, and the program ends when no
    /// window is left (see [`hub::plan`]).
    pub(crate) fn leave(&mut self, how: hub::Leaving) {
        self.win.leaving = Some(how);
    }

    /// Carry out an update staged by **Update now**, then exit — what the
    /// program does once every window has said it is leaving.
    pub(crate) fn exit_program(pending_update: Option<&std::path::Path>) -> ! {
        if let Some(source) = pending_update {
            spawn_update_script(source);
        }
        std::process::exit(0);
    }

    pub(crate) fn open(&mut self, path: &str) {
        self.open_with(path, None)
    }

    /// Open, asking for a password if the file turns out to need one.
    ///
    /// A refusal is not the right answer here. A great many working documents
    /// are encrypted — a catalogue extract saved out of another editor, a
    /// drawing issued under restriction — and "document is password protected"
    /// with no way to supply one is a dead end in a program whose whole
    /// interface is a place to type things.
    pub(crate) fn open_with(&mut self, path: &str, password: Option<&str>) {
        let typing = self.outlined_font_bytes();
        let opened = Session::open_with_password(path, password).and_then(|session| {
            let count = session.page_count()?;
            let sizes = session.page_sizes()?;
            // The same fonts the outline matcher uses — the bundled ones plus
            // whatever the reader has added. A font good enough to recognise a
            // page's letters by is a font good enough to write them with, and
            // asking somebody to add the same file twice would be a poor joke.
            session.set_typing_fonts(typing)?;
            Ok((session, count, sizes))
        });

        match opened {
            Err(PdfError::PasswordRequired) | Err(PdfError::IncorrectPassword) => {
                let again = password.is_some();
                self.tab_mut().awaiting_password = Some(Awaiting::Open(path.to_string()));
                self.say_info(if again {
                    "that password was not accepted — type it again, or Escape to give up."
                } else {
                    "this file is encrypted. Type its password, or Escape to give up."
                });
            }
            Ok((session, page_count, sizes)) => {
                // A document already open keeps its own tab — every
                // successful open lands in a fresh one, except the still-
                // empty tab this app started in (or one left empty by a
                // password prompt that hasn't succeeded yet). Deciding this
                // on success, not before asking for a password, means a
                // failed or abandoned password attempt never leaves a stray
                // blank tab behind — the tab that asked is the tab that
                // either gets filled or stays exactly as it was.
                if self.tab().doc.is_some() {
                    // The newest tab is the leftmost: the strip is one row, and
                    // the oldest are the ones that fall behind its menu.
                    self.tabs.insert(0, DocTab::new());
                    self.active_tab = 0;
                    // A question already up names its tab by place, and every
                    // tab has just moved one place along.
                    for tab in self.tabs.iter_mut().skip(1) {
                        if let Some(Closing::Tab(at)) = &mut tab.closing {
                            *at += 1;
                        }
                    }
                }

                let name = session
                    .path()
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.to_string());

                self.tab_mut().markup.clear();
                self.tab_mut().calibration = Calibration::default();

                // Anything this document already carries comes back as live
                // geometry, not as ink — phase 8's whole point.
                let mut restored = 0;
                for page in 0..page_count {
                    if let Ok(Some(layer)) = session.restore_markup(page) {
                        restored += layer.len();
                        let height = layer.space().height_pt();
                        *self.tab_mut().markup.page(page, height) = layer;
                    }
                }

                // Land on the document rather than leaving the backstage view
                // covering the thing that was just opened.
                if self.tab_mut().ribbon == Tab::File {
                    self.tab_mut().ribbon = Tab::Home;
                }
                self.recent.record(session.path(), page_count, pagify_shell::recent::now());
                if !cfg!(test) {
                    self.recent.save();
                }
                self.cmd.prompt_mut().document = Some(name.clone());
                self.say_info(format!(
                    "{name} — {page_count} page{}.{}",
                    if page_count == 1 { "" } else { "s" },
                    if restored > 0 { format!(" {restored} marks restored.") } else { String::new() }
                ));
                // **Said, because it was not as it was found.** An earlier build
                // damaged the file when it saved it (the cross-reference stream);
                // it was mended as it opened, in memory. Saving writes a sound one.
                if session.repaired_on_open() {
                    self.say_info(format!(
                        "{name} had been damaged by a save in an older version of Pagify. \
                         It was repaired as it opened; save it to keep the repair."
                    ));
                }

                self.tab_mut().doc = Some(Doc {
                    session: std::sync::Arc::new(session),
                    id: NEXT_DOC_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                    render_epoch: 0,
                    strip: Strip::new(&sizes, PAGE_GAP_PT),
                    page_count,
                    caches: Default::default(),
                });
                self.tab_mut().page = 0;
                self.tab_mut().zoom_settle.zoom_basis = 0;
                // A passcode belongs to the document it was typed for, and this
                // is a different one.
                self.forget_passcode();
                // **The password that opened the file locks the same file's
                // own content too, by default.** Reported from use: having
                // just typed the one password this document asks for, being
                // asked for it again to lock or unlock a passage inside it
                // read as the same password not being remembered rather than
                // as a second, deliberately different one. `hold_or_drop`
                // still drops it the moment it fails against an actual lock,
                // so a document whose content really is sealed under a
                // different passcode only ever asks once more, not every time.
                if let Some(password) = password {
                    self.tab_mut().held_passcode = Some(zeroize::Zeroizing::new(password.to_owned()));
                }
                self.tab_mut().view_state.saved_revision = self.tab_mut().markup.revision();

                // **A badge over a picture that is still there is finished
                // now, not carried.** Documents written while a lock could
                // record its badge and then fail to take the picture off the
                // page show a chequerboard over a picture that is plainly
                // still drawn — reported from use as a grey layer that could
                // not be selected or sent back. Repaired on open, and said.
                self.repair_locks(false);
                // Said once, on opening, and only when there is something wrong
                // — otherwise selection silently doing nothing is left for the
                // reader to work out.
                self.report_text_layer(false);
                self.tab_mut().view_state.scroll_pt = 0.0;
                self.tab_mut().tool = None;
                // A fresh document's own bookmarks, not whatever the last
                // one left behind — the panel is closed already (nothing
                // above reopens it), but the page-corner icon's cache is
                // not tied to the panel and would otherwise still be
                // pointing at the previous file's pages.
                self.sync_bookmarks();
            }
            Err(e) => {
                // The raw error goes in the session log whatever is shown.
                self.session_log.record("open-error", &format!("{path}: {e}"));
                match e {
                    // The library itself is missing: where it was looked for is
                    // the one useful thing to say.
                    PdfError::LibraryUnavailable(_) => {
                        self.say_error(format!("{e}"));
                        let where_ = pagify_shell::pdfium::describe();
                        self.say_info(format!("PDFium was looked for at: {where_}"));
                    }
                    // **The file, not the library.** "PdfiumLibraryInternalError(
                    // Unknown)" followed by where PDFium was looked for read as a
                    // broken install, when PDFium had loaded and only this file had
                    // not — reported from use, twice, on a document an older build
                    // had damaged.
                    PdfError::Pdfium(_) | PdfError::MalformedDocument => {
                        let name = std::path::Path::new(path)
                            .file_name()
                            .map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned());
                        let why = pagify_shell::diagnose::why_unreadable(std::path::Path::new(path))
                            .unwrap_or_else(|| "its structure is damaged".to_string());
                        self.say_error(format!("{name} could not be opened — {why}."));
                        self.say_info(
                            "Nothing is wrong with Pagify itself. If this file opened before, \
                             it may have been damaged since: send the file with the session log \
                             (`sessionlog` says where it is).",
                        );
                    }
                    other => self.say_error(format!("{other}")),
                }
            }
        }
    }
}
