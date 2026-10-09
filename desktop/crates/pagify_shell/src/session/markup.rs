//! Committing and restoring the markup layer a page carries.
//!
//! Part of the `session` module — split out of the single file the design
//! review flagged (Phase 4, file splits). Methods keep living on `Session`.
use super::*;

impl Session {
    /// Write a page's markup into the document: real ink for anyone to see,
    /// plus the live geometry so it is still editable when reopened.
    pub fn commit_markup(
        &self,
        page: usize,
        layer: &crate::markup::Layer,
        colour: pdf_core::document::Color,
        width: f32,
    ) -> Result<crate::commit::Committed> {
        registry::with_session(self.handle, |s| {
            crate::commit::commit_page(s, page, layer, colour, width)
        })
    }

    /// Read a page's markup back, rebuilt as a live layer.
    ///
    /// `Ok(None)` means this page has no Pagify markup — which is the ordinary
    /// case for a document nobody has marked up, and not an error.
    pub fn restore_markup(&self, page: usize) -> Result<Option<crate::markup::Layer>> {
        registry::with_session(self.handle, |s| {
            match crate::commit::restore_page(&*s.document, page) {
                Ok(Some(stored)) => Ok(Some(crate::commit::to_layer(&stored))),
                Ok(None) => Ok(None),
                Err(problem) => Err(pdf_core::PdfError::InvalidArgument(problem)),
            }
        })
    }
}
