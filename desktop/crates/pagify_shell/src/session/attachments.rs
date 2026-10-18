//! Files riding with a document: embedded files (`attach`) and
//! file-attachment annotations (`attachcomment`).
//!
//! Part of the `session` module — split out of the single file the design
//! review flagged (Phase 4, file splits). Methods keep living on `Session`.

use super::*;

impl Session {
    /// Embed `bytes` in the document as an attached file named `name`;
    /// `false` when the backend refuses (an empty or already-used name).
    pub fn add_attachment(&self, name: &str, bytes: &[u8]) -> Result<bool> {
        registry::with_session(self.handle, |s| s.document.add_attachment(name, bytes))
    }

    /// The names of the document's embedded files, in the file's own order.
    pub fn attachments(&self) -> Result<Vec<String>> {
        registry::with_session(self.handle, |s| s.document.attachments())
    }

    /// Attach a file to a page as a file-attachment annotation; `false` when
    /// the annotation cannot be made.
    pub fn add_annot_attachment(&self, page: usize, name: &str, bytes: &[u8]) -> Result<bool> {
        registry::with_session(self.handle, |s| {
            s.document.add_annot_attachment(page, name, bytes)
        })
    }

    /// The names of the files riding on a page's annotations.
    pub fn page_attachments(&self, page: usize) -> Result<Vec<String>> {
        registry::with_session(self.handle, |s| s.document.page_attachments(page))
    }

    /// The document's embedded files with their sizes, in the file's own
    /// order — what the Portfolio panel lists. [`Session::attachments`] has
    /// the names alone.
    pub fn attachment_details(&self) -> Result<Vec<(String, usize)>> {
        registry::with_session(self.handle, |s| s.document.attachment_details())
    }

    /// The bytes of one embedded file, by position in
    /// [`Session::attachment_details`].
    pub fn attachment_bytes(&self, index: usize) -> Result<Vec<u8>> {
        registry::with_session(self.handle, |s| s.document.attachment_bytes(index))
    }

    /// Remove one embedded file, by position; `false` when there was none
    /// there. **Not undoable**, like `add_attachment` — the app says so.
    pub fn remove_attachment(&self, index: usize) -> Result<bool> {
        registry::with_session(self.handle, |s| s.document.remove_attachment(index))
    }
}
