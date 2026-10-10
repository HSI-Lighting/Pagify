//! Arming a tool, resolving a pick once it has what it wants, and the click
//! that leaves an open run editor.
//!
//! `resolve_tool`'s own dispatch match moved into `Tool::on_click`
//! (`tool.rs`) — `Pagify-Phase2-BigTasks.md` §2.3 steps 2–3. This file now
//! only arms, collects picks, and applies the `ToolEffect` that comes back.

use crate::{ArmedTool, Tool, ToolEffect};
use pagify_shell::command::Kind;
use pagify_shell::markup::HIT_TOLERANCE_PT;
use pagify_shell::page_space::AppPoint;

impl crate::PagifyApp {
    pub(crate) fn arm_tool(&mut self, tool: Tool, page: usize) {
        self.arm_tool_without_saying(tool, page);
        let prompt = self.tab().tool.as_ref().map(|t| t.kind.prompt(0, 0));
        if let Some(prompt) = prompt {
            self.say_info(prompt);
        }
    }

    /// [`Self::arm_tool`] without the announcement — for putting a
    /// repeating tool back in hand *after something the person has to
    /// read*. `resolve_tool` uses it to re-arm quietly after a failure,
    /// so the error stays the last thing said instead of being covered by
    /// the tool's own prompt in the same frame (reported from use: a
    /// click that found no text said so, and the tool re-armed itself and
    /// said "click the words to change" over it in the same frame, so the
    /// error only ever showed in the history, which is folded away by
    /// default).
    pub(crate) fn arm_tool_without_saying(&mut self, tool: Tool, page: usize) {
        // The object tool and an armed `Tool` are mutually exclusive — see
        // `take_up_object_tool`'s own clearing of `self.tab_mut().tool` for
        // the other direction. **Reported from use, with a screenshot:
        // after using Edit Object, arming Edit Text left both ribbon
        // buttons lit at once.** Worse than the cosmetic double-highlight:
        // every click kept reaching the object tool's own click-to-select
        // instead of resolving the pick this was arming, because
        // `interact` checks `self.tab_mut().object_tool.is_some()` first
        // and takes the pointer outright when it is.
        if self.tab_mut().object_tool.take().is_some() {
            self.tab_mut().selection.selected = None;
            self.tab_mut().selection.grab = None;
            self.tab_mut().selection.group = Vec::new();
            self.tab_mut().selection.marquee = None;
            self.tab_mut().selection.group_grab = None;
        }
        self.put_down_page_editors("armed a different tool");
        self.tab_mut().tool = Some(ArmedTool { kind: tool, page, objects: Vec::new(), points: Vec::new() });
    }

    /// A click on the page away from the open editor: **applies what was typed**,
    /// except that an edit the engine has already refused, and that has not
    /// changed since, is let go instead of being asked again. Without that a refused
    /// editor would be a trap: every click away would apply it, be refused, and
    /// bring it back, with only Escape — which discards — to get out. The words go
    /// to the clipboard, so letting go loses nothing.
    ///
    /// Returns whether the editor is still open afterwards, so that the click is
    /// not also taken as a pick of whatever is under it.
    pub(crate) fn leave_editor_by_click(&mut self) -> bool {
        let already_refused = self
            .tab()
            .edit.editing_run
            .as_ref()
            .is_some_and(|edit| edit.refusal.as_ref().is_some_and(|refusal| refusal.matches(edit)));
        if already_refused {
            if let Some(edit) = self.tab_mut().edit.editing_run.take() {
                self.offer_typed_text(&edit);
                // The reason was said when it was refused, and is not said again:
                // it can hold the very words that were typed, and the log is not
                // the place for them.
                self.say_info(
                    "let go of the edit the engine refused — nothing was changed, and what was typed is on the clipboard.",
                );
            }
            return false;
        }
        self.apply_editing_page();
        self.tab().edit.editing_run.is_some()
    }

    // -- picks --------------------------------------------------------------

    pub(crate) fn take_pick(&mut self, at: AppPoint) {
        if let Some(armed) = self.tab_mut().tool.as_ref() {
            let page = armed.page;
            let wants_object = armed.objects.len() < armed.kind.wants_objects();
            if wants_object {
                // A generous tolerance: you are aiming at a line with a
                // mouse, and a miss here costs the whole operation.
                let hit = self.tab_mut()
                    .markup
                    .existing(page)
                    .and_then(|layer| layer.hit(at, HIT_TOLERANCE_PT * 3.0));
                match hit {
                    Some(index) => {
                        if let Some(armed) = self.tab_mut().tool.as_mut() {
                            armed.objects.push((index, at));
                        }
                    }
                    // Quietest possible miss: `tool` is not touched at all,
                    // not even to record the attempt.
                    None => {
                        self.say_info("nothing there — click on a mark.");
                        return;
                    }
                }
            } else if let Some(armed) = self.tab_mut().tool.as_mut() {
                armed.points.push(at);
            }
            let ready = self.tab_mut().tool.as_ref().is_some_and(|t| {
                t.objects.len() >= t.kind.wants_objects() && t.points.len() >= t.kind.wants_points()
            });
            if ready {
                self.resolve_tool();
            } else if let Some(armed) = self.tab_mut().tool.as_ref() {
                let prompt = armed.kind.prompt(armed.objects.len(), armed.points.len());
                self.say_info(prompt);
            }
        }
    }

    /// Applies whatever `Tool::on_click` says should happen, and runs the
    /// generic re-arm decision afterward — `repeats()`, quietly after a
    /// failure — unconditioned on which effect came back, since that
    /// decision is the same for every kind alike and was never actually
    /// part of any one arm's own logic (see `Tool::on_click`'s own doc).
    /// This is the one place left that calls `self.say_info`/`self.say_error`
    /// for a tool's own resolution, and the one place that knows what a
    /// `ToolEffect` means.
    pub(crate) fn resolve_tool(&mut self) {
        let Some(armed) = self.tab_mut().tool.take() else { return };
        let page = armed.page;
        let repeats = armed.kind.repeats();
        let kind_for_rearm = armed.kind.clone();
        let effect = armed.kind.on_click(self, page, armed.objects, armed.points);
        let failed = matches!(effect, ToolEffect::Say(Kind::Error, _));
        match effect {
            ToolEffect::Say(Kind::Error, text) => self.say_error(text),
            ToolEffect::Say(_, text) => self.say_info(text),
            ToolEffect::OpenPasscodePrompt(awaiting) => self.ask_or_reuse_passcode(
                awaiting,
                "type a passcode to lock it with, or Escape to give up.",
            ),
            ToolEffect::OpenArticleBoxPrompt(pending) => {
                self.tab_mut().panels.pending_article_box = Some(pending);
            }
            ToolEffect::None | ToolEffect::Cancelled => {
                unreachable!("on_click never returns these — only on_pointer/on_cancel do")
            }
            ToolEffect::Finish => {
                unreachable!("on_click never returns Finish — only on_key does, and the frame applies it itself")
            }
        }
        // Back in hand, ready for the next one, quietly after a failure so
        // the error stays the last thing said — see `arm_tool_without_saying`'s
        // doc.
        if repeats && self.tab_mut().edit.editing_run.is_none() {
            if failed {
                self.arm_tool_without_saying(kind_for_rearm, page);
            } else {
                self.arm_tool(kind_for_rearm, page);
            }
        }
    }
}
