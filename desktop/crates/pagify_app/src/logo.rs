//! The application mark.
//!
//! Embedded rather than loaded from disk. A bundle should be self-contained:
//! a logo read from a path beside the executable is a logo that goes missing
//! the first time someone copies the binary somewhere, and on Windows it would
//! live in Program Files where the app cannot rely on reading anything.

use std::sync::Arc;

/// The mark, at 512px. Decoded once at startup.
const BYTES: &[u8] = include_bytes!("../../../assets/pagify-logo.png");

fn decode() -> Option<(Vec<u8>, u32, u32)> {
    let decoded = image::load_from_memory_with_format(BYTES, image::ImageFormat::Png).ok()?;
    let rgba = decoded.to_rgba8();
    let (width, height) = rgba.dimensions();
    Some((rgba.into_raw(), width, height))
}

/// The window and dock icon.
pub fn icon() -> Option<Arc<egui::IconData>> {
    let (rgba, width, height) = decode()?;
    Some(Arc::new(egui::IconData { rgba, width, height }))
}

/// The title-bar mark, as a texture.
///
/// Returns `None` if the image cannot be decoded, and the caller draws the
/// lettered tile instead — a missing logo should cost a nicer logo, not a
/// missing title bar.
pub fn texture(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    let (rgba, width, height) = decode()?;
    let image = egui::ColorImage::from_rgba_unmultiplied([width as usize, height as usize], &rgba);
    Some(ctx.load_texture("pagify-logo", image, egui::TextureOptions::LINEAR))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_logo_decodes() {
        // It is compiled into the binary, so a bad or truncated file is a build
        // that ships without a logo and says nothing about it.
        let (rgba, width, height) = decode().expect("the embedded logo did not decode");
        assert!(width >= 128 && height >= 128, "too small to look right on a retina display");
        assert_eq!(rgba.len(), width as usize * height as usize * 4);
    }

    #[test]
    fn it_is_square_enough_to_sit_in_a_tile() {
        let (_, width, height) = decode().expect("decodes");
        let ratio = width as f32 / height as f32;
        assert!((0.9..=1.1).contains(&ratio), "the mark is {ratio:.2}:1, which will squash");
    }
}
