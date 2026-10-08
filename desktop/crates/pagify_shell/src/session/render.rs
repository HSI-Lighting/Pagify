//! Page geometry and raster rendering: counts and sizes, page and region
//! rendering (with rotation), and prefetching the next scale.
//!
//! Part of the `session` module — split out of the single file the design
//! review flagged (Phase 4, file splits). Methods keep living on `Session`.
use super::*;

impl Session {
    pub fn page_count(&self) -> Result<usize> {
        registry::with_session(self.handle, |s| Ok(s.document.page_count()))
    }

    /// The page's size in points — the height this page's [`PageSpace`] must be
    /// built from.
    ///
    /// [`PageSpace`]: crate::page_space::PageSpace
    pub fn page_size(&self, index: usize) -> Result<pdf_core::PageSize> {
        registry::with_session(self.handle, |s| s.document.page_size(index))
    }

    /// The pixel dimensions a render at `scale` will produce, without rendering.
    pub fn page_pixel_size(&self, index: usize, scale: f32) -> Result<(u32, u32)> {
        self.page_pixel_size_rotated(index, scale, Rotation::None)
    }

    /// As [`Self::page_pixel_size`], for a rotated view. A quarter turn swaps
    /// the axes, so a caller sizing a texture must ask through here rather than
    /// using the raw page size.
    pub fn page_pixel_size_rotated(
        &self,
        index: usize,
        scale: f32,
        rotation: Rotation,
    ) -> Result<(u32, u32)> {
        let request = RenderRequest { scale, rotation, ..Default::default() };
        registry::with_session(self.handle, |s| engine::page_pixel_size(s, index, &request))
    }

    /// Rasterise a page at `scale`, where 1.0 is 72 dpi.
    pub fn render_page(&self, index: usize, scale: f32) -> Result<PageRaster> {
        self.render_page_rotated(index, scale, Rotation::None)
    }

    /// Rasterise a page at `scale`, turned a quarter at a time.
    ///
    /// This is a *view* rotation: it changes what is drawn, not what is stored.
    /// Rotating the page in the document is an edit, goes through the command
    /// stack, and belongs to the Organize tab in phase 10.
    pub fn render_page_rotated(
        &self,
        index: usize,
        scale: f32,
        rotation: Rotation,
    ) -> Result<PageRaster> {
        let request = RenderRequest { scale, rotation, ..Default::default() };

        registry::with_session(self.handle, |s| {
            let (width, height) = engine::page_pixel_size(s, index, &request)?;

            let mut pixels = vec![0u8; width as usize * height as usize * 4];
            let outcome = {
                let mut target = RenderTarget::new(
                    width,
                    height,
                    width as usize * 4,
                    PixelOrder::Rgba,
                    &mut pixels,
                )?;
                engine::render_page_into(s, index, &request, &mut target)?
            };

            Ok(PageRaster {
                width,
                height,
                pixels,
                from_cache: outcome == RenderOutcome::CacheHit,
            })
        })
    }

    /// Rasterise one crop of a page, at `scale` — the engine capability
    /// behind printing and export, reused here so a detail on a page too
    /// large to raster whole at the zoom asked for (an A1 drawing, say) can
    /// still be shown sharp: the crop, not the whole sheet, is what the
    /// render ceiling has to cover. `crop` is in page points, top-left
    /// origin, y increasing downwards — the same space [`Self::page_crop`]
    /// and every `Annotation` already use.
    ///
    /// **Not cached** — same as the engine's own `render_region`, and for
    /// the same reason: this is for the one crop currently on screen, which
    /// the caller (`PagifyApp::detail_texture_for`) caches itself, keyed by
    /// the crop and the quantised zoom, exactly as the whole-page path does.
    pub fn render_page_region(&self, index: usize, crop: pdf_core::document::Rect, scale: f32) -> Result<PageRaster> {
        let request = pdf_core::document::RegionRequest { crop, scale, ..Default::default() };
        registry::with_session(self.handle, |s| {
            let bitmap = engine::render_region(s.document.as_ref(), index, &request)?;
            Ok(PageRaster {
                width: bitmap.width,
                height: bitmap.height,
                pixels: bitmap.data,
                from_cache: false,
            })
        })
    }

    /// Rasterise a page into the cache with no on-screen destination, so a
    /// later scroll onto it resolves to a copy. Phase 2 wires this to the
    /// viewport's neighbours; it is here now because the engine already has it.
    ///
    /// Returns whether it actually rasterised — `false` means the cache already
    /// held this page at this zoom. Propagated rather than swallowed because it
    /// is how a prefetch strategy can tell useful work from wasted work.
    pub fn prefetch_page(&self, index: usize, scale: f32) -> Result<bool> {
        let request = RenderRequest { scale, ..Default::default() };
        registry::with_session(self.handle, |s| engine::prefetch_page(s, index, &request))
    }
}
