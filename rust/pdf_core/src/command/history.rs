//! Bounded undo/redo stacks.

use crate::command::Command;
use crate::document::EditableDocument;
use crate::error::Result;

pub const DEFAULT_UNDO_DEPTH: usize = 64;

pub struct CommandHistory {
    undo_stack: Vec<Box<dyn Command>>,
    redo_stack: Vec<Box<dyn Command>>,
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

    /// Run a command and record it.
    ///
    /// A command that fails is *not* recorded, and the redo stack is left alone —
    /// otherwise a failed edit would silently discard the user's redo history.
    pub fn execute(
        &mut self,
        mut command: Box<dyn Command>,
        doc: &mut dyn EditableDocument,
    ) -> Result<()> {
        command.execute(doc)?;

        // Any new edit invalidates the redo branch.
        self.redo_stack.clear();
        self.undo_stack.push(command);

        if self.undo_stack.len() > self.depth {
            self.undo_stack.remove(0);
        }
        Ok(())
    }

    /// Undo the most recent command, returning the pages it touched so their
    /// cached rasters can be invalidated. `None` means nothing to undo.
    pub fn undo(&mut self, doc: &mut dyn EditableDocument) -> Result<Option<Vec<usize>>> {
        let Some(mut command) = self.undo_stack.pop() else {
            return Ok(None);
        };

        if let Err(e) = command.undo(doc) {
            // Put it back: a failed undo must leave the stack as it was, or the
            // user loses the ability to retry.
            self.undo_stack.push(command);
            return Err(e);
        }

        let pages = command.affected_pages();
        self.redo_stack.push(command);
        Ok(Some(pages))
    }

    pub fn redo(&mut self, doc: &mut dyn EditableDocument) -> Result<Option<Vec<usize>>> {
        let Some(mut command) = self.redo_stack.pop() else {
            return Ok(None);
        };

        if let Err(e) = command.execute(doc) {
            self.redo_stack.push(command);
            return Err(e);
        }

        let pages = command.affected_pages();
        self.undo_stack.push(command);
        Ok(Some(pages))
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    pub fn undo_description(&self) -> Option<String> {
        self.undo_stack.last().map(|c| c.description())
    }

    pub fn redo_description(&self) -> Option<String> {
        self.redo_stack.last().map(|c| c.description())
    }

    pub fn clear(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Annotation, Signature};
    use crate::error::PdfError;
    use std::sync::{Arc, Mutex};

    /// Stand-in for a real editable document. The commands below record their own
    /// ordering into a shared log, so this only has to satisfy the trait.
    #[derive(Default)]
    struct RecordingDoc;

    impl EditableDocument for RecordingDoc {
        fn add_annotation(&mut self, _page: usize, _annotation: Annotation) -> Result<()> {
            Ok(())
        }
        fn add_signature(&mut self, _page: usize, _signature: Signature) -> Result<()> {
            Ok(())
        }
        fn remove_annotation(&mut self, _page: usize, _id: u64) -> Result<()> {
            Ok(())
        }
        fn save(&self, _path: &str) -> Result<()> {
            Ok(())
        }
    }

    struct TestCommand {
        name: String,
        page: usize,
        log: Arc<Mutex<Vec<String>>>,
        fail_execute: bool,
        fail_undo: bool,
    }

    impl TestCommand {
        fn new(name: &str, page: usize, log: &Arc<Mutex<Vec<String>>>) -> Box<dyn Command> {
            Box::new(TestCommand {
                name: name.to_string(),
                page,
                log: Arc::clone(log),
                fail_execute: false,
                fail_undo: false,
            })
        }
    }

    impl Command for TestCommand {
        fn execute(&mut self, _doc: &mut dyn EditableDocument) -> Result<()> {
            if self.fail_execute {
                return Err(PdfError::Unsupported("test failure"));
            }
            self.log.lock().unwrap().push(format!("do:{}", self.name));
            Ok(())
        }
        fn undo(&mut self, _doc: &mut dyn EditableDocument) -> Result<()> {
            if self.fail_undo {
                return Err(PdfError::Unsupported("test failure"));
            }
            self.log.lock().unwrap().push(format!("undo:{}", self.name));
            Ok(())
        }
        fn description(&self) -> String {
            self.name.clone()
        }
        fn affected_pages(&self) -> Vec<usize> {
            vec![self.page]
        }
    }

    fn fixtures() -> (CommandHistory, RecordingDoc, Arc<Mutex<Vec<String>>>) {
        (
            CommandHistory::default(),
            RecordingDoc::default(),
            Arc::new(Mutex::new(Vec::new())),
        )
    }

    #[test]
    fn undo_and_redo_run_in_the_right_order() {
        let (mut history, mut doc, log) = fixtures();

        history.execute(TestCommand::new("a", 0, &log), &mut doc).unwrap();
        history.execute(TestCommand::new("b", 1, &log), &mut doc).unwrap();

        assert_eq!(history.undo(&mut doc).unwrap(), Some(vec![1]));
        assert_eq!(history.undo(&mut doc).unwrap(), Some(vec![0]));
        assert_eq!(history.redo(&mut doc).unwrap(), Some(vec![0]));

        assert_eq!(
            *log.lock().unwrap(),
            vec!["do:a", "do:b", "undo:b", "undo:a", "do:a"]
        );
    }

    #[test]
    fn undoing_an_empty_history_is_not_an_error() {
        let (mut history, mut doc, _) = fixtures();
        assert_eq!(history.undo(&mut doc).unwrap(), None);
        assert_eq!(history.redo(&mut doc).unwrap(), None);
        assert!(!history.can_undo());
        assert!(!history.can_redo());
    }

    #[test]
    fn a_new_edit_discards_the_redo_branch() {
        let (mut history, mut doc, log) = fixtures();
        history.execute(TestCommand::new("a", 0, &log), &mut doc).unwrap();
        history.undo(&mut doc).unwrap();
        assert!(history.can_redo());

        history.execute(TestCommand::new("b", 0, &log), &mut doc).unwrap();

        assert!(!history.can_redo(), "redoing past a new edit would corrupt the document");
    }

    #[test]
    fn a_failed_command_is_not_recorded_and_leaves_redo_intact() {
        let (mut history, mut doc, log) = fixtures();
        history.execute(TestCommand::new("a", 0, &log), &mut doc).unwrap();
        history.undo(&mut doc).unwrap();

        let failing = Box::new(TestCommand {
            name: "boom".into(),
            page: 0,
            log: Arc::clone(&log),
            fail_execute: true,
            fail_undo: false,
        });
        assert!(history.execute(failing, &mut doc).is_err());

        assert!(!history.can_undo(), "the failed command was not pushed");
        assert!(history.can_redo(), "the redo branch survived the failure");
    }

    #[test]
    fn a_failed_undo_leaves_the_command_where_it_was() {
        let (mut history, mut doc, log) = fixtures();
        let failing = Box::new(TestCommand {
            name: "stuck".into(),
            page: 2,
            log: Arc::clone(&log),
            fail_execute: false,
            fail_undo: true,
        });
        history.execute(failing, &mut doc).unwrap();

        assert!(history.undo(&mut doc).is_err());
        assert!(history.can_undo(), "the user must be able to retry the undo");
        assert!(!history.can_redo());
    }

    #[test]
    fn the_stack_is_bounded_and_drops_the_oldest_entries() {
        let mut history = CommandHistory::new(2);
        let mut doc = RecordingDoc::default();
        let log = Arc::new(Mutex::new(Vec::new()));

        for name in ["a", "b", "c"] {
            history.execute(TestCommand::new(name, 0, &log), &mut doc).unwrap();
        }

        assert_eq!(history.undo_description().as_deref(), Some("c"));
        history.undo(&mut doc).unwrap();
        assert_eq!(history.undo_description().as_deref(), Some("b"));
        history.undo(&mut doc).unwrap();
        assert!(!history.can_undo(), "'a' fell off the bottom of the stack");
    }

    #[test]
    fn descriptions_follow_the_top_of_each_stack() {
        let (mut history, mut doc, log) = fixtures();
        history.execute(TestCommand::new("highlight", 0, &log), &mut doc).unwrap();

        assert_eq!(history.undo_description().as_deref(), Some("highlight"));
        assert_eq!(history.redo_description(), None);

        history.undo(&mut doc).unwrap();
        assert_eq!(history.undo_description(), None);
        assert_eq!(history.redo_description().as_deref(), Some("highlight"));
    }
}
