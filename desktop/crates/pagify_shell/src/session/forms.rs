//! Form fields: reading what an AcroForm holds.
//!
//! Part of the `session` module — split out of the single file the design
//! review flagged (Phase 4, file splits). Methods keep living on `Session`.

use super::*;

impl Session {
    /// Every form widget in the document, page by page, in page order — the
    /// same widget appearing on two pages is two entries, as a reader walks
    /// them.
    pub fn form_fields(&self) -> Result<Vec<pdf_core::document::FormFieldInfo>> {
        registry::with_session(self.handle, |s| s.document.form_fields())
    }
}
