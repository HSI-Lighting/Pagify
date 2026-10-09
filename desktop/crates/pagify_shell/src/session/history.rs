//! Edit history: running a `Command`, undo, redo and the generation counter
//! the caches key on.
//!
//! Part of the `session` module — split out of the single file the design
//! review flagged (Phase 4, file splits). Methods keep living on `Session`.
use super::*;

impl Session {
    pub fn execute(&self, command: pdf_core::command::Command) -> Result<pdf_core::engine::EditState> {
        registry::with_session(self.handle, |s| pdf_core::engine::execute(s, command))
    }

    pub fn undo(&self) -> Result<(bool, pdf_core::engine::EditState)> {
        registry::with_session(self.handle, pdf_core::engine::undo)
    }

    pub fn redo(&self) -> Result<(bool, pdf_core::engine::EditState)> {
        registry::with_session(self.handle, pdf_core::engine::redo)
    }

    /// How many times the document's own command history has changed — a
    /// command applied, undone or redone. `0` once the handle is gone rather
    /// than an error: this is read to *compare* recency against a separate
    /// undo stack (the markup layer's own edit counter), and a session that
    /// no longer exists cannot be the more recently changed one.
    ///
    /// **Never waits, and is never behind.** The app reads this every frame, and
    /// the engine is locked for the whole of a page render — on another thread
    /// now, so as not to stop the frames. A frame that waited here waited for
    /// the render (measured: 270 ms, on a drawing that takes that long to
    /// draw). So this reads the history's own counter, an atomic that moves at
    /// the moment the history does, rather than asking the engine for it.
    pub fn undo_generation(&self) -> u64 {
        self.generation.load(std::sync::atomic::Ordering::Relaxed)
    }
}
