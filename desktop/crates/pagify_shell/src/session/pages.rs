//! Page-level operations: the crop box a page carries, duplicating pages
//! and importing pages from another file.
//!
//! Part of the `session` module — split out of the single file the design
//! review flagged (Phase 4, file splits). Methods keep living on `Session`.
use super::*;

impl Session {
    /// A page's crop box, in page points with a top-left origin.
    pub fn page_crop(&self, index: usize) -> Result<pdf_core::document::Rect> {
        registry::with_session(self.handle, |s| {
            s.document
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("reading this document's boundaries"))
                .and_then(|d| d.page_crop(index))
        })
    }

    /// Copy pages within this document, inserting the copies at `at`.
    ///
    /// The same machinery as importing from another file, aimed at itself: the
    /// pages are extracted into a document of their own, saved to bytes, and
    /// brought back in through `ImportPages`. There is no cheaper route — a page
    /// is not a value that can be cloned, it is a node in a tree with resources
    /// hanging off it, and PDFium's own copy path is the extract.
    ///
    /// It goes through `execute`, so a duplicate can be undone like any other
    /// edit.
    pub fn duplicate_pages(&self, pages: &[usize], at: usize) -> Result<usize> {
        if pages.is_empty() {
            return Err(pdf_core::PdfError::InvalidArgument("no pages to duplicate".into()));
        }
        let command = registry::with_session(self.handle, |s| {
            let mut slice = s
                .document
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("duplicating in this document"))?
                .extract_pages(pages)?;
            let mut bytes = Vec::new();
            slice
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("reading those pages"))
                .and_then(|d| d.save_full_copy(&mut bytes).map(|_| ()))?;
            Ok(pdf_core::command::Command::ImportPages { at, pdf: bytes })
        })?;

        self.execute(command).map(|state| state.page_count)
    }

    /// Bring pages in from another file.
    pub fn import_from(&self, source: &Path, pages: &[usize], at: usize) -> Result<usize> {
        let source = Session::open(source)?;
        self.import_from_session(&source, pages, at)
    }

    /// [`Self::import_from`], with the source already open — for the walks
    /// below, which would otherwise reopen the same file once per page.
    fn import_from_session(&self, source: &Session, pages: &[usize], at: usize) -> Result<usize> {
        let command = registry::with_session(source.handle, |src| {
            let mut slice = src
                .document
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("reading from that document"))?
                .extract_pages(pages)?;
            let mut bytes = Vec::new();
            slice
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("reading those pages"))
                .and_then(|d| d.save_full_copy(&mut bytes).map(|_| ()))?;
            Ok(pdf_core::command::Command::ImportPages { at, pdf: bytes })
        })?;

        self.execute(command).map(|state| state.page_count)
    }

    /// Replace the page at `at` with pages from another file.
    ///
    /// Two commands — the import, then the deletion of the page it displaced
    /// — so each half undoes exactly like the same edits made by hand: the
    /// shape `deletepage 1-3` already has, and the reason `import_at`'s own
    /// note about self-contained commands applies here too.
    pub fn replace_page_from(&self, source: &Path, pages: &[usize], at: usize) -> Result<usize> {
        if pages.is_empty() {
            return Err(pdf_core::PdfError::InvalidArgument(
                "no pages to put in its place".into(),
            ));
        }
        let source = Session::open(source)?;
        self.import_from_session(&source, pages, at)?;
        self.execute(pdf_core::command::Command::DeletePage { index: at + pages.len() })
            .map(|state| state.page_count)
    }

    /// Alternate pages from another file into this one: the first lands just
    /// after page `after`, the next just after the document page that
    /// follows, and so on; once the document runs out the rest are appended.
    ///
    /// One command per page, so each insertion can be taken back on its own
    /// — the same shape as `deletepage 1-3`.
    pub fn interleave_from(&self, source: &Path, pages: &[usize], after: usize) -> Result<usize> {
        let source = Session::open(source)?;
        for (i, page) in pages.iter().enumerate() {
            let at = (after + 1 + 2 * i).min(self.page_count()?);
            self.import_from_session(&source, std::slice::from_ref(page), at)?;
        }
        self.page_count()
    }
    /// Flatten a page: annotations and form fields are baked into the
    /// page's own content. **Not undoable** — PDFium's own flatten, not one
    /// of this engine's commands.
    pub fn flatten_page(&self, page: usize) -> Result<()> {
        registry::with_session(self.handle, |s| s.document.flatten_page(page))
    }

}
