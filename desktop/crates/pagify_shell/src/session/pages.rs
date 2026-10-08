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
}
