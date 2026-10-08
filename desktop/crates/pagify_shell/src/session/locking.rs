//! Locking: the redaction preview that decides what a lock covers, the two
//! lock commands, and lock repair.
//!
//! Part of the `session` module — split out of the single file the design
//! review flagged (Phase 4, file splits). Methods keep living on `Session`.
use super::*;

impl Session {
    /// What a redaction of this rectangle would destroy, and what it could not.
    ///
    /// Runs the survey and stops. A caller shows what is in the way and, where
    /// that is an image beside the words rather than the words themselves, asks
    /// the one person who can say whether it matters.
    ///
    /// `outlined_fonts`, given, are candidate faces' own bytes — `.ttf`/`.otf`
    /// believed to include the one type-converted-to-curves on this page
    /// used. Every candidate's glyphs are merged into one
    /// [`pdf_core::document::glyphs::Catalogue`] over a broad common alphabet
    /// here, rather than asking every caller to build one: almost every
    /// caller wants exactly that alphabet, none of them should have to know
    /// this type exists to redact an ordinary page, and — measured, not
    /// assumed — merging a body face with its bold measurably helped
    /// recognition on a real fixture rather than confusing it, since the
    /// shape distance decides which candidate a letter matches regardless of
    /// which one is closer. An empty slice — the overwhelming common case,
    /// since most redactions run against ordinary text — costs nothing extra
    /// and changes nothing from before this parameter existed.
    pub fn preview_redaction(
        &self,
        page_index: usize,
        area: pdf_core::document::Rect,
        outlined_fonts: &[&[u8]],
    ) -> Result<pdf_core::document::RedactionReport> {
        let catalogue = merged_catalogue(outlined_fonts);
        registry::with_session(self.handle, |s| {
            s.document
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("redacting this document"))?
                .preview_redaction(
                    &pdf_core::document::Redaction::new(page_index, area),
                    catalogue.as_ref(),
                )
        })
    }

    /// Hide an area, keeping a sealed copy a passcode can bring back.
    ///
    /// The passcode is borrowed and never stored — not here, not in the command
    /// history, not in the recorder. `outlined_fonts` is the same candidate list
    /// [`Session::preview_redaction`] takes, for the same reason.
    /// Hide an area, keeping a sealed copy so a passcode can bring it back.
    ///
    /// `require_complete` is the difference between the two ways a reader can
    /// ask for this, and it is not a detail:
    ///
    /// - **A dragged rectangle** means *this area*, so anything inside it that
    ///   cannot be removed — an image the words sit on — has to refuse. Leaving
    ///   it would be a document somebody believes is clear and is not.
    /// - **A text selection** means *these words*. An image underneath them was
    ///   never part of what was picked, and refusing on account of it declines a
    ///   thing the reader did not ask for. They can right-click the image and
    ///   lock that too, which is a separate decision.
    pub fn lock_area(
        &self,
        page_index: usize,
        area: pdf_core::document::Rect,
        passcode: &[u8],
        outlined_fonts: &[&[u8]],
        require_complete: bool,
    ) -> Result<pdf_core::document::RedactionReport> {
        self.lock_shapes(page_index, &[area], passcode, outlined_fonts, require_complete)
    }

    /// Lock an exact set of shapes, which is what a text selection is.
    ///
    /// **A selection over two lines is not a rectangle.** The smallest
    /// rectangle holding it also holds the head of the first line and the tail
    /// of the last, and locking that takes words nobody picked — measured, a
    /// 39-character selection took 68. So the shapes travel whole, one per
    /// line, and the engine keeps their union only for the badge that undoes
    /// the lock. A dragged rectangle is one shape and takes the same path.
    pub fn lock_shapes(
        &self,
        page_index: usize,
        shapes: &[pdf_core::document::Rect],
        passcode: &[u8],
        outlined_fonts: &[&[u8]],
        require_complete: bool,
    ) -> Result<pdf_core::document::RedactionReport> {
        let catalogue = merged_catalogue(outlined_fonts);
        let request = pdf_core::document::Redaction::over(page_index, shapes.to_vec())
            .ok_or(pdf_core::PdfError::InvalidArgument("nothing to lock".into()))?;
        let request = pdf_core::document::Redaction { require_complete, ..request };
        registry::with_session(self.handle, |s| {
            let report = s
                .document
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("locking this document"))?
                .lock_area(&request, passcode, catalogue.as_ref())?;
            // Edits the page's own bytes directly rather than through
            // `execute`, which is what normally invalidates this cache after
            // a change — see `engine::invalidate`. `lock_pages`, `lock_image`
            // and `unlock_item` all edit the page the same way outside
            // `execute`, so each clears the cache itself rather than leaving
            // a stale raster for whatever next asks this session to render
            // this page at a scale it already rendered once before.
            s.cache.clear();
            Ok(report)
        })
    }

    /// Finish any lock that recorded its badge but never took its picture off
    /// the page. Returns how many were completed and how many dropped.
    ///
    /// Nothing is touched on a document with no such lock — see
    /// [`pdf_core::document::DocumentMut::repair_locks`].
    pub fn repair_locks(&self) -> Result<(usize, usize)> {
        registry::with_session(self.handle, |s| {
            s.document
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("repairing this document"))?
                .repair_locks()
        })
    }
}
