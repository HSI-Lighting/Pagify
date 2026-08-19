//! Encoding a rendered region into a file someone can keep.
//!
//! In the core rather than in Kotlin so the whole capture path is
//! platform-neutral — iOS and desktop inherit it — and so the export can be
//! tested on the host down to the bytes, which a call into `Bitmap.compress`
//! could not be.

use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::{ColorType, ImageEncoder};

use crate::error::{PdfError, Result};
use crate::render::bitmap::{Bitmap, PixelOrder, BYTES_PER_PIXEL};

/// What to encode a capture as.
///
/// PNG is the default because a page is line art and type, where PNG is both
/// lossless and small. JPEG exists for the other case: a scanned page is a
/// photograph, and a lossless encode of a photograph is enormous — the 2.9 GB
/// catalogue is exactly that, and is why the choice is offered rather than
/// decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    /// Quality is clamped to 1..=100.
    Jpeg { quality: u8 },
}

impl ImageFormat {
    /// The extension to save under, so callers do not each invent one.
    pub fn extension(self) -> &'static str {
        match self {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg { .. } => "jpg",
        }
    }

    pub fn mime_type(self) -> &'static str {
        match self {
            ImageFormat::Png => "image/png",
            ImageFormat::Jpeg { .. } => "image/jpeg",
        }
    }

    /// Parse the name the app sends across the wire.
    pub fn parse(name: &str, quality: u8) -> Result<Self> {
        match name.to_ascii_lowercase().as_str() {
            "png" => Ok(ImageFormat::Png),
            "jpeg" | "jpg" => Ok(ImageFormat::Jpeg { quality }),
            other => Err(PdfError::InvalidArgument(format!(
                "unknown export format {other:?}; expected png or jpeg"
            ))),
        }
    }
}

/// Encode a rendered region.
///
/// Both formats are written as **opaque RGB**. A capture is a picture of a page,
/// and a page is opaque — it was rendered onto white. Carrying an alpha channel
/// through would only invite a viewer somewhere to composite the capture over
/// something dark and show the user a page they do not recognise.
pub fn encode(bitmap: &Bitmap, format: ImageFormat) -> Result<Vec<u8>> {
    let rgb = to_rgb(bitmap);
    let mut out = Vec::new();

    match format {
        ImageFormat::Png => PngEncoder::new_with_quality(
            &mut out,
            // The capture is written once and looked at many times, usually after
            // being shared, so paying for the smaller file is the right trade.
            CompressionType::Best,
            FilterType::Adaptive,
        )
        .write_image(&rgb, bitmap.width, bitmap.height, ColorType::Rgb8.into())
        .map_err(|e| PdfError::InvalidBitmap(format!("png encode failed: {e}")))?,

        ImageFormat::Jpeg { quality } => {
            JpegEncoder::new_with_quality(&mut out, quality.clamp(1, 100))
                .write_image(&rgb, bitmap.width, bitmap.height, ColorType::Rgb8.into())
                .map_err(|e| PdfError::InvalidBitmap(format!("jpeg encode failed: {e}")))?
        }
    }

    Ok(out)
}

/// Drop the alpha channel, and the row padding with it.
///
/// Both are the encoder's problem rather than the caller's: a `Bitmap` may carry a
/// stride wider than its visible row, and passing those padding bytes to an
/// encoder would shear the image diagonally.
fn to_rgb(bitmap: &Bitmap) -> Vec<u8> {
    let width = bitmap.width as usize;
    let mut rgb = Vec::with_capacity(width * bitmap.height as usize * 3);

    for row in 0..bitmap.height as usize {
        let start = row * bitmap.stride;
        let visible = &bitmap.data[start..start + width * BYTES_PER_PIXEL];
        for pixel in visible.chunks_exact(BYTES_PER_PIXEL) {
            match bitmap.order {
                PixelOrder::Rgba => rgb.extend_from_slice(&pixel[..3]),
                PixelOrder::Bgra => rgb.extend_from_slice(&[pixel[2], pixel[1], pixel[0]]),
            }
        }
    }

    rgb
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two pixels wide, two tall: red, green / blue, white.
    fn swatch(order: PixelOrder) -> Bitmap {
        let mut bitmap = Bitmap::new(2, 2, order).unwrap();
        let pixels: [[u8; 3]; 4] = [
            [255, 0, 0],
            [0, 255, 0],
            [0, 0, 255],
            [255, 255, 255],
        ];
        for (i, rgb) in pixels.iter().enumerate() {
            let at = i * BYTES_PER_PIXEL;
            let stored = match order {
                PixelOrder::Rgba => [rgb[0], rgb[1], rgb[2]],
                PixelOrder::Bgra => [rgb[2], rgb[1], rgb[0]],
            };
            bitmap.data[at..at + 3].copy_from_slice(&stored);
            bitmap.data[at + 3] = 255;
        }
        bitmap
    }

    fn decode(bytes: &[u8]) -> Vec<[u8; 3]> {
        let decoded = image::load_from_memory(bytes).expect("decode").to_rgb8();
        decoded.pixels().map(|p| p.0).collect()
    }

    #[test]
    fn a_png_round_trips_every_pixel_exactly() {
        let encoded = encode(&swatch(PixelOrder::Rgba), ImageFormat::Png).unwrap();
        assert_eq!(&encoded[..8], b"\x89PNG\r\n\x1a\n", "not a PNG");
        assert_eq!(
            decode(&encoded),
            vec![[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 255]],
        );
    }

    #[test]
    fn a_bgra_bitmap_encodes_to_the_same_colours_as_an_rgba_one() {
        // The trap this guards is silent: swapped channels are still a valid PNG,
        // and a red highlight simply comes out blue.
        let from_rgba = encode(&swatch(PixelOrder::Rgba), ImageFormat::Png).unwrap();
        let from_bgra = encode(&swatch(PixelOrder::Bgra), ImageFormat::Png).unwrap();
        assert_eq!(decode(&from_rgba), decode(&from_bgra));
    }

    #[test]
    fn row_padding_is_skipped_rather_than_encoded_as_pixels() {
        // One pixel wide, two rows, with four bytes of padding per row. Encoded
        // naively the padding becomes pixels and the image shears.
        let mut bitmap = Bitmap::new(1, 2, PixelOrder::Rgba).unwrap();
        bitmap.stride = 8;
        bitmap.data = vec![
            255, 0, 0, 255, 9, 9, 9, 9, //
            0, 0, 255, 255, 9, 9, 9, 9,
        ];

        let encoded = encode(&bitmap, ImageFormat::Png).unwrap();
        assert_eq!(decode(&encoded), vec![[255, 0, 0], [0, 0, 255]]);
    }

    #[test]
    fn a_jpeg_is_a_jpeg_and_is_close_enough_to_the_original() {
        let encoded = encode(&swatch(PixelOrder::Rgba), ImageFormat::Jpeg { quality: 90 }).unwrap();
        assert_eq!(&encoded[..2], b"\xff\xd8", "not a JPEG");

        // Lossy, so exactness is the wrong assertion; that each pixel is still
        // recognisably its own colour is the right one.
        let decoded = decode(&encoded);
        assert_eq!(decoded.len(), 4);
        assert!(decoded[0][0] > 200 && decoded[0][1] < 80, "red became {:?}", decoded[0]);
        assert!(decoded[3].iter().all(|&c| c > 200), "white became {:?}", decoded[3]);
    }

    #[test]
    fn encoding_is_deterministic() {
        // A capture that encodes differently run to run cannot be golden-tested,
        // and would make any later comparison of two exports meaningless.
        let once = encode(&swatch(PixelOrder::Rgba), ImageFormat::Png).unwrap();
        let twice = encode(&swatch(PixelOrder::Rgba), ImageFormat::Png).unwrap();
        assert_eq!(once, twice);
    }

    #[test]
    fn an_absurd_jpeg_quality_is_clamped_rather_than_rejected() {
        for quality in [0, 200] {
            assert!(encode(&swatch(PixelOrder::Rgba), ImageFormat::Jpeg { quality }).is_ok());
        }
    }

    #[test]
    fn format_names_are_parsed_case_insensitively_and_unknown_ones_rejected() {
        assert_eq!(ImageFormat::parse("PNG", 90).unwrap(), ImageFormat::Png);
        assert_eq!(
            ImageFormat::parse("jpg", 80).unwrap(),
            ImageFormat::Jpeg { quality: 80 }
        );
        assert!(matches!(
            ImageFormat::parse("tiff", 90),
            Err(PdfError::InvalidArgument(_))
        ));
    }
}
