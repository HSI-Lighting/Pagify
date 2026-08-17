//! Rasterisation: where a page gets drawn to, and how cached rasters get there.

pub mod bitmap;
pub mod cache;

pub use bitmap::{Bitmap, PixelOrder, BYTES_PER_PIXEL};
pub use cache::{CacheKey, CacheStats, PageCache};

use crate::error::{PdfError, Result};

/// Default cache budget. Sized so a mid-range phone holds roughly three or four
/// full-screen pages — enough for one page either side of the current one, which
/// is the whole point of prefetching.
pub const DEFAULT_CACHE_BUDGET_BYTES: usize = 48 * 1024 * 1024;

/// A borrowed destination for a render.
///
/// The buffer is borrowed rather than owned so the on-screen path can point
/// straight at an Android bitmap's locked pixels: PDFium writes into the Java
/// heap object with no intermediate allocation and no copy.
pub struct RenderTarget<'a> {
    pub width: u32,
    pub height: u32,
    /// Bytes per row, which may exceed `width * 4` when the destination pads rows.
    pub stride: usize,
    /// The byte order the *consumer* expects, not what PDFium produces.
    pub order: PixelOrder,
    pub pixels: &'a mut [u8],
}

impl<'a> RenderTarget<'a> {
    pub fn new(
        width: u32,
        height: u32,
        stride: usize,
        order: PixelOrder,
        pixels: &'a mut [u8],
    ) -> Result<Self> {
        bitmap::validate_dimensions(width, height)?;

        let minimum_stride = width as usize * BYTES_PER_PIXEL;
        if stride < minimum_stride {
            return Err(PdfError::InvalidBitmap(format!(
                "stride {stride} is too small for a {width}px-wide row (needs {minimum_stride})"
            )));
        }

        // Only the last row's *visible* bytes must be present; trailing padding on
        // the final row is not required to exist.
        let required = stride * (height as usize - 1) + minimum_stride;
        if pixels.len() < required {
            return Err(PdfError::InvalidBitmap(format!(
                "buffer holds {} bytes but a {}x{} render at stride {} needs {}",
                pixels.len(),
                width,
                height,
                stride,
                required
            )));
        }

        Ok(RenderTarget {
            width,
            height,
            stride,
            order,
            pixels,
        })
    }

    /// Borrow the whole of an owned [`Bitmap`] as a render destination.
    pub fn from_bitmap(bmp: &'a mut Bitmap) -> Result<Self> {
        let (width, height, stride, order) = (bmp.width, bmp.height, bmp.stride, bmp.order);
        RenderTarget::new(width, height, stride, order, &mut bmp.data)
    }

    pub fn is_tightly_packed(&self) -> bool {
        self.stride == self.width as usize * BYTES_PER_PIXEL
    }

    /// Reinterpret whatever PDFium just wrote (always BGRA) as the order this
    /// target's consumer expects. Safe to call unconditionally.
    pub fn normalise_from_pdfium(&mut self) {
        if self.order == PixelOrder::Bgra {
            return;
        }
        bitmap::swap_red_blue_rows(self.pixels, self.width, self.height, self.stride);
    }

    /// Blit a cached raster into this target. This is the prefetch payoff path:
    /// a memcpy instead of a re-render.
    pub fn copy_from(&mut self, source: &Bitmap) -> Result<()> {
        if source.width != self.width || source.height != self.height {
            return Err(PdfError::InvalidBitmap(format!(
                "cached page is {}x{} but the target is {}x{}",
                source.width, source.height, self.width, self.height
            )));
        }

        let visible = self.width as usize * BYTES_PER_PIXEL;
        for row in 0..self.height as usize {
            let dst = row * self.stride;
            let src = row * source.stride;
            self.pixels[dst..dst + visible].copy_from_slice(&source.data[src..src + visible]);
        }

        if source.order != self.order {
            bitmap::swap_red_blue_rows(self.pixels, self.width, self.height, self.stride);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_buffer_that_is_too_small_is_rejected() {
        let mut buf = vec![0u8; 4 * 4 * 4 - 1];
        let err = RenderTarget::new(4, 4, 16, PixelOrder::Rgba, &mut buf);
        assert!(matches!(err, Err(PdfError::InvalidBitmap(_))));
    }

    #[test]
    fn a_stride_narrower_than_the_row_is_rejected() {
        let mut buf = vec![0u8; 256];
        let err = RenderTarget::new(4, 4, 8, PixelOrder::Rgba, &mut buf);
        assert!(matches!(err, Err(PdfError::InvalidBitmap(_))));
    }

    #[test]
    fn the_final_rows_trailing_padding_need_not_be_allocated() {
        // 2x2 at stride 12: 12 + 8 = 20 bytes is enough, even though 2*12 = 24.
        let mut buf = vec![0u8; 20];
        assert!(RenderTarget::new(2, 2, 12, PixelOrder::Rgba, &mut buf).is_ok());

        let mut short = vec![0u8; 19];
        assert!(RenderTarget::new(2, 2, 12, PixelOrder::Rgba, &mut short).is_err());
    }

    #[test]
    fn tight_packing_is_detected() {
        let mut buf = vec![0u8; 64];
        let tight = RenderTarget::new(4, 4, 16, PixelOrder::Rgba, &mut buf).unwrap();
        assert!(tight.is_tightly_packed());

        let mut padded_buf = vec![0u8; 128];
        let padded = RenderTarget::new(4, 4, 24, PixelOrder::Rgba, &mut padded_buf).unwrap();
        assert!(!padded.is_tightly_packed());
    }

    #[test]
    fn normalising_a_bgra_target_is_a_no_op() {
        let mut buf: Vec<u8> = (0u8..16).collect();
        let original = buf.clone();
        let mut target = RenderTarget::new(2, 2, 8, PixelOrder::Bgra, &mut buf).unwrap();
        target.normalise_from_pdfium();
        assert_eq!(buf, original);
    }

    #[test]
    fn normalising_an_rgba_target_swaps_red_and_blue() {
        let mut buf = vec![1, 2, 3, 4, 5, 6, 7, 8];
        let mut target = RenderTarget::new(2, 1, 8, PixelOrder::Rgba, &mut buf).unwrap();
        target.normalise_from_pdfium();
        assert_eq!(buf, vec![3, 2, 1, 4, 7, 6, 5, 8]);
    }

    #[test]
    fn copying_a_cached_page_converts_order_and_respects_stride() {
        let mut source = Bitmap::new(2, 2, PixelOrder::Bgra).unwrap();
        source.data.copy_from_slice(&[
            1, 2, 3, 4, 5, 6, 7, 8, //
            9, 10, 11, 12, 13, 14, 15, 16,
        ]);

        // Destination is padded: stride 12 for an 8-byte row.
        let mut buf = vec![0xFFu8; 24];
        let mut target = RenderTarget::new(2, 2, 12, PixelOrder::Rgba, &mut buf).unwrap();
        target.copy_from(&source).unwrap();

        assert_eq!(&buf[0..8], &[3, 2, 1, 4, 7, 6, 5, 8]);
        assert_eq!(&buf[8..12], &[0xFF, 0xFF, 0xFF, 0xFF], "padding untouched");
        assert_eq!(&buf[12..20], &[11, 10, 9, 12, 15, 14, 13, 16]);
    }

    #[test]
    fn copying_a_mismatched_size_is_an_error_not_a_partial_blit() {
        let source = Bitmap::new(3, 3, PixelOrder::Rgba).unwrap();
        let mut buf = vec![0u8; 64];
        let mut target = RenderTarget::new(4, 4, 16, PixelOrder::Rgba, &mut buf).unwrap();
        assert!(matches!(
            target.copy_from(&source),
            Err(PdfError::InvalidBitmap(_))
        ));
    }

    #[test]
    fn from_bitmap_borrows_the_whole_buffer() {
        let mut bmp = Bitmap::new(5, 3, PixelOrder::Rgba).unwrap();
        let target = RenderTarget::from_bitmap(&mut bmp).unwrap();
        assert_eq!((target.width, target.height), (5, 3));
        assert!(target.is_tightly_packed());
    }
}
