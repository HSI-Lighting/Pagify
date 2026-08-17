//! Orchestration between a document, its cache and a render destination.
//!
//! Deliberately free of JNI types so the cache-hit/miss logic — the part with real
//! branching — can be unit-tested on the host against a fake document, rather than
//! only being exercisable on a device.

use crate::document::{RenderRequest, Rotation};
use crate::error::Result;
use crate::registry::DocumentSession;
use crate::render::bitmap::{Bitmap, PixelOrder};
use crate::render::{CacheKey, RenderTarget};

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
    let (mut w, mut h) = session.document.page_size(index)?.pixel_size(effective.scale);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::metadata::DocumentMetadata;
    use crate::document::{Document, Page, PageSize};
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
}
