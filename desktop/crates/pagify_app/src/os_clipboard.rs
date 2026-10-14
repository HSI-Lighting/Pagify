//! **What the system clipboard holds that Pagify did not put there**: words
//! copied in another program, a picture copied from a browser or the Snipping
//! Tool, or an image file copied in Explorer.
//!
//! egui-winit only ever reads *text* from the system clipboard, and on Ctrl+V it
//! swallows the key press (no `Key::V` press, no `Event::Paste`) when there is
//! none, so a picture is invisible to the app unless it is read here.
//!
//! Pagify's own copies are told apart by [`crate::COPIED_IN_PAGIFY`]: every copy
//! the app makes leaves that placeholder on the system clipboard, so anything
//! else there was copied since, somewhere else, and wins.

use std::path::Path;

use crate::{ObjectClipboard, COPIED_IN_PAGIFY};

/// A picture from the clipboard: straight RGBA, `width * height * 4` bytes.
pub(crate) struct Picture {
    pub(crate) rgba: Vec<u8>,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

/// Something copied outside Pagify.
pub(crate) enum Outside {
    Text(String),
    Picture(Picture),
}

/// How long after an `Event::Paste` the release of V is still that paste's.
const PASTE_RELEASE_WITHIN: f64 = 2.0;

/// Tells a Ctrl+V from typing "v", and a Ctrl+V that carried text from one that
/// did not — the only trace a picture-only Ctrl+V leaves is the release of V.
///
/// egui-winit swallows the *press* of Ctrl+V (it sends `Event::Paste` instead,
/// and only when there is text), but a plain "v" is sent as a key press. So a
/// release of V with no press of V before it is a paste command, whichever of
/// Ctrl and V was let go first — and when an `Event::Paste` came with it, the
/// text has already been pasted and there is nothing more to do.
#[derive(Default)]
pub(crate) struct PasteKey {
    /// A press of V was seen and its release has not been: it is typing.
    typed_v_down: bool,
    /// When the last `Event::Paste` arrived (egui time).
    paste_event_at: Option<f64>,
}

impl PasteKey {
    /// One frame's `events`, in order, at egui time `now`. `true` when a Ctrl+V
    /// has just finished that pasted no text — so a picture should be looked for.
    pub(crate) fn frame(&mut self, events: &[egui::Event], now: f64) -> bool {
        let mut picture = false;
        for event in events {
            match event {
                egui::Event::Paste(_) => self.paste_event_at = Some(now),
                egui::Event::Key { key: egui::Key::V, pressed: true, .. } => self.typed_v_down = true,
                egui::Event::Key { key: egui::Key::V, pressed: false, .. } => {
                    let typed = std::mem::take(&mut self.typed_v_down);
                    let text = self.paste_event_at.take().is_some_and(|at| now - at < PASTE_RELEASE_WITHIN);
                    picture |= !typed && !text;
                }
                _ => {}
            }
        }
        picture
    }
}

/// Whether `text` is words copied elsewhere — not our own placeholder, not blank.
pub(crate) fn is_text(text: &str) -> bool {
    text != COPIED_IN_PAGIFY && !text.trim().is_empty()
}

/// Words as something to paste: 12 pt black, one entry per line, in a face the
/// page already has (`face: None`) — the same as text pasted in from elsewhere
/// has always been.
pub(crate) fn text_content(text: &str) -> ObjectClipboard {
    ObjectClipboard::Text {
        lines: text.lines().map(str::to_string).collect(),
        size: 12.0,
        color: pdf_core::document::Color { r: 0, g: 0, b: 0, a: 255 },
        face: None,
        track: 1.0,
    }
}

/// The size, in page points, a picture of `width` × `height` pixels is pasted
/// at on a page of `page` points: 96 dpi (0.75 pt a pixel), but never more than
/// 60% of the page either way — a screenshot is a few thousand pixels wide and
/// the page is 612 points. The aspect ratio is kept.
pub(crate) fn fit(width: u32, height: u32, page: (f32, f32)) -> (f32, f32) {
    let (w, h) = (width as f32 * 0.75, height as f32 * 0.75);
    let k = (page.0 * 0.6 / w).min(page.1 * 0.6 / h).min(1.0);
    (w * k, h * k)
}

/// A picture as something to paste, sized for a page of `page` points. It is
/// the same `Image` an internal copy of a placed picture makes, so placing it,
/// drawing its ghost and undoing it are the paths that already exist.
pub(crate) fn picture_content(picture: Picture, page: (f32, f32)) -> ObjectClipboard {
    let (w, h) = fit(picture.width, picture.height, page);
    ObjectClipboard::Image {
        rgba: picture.rgba,
        width: picture.width,
        height: picture.height,
        rect: pdf_core::document::Rect { left: 0.0, top: 0.0, right: w, bottom: h },
    }
}

/// A PNG or JPEG file, decoded. The only two decoders this build carries.
fn picture_from_file(path: &Path) -> Option<Picture> {
    let image = image::open(path).ok()?.into_rgba8();
    let (width, height) = image.dimensions();
    (width > 0 && height > 0).then(|| Picture { rgba: image.into_raw(), width, height })
}

/// What the system clipboard holds from outside Pagify, if anything: words
/// first (as egui-winit does), else a picture, else the first PNG/JPEG among
/// the files copied in Explorer.
pub(crate) fn read() -> Option<Outside> {
    let mut clipboard = arboard::Clipboard::new().ok()?;
    if let Ok(text) = clipboard.get_text() {
        let text = text.replace("\r\n", "\n");
        if is_text(&text) {
            return Some(Outside::Text(text));
        }
    }
    if let Ok(image) = clipboard.get_image() {
        let (width, height) = (image.width as u32, image.height as u32);
        if width > 0 && height > 0 {
            return Some(Outside::Picture(Picture { rgba: image.bytes.into_owned(), width, height }));
        }
    }
    let files = clipboard.get().file_list().ok()?;
    files.iter().find_map(|path| picture_from_file(path)).map(Outside::Picture)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LETTER: (f32, f32) = (612.0, 792.0);

    fn v(pressed: bool) -> egui::Event {
        egui::Event::Key { key: egui::Key::V, physical_key: None, pressed, repeat: false, modifiers: egui::Modifiers::NONE }
    }

    #[test]
    fn a_release_of_v_with_no_press_is_a_paste_command_that_carried_no_text() {
        let mut key = PasteKey::default();
        assert!(key.frame(&[v(false)], 1.0));
    }

    #[test]
    fn typing_v_is_not_a_paste_whether_or_not_the_frames_split_it() {
        let mut key = PasteKey::default();
        assert!(!key.frame(&[v(true), v(false)], 1.0), "press and release in one frame");
        assert!(!key.frame(&[v(true)], 2.0) && !key.frame(&[v(false)], 2.1), "press and release in two");
        assert!(key.frame(&[v(false)], 3.0), "the typed V must not leave the next paste swallowed");
    }

    #[test]
    fn a_paste_that_carried_text_is_not_pasted_again_by_its_release() {
        let mut key = PasteKey::default();
        let paste = || egui::Event::Paste("words".into());
        assert!(!key.frame(&[paste()], 1.0) && !key.frame(&[v(false)], 1.2), "release in a later frame");
        assert!(!key.frame(&[paste(), v(false)], 2.0), "press and release in one frame");
        assert!(key.frame(&[v(false)], 3.0), "and the next, text-less paste is found again");
    }

    #[test]
    fn an_old_paste_event_does_not_swallow_a_later_picture_paste() {
        let mut key = PasteKey::default();
        key.frame(&[egui::Event::Paste("from Shift+Insert".into())], 1.0);
        assert!(key.frame(&[v(false)], 60.0));
    }

    #[test]
    fn our_own_placeholder_and_blank_text_are_not_from_outside() {
        assert!(!is_text(COPIED_IN_PAGIFY));
        assert!(!is_text(""));
        assert!(!is_text("  \n\t"));
        assert!(is_text("some words"));
    }

    #[test]
    fn text_becomes_one_line_per_line() {
        let ObjectClipboard::Text { lines, .. } = text_content("one\ntwo\n\nfour") else { panic!("not text") };
        assert_eq!(lines, ["one", "two", "", "four"]);
    }

    #[test]
    fn a_small_picture_is_96_dpi() {
        let (w, h) = fit(100, 40, LETTER);
        assert!((w - 75.0).abs() < 1e-3 && (h - 30.0).abs() < 1e-3, "{w} x {h}");
    }

    #[test]
    fn a_screenshot_is_fitted_into_the_page_with_its_shape_kept() {
        let (w, h) = fit(3840, 2160, LETTER);
        assert!((w - 612.0 * 0.6).abs() < 1e-2, "{w}");
        assert!(((w / h) - 3840.0 / 2160.0).abs() < 1e-3, "aspect changed: {w} x {h}");
    }

    #[test]
    fn a_tall_picture_is_held_to_the_page_height_too() {
        let (w, h) = fit(400, 6000, LETTER);
        assert!((h - 792.0 * 0.6).abs() < 1e-2, "{h}");
        assert!(w < 612.0 * 0.6);
    }

    /// **Overwrites the real system clipboard**, so it runs only by hand:
    /// `cargo test -p pagify_app reads_the_real_clipboard -- --ignored`. It is
    /// the one check of the arboard calls themselves; every other test stubs
    /// them. Puts back the text that was there, if any — even when an assertion
    /// fails, which is when it matters.
    #[test]
    #[ignore = "overwrites the real system clipboard"]
    fn reads_the_real_clipboard() {
        struct Restore(Option<String>);
        impl Drop for Restore {
            fn drop(&mut self) {
                if let Ok(mut clipboard) = arboard::Clipboard::new() {
                    let _ = match self.0.take() {
                        Some(text) => clipboard.set_text(text),
                        None => clipboard.clear(),
                    };
                }
            }
        }
        let mut clipboard = arboard::Clipboard::new().expect("no clipboard");
        let _restore = Restore(clipboard.get_text().ok());

        let pixels = vec![255, 0, 0, 255, 0, 255, 0, 255];
        clipboard.set_image(arboard::ImageData { width: 2, height: 1, bytes: pixels.clone().into() }).unwrap();
        let Some(Outside::Picture(picture)) = read() else { panic!("a picture on the clipboard was not read") };
        assert_eq!((picture.width, picture.height, picture.rgba), (2, 1, pixels));

        clipboard.set_text(COPIED_IN_PAGIFY).unwrap();
        assert!(read().is_none(), "our own placeholder was read as something from outside");
        clipboard.set_text("words from outside").unwrap();
        assert!(matches!(read(), Some(Outside::Text(t)) if t == "words from outside"));

        let dir = std::env::temp_dir().join(format!("pagify-os-clipboard-real-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let png = dir.join("pixel.png");
        image::RgbaImage::from_pixel(3, 2, image::Rgba([1, 2, 3, 255])).save(&png).unwrap();
        // Cleared first: setting files leaves the other formats, and the words
        // above would be read in preference to the file.
        clipboard.clear().unwrap();
        clipboard.set().file_list(&[&png]).unwrap();
        let outcome = read();
        let _ = std::fs::remove_dir_all(&dir);
        let Some(Outside::Picture(file)) = outcome else { panic!("a copied image file was not read") };
        assert_eq!((file.width, file.height), (3, 2));
    }

    #[test]
    fn a_png_file_is_read_and_a_text_file_is_not() {
        let dir = std::env::temp_dir().join(format!("pagify-os-clipboard-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let png = dir.join("pixel.png");
        image::RgbaImage::from_pixel(3, 2, image::Rgba([10, 20, 30, 255])).save(&png).unwrap();
        let text = dir.join("words.txt");
        std::fs::write(&text, "not a picture").unwrap();

        let picture = picture_from_file(&png).expect("a PNG should be read");
        assert_eq!((picture.width, picture.height, picture.rgba.len()), (3, 2, 3 * 2 * 4));
        assert_eq!(&picture.rgba[..4], &[10, 20, 30, 255]);
        assert!(picture_from_file(&text).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
