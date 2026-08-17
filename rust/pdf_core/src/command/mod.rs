//! Undo/redo for editing operations (roadmap phase 3).
//!
//! Present now because retrofitting a command stack after annotation code is
//! written means rewriting that code. The history manager below is fully
//! implemented and tested; only the concrete commands await `EditableDocument`.

pub mod history;

pub use history::CommandHistory;

use crate::document::EditableDocument;
use crate::error::Result;

pub trait Command: Send {
    fn execute(&mut self, doc: &mut dyn EditableDocument) -> Result<()>;
    fn undo(&mut self, doc: &mut dyn EditableDocument) -> Result<()>;
    /// Shown in the UI ("Undo highlight"), so it should read as a user action.
    fn description(&self) -> String;
    /// Pages whose cached rasters this command invalidates.
    fn affected_pages(&self) -> Vec<usize>;
}
