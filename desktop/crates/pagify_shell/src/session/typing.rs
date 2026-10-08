//! Typing fonts: the faces text typed into the document is set in, the
//! substituted face in use, and whether the file was repaired on open.
//!
//! Part of the `session` module — split out of the single file the design
//! review flagged (Phase 4, file splits). Methods keep living on `Session`.
use super::*;

impl Session {
    /// Lock whole pages, sealing each and leaving it blank.
    ///
    /// Takes no candidate fonts, and needs none: nothing here has to identify
    /// what is on the page in order to remove it — see
    /// [`pdf_core::document::DocumentMut::lock_pages`], which is why this works
    /// on outlined and scanned pages that [`Self::lock_area`] must refuse.
    /// Offer fonts for typing characters a document's own fonts cannot spell.
    ///
    /// The reader's own fonts, not this program's: a licensed typeface is
    /// theirs to supply. See [`pdf_core::pdf::embed`].
    pub fn set_typing_fonts(&self, fonts: Vec<Vec<u8>>) -> Result<()> {
        registry::with_session(self.handle, |s| {
            if let Some(doc) = s.document.as_document_mut() {
                doc.set_typing_fonts(fonts);
            }
            Ok(())
        })
    }

    /// Offer one more font for typing — a font picked at the moment of
    /// editing, kept alongside whatever [`Self::set_typing_fonts`] already
    /// holds rather than replacing it. See
    /// [`pdf_core::document::DocumentMut::add_typing_font`].
    pub fn add_typing_font(&self, font: Vec<u8>) -> Result<()> {
        registry::with_session(self.handle, |s| {
            if let Some(doc) = s.document.as_document_mut() {
                doc.add_typing_font(font);
            }
            Ok(())
        })
    }

    /// The face the last edit fell back to, if it was not the run's own.
    pub fn substituted_face(&self) -> Option<String> {
        registry::with_session(self.handle, |s| Ok(s.document.substituted_face()))
            .ok()
            .flatten()
    }

    /// Whether the file had to be mended in memory to be opened — see
    /// [`pdf_core::document::Document::repaired_on_open`].
    pub fn repaired_on_open(&self) -> bool {
        registry::with_session(self.handle, |s| Ok(s.document.repaired_on_open())).unwrap_or(false)
    }
}
