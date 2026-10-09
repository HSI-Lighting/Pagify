//! The document's text runs and edit state: how many edits are pending,
//! whether only a full copy can be written, extracting pages, finding and
//! measuring runs, the page text snapshot and the page-scale reading.
//!
//! Part of the `session` module — split out of the single file the design
//! review flagged (Phase 4, file splits). Methods keep living on `Session`.
use super::*;

impl Session {
    /// Whether saving must rewrite the file rather than append to it.
    ///
    /// True once anything has been redacted. An incremental save keeps the
    /// original bytes and appends a delta, so the removed words would still be
    /// in the file — see `DocumentMut::must_save_full_copy`.
    pub fn must_save_full_copy(&self) -> bool {
        registry::with_session(self.handle, |s| {
            Ok(s.document.as_document_mut().is_some_and(|d| d.must_save_full_copy()))
        })
        .unwrap_or(false)
    }

    pub fn edit_state(&self) -> Result<pdf_core::engine::EditState> {
        registry::with_session(self.handle, |s| Ok(pdf_core::engine::edit_state(s)))
    }

    /// Pull pages out into a new document on disk.
    ///
    /// Staged and renamed like a save: the destination used to be created —
    /// truncated — before the pages were checked, so a bad page number left
    /// an empty file where a document may have been. Found by audit.
    pub fn extract_to(&self, pages: &[usize], dest: &Path) -> Result<usize> {
        let mut count = 0;
        write_then_rename(dest, |file| {
            registry::with_session(self.handle, |s| {
                let mut extracted = s
                    .document
                    .as_document_mut()
                    .ok_or(pdf_core::PdfError::Unsupported("extracting from this document"))?
                    .extract_pages(pages)?;
                count = extracted.page_count();
                extracted
                    .as_document_mut()
                    .ok_or(pdf_core::PdfError::Unsupported("saving the extracted pages"))?
                    .save_full_copy(file)
            })
        })?;
        Ok(count)
    }

    /// Every run of text on a page, in the order the file stores them.
    pub fn text_runs(&self, page: usize) -> Result<Vec<pdf_core::document::TextRun>> {
        registry::with_session(self.handle, |s| s.document.text_runs(page))
    }

    /// Every text object's bounding rect, without its words — see
    /// [`pdf_core::document::Document::text_run_rects`].
    pub fn text_run_rects(&self, page: usize) -> Result<Vec<(usize, pdf_core::document::Rect)>> {
        registry::with_session(self.handle, |s| s.document.text_run_rects(page))
    }

    /// One run, by object number, words included — see
    /// [`pdf_core::document::Document::text_run_at`].
    pub fn text_run_at(
        &self,
        page: usize,
        object: usize,
    ) -> Result<Option<pdf_core::document::TextRun>> {
        registry::with_session(self.handle, |s| s.document.text_run_at(page, object))
    }

    /// Full runs, words included, for just the given objects — see
    /// [`pdf_core::document::Document::text_runs_some`].
    pub fn text_runs_some(
        &self,
        page: usize,
        wanted: &std::collections::HashSet<usize>,
    ) -> Result<Vec<pdf_core::document::TextRun>> {
        registry::with_session(self.handle, |s| s.document.text_runs_some(page, wanted))
    }

    /// Everything the paragraph detector reads about one page's text, in one
    /// pass: every text object (nothing filtered), its font identity and
    /// weight, its font name, and the page's drawn shapes.
    ///
    /// **One registry lock for all four reads**, not four. Each of them walks
    /// the page, and taking the lock separately for each would let another
    /// thread's open, render or edit slip in between them — a run list from
    /// before an edit beside a shape list from after it. See
    /// [`PageTextSnapshot`] for what each part holds, and
    /// [`pdf_core::document::Document::text_runs_unfiltered`] for why the runs
    /// are not `text_runs`' (which drops those with no ink area).
    ///
    /// The shapes are not read at all for a page with no text objects: a page
    /// of only outlines has tens of thousands of them, and the walk that lists
    /// them is the slowest of the four.
    pub fn page_text_snapshot(&self, page: usize) -> Result<PageTextSnapshot> {
        registry::with_session(self.handle, |s| {
            let runs = s.document.text_runs_unfiltered(page)?;
            let styles = s.document.run_styles(page)?;
            let faces = s.document.run_font_names(page)?;
            let shapes = if runs.is_empty() { Vec::new() } else { s.document.drawn_shapes(page)? };
            Ok(PageTextSnapshot { runs, styles, faces, shapes })
        })
    }

    /// How many objects a page holds and how many of them are text, **counted
    /// without reading any of it** — see [`pdf_core::document::PageScale`].
    ///
    /// For deciding whether to call [`Session::page_text_snapshot`] at all: that
    /// reads every text object's words and lists every path, and what it costs
    /// grows with these numbers — linearly now (a page of 40,000 words in about
    /// 0.4 s; it took five minutes while the words were read one object at a
    /// time), and a drawing of 880,000 paths is still seconds. This is a page
    /// open and one question per object: milliseconds on a datasheet page, a
    /// second on a drawing of 880,000 paths (PDFium parses the whole page to open
    /// it — no count can be had without that).
    pub fn page_scale(&self, page: usize) -> Result<pdf_core::document::PageScale> {
        registry::with_session(self.handle, |s| s.document.page_scale(page))
    }

    /// **Temporary diagnostic** — see
    /// [`pdf_core::document::Document::take_last_batch_timing`].
    pub fn take_last_batch_timing(&self) -> Vec<(&'static str, std::time::Duration)> {
        registry::with_session(self.handle, |s| Ok(s.document.take_last_batch_timing()))
            .unwrap_or_default()
    }

    /// One object per character, in place of a run of them — see
    /// [`pdf_core::document::Document::split_run_into_characters`].
    pub fn split_run_into_characters(&self, page: usize, object: usize) -> Result<()> {
        registry::with_session(self.handle, |s| s.document.split_run_into_characters(page, object))
    }
}
