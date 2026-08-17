//! Pixel buffers and the format conversion between PDFium and Android.
//!
//! ## The one thing to understand here
//!
//! PDFium can only write `BGRA` (there is no RGBA output format in its API).
//! Android's `Bitmap.Config.ARGB_8888` is, despite the name, **RGBA in memory
//! order** on every supported device. So a byte-for-byte handover shows every
//! document with red and blue transposed. Exactly one place fixes that:
//! [`swap_red_blue_in_place`], called after PDFium finishes writing.

use crate::error::{PdfError, Result};

/// Byte order of a 4-bytes-per-pixel buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelOrder {
    /// What PDFium writes.
    Bgra,
    /// What Android's ARGB_8888 expects in memory.
    Rgba,
}

pub const BYTES_PER_PIXEL: usize = 4;

/// Guard rail against a hostile or simply enormous document asking for a buffer
/// that would OOM the app. 16384 px is comfortably past any sane zoom of an A0
/// sheet, and the area cap keeps the worst case near 256 MB.
pub const MAX_DIMENSION_PX: u32 = 16_384;
pub const MAX_PIXELS: u64 = 64 * 1024 * 1024;

pub fn validate_dimensions(width: u32, height: u32) -> Result<()> {
    if width == 0 || height == 0 {
        return Err(PdfError::InvalidBitmap(format!(
            "zero-sized bitmap {width}x{height}"
        )));
    }
    if width > MAX_DIMENSION_PX || height > MAX_DIMENSION_PX {
        return Err(PdfError::RenderTooLarge { width, height });
    }
    if u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err(PdfError::RenderTooLarge { width, height });
    }
    Ok(())
}

/// Swap the R and B channels of a 4bpp buffer in place, leaving alpha untouched.
///
/// `chunks_exact_mut(4)` rather than indexing so LLVM can vectorise it; a trailing
/// partial pixel (which would mean a malformed stride) is left alone rather than
/// panicking.
pub fn swap_red_blue_in_place(buffer: &mut [u8]) {
    for pixel in buffer.chunks_exact_mut(BYTES_PER_PIXEL) {
        pixel.swap(0, 2);
    }
}

/// Same conversion, but respecting a stride that is wider than the visible row.
/// PDFium aligns rows to 4 bytes, and Android bitmaps can carry their own padding,
/// so the padding bytes must be skipped rather than treated as pixels.
pub fn swap_red_blue_rows(buffer: &mut [u8], width_px: u32, height_px: u32, stride: usize) {
    let visible = width_px as usize * BYTES_PER_PIXEL;
    for row in 0..height_px as usize {
        let start = row * stride;
        let end = start + visible;
        if end > buffer.len() {
            break;
        }
        swap_red_blue_in_place(&mut buffer[start..end]);
    }
}

/// An owned ARGB pixel buffer. Used for cached and off-screen renders; the on-screen
/// path renders directly into an Android bitmap and never allocates one of these.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bitmap {
    pub width: u32,
    pub height: u32,
    pub stride: usize,
    pub order: PixelOrder,
    pub data: Vec<u8>,
}

impl Bitmap {
    pub fn new(width: u32, height: u32, order: PixelOrder) -> Result<Self> {
        validate_dimensions(width, height)?;
        let stride = width as usize * BYTES_PER_PIXEL;
        // Checked above, so this cannot overflow on any target we build for.
        let len = stride * height as usize;
        Ok(Bitmap {
            width,
            height,
            stride,
            order,
            data: vec![0u8; len],
        })
    }

    pub fn byte_len(&self) -> usize {
        self.data.len()
    }

    /// Convert in place to the requested order. A no-op when already correct, so
    /// callers can invoke it unconditionally without tracking state.
    pub fn convert_to(&mut self, order: PixelOrder) {
        if self.order == order {
            return;
        }
        swap_red_blue_rows(&mut self.data, self.width, self.height, self.stride);
        self.order = order;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swapping_moves_red_and_blue_but_leaves_green_and_alpha() {
        // Two pixels: B=1,G=2,R=3,A=4 and B=10,G=20,R=30,A=40.
        let mut buf = vec![1, 2, 3, 4, 10, 20, 30, 40];
        swap_red_blue_in_place(&mut buf);
        assert_eq!(buf, vec![3, 2, 1, 4, 30, 20, 10, 40]);
    }

    #[test]
    fn swapping_twice_is_the_identity() {
        let original: Vec<u8> = (0u8..=63).collect();
        let mut buf = original.clone();
        swap_red_blue_in_place(&mut buf);
        assert_ne!(buf, original, "one swap must actually change the buffer");
        swap_red_blue_in_place(&mut buf);
        assert_eq!(buf, original);
    }

    #[test]
    fn a_trailing_partial_pixel_is_left_alone_rather_than_panicking() {
        let mut buf = vec![1, 2, 3, 4, 9, 9];
        swap_red_blue_in_place(&mut buf);
        assert_eq!(buf, vec![3, 2, 1, 4, 9, 9]);
    }

    #[test]
    fn stride_padding_is_skipped_not_swapped() {
        // 1px wide, 2 rows, stride 8 => 4 padding bytes per row.
        let mut buf = vec![
            1, 2, 3, 4, 0xAA, 0xBB, 0xCC, 0xDD, // row 0 + padding
            5, 6, 7, 8, 0xAA, 0xBB, 0xCC, 0xDD, // row 1 + padding
        ];
        swap_red_blue_rows(&mut buf, 1, 2, 8);
        assert_eq!(
            buf,
            vec![
                3, 2, 1, 4, 0xAA, 0xBB, 0xCC, 0xDD,
                7, 6, 5, 8, 0xAA, 0xBB, 0xCC, 0xDD,
            ],
            "padding bytes must survive untouched"
        );
    }

    #[test]
    fn stride_swap_stops_at_the_buffer_end_instead_of_reading_past_it() {
        // Claims 3 rows but only holds 2 rows' worth of bytes.
        let mut buf = vec![1, 2, 3, 4, 5, 6, 7, 8];
        swap_red_blue_rows(&mut buf, 1, 3, 4);
        assert_eq!(buf, vec![3, 2, 1, 4, 7, 6, 5, 8]);
    }

    #[test]
    fn zero_sized_renders_are_rejected() {
        assert!(matches!(
            validate_dimensions(0, 100),
            Err(PdfError::InvalidBitmap(_))
        ));
        assert!(matches!(
            validate_dimensions(100, 0),
            Err(PdfError::InvalidBitmap(_))
        ));
    }

    #[test]
    fn oversized_renders_are_rejected_by_dimension_and_by_area() {
        assert!(matches!(
            validate_dimensions(MAX_DIMENSION_PX + 1, 10),
            Err(PdfError::RenderTooLarge { .. })
        ));
        // Each side is legal on its own, but the area is not.
        assert!(matches!(
            validate_dimensions(16_000, 16_000),
            Err(PdfError::RenderTooLarge { .. })
        ));
        assert!(validate_dimensions(4_000, 4_000).is_ok());
    }

    #[test]
    fn convert_to_is_idempotent_and_flips_the_recorded_order() {
        let mut bmp = Bitmap::new(2, 2, PixelOrder::Bgra).unwrap();
        bmp.data.copy_from_slice(&[
            1, 2, 3, 4, 5, 6, 7, 8, //
            9, 10, 11, 12, 13, 14, 15, 16,
        ]);

        bmp.convert_to(PixelOrder::Rgba);
        assert_eq!(bmp.order, PixelOrder::Rgba);
        assert_eq!(bmp.data[0], 3);

        let snapshot = bmp.data.clone();
        bmp.convert_to(PixelOrder::Rgba); // already converted — must do nothing
        assert_eq!(bmp.data, snapshot);
    }

    #[test]
    fn new_bitmap_is_zeroed_and_correctly_sized() {
        let bmp = Bitmap::new(3, 5, PixelOrder::Rgba).unwrap();
        assert_eq!(bmp.stride, 12);
        assert_eq!(bmp.byte_len(), 60);
        assert!(bmp.data.iter().all(|&b| b == 0));
    }
}
