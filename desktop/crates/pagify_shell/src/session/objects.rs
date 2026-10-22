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
        let mut objects = registry::with_session(self.handle, |s| s.document.drawn_objects(page_index))?;
        let f = self.frame(page_index);
        objects.iter_mut().for_each(|d| f.drawn(d));
        Ok(objects)
    }

    /// Some of a page's pictures and shapes, lifted into a one-page clip of
    /// their own operators — see [`pdf_core::document::Document::lift_objects`].
    pub fn lift_objects(&self, page_index: usize, objects: &[usize]) -> Result<pdf_core::document::Lifted> {
        let mut lifted = registry::with_session(self.handle, |s| s.document.lift_objects(page_index, objects))?;
        self.frame(page_index).lifted(&mut lifted);
        Ok(lifted)
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
        let f = self.frame(page);
        let (anchor, (sx, sy)) = (f.point_back(anchor), if f.swaps_axes() { (sy, sx) } else { (sx, sy) });
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
        // Through the undo history: it went straight to the document, and
        // undo after it could not take it back (0.1.80).
        self.execute(pdf_core::command::Command::SetOpacity { page_index: page, object, opacity }).map(|_| ())
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
        let mut found = registry::with_session(self.handle, |s| s.document.stacking_neighbour(page_index, object, up))?;
        if let Some(d) = found.as_mut() {
            self.frame(page_index).drawn(d);
        }
        Ok(found)
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
        // **Through the undo history.** Straight to the document, undo could
        // not take it back — "it goes behind an invisible layer" — and, since
        // a restack renumbers the page's objects, the undo of an earlier move
        // then moved the wrong ones (0.1.80).
        self.execute(pdf_core::command::Command::Restack { page_index, object, where_to }).map(|_| ())
    }
}
