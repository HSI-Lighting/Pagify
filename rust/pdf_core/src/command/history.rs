//! Bounded undo/redo stacks.
//!
//! The undo side stores `(Command, UndoRecord)` pairs; the redo side stores bare
//! commands. That asymmetry is the design: an undo record belongs to one
//! execution and is spent when it is used, while the command that produced it can
//! simply be run again to make a fresh one.

use crate::command::{Command, UndoRecord};
use crate::document::DocumentMut;
use crate::error::Result;

pub const DEFAULT_UNDO_DEPTH: usize = 64;

pub struct CommandHistory {
    /// Applied changes, each with what it takes to reverse them.
    undo_stack: Vec<(Command, UndoRecord)>,
    /// Reversed changes. Only the intent is kept — redoing re-executes it, which
    /// produces a new undo record against the document's current state.
    redo_stack: Vec<Command>,
    depth: usize,
}

impl Default for CommandHistory {
    fn default() -> Self {
        CommandHistory::new(DEFAULT_UNDO_DEPTH)
    }
}

impl CommandHistory {
    pub fn new(depth: usize) -> Self {
        CommandHistory {
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            depth: depth.max(1),
        }
    }

    /// Run a command and record it, returning the pages whose cached rasters it
    /// invalidates.
    ///
    /// A command that fails is *not* recorded, and the redo stack is left alone —
    /// otherwise a failed edit would silently discard the user's redo history.
    pub fn execute(&mut self, command: Command, doc: &mut dyn DocumentMut) -> Result<Vec<usize>> {
        let undo = command.execute(doc)?;
        let affected = command.affected_pages();

        // Any new edit makes the redo branch unreachable: there is no longer a
        // history in which those changes come next.
        self.redo_stack.clear();
        self.undo_stack.push((command, undo));

        if self.undo_stack.len() > self.depth {
            self.undo_stack.remove(0);
        }
        Ok(affected)
    }

    /// Undo the most recent command. `None` means there was nothing to undo.
    pub fn undo(&mut self, doc: &mut dyn DocumentMut) -> Result<Option<Vec<usize>>> {
        let Some((command, undo)) = self.undo_stack.pop() else {
            return Ok(None);
        };

        let affected = command.affected_pages();
        // `revert` consumes the record, so — unlike the old self-inverting
        // version — a failure here cannot put the pair back. Reversal is
        // all-or-nothing per command for that reason: a half-reverted change with
        // its record already spent would leave the document in a state nothing
        // could describe.
        undo.revert(doc)?;

        self.redo_stack.push(command);
        Ok(Some(affected))
    }

    pub fn redo(&mut self, doc: &mut dyn DocumentMut) -> Result<Option<Vec<usize>>> {
        let Some(command) = self.redo_stack.pop() else {
            return Ok(None);
        };

        match command.execute(doc) {
            Ok(undo) => {
                let affected = command.affected_pages();
                self.undo_stack.push((command, undo));
                Ok(Some(affected))
            }
            Err(e) => {
                // Nothing was consumed, so the redo can be retried.
                self.redo_stack.push(command);
                Err(e)
            }
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    pub fn undo_description(&self) -> Option<String> {
        self.undo_stack.last().map(|(c, _)| c.description())
    }

    pub fn redo_description(&self) -> Option<String> {
        self.redo_stack.last().map(|c| c.description())
    }

    /// The applied history as intent alone — an Action Wizard script.
    ///
    /// Undo records are deliberately left behind: a script replayed against a
    /// different document must not carry the first one's removed pages.
    pub fn as_script(&self) -> Vec<Command> {
        self.undo_stack.iter().map(|(c, _)| c.clone()).collect()
    }

    pub fn clear(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Annotation, Document, PageSize, RemovedPage};
    use crate::error::PdfError;
    use std::io::Write;

    /// A page tree and nothing else, which is all these tests are about.
    ///
    /// Pages are identified by width, so a reorder or a deletion is visible
    /// without rendering anything — the same trick the round-trip fixtures use.
    struct FakeDoc {
        widths: Vec<f32>,
        rotations: Vec<u8>,
        fail_next: bool,
    }

    impl FakeDoc {
        fn with_pages(widths: &[f32]) -> Self {
            FakeDoc {
                widths: widths.to_vec(),
                rotations: vec![0; widths.len()],
                fail_next: false,
            }
        }
    }

    impl DocumentMut for FakeDoc {
        fn reorder_pages(&mut self, order: &[usize]) -> Result<()> {
            let mut moved = vec![0f32; self.widths.len()];
            let mut turned = vec![0u8; self.widths.len()];
            for (from, &to) in order.iter().enumerate() {
                moved[to] = self.widths[from];
                turned[to] = self.rotations[from];
            }
            self.widths = moved;
            self.rotations = turned;
            Ok(())
        }

        fn delete_page(&mut self, index: usize) -> Result<RemovedPage> {
            if self.fail_next {
                return Err(PdfError::Pdfium("refused".into()));
            }
            let width = self.widths.remove(index);
            self.rotations.remove(index);
            Ok(RemovedPage::new(
                PageSize {
                    width_pt: width,
                    height_pt: 100.0,
                },
                Vec::new(),
            ))
        }

        fn insert_page(&mut self, at: usize, page: RemovedPage) -> Result<()> {
            self.widths.insert(at, page.size.width_pt);
            self.rotations.insert(at, 0);
            Ok(())
        }

        fn insert_blank_page(&mut self, at: usize, size: PageSize) -> Result<()> {
            self.widths.insert(at, size.width_pt);
            self.rotations.insert(at, 0);
            Ok(())
        }

        fn set_page_rotation(&mut self, index: usize, quarter_turns: u8) -> Result<()> {
            self.rotations[index] = quarter_turns;
            Ok(())
        }

        fn page_rotation(&self, index: usize) -> Result<u8> {
            Ok(self.rotations[index])
        }

        fn add_annotation(&mut self, _page: usize, _a: &Annotation) -> Result<usize> {
            Ok(0)
        }

        fn remove_annotation(&mut self, _page: usize, _index: usize) -> Result<()> {
            Ok(())
        }

        fn take_annotation(&mut self, _page: usize, _index: usize) -> Result<Annotation> {
            Err(PdfError::Unsupported("annotations in these tests"))
        }

        fn extract_pages(&self, _range: &[usize]) -> Result<Box<dyn Document>> {
            Err(PdfError::Pdfium("not needed by these tests".into()))
        }

        fn import_pages(&mut self, _from: &dyn Document, _r: &[usize], _at: usize) -> Result<()> {
            Ok(())
        }

        fn save_incremental(&mut self, _dest: &mut dyn Write) -> Result<()> {
            Ok(())
        }

        fn save_full_copy(&mut self, _dest: &mut dyn Write) -> Result<()> {
            Ok(())
        }

        fn is_dirty(&self) -> bool {
            true
        }
    }

    #[test]
    fn nothing_to_undo_on_an_untouched_document() {
        let mut history = CommandHistory::default();
        let mut doc = FakeDoc::with_pages(&[10.0, 20.0]);
        assert!(!history.can_undo());
        assert!(history.undo(&mut doc).unwrap().is_none());
    }

    #[test]
    fn a_deleted_page_comes_back_with_its_content() {
        let mut history = CommandHistory::default();
        let mut doc = FakeDoc::with_pages(&[10.0, 20.0, 30.0]);

        history
            .execute(Command::DeletePage { index: 1 }, &mut doc)
            .unwrap();
        assert_eq!(vec![10.0, 30.0], doc.widths);

        history.undo(&mut doc).unwrap();
        assert_eq!(
            vec![10.0, 20.0, 30.0],
            doc.widths,
            "the page must return to its own position carrying its own content",
        );
    }

    #[test]
    fn redo_reapplies_and_can_be_undone_again() {
        let mut history = CommandHistory::default();
        let mut doc = FakeDoc::with_pages(&[10.0, 20.0, 30.0]);

        history
            .execute(Command::DeletePage { index: 0 }, &mut doc)
            .unwrap();
        history.undo(&mut doc).unwrap();
        assert!(history.can_redo());

        history.redo(&mut doc).unwrap();
        assert_eq!(vec![20.0, 30.0], doc.widths);

        // The redo produced a *fresh* undo record; without one this would fail.
        history.undo(&mut doc).unwrap();
        assert_eq!(vec![10.0, 20.0, 30.0], doc.widths);
    }

    /// A rotation is not its own inverse, so this catches an undo that merely
    /// replays the command it was meant to reverse.
    #[test]
    fn undoing_a_reorder_restores_the_original_order() {
        let mut history = CommandHistory::default();
        let mut doc = FakeDoc::with_pages(&[10.0, 20.0, 30.0]);

        history
            .execute(
                Command::ReorderPages {
                    order: vec![1, 2, 0],
                },
                &mut doc,
            )
            .unwrap();
        assert_eq!(vec![30.0, 10.0, 20.0], doc.widths);

        history.undo(&mut doc).unwrap();
        assert_eq!(vec![10.0, 20.0, 30.0], doc.widths);
    }

    #[test]
    fn an_inserted_page_is_removed_again_by_undo() {
        let mut history = CommandHistory::default();
        let mut doc = FakeDoc::with_pages(&[10.0, 20.0]);

        history
            .execute(
                Command::InsertBlankPage {
                    at: 1,
                    width_pt: 99.0,
                    height_pt: 100.0,
                },
                &mut doc,
            )
            .unwrap();
        assert_eq!(vec![10.0, 99.0, 20.0], doc.widths);

        history.undo(&mut doc).unwrap();
        assert_eq!(vec![10.0, 20.0], doc.widths);
    }

    #[test]
    fn a_new_edit_abandons_the_redo_branch() {
        let mut history = CommandHistory::default();
        let mut doc = FakeDoc::with_pages(&[10.0, 20.0, 30.0]);

        history
            .execute(Command::DeletePage { index: 0 }, &mut doc)
            .unwrap();
        history.undo(&mut doc).unwrap();
        assert!(history.can_redo());

        history
            .execute(Command::DeletePage { index: 2 }, &mut doc)
            .unwrap();
        assert!(!history.can_redo());
    }

    #[test]
    fn a_failed_command_is_not_recorded_and_keeps_the_redo_branch() {
        let mut history = CommandHistory::default();
        let mut doc = FakeDoc::with_pages(&[10.0, 20.0, 30.0]);

        history
            .execute(Command::DeletePage { index: 0 }, &mut doc)
            .unwrap();
        history.undo(&mut doc).unwrap();

        doc.fail_next = true;
        assert!(history
            .execute(Command::DeletePage { index: 0 }, &mut doc)
            .is_err());

        assert!(!history.can_undo(), "a failed edit must not be undoable");
        assert!(history.can_redo(), "and must not discard the redo branch");
    }

    #[test]
    fn the_stack_is_bounded_and_drops_the_oldest() {
        let mut history = CommandHistory::new(2);
        let mut doc = FakeDoc::with_pages(&[1.0, 2.0, 3.0, 4.0, 5.0]);

        for _ in 0..4 {
            history
                .execute(Command::DeletePage { index: 0 }, &mut doc)
                .unwrap();
        }

        let mut undone = 0;
        while history.undo(&mut doc).unwrap().is_some() {
            undone += 1;
        }
        assert_eq!(2, undone);
    }

    #[test]
    fn descriptions_come_from_the_top_of_each_stack() {
        let mut history = CommandHistory::default();
        let mut doc = FakeDoc::with_pages(&[10.0, 20.0]);

        history
            .execute(Command::DeletePage { index: 1 }, &mut doc)
            .unwrap();
        assert_eq!(Some("Delete page 2".to_string()), history.undo_description());
        assert_eq!(None, history.redo_description());

        history.undo(&mut doc).unwrap();
        assert_eq!(None, history.undo_description());
        assert_eq!(Some("Delete page 2".to_string()), history.redo_description());
    }

    /// A script is intent alone. If undo records ever leaked into one, replaying
    /// it against a second document would insert the first document's pages.
    #[test]
    fn a_script_carries_intent_and_nothing_else() {
        let mut history = CommandHistory::default();
        let mut doc = FakeDoc::with_pages(&[10.0, 20.0, 30.0]);

        history
            .execute(Command::DeletePage { index: 2 }, &mut doc)
            .unwrap();
        history
            .execute(
                Command::SetPageRotation {
                    index: 0,
                    quarter_turns: 1,
                },
                &mut doc,
            )
            .unwrap();

        let script = history.as_script();
        assert_eq!(
            vec![
                Command::DeletePage { index: 2 },
                Command::SetPageRotation {
                    index: 0,
                    quarter_turns: 1
                },
            ],
            script,
        );

        // And it survives a trip through the wire, which is the point of it.
        let json = serde_json::to_string(&script).unwrap();
        let back: Vec<Command> = serde_json::from_str(&json).unwrap();
        assert_eq!(script, back);
    }

    #[test]
    fn execute_reports_the_pages_whose_rasters_are_now_stale() {
        let mut history = CommandHistory::default();
        let mut doc = FakeDoc::with_pages(&[10.0, 20.0]);

        let affected = history
            .execute(
                Command::SetPageRotation {
                    index: 1,
                    quarter_turns: 2,
                },
                &mut doc,
            )
            .unwrap();
        assert_eq!(vec![1], affected);
    }
}
