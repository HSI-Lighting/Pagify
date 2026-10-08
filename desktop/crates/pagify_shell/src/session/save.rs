//! Dirty tracking and writing the document out, incrementally or as a
//! full copy.
//!
//! Part of the `session` module — split out of the single file the design
//! review flagged (Phase 4, file splits). Methods keep living on `Session`.
use super::*;

impl Session {
    /// Whether anything has been changed since the document was opened.
    pub fn is_dirty(&self) -> Result<bool> {
        registry::with_session(self.handle, |s| {
            Ok(s.document.as_document_mut().map(|d| d.is_dirty()).unwrap_or(false))
        })
    }

    /// Write the document out.
    ///
    /// `incremental` appends a delta and leaves the original bytes untouched,
    /// which is what keeps an existing digital signature valid. A full copy
    /// rewrites and compacts the file and destroys every signature over it, so
    /// it is an explicit choice and never the default.
    ///
    /// ## Why this writes somewhere else first
    ///
    /// Saving over the document that is open is the ordinary case — it is what
    /// ⌘S means — and writing straight to that path **destroys it**. The file
    /// is what PDFium is reading the document from, and creating it for writing
    /// truncates it to nothing before a byte of output is produced. PDFium then
    /// refuses the save, having had its source pulled out from under it, and
    /// what is left on disk is an empty file where the document was.
    ///
    /// Measured, not reasoned about: `save` on an open fixture reduced it to
    /// zero bytes and reported "PDFium refused to save". Every earlier test
    /// passed because they all saved to a *different* path.
    ///
    /// So the output goes to a sibling temporary file and is renamed over the
    /// target once it is complete. That fixes the truncation, and it makes the
    /// save atomic as a side effect: a crash or a full disk halfway through
    /// leaves the original document exactly as it was, rather than half of a
    /// new one.
    pub fn save_to(&self, path: &Path, incremental: bool) -> Result<()> {
        write_then_rename(path, |file| {
            registry::with_session(self.handle, |s| pdf_core::engine::save(s, file, incremental))
        })
    }
}
