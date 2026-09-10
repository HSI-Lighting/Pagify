//! The focus guard — build plan §5.5.
//!
//! SIMLUX's own documentation carries the warning, and it transfers verbatim
//! because it is a property of immediate-mode UI rather than of CAD:
//!
//! > Read globally and acted on regardless, Enter repeats the last command and
//! > Delete erases the selection, so **typing a number into a wall-height box
//! > could empty your drawing**.
//!
//! Pagify will have dozens of numeric fields — line width, font size, page
//! range, opacity, redaction padding. Every one is a loaded gun pointed at the
//! document unless the global key cascade is gated on where focus actually is.
//!
//! ## Why it is captured at frame start
//!
//! The subtlety that makes this hard: a field which commits on Enter
//! *surrenders* focus while it is being drawn. Asking afterwards returns
//! nothing — exactly when it matters. So focus is read once, at the top of the
//! frame, before any widget runs, and every decision below uses that snapshot.
//!
//! ## Why it is an allow-list
//!
//! The plan says to gate the cascade on "the command box actually holding
//! focus". Stated as a deny-list — *block the keys while a field has focus* —
//! that is only as good as the list of fields, and the first widget someone
//! forgets to register is the one that eats a document.
//!
//! So it is inverted here. Document keys fire only when **nothing at all** has
//! focus, and submit keys only when the command box itself does. A numeric
//! field added in phase 3 is then safe by construction, because it is not on
//! the allow-list and nobody has to remember to add it to anything.

/// Where focus was when this frame began, before any widget ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Focus {
    widget: Option<egui::Id>,
}

impl Focus {
    /// Read focus once, at the top of the frame.
    pub fn capture(ctx: &egui::Context) -> Self {
        Focus { widget: ctx.memory(|m| m.focused()) }
    }

    #[cfg(test)]
    fn on(widget: Option<egui::Id>) -> Self {
        Focus { widget }
    }

    /// Keys that submit or edit the command line — Enter, Space, history
    /// recall. Only when the command box itself has focus.
    pub fn allows_submit(&self, command_box: egui::Id) -> bool {
        self.widget == Some(command_box)
    }

    /// Keys that act on the document — page navigation, zoom, and in later
    /// phases Delete and Enter-repeats-last.
    ///
    /// Allowed only when nothing has focus. Deliberately strict: a text field
    /// with focus means the user is typing, and typing must never be able to
    /// reach the document.
    pub fn allows_document_keys(&self) -> bool {
        self.widget.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids() -> (egui::Id, egui::Id) {
        (egui::Id::new("command_box"), egui::Id::new("a_numeric_field"))
    }

    #[test]
    fn nothing_focused_lets_the_document_keys_through() {
        let focus = Focus::on(None);
        let (command_box, _) = ids();

        assert!(focus.allows_document_keys());
        assert!(!focus.allows_submit(command_box), "nothing to submit into");
    }

    #[test]
    fn the_command_box_submits_but_does_not_drive_the_document() {
        let focus = Focus::on(Some(egui::Id::new("command_box")));
        let (command_box, _) = ids();

        assert!(focus.allows_submit(command_box));
        assert!(
            !focus.allows_document_keys(),
            "arrows must move the caret while typing, not turn the page"
        );
    }

    /// The test that is the whole point of the module.
    ///
    /// A numeric field the guard has never heard of must still block every
    /// document key. This passes because the rule is an allow-list — under a
    /// deny-list it would depend on someone having registered the field.
    #[test]
    fn a_field_the_guard_has_never_heard_of_still_blocks_everything() {
        let focus = Focus::on(Some(egui::Id::new("a_numeric_field")));
        let (command_box, _) = ids();

        assert!(!focus.allows_document_keys(), "typing a page number could turn the page");
        assert!(!focus.allows_submit(command_box), "typing a page number could run a command");
    }

    #[test]
    fn focus_is_compared_by_identity_not_by_presence() {
        // The bug this rules out: treating "something has focus" as "the
        // command box has focus", which is right exactly once.
        let (command_box, other) = ids();
        assert!(!Focus::on(Some(other)).allows_submit(command_box));
        assert!(Focus::on(Some(command_box)).allows_submit(command_box));
    }
}
