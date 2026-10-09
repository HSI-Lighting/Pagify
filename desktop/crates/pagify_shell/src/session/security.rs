//! The mark-editing and security commands that sit together in the
//! sensitive-content menu: stamp marks, sensitivity flags, whiteout, hidden
//! data, document passwords and the secured-document queries.
//!
//! Part of the `session` module — split out of the single file the design
//! review flagged (Phase 4, file splits). Methods keep living on `Session`.
use super::*;

impl Session {
    /// Put a tick, a cross or a dot on a page.
    pub fn stamp_mark(
        &self,
        page_index: usize,
        mark: pdf_core::document::FillMark,
        at: pdf_core::document::Point,
        size: f32,
    ) -> Result<()> {
        registry::with_session(self.handle, |s| {
            s.document
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("filling in this document"))?
                .stamp_mark(page_index, mark, at, size)
        })
    }

    /// Mark how far this document may travel.
    pub fn set_sensitivity(
        &self,
        level: pdf_core::document::sensitivity::Sensitivity,
    ) -> Result<()> {
        registry::with_session(self.handle, |s| {
            s.document
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("marking this document"))?
                .set_sensitivity(level)
        })
    }

    /// Take the marking off.
    pub fn clear_sensitivity(&self) -> Result<()> {
        registry::with_session(self.handle, |s| {
            s.document
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("marking this document"))?
                .clear_sensitivity()
        })
    }

    /// How this document is marked, if at all.
    pub fn sensitivity(&self) -> Option<pdf_core::document::sensitivity::Sensitivity> {
        registry::with_session(self.handle, |s| {
            Ok(s.document.as_document_mut().and_then(|d| d.sensitivity()))
        })
        .ok()
        .flatten()
    }

    /// Paint over an area, covering what is there without removing it.
    pub fn whiteout(
        &self,
        page_index: usize,
        area: pdf_core::document::Rect,
        colour: pdf_core::document::Color,
    ) -> Result<()> {
        registry::with_session(self.handle, |s| {
            s.document
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("painting over this document"))?
                .whiteout(page_index, area, colour)
        })
    }

    /// Things on a page somebody would not want to send out.
    pub fn sensitive_on(&self, page_index: usize) -> Result<Vec<pdf_core::document::SensitiveOnPage>> {
        registry::with_session(self.handle, |s| s.document.sensitive_on(page_index))
    }

    /// What this document carries that is not on its pages.
    pub fn hidden_data(&self) -> Result<pdf_core::pdf::hidden::Hidden> {
        registry::with_session(self.handle, |s| {
            s.document
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("surveying this document"))?
                .hidden_data()
        })
    }

    /// Take that data out, and say what went — and what did not.
    pub fn remove_hidden_data(&self) -> Result<pdf_core::pdf::hidden::Sanitised> {
        registry::with_session(self.handle, |s| {
            s.document
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("sanitising this document"))?
                .remove_hidden_data()
        })
    }

    /// Put a password on the file, written when it is next saved.
    ///
    /// A different promise from locking: that hides content inside a document
    /// anyone can open, this shuts the whole file to anyone without the
    /// password — in Pagify or in any other reader.
    pub fn secure_document(
        &self,
        user: &[u8],
        owner: Option<&[u8]>,
        permissions: pdf_core::pdf::encrypt::Permissions,
    ) -> Result<()> {
        registry::with_session(self.handle, |s| {
            s.document
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("securing this document"))?
                .secure_document(user, owner, permissions)
        })
    }

    /// Put Pagify's own password on the file — stronger, and readable by
    /// nothing else.
    pub fn secure_document_plus(&self, user: &[u8]) -> Result<()> {
        registry::with_session(self.handle, |s| {
            s.document
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("securing this document"))?
                .secure_document_plus(user)
        })
    }

    /// Whether the password on this document is Pagify's own.
    pub fn is_secure_plus(&self) -> bool {
        registry::with_session(self.handle, |s| {
            Ok(s.document.as_document_mut().is_some_and(|d| d.is_secure_plus()))
        })
        .unwrap_or(false)
    }

    /// Take the password off again, before it has been saved with one.
    pub fn unsecure_document(&self) -> Result<()> {
        registry::with_session(self.handle, |s| {
            s.document
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("securing this document"))?
                .unsecure_document()
        })
    }

    /// Whether the file this came from had a password when it was opened.
    pub fn had_password_on_open(&self) -> bool {
        registry::with_session(self.handle, |s| {
            Ok(s.document.as_document_mut().is_some_and(|d| d.had_password_on_open()))
        })
        .unwrap_or(false)
    }

    /// Whether this is the password the document was opened with.
    pub fn password_matches(&self, typed: &[u8]) -> bool {
        registry::with_session(self.handle, |s| {
            Ok(s.document.as_document_mut().is_some_and(|d| d.password_matches(typed)))
        })
        .unwrap_or(false)
    }

    /// Whether the file this document came from already has a password.
    pub fn already_has_password(&self) -> bool {
        registry::with_session(self.handle, |s| {
            Ok(s.document.as_document_mut().is_some_and(|d| d.already_has_password()))
        })
        .unwrap_or(false)
    }

    /// Whether a password is waiting to be written on the next save.
    pub fn is_secured(&self) -> bool {
        registry::with_session(self.handle, |s| {
            Ok(s.document.as_document_mut().is_some_and(|d| d.is_secured()))
        })
        .unwrap_or(false)
    }
}
