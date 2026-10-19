//! Marks written as annotations — highlight, note, link — and the reading
//! queries beside them: classification, characters, page sizes, bookmarks.
//!
//! Part of the `session` module — split out of the single file the design
//! review flagged (Phase 4, file splits). Methods keep living on `Session`.
use super::*;

impl Session {
    /// Highlight a run of text, as one mark covering however many lines.
    pub fn highlight(
        &self,
        page: usize,
        rects: Vec<pdf_core::document::Rect>,
        colour: pdf_core::document::Color,
    ) -> Result<pdf_core::engine::EditState> {
        self.execute(pdf_core::command::Command::AddAnnotation {
            page_index: page,
            annotation: pdf_core::document::Annotation::Highlight { rects, color: colour },
        })
    }

    /// Anchor a note to a point on the page.
    pub fn note(
        &self,
        page: usize,
        rect: pdf_core::document::Rect,
        contents: String,
        colour: pdf_core::document::Color,
    ) -> Result<pdf_core::engine::EditState> {
        self.execute(pdf_core::command::Command::AddAnnotation {
            page_index: page,
            annotation: pdf_core::document::Annotation::Note {
                rect,
                contents,
                color: colour,
                reply_to: None,
            },
        })
    }

    /// Answer a note with a reply that stays in the PDF's own thread.
    ///
    /// `parent` is PDFium's annotation index for the note, on `page` — the
    /// same index the Comments panel hands out. The reply is created like any
    /// note; the `/IRT` link that makes it a reply is the one reference
    /// PDFium's annotation API cannot write, so it is appended to the file
    /// when the document is saved (see `pdf_core`'s `PendingReply`). The
    /// thread shows in this session as soon as the reply is added.
    pub fn reply_note(
        &self,
        page: usize,
        parent: usize,
        rect: pdf_core::document::Rect,
        contents: String,
        colour: pdf_core::document::Color,
    ) -> Result<pdf_core::engine::EditState> {
        self.execute(pdf_core::command::Command::AddAnnotation {
            page_index: page,
            annotation: pdf_core::document::Annotation::Note {
                rect,
                contents,
                color: colour,
                reply_to: Some(parent),
            },
        })
    }

    /// Make one rectangle of the page a clickable link to `uri`.
    ///
    /// One call per line — see [`pdf_core::document::Annotation::Link`]'s own
    /// doc for why a link over wrapped text is several of these rather than
    /// one annotation with several rects.
    pub fn add_link(
        &self,
        page: usize,
        rect: pdf_core::document::Rect,
        uri: String,
    ) -> Result<pdf_core::engine::EditState> {
        self.execute(pdf_core::command::Command::AddAnnotation {
            page_index: page,
            annotation: pdf_core::document::Annotation::Link { rect, uri },
        })
    }

    /// What kind of text, if any, a page has.
    pub fn classify(&self, page: usize) -> Result<pdf_core::document::PageClassification> {
        registry::with_session(self.handle, |s| s.document.page(page)?.classify())
    }

    /// The characters on a page, unpacked for selection.
    pub fn characters(&self, page: usize) -> Result<crate::reader::Characters> {
        registry::with_session(self.handle, |s| {
            let raw = s.document.page(page)?.characters()?;
            Ok(crate::reader::Characters::new(raw))
        })
    }

    /// Every page's size, for laying out the continuous strip.
    pub fn page_sizes(&self) -> Result<Vec<pdf_core::PageSize>> {
        registry::with_session(self.handle, |s| {
            (0..s.document.page_count()).map(|i| s.document.page_size(i)).collect()
        })
    }

    /// Every bookmark in the document's own outline, title and the page it
    /// goes to, top level only — see [`pdf_core::document::Document::
    /// bookmarks`]'s own doc for why nesting is not modelled.
    /// The document's outline as a tree, children and all — what the
    /// Contents panel shows. [`Session::bookmarks`] stays the flat top
    /// level its callers asked for.
    pub fn outline(&self) -> Result<Vec<pdf_core::document::OutlineEntry>> {
        registry::with_session(self.handle, |s| s.document.outline_entries())
    }

    pub fn bookmarks(&self) -> Result<Vec<(String, usize)>> {
        registry::with_session(self.handle, |s| s.document.bookmarks())
    }
}
