//! What the text pickers read: page recognition (including drawn words),
//! per-run fonts and styles, `move_object`, object bounds and outlines,
//! annotations and internal links.
//!
//! Part of the `session` module — split out of the single file the design
//! review flagged (Phase 4, file splits). Methods keep living on `Session`.
use super::*;

impl Session {
    /// Run an engine command: undoable, redoable, and — because these commands
    /// were built serialisable — recordable.
    /// Read a page with OCR, without letting a page handle escape the lock.
    ///
    /// The whole pipeline runs inside `with_session` on purpose. Rasterising is
    /// PDFium work and recognition takes about a second a page, so the
    /// temptation is to render inside the lock and recognise outside it — but
    /// that means holding a `Page` across the boundary, and every other PDFium
    /// access in this program is serialised precisely because sharing one
    /// across threads is what produced the SIGABRT this design exists to
    /// prevent.
    pub fn recognise_page(
        &self,
        index: usize,
        recogniser: &dyn pdf_core::ocr::Recogniser,
        options: &pdf_core::ocr::pipeline::Options,
    ) -> Result<pdf_core::ocr::pipeline::PageReading> {
        self.recognise_page_until(index, recogniser, options, &|| false)
    }

    /// As [`Session::recognise_page`], stopping if asked.
    pub fn recognise_page_until(
        &self,
        index: usize,
        recogniser: &dyn pdf_core::ocr::Recogniser,
        options: &pdf_core::ocr::pipeline::Options,
        stop: &dyn Fn() -> bool,
    ) -> Result<pdf_core::ocr::pipeline::PageReading> {
        registry::with_session(self.handle, |s| {
            let page = s.document.page(index)?;
            pdf_core::ocr::pipeline::read_page_until(&*page, recogniser, options, stop)
        })
    }

    /// The fast path for "Extract Text" on a page whose type was converted to
    /// outlines — real geometric matching in place of rasterising the page and
    /// running the neural recogniser on it, when a candidate face actually
    /// resolves the page's own letters.
    ///
    /// `Ok(None)` covers two cases the caller does not need to tell apart: the
    /// page is not [`pdf_core::document::PageTextKind::Outlined`] at all, or
    /// it is but none of `outlined_fonts` genuinely resolves it — see
    /// `outlined_words_are_trustworthy` on `pdf_core`'s `Page` trait for what
    /// tells "found nothing" apart from "found the wrong thing", by comparing
    /// what matched against how many type-shaped paths `classify` already
    /// counted. Either way the honest answer is the same: fall back to OCR,
    /// which this method never runs itself — that stays the caller's call,
    /// exactly as it is today.
    pub fn recognise_outlined_words_if_trustworthy(
        &self,
        index: usize,
        outlined_fonts: &[&[u8]],
    ) -> Result<Option<Vec<pdf_core::document::RecognisedWord>>> {
        let Some(catalogue) = merged_catalogue(outlined_fonts) else { return Ok(None) };
        registry::with_session(self.handle, |s| {
            let page = s.document.page(index)?;
            if page.classify()?.kind != pdf_core::document::PageTextKind::Outlined {
                return Ok(None);
            }
            let words = page.recognise_outlined_words(&catalogue)?;
            if page.outlined_words_are_trustworthy(&words)? {
                Ok(Some(words))
            } else {
                Ok(None)
            }
        })
    }

    /// The font program a run is drawn with, when the document carries one.
    pub fn run_font_data(&self, page: usize, object: usize) -> Result<Option<Vec<u8>>> {
        registry::with_session(self.handle, |s| s.document.run_font_data(page, object))
    }

    /// The name of a run's own font — its `/BaseFont` — for showing which
    /// font a run is written in, whether or not that font is embedded.
    pub fn run_font_name(&self, page: usize, object: usize) -> Result<Option<String>> {
        registry::with_session(self.handle, |s| s.document.run_font_name(page, object))
    }

    /// Every text run's own font name on a page, in one pass — see
    /// `Document::run_font_names`'s own doc for why this exists alongside
    /// `run_font_name` rather than instead of it.
    pub fn run_font_names(&self, page: usize) -> Result<std::collections::HashMap<usize, String>> {
        registry::with_session(self.handle, |s| s.document.run_font_names(page))
    }

    /// Every text run's style identity on a page, in one pass — which font
    /// program draws it, how thick that font's strokes are and which way it
    /// runs. For the page where `run_font_names` reads one name throughout;
    /// see `pdf_core::document::RunStyle`.
    pub fn run_styles(
        &self,
        page: usize,
    ) -> Result<std::collections::HashMap<usize, pdf_core::document::RunStyle>> {
        registry::with_session(self.handle, |s| s.document.run_styles(page))
    }

    /// Whether a run's font is embedded in the document rather than
    /// substituted by whatever reader opened it.
    pub fn run_font_is_embedded(&self, page: usize, object: usize) -> Result<bool> {
        registry::with_session(self.handle, |s| s.document.run_font_is_embedded(page, object))
    }

    /// Shift one page object by a distance, in page points.
    pub fn move_object(
        &self,
        page: usize,
        object: usize,
        by: pdf_core::document::Point,
    ) -> Result<()> {
        registry::with_session(self.handle, |s| s.document.move_object(page, object, by))
    }

    /// The area one page object covers.
    pub fn object_bounds(&self, page: usize, object: usize) -> Result<pdf_core::document::Rect> {
        registry::with_session(self.handle, |s| s.document.object_bounds(page, object))
    }

    /// One path object's own ink — see [`pdf_core::document::Document::
    /// object_outline`] for why this is for hit-testing a click against the
    /// actual shape rather than its bounding box.
    pub fn object_outline(&self, page: usize, object: usize) -> Result<Vec<Vec<(f32, f32)>>> {
        registry::with_session(self.handle, |s| s.document.object_outline(page, object))
    }

    /// The words a page *draws*, whatever else is on it.
    ///
    /// **Not gated on the page's classification**, unlike
    /// [`Session::recognise_outlined_words_if_trustworthy`]. `Outlined` means a
    /// page with *no* characters at all, and a page can perfectly well carry a
    /// text header over a body of type converted to outlines — measured on a
    /// real report: twenty-nine text runs and fifteen thousand curve operators
    /// on the same page, classified `Hybrid`, which that gate refuses.
    ///
    /// The caller that wants this has already established the thing the
    /// classification was standing in for: a click landed somewhere with no
    /// text under it. The question left is whether there are drawn words there,
    /// and that is what this answers — still refusing when the match is not
    /// trustworthy, which is the check that actually protects anybody.
    pub fn recognise_drawn_words(
        &self,
        index: usize,
        outlined_fonts: &[&[u8]],
    ) -> Result<Option<Vec<pdf_core::document::RecognisedWord>>> {
        let Some(catalogue) = merged_catalogue(outlined_fonts) else { return Ok(None) };
        registry::with_session(self.handle, |s| {
            let page = s.document.page(index)?;
            let words = page.recognise_outlined_words(&catalogue)?;
            if words.is_empty() {
                return Ok(None);
            }
            if page.outlined_words_are_trustworthy(&words)? {
                Ok(Some(words))
            } else {
                Ok(None)
            }
        })
    }

    /// The annotations already on a page.
    ///
    /// Everything on it, not only Pagify's own marks — a highlight another
    /// program made is still a highlight.
    pub fn annotations(
        &self,
        page: usize,
    ) -> Result<Vec<pdf_core::document::IndexedAnnotation>> {
        registry::with_session(self.handle, |s| s.document.annotations(page))
    }

    /// The links on a page that go to another page of this document — see
    /// [`pdf_core::document::Document::internal_links`].
    pub fn internal_links(&self, page: usize) -> Result<Vec<pdf_core::document::InternalLink>> {
        registry::with_session(self.handle, |s| s.document.internal_links(page))
    }
}
