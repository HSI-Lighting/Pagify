//! Object manipulation and stacking: listing a page's drawn objects,
//! scaling, removing and re-opaquing one, and the stacking order queries
//! and restack command.
//!
//! Part of the `session` module — split out of the single file the design
//! review flagged (Phase 4, file splits). Methods keep living on `Session`.
use super::*;

impl Session {
    /// What a page draws, in the order it draws it — bottom first.
    pub fn drawn_objects(&self, page_index: usize) -> Result<Vec<pdf_core::document::DrawnObject>> {
        registry::with_session(self.handle, |s| s.document.drawn_objects(page_index))
    }

    /// Resize one thing about a point — see
    /// [`pdf_core::document::DocumentMut::scale_object`].
    pub fn scale_object(
        &self,
        page: usize,
        object: usize,
        anchor: pdf_core::document::Point,
        sx: f32,
        sy: f32,
    ) -> Result<()> {
        registry::with_session(self.handle, |s| s.document.scale_object(page, object, anchor, sx, sy))
    }

    /// Take one thing off the page entirely — see
    /// [`pdf_core::document::DocumentMut::remove_object`].
    pub fn remove_object(&self, page: usize, object: usize) -> Result<()> {
        registry::with_session(self.handle, |s| s.document.remove_object(page, object))
    }

    /// Make one thing more or less see-through — see
    /// [`pdf_core::document::DocumentMut::set_opacity`].
    pub fn set_opacity(&self, page: usize, object: usize, opacity: f32) -> Result<()> {
        registry::with_session(self.handle, |s| s.document.set_opacity(page, object, opacity))
    }

    /// What one step up or down would pass: the nearest thing that overlaps
    /// this one in that direction — see
    /// [`pdf_core::document::Document::stacking_neighbour`].
    pub fn stacking_neighbour(
        &self,
        page_index: usize,
        object: usize,
        up: bool,
    ) -> Result<Option<pdf_core::document::DrawnObject>> {
        registry::with_session(self.handle, |s| s.document.stacking_neighbour(page_index, object, up))
    }

    /// Put one thing at the front or the back of a page's drawing order.
    ///
    /// **The only stacking a PDF has is the order it draws things in**, so this
    /// moves the operators that draw it and carries the state they were drawn
    /// under along with them. See [`pdf_core::document::DocumentMut::restack`]
    /// for what it declines.
    pub fn restack(
        &self,
        page_index: usize,
        object: usize,
        where_to: pdf_core::document::Stacking,
    ) -> Result<()> {
        registry::with_session(self.handle, |s| s.document.restack(page_index, object, where_to))
    }
}
