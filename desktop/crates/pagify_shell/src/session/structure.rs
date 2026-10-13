//! A page's tags: the structure tree PDFium reads back.
//!
//! Part of the `session` module — split out of the single file the design
//! review flagged (Phase 4, file splits). Methods keep living on `Session`.

use super::*;

impl Session {
    /// A page's structure tree, as the Tags panel lists it. Empty for an
    /// untagged page.
    pub fn structure_outline(&self, page: usize) -> Result<Vec<pdf_core::document::TagNode>> {
        registry::with_session(self.handle, |s| s.document.structure_outline(page))
    }
}
