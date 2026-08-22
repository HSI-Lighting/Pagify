//! Orchestration between a document, its cache and a render destination.
//!
//! Deliberately free of JNI types so the cache-hit/miss logic — the part with real
//! branching — can be unit-tested on the host against a fake document, rather than
//! only being exercisable on a device.

use serde::{Deserialize, Serialize};

use crate::command::Command;
use crate::document::Color;
use crate::document::{Document, Point, RegionRequest, RenderRequest, Rotation};
use crate::error::{PdfError, Result};
use crate::registry::DocumentSession;
use crate::render::bitmap::{Bitmap, PixelOrder};
use crate::render::{
    CacheKey, ImageFormat, Markup, RegionPixels, RenderTarget, ViewportPlan, ViewportRequest,
};

/// Pixel dimensions a page occupies for the given request, accounting for rotation.
pub fn page_pixel_size(
    session: &DocumentSession,
    index: usize,
    request: &RenderRequest,
) -> Result<(u32, u32)> {
    let (mut w, mut h) = session.document.page_size(index)?.pixel_size(request.scale);
    if request.rotation.swaps_axes() {
        std::mem::swap(&mut w, &mut h);
    }
    Ok((w, h))
}

fn cache_key(index: usize, request: &RenderRequest) -> CacheKey {
    let turns = match request.rotation {
        Rotation::None => 0,
        Rotation::Clockwise90 => 1,
        Rotation::Clockwise180 => 2,
        Rotation::Clockwise270 => 3,
    };
    CacheKey::new(index, request.scale, turns)
}

/// Outcome of a render, so callers (and tests) can tell a cache hit from a miss.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderOutcome {
    /// Blitted from a previously prefetched raster.
    CacheHit,
    /// Rasterised now, straight into the caller's target.
    Rendered,
}

/// Draw a page into `target`, using the cache when it holds a usable raster.
///
/// A cached entry whose dimensions do not match the target is treated as a miss
/// and dropped: that happens when Kotlin's page-size arithmetic rounds differently
/// from the quantised zoom the cache stored, and silently rejecting it is what
/// keeps a stale raster from ever being stretched into the wrong-sized bitmap.
pub fn render_page_into(
    session: &mut DocumentSession,
    index: usize,
    request: &RenderRequest,
    target: &mut RenderTarget<'_>,
) -> Result<RenderOutcome> {
    session.document.validate_page_index(index)?;
    let key = cache_key(index, request);

    let usable = match session.cache.get(&key) {
        Some(cached) => cached.width == target.width && cached.height == target.height,
        None => false,
    };

    if usable {
        // Re-fetched rather than held across the check to keep the borrow short.
        let cached = session
            .cache
            .get(&key)
            .expect("entry was present a moment ago and nothing can evict it in between");
        target.copy_from(cached)?;
        return Ok(RenderOutcome::CacheHit);
    }

    if session.cache.contains(&key) {
        session.cache.remove(&key);
    }

    let page = session.document.page(index)?;
    page.render_into(request, target)?;
    Ok(RenderOutcome::Rendered)
}

/// Rasterise a page into the cache without any on-screen destination.
///
/// This is what makes the cache worth having: called on a background thread for
/// the pages either side of the current one, so a swipe resolves to a memcpy.
pub fn prefetch_page(
    session: &mut DocumentSession,
    index: usize,
    request: &RenderRequest,
) -> Result<bool> {
    session.document.validate_page_index(index)?;
    let key = cache_key(index, request);
    if session.cache.contains(&key) {
        return Ok(false);
    }

    // Render at the zoom the *key* represents, not the raw requested zoom.
    // Storing a raster rendered at 1.6x under a key that means 1.75x would make
    // every later hit a size mismatch, and the cache would never pay off.
    let effective = RenderRequest {
        scale: key.effective_zoom(),
        ..*request
    };

    // Size first, without loading the page: if the raster would be too large to
    // cache there is no point paying for a page load at all.
    let (mut w, mut h) = session
        .document
        .page_size(index)?
        .pixel_size(effective.scale);
    if effective.rotation.swaps_axes() {
        std::mem::swap(&mut w, &mut h);
    }

    let mut bitmap = Bitmap::new(w, h, PixelOrder::Rgba)?;
    {
        let page = session.document.page(index)?;
        let mut target = RenderTarget::from_bitmap(&mut bitmap)?;
        page.render_into(&effective, &mut target)?;
    }

    session.cache.put(key, bitmap);
    Ok(true)
}

/// Rasterise one region of a page for export.
///
/// Deliberately **not** cached. The page cache exists so a swipe resolves to a
/// memcpy, and it is budgeted for that; a single 4× export can be larger than the
/// whole budget, so caching one would evict every raster the reader is about to
/// need in order to keep something that is used once and never asked for again.
pub fn render_region(
    document: &dyn Document,
    index: usize,
    request: &RegionRequest,
) -> Result<Bitmap> {
    document.validate_page_index(index)?;
    let page = document.page(index)?;
    page.render_region(request)
}

/// Render a region, draw the markup on it, and encode it — in one call.
///
/// One call rather than three so the intermediate bitmap never crosses the FFI.
/// A 4× capture is tens of megabytes as pixels and a fraction of that encoded;
/// handing the raw buffer to Kotlin only to encode it there would cost a copy
/// into the Java heap of the larger of the two.
///
/// The markup is composited here rather than drawn by the UI and screen-grabbed,
/// for the same reason the capture itself is a re-render: what leaves the app is
/// built from the document and the committed shapes, and can hold nothing else.
pub fn export_region(
    document: &dyn Document,
    index: usize,
    request: &RegionRequest,
    format: ImageFormat,
    marks: &[Markup],
) -> Result<Vec<u8>> {
    let mut bitmap = render_region(document, index, request)?;

    if !marks.is_empty() {
        // Resolved again rather than threaded out of the render: `resolve` is a
        // pure function of the same three inputs, so the two agree by
        // construction — including when the ceiling lowered the scale, which is
        // exactly the case where a separately-computed transform would drift.
        let region =
            RegionPixels::resolve(document.page_size(index)?, request.crop, request.scale)?;
        crate::render::markup::composite(&mut bitmap, marks, region.scale)?;
    }

    crate::render::export::encode(&bitmap, format)
}

/// Capture what is on screen, across however many pages that turns out to be.
///
/// The reader lays pages out in a column, so the region someone drags a box
/// around is very often *not* one page: it straddles a join, or takes the bottom
/// of one page and the top of the next. Capturing only the page the drag started
/// on answers a question nobody asked.
///
/// Each [`Tile`] is one page's share of the picture — which part of that page,
/// and where it belongs in the result. The caller works this out from its own
/// layout, because the layout is the caller's: the engine has no idea where a
/// page happens to sit on a screen, and inventing an idea here would be a second
/// copy of the reader's arithmetic to keep in step.
///
/// Still not a screenshot. Every pixel is rendered from the document, so the gaps
/// between pages come out as the supplied background rather than as whatever the
/// app happened to be drawing there.
pub fn export_viewport(
    document: &dyn Document,
    request: &ViewportRequest,
    format: ImageFormat,
    marks: &[Markup],
    // The ring to keep, in capture units; empty means the whole rectangle.
    mask: &[Point],
) -> Result<Vec<u8>> {
    let plan = ViewportPlan::resolve(request)?;
    let mut bitmap = Bitmap::new(plan.width, plan.height, PixelOrder::Rgba)?;
    fill(&mut bitmap, request.background);

    for tile in &request.tiles {
        let Some(placed) = plan.place(tile) else {
            // Entirely outside the picture, which is normal: the caller lists the
            // pages that *might* contribute and lets this decide.
            continue;
        };

        let rendered = {
            document.validate_page_index(tile.page_index)?;
            let page = document.page(tile.page_index)?;
            page.render_region(&RegionRequest {
                crop: tile.crop,
                scale: placed.scale,
                render_annotations: request.render_annotations,
                render_form_data: request.render_form_data,
            })?
        };

        blit(&mut bitmap, &rendered, placed.left, placed.top);
    }

    // Before the marks, not after. A mark drawn near the edge of the ring is
    // still the user's mark; erasing it because the finger strayed a pixel
    // outside would be the tool eating the annotation it was made for.
    if !mask.is_empty() {
        crate::render::markup::mask_outside(&mut bitmap, mask, plan.scale, request.background)?;
    }

    if !marks.is_empty() {
        crate::render::markup::composite(&mut bitmap, marks, plan.scale)?;
    }

    crate::render::export::encode(&bitmap, format)
}

/// Paint every pixel, including alpha.
///
/// The background shows through wherever no page reaches — between two pages, and
/// around a picture dragged past the edge of one. Left unpainted those pixels
/// would be whatever the allocation held.
fn fill(bitmap: &mut Bitmap, colour: Color) {
    for pixel in bitmap.data.chunks_exact_mut(4) {
        pixel[0] = colour.r;
        pixel[1] = colour.g;
        pixel[2] = colour.b;
        pixel[3] = colour.a;
    }
}

/// Copy `source` into `destination` at a pixel offset, clipped to the edges.
///
/// Clipping rather than asserting: rounding a tile's edges into whole pixels can
/// put its last row or column a pixel past the picture, and losing that pixel is
/// invisible where refusing to draw the tile at all is not.
fn blit(destination: &mut Bitmap, source: &Bitmap, left: i32, top: i32) {
    for row in 0..source.height as i32 {
        let target_row = top + row;
        if target_row < 0 || target_row >= destination.height as i32 {
            continue;
        }

        // The overlap on this row, in source pixels.
        let from = (-left).max(0);
        let to = (source.width as i32).min(destination.width as i32 - left);
        if to <= from {
            continue;
        }

        let source_start = row as usize * source.stride + from as usize * 4;
        let source_end = row as usize * source.stride + to as usize * 4;
        let destination_start =
            target_row as usize * destination.stride + (left + from) as usize * 4;
        let length = (to - from) as usize * 4;

        destination.data[destination_start..destination_start + length]
            .copy_from_slice(&source.data[source_start..source_end]);
    }
}

// ------------------------------------------------------------------- editing --

/// Everything the UI needs to redraw its edit controls after a mutation.
///
/// Returned from every edit entry point rather than left to the caller to
/// assemble: page count, undo availability and dirtiness all change together, and
/// fetching them one at a time across JNI would let the UI paint a state the
/// document was never actually in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditState {
    pub page_count: usize,
    pub can_undo: bool,
    pub can_redo: bool,
    /// `None` when that stack is empty. Already phrased as a user action
    /// ("Delete page 5"), so the UI can label its buttons with no lookup table.
    pub undo_label: Option<String>,
    pub redo_label: Option<String>,
    /// Whether a save would have anything to write.
    pub dirty: bool,
    /// False for a document that cannot be edited at all, so the UI can hide the
    /// controls rather than offer them and then fail.
    pub editable: bool,
}

pub fn edit_state(session: &mut DocumentSession) -> EditState {
    let page_count = session.document.page_count();
    let (editable, dirty) = match session.document.as_document_mut() {
        Some(doc) => (true, doc.is_dirty()),
        None => (false, false),
    };

    EditState {
        page_count,
        can_undo: session.history.can_undo(),
        can_redo: session.history.can_redo(),
        undo_label: session.history.undo_description(),
        redo_label: session.history.redo_description(),
        dirty,
        editable,
    }
}

/// Drop the rasters an edit made stale.
///
/// An empty `affected` means *everything*, which is what
/// [`Command::affected_pages`] returns for any change to the page tree: deleting
/// page 2 renumbers every page after it, so a cache keyed by index has nothing it
/// can safely keep. Getting this wrong is invisible until a user deletes a page
/// and the one after it still draws the old content.
fn invalidate(session: &mut DocumentSession, affected: &[usize]) {
    if affected.is_empty() {
        session.cache.clear();
    } else {
        for &index in affected {
            session.cache.remove_page(index);
        }
    }
}

/// Run a command, record it for undo, and invalidate what it changed.
///
/// The only way a document is mutated. Nothing here inspects the command, so a new
/// [`Command`] variant needs no change at this layer — which is the point of
/// routing every edit through one.
pub fn execute(session: &mut DocumentSession, command: Command) -> Result<EditState> {
    let affected = {
        // `document` and `history` are separate fields, so both can be borrowed at
        // once; the block ends the borrows before `invalidate` takes the session.
        let doc = session
            .document
            .as_document_mut()
            .ok_or(PdfError::Unsupported("editing this document"))?;
        session.history.execute(command, doc)?
    };
    invalidate(session, &affected);
    Ok(edit_state(session))
}

/// Reverse the most recent command.
///
/// `false` means there was nothing to undo, which is not an error — the UI keeps
/// its buttons enabled off [`EditState`], and a redundant tap should be a no-op
/// rather than an exception.
pub fn undo(session: &mut DocumentSession) -> Result<(bool, EditState)> {
    let affected = {
        let doc = session
            .document
            .as_document_mut()
            .ok_or(PdfError::Unsupported("editing this document"))?;
        session.history.undo(doc)?
    };
    apply_outcome(session, affected)
}

/// Re-apply the most recently undone command. `false` means nothing to redo.
pub fn redo(session: &mut DocumentSession) -> Result<(bool, EditState)> {
    let affected = {
        let doc = session
            .document
            .as_document_mut()
            .ok_or(PdfError::Unsupported("editing this document"))?;
        session.history.redo(doc)?
    };
    apply_outcome(session, affected)
}

fn apply_outcome(
    session: &mut DocumentSession,
    affected: Option<Vec<usize>>,
) -> Result<(bool, EditState)> {
    match affected {
        Some(affected) => {
            invalidate(session, &affected);
            Ok((true, edit_state(session)))
        }
        // Nothing moved, so nothing is stale.
        None => Ok((false, edit_state(session))),
    }
}

/// Write the document out.
///
/// `incremental` appends a delta and leaves the original bytes untouched, which is
/// what keeps an existing digital signature valid. The full copy rewrites and
/// compacts the file and destroys every signature over it, so it is offered as an
/// explicit choice and never as the default.
pub fn save(
    session: &mut DocumentSession,
    dest: &mut dyn std::io::Write,
    incremental: bool,
) -> Result<()> {
    let doc = session
        .document
        .as_document_mut()
        .ok_or(PdfError::Unsupported("saving this document"))?;
    if incremental {
        doc.save_incremental(dest)
    } else {
        doc.save_full_copy(dest)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::metadata::DocumentMetadata;
    use crate::document::{
        Annotation, Color, Document, DocumentMut, Page, PageSize, Point, Rect, RemovedPage,
    };
    use crate::error::PdfError;
    use std::cell::Cell;
    use std::rc::Rc;

    /// Paints every pixel a per-page colour so a blit can be verified by content,
    /// and counts renders so cache hits are observable rather than assumed.
    struct FakeDocument {
        pages: usize,
        size: PageSize,
        renders: Rc<Cell<usize>>,
    }

    struct FakePage {
        size: PageSize,
        marker: u8,
        renders: Rc<Cell<usize>>,
    }

    // Sound for the test harness: never shared across threads.
    unsafe impl Send for FakeDocument {}
    unsafe impl Sync for FakeDocument {}

    impl Document for FakeDocument {
        fn page_count(&self) -> usize {
            self.pages
        }
        fn metadata(&self) -> Result<DocumentMetadata> {
            Ok(DocumentMetadata {
                page_count: self.pages,
                ..Default::default()
            })
        }
        fn page(&self, index: usize) -> Result<Box<dyn Page + '_>> {
            self.validate_page_index(index)?;
            Ok(Box::new(FakePage {
                size: self.size,
                marker: (index as u8) + 1,
                renders: Rc::clone(&self.renders),
            }))
        }
    }

    impl Page for FakePage {
        fn size(&self) -> PageSize {
            self.size
        }
        fn render_into(
            &self,
            _request: &RenderRequest,
            target: &mut RenderTarget<'_>,
        ) -> Result<()> {
            self.renders.set(self.renders.get() + 1);
            target.pixels.fill(self.marker);
            Ok(())
        }
        fn text(&self) -> Result<String> {
            Ok(format!("page {}", self.marker))
        }
    }

    fn session_with(pages: usize) -> (DocumentSession, Rc<Cell<usize>>) {
        let renders = Rc::new(Cell::new(0));
        let doc = FakeDocument {
            pages,
            size: PageSize {
                width_pt: 100.0,
                height_pt: 200.0,
            },
            renders: Rc::clone(&renders),
        };
        (DocumentSession::new(Box::new(doc)), renders)
    }

    fn request(scale: f32) -> RenderRequest {
        RenderRequest {
            scale,
            ..Default::default()
        }
    }

    #[test]
    fn page_size_accounts_for_rotation() {
        let (session, _) = session_with(1);
        let upright = page_pixel_size(&session, 0, &request(1.0)).unwrap();
        assert_eq!(upright, (100, 200));

        let turned = page_pixel_size(
            &session,
            0,
            &RenderRequest {
                scale: 1.0,
                rotation: Rotation::Clockwise90,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(turned, (200, 100), "a quarter turn swaps the axes");
    }

    #[test]
    fn a_cold_render_goes_to_the_document_and_fills_the_target() {
        let (mut session, renders) = session_with(2);
        let mut buf = vec![0u8; 100 * 200 * 4];
        let mut target = RenderTarget::new(100, 200, 400, PixelOrder::Rgba, &mut buf).unwrap();

        let outcome = render_page_into(&mut session, 1, &request(1.0), &mut target).unwrap();

        assert_eq!(outcome, RenderOutcome::Rendered);
        assert_eq!(renders.get(), 1);
        assert!(buf.iter().all(|&b| b == 2), "page 1 paints marker 2");
    }

    #[test]
    fn a_prefetched_page_is_served_from_cache_without_rendering_again() {
        let (mut session, renders) = session_with(2);
        let req = request(1.0);

        assert!(prefetch_page(&mut session, 0, &req).unwrap());
        assert_eq!(renders.get(), 1, "the prefetch itself rendered once");

        let mut buf = vec![0u8; 100 * 200 * 4];
        let mut target = RenderTarget::new(100, 200, 400, PixelOrder::Rgba, &mut buf).unwrap();
        let outcome = render_page_into(&mut session, 0, &req, &mut target).unwrap();

        assert_eq!(outcome, RenderOutcome::CacheHit);
        assert_eq!(renders.get(), 1, "no second rasterisation");
        assert!(
            buf.iter().all(|&b| b == 1),
            "the blit must carry the page's actual pixels, not zeros"
        );
    }

    #[test]
    fn prefetching_the_same_page_twice_does_no_extra_work() {
        let (mut session, renders) = session_with(1);
        let req = request(1.0);

        assert!(prefetch_page(&mut session, 0, &req).unwrap());
        assert!(!prefetch_page(&mut session, 0, &req).unwrap());
        assert_eq!(renders.get(), 1);
    }

    #[test]
    fn a_cached_raster_of_the_wrong_size_is_discarded_rather_than_blitted() {
        let (mut session, renders) = session_with(1);

        // Prefetch at 1.0 => a 100x200 raster.
        prefetch_page(&mut session, 0, &request(1.0)).unwrap();
        assert_eq!(renders.get(), 1);

        // Ask for the same cache key but hand over a differently-sized target,
        // as Kotlin rounding can produce.
        let mut buf = vec![0u8; 101 * 200 * 4];
        let mut target = RenderTarget::new(101, 200, 404, PixelOrder::Rgba, &mut buf).unwrap();
        let outcome = render_page_into(&mut session, 0, &request(1.0), &mut target).unwrap();

        assert_eq!(
            outcome,
            RenderOutcome::Rendered,
            "a size mismatch must fall back to a fresh render"
        );
        assert_eq!(renders.get(), 2);
    }

    #[test]
    fn prefetch_renders_at_the_quantised_zoom_so_later_hits_line_up() {
        let (mut session, _) = session_with(1);

        // 1.6 quantises up to 1.75; the cached raster must be 1.75-sized.
        prefetch_page(&mut session, 0, &request(1.6)).unwrap();

        let (w, h) = (175u32, 350u32); // 100pt x 1.75, 200pt x 1.75
        let mut buf = vec![0u8; (w * h * 4) as usize];
        let mut target =
            RenderTarget::new(w, h, (w * 4) as usize, PixelOrder::Rgba, &mut buf).unwrap();

        let outcome = render_page_into(&mut session, 0, &request(1.6), &mut target).unwrap();
        assert_eq!(
            outcome,
            RenderOutcome::CacheHit,
            "prefetch and lookup must agree on the raster's size"
        );
    }

    #[test]
    fn out_of_range_pages_are_refused_by_both_entry_points() {
        let (mut session, _) = session_with(2);
        let mut buf = vec![0u8; 100 * 200 * 4];
        let mut target = RenderTarget::new(100, 200, 400, PixelOrder::Rgba, &mut buf).unwrap();

        assert!(matches!(
            render_page_into(&mut session, 5, &request(1.0), &mut target),
            Err(PdfError::PageOutOfRange { index: 5, count: 2 })
        ));
        assert!(matches!(
            prefetch_page(&mut session, 5, &request(1.0)),
            Err(PdfError::PageOutOfRange { index: 5, count: 2 })
        ));
    }

    #[test]
    fn clearing_the_cache_forces_the_next_render_to_rasterise_again() {
        let (mut session, renders) = session_with(1);
        let req = request(1.0);
        prefetch_page(&mut session, 0, &req).unwrap();

        session.cache.clear();

        let mut buf = vec![0u8; 100 * 200 * 4];
        let mut target = RenderTarget::new(100, 200, 400, PixelOrder::Rgba, &mut buf).unwrap();
        let outcome = render_page_into(&mut session, 0, &req, &mut target).unwrap();

        assert_eq!(outcome, RenderOutcome::Rendered);
        assert_eq!(renders.get(), 2);
    }

    // ---------------------------------------------------------------- editing --

    /// A page tree that can be mutated, with pages identified by width — so a
    /// delete or a reorder is observable without rendering anything.
    struct EditableDoc {
        widths: Vec<f32>,
        rotations: Vec<u8>,
        dirty: bool,
        /// Marks per page, so an add and its undo are observable.
        annotations: std::collections::HashMap<usize, Vec<Annotation>>,
    }

    // Sound for the test harness: never shared across threads.
    unsafe impl Send for EditableDoc {}
    unsafe impl Sync for EditableDoc {}

    impl EditableDoc {
        fn with_pages(count: usize) -> Self {
            EditableDoc {
                widths: (0..count).map(|i| 100.0 + i as f32 * 10.0).collect(),
                rotations: vec![0; count],
                dirty: false,
                annotations: std::collections::HashMap::new(),
            }
        }
    }

    impl Document for EditableDoc {
        fn page_count(&self) -> usize {
            self.widths.len()
        }
        fn metadata(&self) -> Result<DocumentMetadata> {
            Ok(DocumentMetadata {
                page_count: self.widths.len(),
                ..Default::default()
            })
        }
        fn page(&self, index: usize) -> Result<Box<dyn Page + '_>> {
            self.validate_page_index(index)?;
            unreachable!("these tests never rasterise")
        }
        fn page_size(&self, index: usize) -> Result<PageSize> {
            self.validate_page_index(index)?;
            Ok(PageSize {
                width_pt: self.widths[index],
                height_pt: 100.0,
            })
        }
        fn as_document_mut(&mut self) -> Option<&mut dyn DocumentMut> {
            Some(self)
        }
    }

    impl DocumentMut for EditableDoc {
        fn text_mark_restore(&mut self, _page_index: usize, id: i32) -> Result<String> {
            Err(PdfError::InvalidArgument(format!("no text mark {id}")))
        }

        fn remove_text(&mut self, _page_index: usize, id: i32) -> Result<()> {
            Err(PdfError::InvalidArgument(format!("no text mark {id}")))
        }

        fn reorder_pages(&mut self, order: &[usize]) -> Result<()> {
            let mut moved = vec![0f32; self.widths.len()];
            for (from, &to) in order.iter().enumerate() {
                moved[to] = self.widths[from];
            }
            self.widths = moved;
            self.dirty = true;
            Ok(())
        }
        fn delete_page(&mut self, index: usize) -> Result<RemovedPage> {
            let width = self.widths.remove(index);
            self.rotations.remove(index);
            self.dirty = true;
            Ok(RemovedPage::new(
                PageSize {
                    width_pt: width,
                    height_pt: 100.0,
                },
                Vec::new(),
            ))
        }
        fn insert_page(&mut self, at: usize, page: RemovedPage) -> Result<()> {
            self.widths.insert(at, page.size.width_pt);
            self.rotations.insert(at, 0);
            self.dirty = true;
            Ok(())
        }
        fn insert_blank_page(&mut self, at: usize, size: PageSize) -> Result<()> {
            self.widths.insert(at, size.width_pt);
            self.rotations.insert(at, 0);
            self.dirty = true;
            Ok(())
        }
        fn set_page_rotation(&mut self, index: usize, quarter_turns: u8) -> Result<()> {
            self.rotations[index] = quarter_turns;
            self.dirty = true;
            Ok(())
        }
        fn page_rotation(&self, index: usize) -> Result<u8> {
            Ok(self.rotations[index])
        }
        fn add_annotation(&mut self, page: usize, annotation: &Annotation) -> Result<usize> {
            let marks = self.annotations.entry(page).or_default();
            marks.push(annotation.clone());
            self.dirty = true;
            Ok(marks.len() - 1)
        }

        fn remove_annotation(&mut self, page: usize, index: usize) -> Result<()> {
            self.take_annotation(page, index).map(|_| ())
        }

        fn take_annotation(&mut self, page: usize, index: usize) -> Result<Annotation> {
            let marks = self
                .annotations
                .get_mut(&page)
                .ok_or(PdfError::Unsupported("no marks on that page"))?;
            if index >= marks.len() {
                return Err(PdfError::Unsupported("no such mark"));
            }
            self.dirty = true;
            Ok(marks.remove(index))
        }

        fn extract_pages(&self, _range: &[usize]) -> Result<Box<dyn Document>> {
            Err(PdfError::Unsupported("extract in tests"))
        }
        fn import_pages(&mut self, _f: &dyn Document, _r: &[usize], _at: usize) -> Result<()> {
            Ok(())
        }
        fn save_incremental(&mut self, dest: &mut dyn std::io::Write) -> Result<()> {
            dest.write_all(b"incremental")?;
            Ok(())
        }
        fn save_full_copy(&mut self, dest: &mut dyn std::io::Write) -> Result<()> {
            dest.write_all(b"full")?;
            Ok(())
        }
        fn is_dirty(&self) -> bool {
            self.dirty
        }
    }

    fn editable_session(pages: usize) -> DocumentSession {
        DocumentSession::new(Box::new(EditableDoc::with_pages(pages)))
    }

    /// One raster per page at two different zooms, so a test can tell
    /// "invalidated page 2" apart from "cleared everything".
    fn fill_cache(session: &mut DocumentSession, pages: usize) {
        for index in 0..pages {
            for zoom in [1.0f32, 2.0] {
                session.cache.put(
                    CacheKey::new(index, zoom, 0),
                    Bitmap::new(4, 4, PixelOrder::Rgba).unwrap(),
                );
            }
        }
    }

    #[test]
    fn rotating_a_page_invalidates_that_page_at_every_zoom_and_leaves_the_rest() {
        let mut session = editable_session(3);
        fill_cache(&mut session, 3);
        assert_eq!(session.cache.len(), 6);

        execute(
            &mut session,
            Command::SetPageRotation {
                index: 1,
                quarter_turns: 1,
            },
        )
        .unwrap();

        // Both of page 1's rasters go; the other two pages keep theirs.
        assert_eq!(session.cache.len(), 4);
        assert!(!session.cache.contains(&CacheKey::new(1, 1.0, 0)));
        assert!(!session.cache.contains(&CacheKey::new(1, 2.0, 0)));
        assert!(session.cache.contains(&CacheKey::new(0, 1.0, 0)));
        assert!(session.cache.contains(&CacheKey::new(2, 2.0, 0)));
    }

    #[test]
    fn deleting_a_page_clears_the_whole_cache_because_every_later_index_shifts() {
        let mut session = editable_session(3);
        fill_cache(&mut session, 3);

        execute(&mut session, Command::DeletePage { index: 0 }).unwrap();

        assert_eq!(
            session.cache.len(),
            0,
            "page 1's raster is now index 0's; keeping any of them draws the wrong page"
        );
    }

    #[test]
    fn an_edit_and_its_undo_leave_the_page_tree_where_it_started() {
        let mut session = editable_session(3);

        let after = execute(&mut session, Command::DeletePage { index: 1 }).unwrap();
        assert_eq!(after.page_count, 2);
        assert!(after.can_undo);
        assert_eq!(after.undo_label.as_deref(), Some("Delete page 2"));

        let (undone, state) = undo(&mut session).unwrap();
        assert!(undone);
        assert_eq!(state.page_count, 3);
        assert!(!state.can_undo);
        assert!(state.can_redo);

        // The page that comes back is the one removed, not a blank of the same size.
        assert_eq!(session.document.page_size(1).unwrap().width_pt, 110.0);
    }

    #[test]
    fn redo_reapplies_and_a_fresh_edit_discards_the_redo_branch() {
        let mut session = editable_session(3);
        execute(&mut session, Command::DeletePage { index: 2 }).unwrap();
        undo(&mut session).unwrap();

        let (redone, state) = redo(&mut session).unwrap();
        assert!(redone);
        assert_eq!(state.page_count, 2);

        undo(&mut session).unwrap();
        execute(
            &mut session,
            Command::InsertBlankPage {
                at: 0,
                width_pt: 200.0,
                height_pt: 300.0,
            },
        )
        .unwrap();

        let state = edit_state(&mut session);
        assert!(
            !state.can_redo,
            "a fresh edit makes the undone branch unreachable"
        );
    }

    #[test]
    fn undo_and_redo_on_an_empty_history_are_no_ops_rather_than_errors() {
        let mut session = editable_session(2);

        let (undone, _) = undo(&mut session).unwrap();
        let (redone, _) = redo(&mut session).unwrap();

        assert!(
            !undone,
            "nothing to undo is a no-op, not a thrown exception"
        );
        assert!(!redone);
    }

    #[test]
    fn a_read_only_document_reports_itself_uneditable_instead_of_failing_later() {
        let (mut session, _) = session_with(2);

        let state = edit_state(&mut session);
        assert!(!state.editable);
        assert!(!state.dirty);

        assert!(matches!(
            execute(&mut session, Command::DeletePage { index: 0 }),
            Err(PdfError::Unsupported(_))
        ));
    }

    #[test]
    fn dirtiness_tracks_whether_a_save_would_have_anything_to_write() {
        let mut session = editable_session(2);
        assert!(!edit_state(&mut session).dirty);

        execute(
            &mut session,
            Command::SetPageRotation {
                index: 0,
                quarter_turns: 2,
            },
        )
        .unwrap();

        assert!(edit_state(&mut session).dirty);
    }

    #[test]
    fn the_incremental_save_runs_unless_a_full_copy_is_asked_for() {
        let mut session = editable_session(1);

        let mut appended = Vec::new();
        save(&mut session, &mut appended, true).unwrap();
        assert_eq!(appended, b"incremental");

        let mut rewritten = Vec::new();
        save(&mut session, &mut rewritten, false).unwrap();
        assert_eq!(rewritten, b"full");
    }

    // ---------------------------------------------------------- annotations --

    fn yellow() -> Color {
        Color {
            r: 255,
            g: 214,
            b: 0,
            a: 128,
        }
    }

    fn highlight_over(line: f32) -> Annotation {
        Annotation::Highlight {
            rects: vec![Rect {
                left: 10.0,
                top: line,
                right: 200.0,
                bottom: line + 12.0,
            }],
            color: yellow(),
        }
    }

    #[test]
    fn a_mark_and_its_undo_leave_the_page_as_it_was() {
        let mut session = editable_session(2);

        let state = execute(
            &mut session,
            Command::AddAnnotation {
                page_index: 1,
                annotation: highlight_over(40.0),
            },
        )
        .unwrap();

        assert!(state.dirty);
        assert_eq!(state.undo_label.as_deref(), Some("Highlight on page 2"));

        let (undone, _) = undo(&mut session).unwrap();
        assert!(undone);

        // The mark is gone from the page it was put on, and the page tree is
        // untouched — a mark must never renumber anything.
        let doc = session.document.as_document_mut().unwrap();
        assert!(matches!(
            doc.remove_annotation(1, 0),
            Err(PdfError::Unsupported(_))
        ));
        assert_eq!(2, session.document.page_count());
    }

    #[test]
    fn a_mark_invalidates_only_the_page_it_is_on() {
        let mut session = editable_session(3);
        fill_cache(&mut session, 3);
        assert_eq!(session.cache.len(), 6);

        execute(
            &mut session,
            Command::AddAnnotation {
                page_index: 2,
                annotation: highlight_over(10.0),
            },
        )
        .unwrap();

        // Four rasters survive. Marks are made far more often than pages are
        // moved, so clearing the whole cache for one highlight would make the
        // common case pay for the rare one.
        assert_eq!(session.cache.len(), 4);
        assert!(!session.cache.contains(&CacheKey::new(2, 1.0, 0)));
        assert!(session.cache.contains(&CacheKey::new(0, 1.0, 0)));
        assert!(session.cache.contains(&CacheKey::new(1, 2.0, 0)));
    }

    #[test]
    fn erasing_a_mark_puts_it_back_on_undo() {
        let mut session = editable_session(1);
        execute(
            &mut session,
            Command::AddAnnotation {
                page_index: 0,
                annotation: highlight_over(20.0),
            },
        )
        .unwrap();

        execute(
            &mut session,
            Command::RemoveAnnotation {
                page_index: 0,
                index: 0,
            },
        )
        .unwrap();

        let (undone, state) = undo(&mut session).unwrap();
        assert!(undone);
        // Two marks would mean the undo re-added without the erase having taken;
        // none would mean the record lost it.
        assert_eq!(state.undo_label.as_deref(), Some("Highlight on page 1"));
    }

    #[test]
    fn every_tool_the_reader_offers_survives_a_round_trip_through_json() {
        // The wire format is shared with Kotlin and nothing checks it at compile
        // time — the same trap that made setPageRotation undecodable while both
        // suites were green.
        let marks = [
            highlight_over(30.0),
            Annotation::Ink {
                strokes: vec![vec![Point { x: 1.0, y: 2.0 }, Point { x: 3.0, y: 4.0 }]],
                color: yellow(),
                width: 3.5,
            },
            Annotation::Note {
                rect: Rect {
                    left: 5.0,
                    top: 5.0,
                    right: 25.0,
                    bottom: 25.0,
                },
                contents: "check this".into(),
                color: yellow(),
            },
        ];

        for mark in marks {
            let command = Command::AddAnnotation {
                page_index: 4,
                annotation: mark.clone(),
            };
            let json = serde_json::to_string(&command).expect("encode");
            let decoded: Command = serde_json::from_str(&json)
                .unwrap_or_else(|e| panic!("{json} failed to decode: {e}"));
            assert_eq!(command, decoded);
        }
    }

    #[test]
    fn the_json_an_annotation_command_produces_is_all_camel_case() {
        let json = serde_json::to_string(&Command::AddAnnotation {
            page_index: 2,
            annotation: Annotation::Ink {
                strokes: vec![vec![Point { x: 1.0, y: 2.0 }]],
                color: yellow(),
                width: 2.0,
            },
        })
        .expect("encode");

        // `page_index` is the one that would have slipped through: every other
        // field in this enum is a single word.
        assert!(json.contains(r#""pageIndex":2"#), "got {json}");
        assert!(json.contains(r#""op":"addAnnotation""#), "got {json}");
        assert!(json.contains(r#""kind":"ink""#), "got {json}");
    }
}
