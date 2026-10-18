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

    /// Every path a page draws, with the geometry and paint it is drawn with
    /// — the HTML export's rules and boxes, laid out in page points with a
    /// top-left origin. See [`pdf_core::document::Document::page_paints`].
    pub fn page_paints(&self, page: usize) -> Result<Vec<pdf_core::document::PagePaint>> {
        registry::with_session(self.handle, |s| s.document.page_paints(page))
    }

    /// The bytes the PDF itself holds for one of a page's images, **when a
    /// browser can show them as they are** — a plain RGB or grey JPEG, which
    /// the export embeds whole instead of re-encoding. `None` means "read the
    /// pixels and encode them": see [`Self::export_image_png`] and
    /// [`pdf_core::document::Document::image_raw`].
    pub fn image_raw(&self, page: usize, object: usize) -> Result<Option<(Vec<u8>, &'static str)>> {
        registry::with_session(self.handle, |s| s.document.image_raw(page, object))
    }

    /// Every font a page draws with, program bytes and all, in
    /// [`pdf_core::document::RunStyle::font`]'s own numbering — the HTML
    /// export embeds the document's own programs as `@font-face`, which is
    /// what keeps a substituted face's different widths from overlapping the
    /// pieces of a kerned word. See
    /// [`pdf_core::document::Document::page_fonts`].
    pub fn page_fonts(&self, page: usize) -> Result<Vec<pdf_core::document::PageFont>> {
        registry::with_session(self.handle, |s| s.document.page_fonts(page))
    }

    /// Rasterise a page and encode it as a PNG — the engine's own encoder,
    /// behind the Conversion tab's `toimage`. Kept here so the app writes
    /// files and never touches pixel formats: a render is RGBA with a
    /// top-left origin, which is also what a PNG is.
    pub fn export_page_png(&self, index: usize, scale: f32) -> Result<Vec<u8>> {
        self.render_page(index, scale)?.to_png()
    }

    /// One embedded image's pixels re-encoded as a PNG — the Conversion
    /// tab's `exportimages`. `object` is the address
    /// [`Session::images_on`] reports for it.
    pub fn export_image_png(&self, page: usize, object: usize) -> Result<Vec<u8>> {
        let decoded = registry::with_session(self.handle, |s| s.document.image_data(page, object))?
            .ok_or_else(|| {
                pdf_core::PdfError::Unsupported(
                    "reading this image's pixels — its colour space or depth is one this \
                     build does not expand",
                )
            })?;
        let bitmap = pdf_core::render::Bitmap {
            width: decoded.width,
            height: decoded.height,
            stride: decoded.width as usize * pdf_core::render::BYTES_PER_PIXEL,
            order: pdf_core::render::PixelOrder::Rgba,
            data: decoded.rgba,
        };
        pdf_core::render::export::encode(&bitmap, pdf_core::render::ImageFormat::Png)
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
    /// A page's current rotation, in quarter-turns clockwise.
    ///
    /// Needed because `SetPageRotation` is absolute and rotating is relative: a
    /// document can arrive with pages already at different angles — a landscape
    /// drawing among portrait sheets — and setting them all to one value
    /// straightens some and turns others sideways.
    pub fn page_rotation(&self, index: usize) -> Result<u8> {
        registry::with_session(self.handle, |s| {
            s.document.as_document_mut().map_or(Ok(0), |d| d.page_rotation(index))
        })
    }
}
