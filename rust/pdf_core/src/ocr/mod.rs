//! Making a scan searchable.
//!
//! Not "OCR" as a menu item — **"Make searchable."** A page is recognised once,
//! and the words are written back as an *invisible* text layer positioned over
//! the image at the recognised word boxes. The page looks identical; it becomes
//! selectable and searchable; and every later read of it goes through the same
//! extraction path as any other native text. One path, not two.
//!
//! ## What is here and what is not
//!
//! Everything except the recogniser. Writing the layer, positioning it,
//! undoing it and reading it back are all here and tested. The recognition
//! itself is behind the [`Recogniser`] trait, because a model is *data* — one
//! per script family, fetched rather than committed, the same way PDFium is —
//! and shipping one is a product decision about model licensing and download
//! size, not an engine one.
//!
//! That split is deliberate rather than an omission. The positioning maths is
//! the part that is easy to get subtly wrong and hard to notice, and it can be
//! proved without any model at all.

use serde::{Deserialize, Serialize};

use crate::document::Rect;
use crate::error::Result;

#[cfg(feature = "ocr-engine")]
pub mod engine;
pub mod geometry;
pub mod pipeline;
pub mod preprocess;
pub mod script;
pub mod tiling;

pub use script::Script;

/// The id every word of a recognised layer is written under.
///
/// One id for the whole layer, not one per word: a text layer is a single thing
/// to a person, so removing it should be a single call, and undo then needs
/// nothing that does not already exist.
pub const TEXT_LAYER_ID: i32 = 0x004F_4352; // "OCR"

/// One word a recogniser found, and where it sits on the page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecognisedWord {
    pub text: String,
    /// Page points, top-left origin — the same convention every other
    /// coordinate in this crate uses.
    pub rect: Rect,
    /// 0–1. Reported rather than thresholded here: a caller deciding what to
    /// keep needs the number, and a page that says it could not be read
    /// reliably is more useful than one that quietly returns plausible nonsense.
    pub confidence: f32,

    /// One entry per character of `text`, where the recogniser supplies them.
    ///
    /// Carried from the first commit and **never discarded**, because a review
    /// pass cannot be retrofitted: once the per-character numbers are gone the
    /// only way back is to run recognition again over the whole document.
    ///
    /// It is the characters that matter on a specification sheet. `IP65` read
    /// as `IP66` has no linguistic context to fall back on and no word-level
    /// score will single it out — the word looks perfectly plausible either
    /// way. The one weak character is the whole signal.
    #[serde(default)]
    pub char_confidence: Vec<f32>,
}

impl RecognisedWord {
    /// The least confident character in this word, if the recogniser said.
    ///
    /// What a review pass sorts on. A word can score well overall and still
    /// contain the one character that turned a part number into a different
    /// part number.
    pub fn weakest_character(&self) -> Option<f32> {
        self.char_confidence
            .iter()
            .copied()
            .fold(None, |worst: Option<f32>, c| Some(worst.map_or(c, |w| w.min(c))))
    }

    /// The type size that fills this word's box.
    ///
    /// Height rather than width, because a text layer is judged by whether the
    /// selection highlight lines up with the ink underneath, and the line
    /// height is what governs that.
    pub fn font_size(&self) -> f32 {
        (self.rect.bottom - self.rect.top).abs().max(1.0)
    }

    /// Where the word goes: its text, its left edge, its baseline, and the size
    /// to set it at.
    ///
    /// **One run, not one object per letter**, and that is not a simplification
    /// — it is the only arrangement that reads back correctly.
    ///
    /// Placing each letter individually needs an advance per letter, and a
    /// recogniser reports a box per *word*. A constant advance across the box
    /// therefore leaves a gap after every narrow letter: measured on Helvetica
    /// at 16pt with an 8pt advance, `I` is 4.45pt wide and leaves 3.55pt of
    /// space behind it, which extraction reads as a word break. "LUMINAIRE"
    /// came back as "LUMI NAI RE" — a layer that survives the save, sits in the
    /// right place, and cannot be searched.
    ///
    /// Handing the whole word to PDFium as one run makes the font's own metrics
    /// do the spacing, and there are no gaps to misread.
    pub fn placement(&self) -> Option<Placement> {
        let text = self.text.trim();
        if text.is_empty() {
            return None;
        }

        // Sized to fill the box's height, then reduced if that would overflow
        // its width. Half an em is about Helvetica's average advance, so a word
        // of n letters is roughly n/2 ems wide.
        let by_height = self.font_size();
        let width = (self.rect.right - self.rect.left).abs();
        let by_width = width / (text.chars().count() as f32 * 0.5).max(1.0);

        Some(Placement {
            text: text.to_string(),
            left: self.rect.left,
            // The baseline, not the box top: placing from the top puts every
            // word a line height above the ink it is meant to cover, and the
            // selection highlight then sits over the line above.
            baseline: self.rect.bottom - by_height * 0.2,
            size: by_height.min(by_width).max(1.0),
        })
    }
}

/// A word, ready to be written.
#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    pub text: String,
    pub left: f32,
    pub baseline: f32,
    pub size: f32,
}

/// Eight-bit greyscale, tightly packed, no stride padding.
///
/// Defined once here rather than at each boundary: every recogniser wants
/// greyscale and none of them agree on how it should be laid out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GreyImage {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl GreyImage {
    /// A white page. White rather than black because an out-of-bounds read
    /// should look like paper, not like ink — a black margin would be detected
    /// as text.
    pub fn white(width: u32, height: u32) -> Self {
        GreyImage { width, height, data: vec![0xFF; width as usize * height as usize] }
    }

    pub fn get(&self, x: u32, y: u32) -> u8 {
        if x >= self.width || y >= self.height {
            return 0xFF;
        }
        self.data[y as usize * self.width as usize + x as usize]
    }

    pub fn set(&mut self, x: u32, y: u32, value: u8) {
        if x < self.width && y < self.height {
            let index = y as usize * self.width as usize + x as usize;
            self.data[index] = value;
        }
    }

    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }
}

/// A detected text line, in **image pixel** space.
///
/// A quadrilateral rather than a rectangle, and not for tidiness: detectors
/// return rotated boxes, and a photographed page is never square to the sensor.
/// Flattening to an upright rect at this boundary discards the skew before
/// anything downstream can correct for it. Corners clockwise from top-left.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LineBox {
    pub quad: [(f32, f32); 4],
    pub confidence: f32,
}

impl LineBox {
    /// An upright box, for a detector that only reports one.
    pub fn upright(left: f32, top: f32, right: f32, bottom: f32, confidence: f32) -> Self {
        LineBox {
            quad: [(left, top), (right, top), (right, bottom), (left, bottom)],
            confidence,
        }
    }

    /// The upright rectangle containing the quad.
    pub fn bounds(&self) -> (f32, f32, f32, f32) {
        let xs: Vec<f32> = self.quad.iter().map(|p| p.0).collect();
        let ys: Vec<f32> = self.quad.iter().map(|p| p.1).collect();
        (
            xs.iter().copied().fold(f32::MAX, f32::min),
            ys.iter().copied().fold(f32::MAX, f32::min),
            xs.iter().copied().fold(f32::MIN, f32::max),
            ys.iter().copied().fold(f32::MIN, f32::max),
        )
    }

    /// How far off the horizontal the line sits, in radians.
    pub fn skew(&self) -> f32 {
        let (a, b) = (self.quad[0], self.quad[1]);
        (b.1 - a.1).atan2(b.0 - a.0)
    }

    pub fn height(&self) -> f32 {
        let (_, top, _, bottom) = self.bounds();
        bottom - top
    }

    /// Overlap with another box as intersection over union.
    ///
    /// How duplicates are recognised where two tiles both saw the same line.
    pub fn overlap(&self, other: &LineBox) -> f32 {
        let (al, at, ar, ab) = self.bounds();
        let (bl, bt, br, bb) = other.bounds();

        let width = (ar.min(br) - al.max(bl)).max(0.0);
        let height = (ab.min(bb) - at.max(bt)).max(0.0);
        let overlap = width * height;

        let union = (ar - al) * (ab - at) + (br - bl) * (bb - bt) - overlap;
        if union <= 0.0 {
            0.0
        } else {
            overlap / union
        }
    }
}

/// One recognised line: its words, and what it turned out to be written in.
#[derive(Debug, Clone, PartialEq)]
pub struct RecognisedLine {
    pub words: Vec<RecognisedWord>,
    pub script: Script,
    pub direction: crate::document::layout::Direction,
}

/// Something that can read words off a rasterised page.
///
/// **Two stages, deliberately.** Finding where the text sits is
/// language-agnostic; reading it is per-script. That is not an implementation
/// detail of one model family — it is the fact that makes multi-script
/// tractable at all, because a single detection model of a few megabytes serves
/// every language while only the recogniser has to be fetched per script.
///
/// Collapsing the two into "image in, words out" hides that structure, and
/// makes adding a second script a rewrite rather than a download.
///
/// A trait rather than an implementation because a model is *data*, and which
/// one to ship is a product decision about licensing and download size. It is
/// also the seam that lets a platform substitute Vision or ML Kit later without
/// touching anything else — and the pixel-identity test holds whatever is
/// behind it to the same standard.
pub trait Recogniser: Send + Sync {
    /// Where the text lines are. Language-agnostic.
    fn detect(&self, image: &GreyImage) -> Result<Vec<LineBox>>;

    /// What one line says. Per-script.
    fn recognise(&self, image: &GreyImage, line: &LineBox, script: Script)
        -> Result<RecognisedLine>;

    fn supported_scripts(&self) -> &[Script];
}

/// A recogniser that finds nothing.
///
/// Not a placeholder to be deleted later. It is what lets the coordinate
/// mapping, the tiling, the layer writer, the tagging, the command and most of
/// the test suite be built and run **before any model exists** — which is the
/// majority of the code, and all of it would otherwise sit behind a download
/// and a runtime decision that has not been made yet.
#[derive(Debug, Default)]
pub struct NullRecogniser;

impl Recogniser for NullRecogniser {
    fn detect(&self, _image: &GreyImage) -> Result<Vec<LineBox>> {
        Ok(Vec::new())
    }

    fn recognise(&self, _image: &GreyImage, _line: &LineBox, script: Script)
        -> Result<RecognisedLine> {
        Ok(RecognisedLine {
            words: Vec::new(),
            script,
            direction: crate::document::layout::Direction::Ltr,
        })
    }

    fn supported_scripts(&self) -> &[Script] {
        &[]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(text: &str, left: f32, top: f32, right: f32, bottom: f32) -> RecognisedWord {
        RecognisedWord {
            text: text.into(),
            rect: Rect { left, top, right, bottom },
            confidence: 0.9, char_confidence: Vec::new(),
        }
    }

    #[test]
    fn a_word_is_placed_as_one_run_at_its_left_edge() {
        let w = word("cat", 100.0, 200.0, 130.0, 212.0);
        let placed = w.placement().expect("a placement");

        assert_eq!(placed.text, "cat");
        assert!((placed.left - 100.0).abs() < 1e-6, "not at the left edge");
    }

    /// The bug this whole shape exists to prevent.
    ///
    /// Individually placed letters need an advance per letter, and a recogniser
    /// reports one box per word. A constant advance leaves a gap after every
    /// narrow letter, and extraction reads gaps as spaces — measured, that
    /// turned "LUMINAIRE" into "LUMI NAI RE".
    #[test]
    fn a_word_is_never_broken_into_separately_placed_letters() {
        let w = word("LUMINAIRE", 40.0, 80.0, 400.0, 96.0);
        let placed = w.placement().expect("a placement");
        assert_eq!(placed.text, "LUMINAIRE", "the word was split");
        assert!(!placed.text.contains(' '), "a space crept in: {:?}", placed.text);
    }

    #[test]
    fn a_long_word_in_a_narrow_box_is_set_smaller_rather_than_overflowing() {
        let cramped = word("SPECIFICATION", 0.0, 0.0, 40.0, 40.0);
        let roomy = word("SPECIFICATION", 0.0, 0.0, 400.0, 40.0);

        assert!(
            cramped.placement().unwrap().size < roomy.placement().unwrap().size,
            "the narrow box did not reduce the type size"
        );
    }

    #[test]
    fn font_size_fills_the_box_height() {
        assert!((word("x", 0.0, 100.0, 10.0, 118.0).font_size() - 18.0).abs() < 1e-6);
        // A degenerate box must not produce a zero or negative size.
        assert!(word("x", 0.0, 100.0, 10.0, 100.0).font_size() >= 1.0);
    }

    #[test]
    fn an_empty_word_places_nothing_rather_than_dividing_by_zero() {
        assert!(word("", 0.0, 0.0, 10.0, 10.0).placement().is_none());
        assert!(word("   ", 0.0, 0.0, 10.0, 10.0).placement().is_none());
    }

    #[test]
    fn pixel_boxes_become_page_points() {
        // The step that ruins a layer silently: recognise at 300 dpi, write the
        // numbers as points, and every word lands four times too far from the
        // corner. Now done by `geometry::PageImage`, which also undoes rotation.
        use crate::document::Rotation;
        use crate::ocr::geometry::PageImage;

        let mapping = PageImage::new(612.0, 792.0, 300.0, Rotation::None);
        let (x, y) = mapping.to_page(1000.0, 560.0);

        assert!((x - 240.0).abs() < 0.01, "got {x}");
        assert!((y - 134.4).abs() < 0.01, "got {y}");
    }

    #[test]
    fn a_line_box_knows_its_own_shape() {
        let upright = LineBox::upright(10.0, 20.0, 110.0, 40.0, 0.9);
        assert_eq!(upright.bounds(), (10.0, 20.0, 110.0, 40.0));
        assert_eq!(upright.height(), 20.0);
        assert!(upright.skew().abs() < 1e-6, "an upright box reported skew");

        // A quad tilted two degrees reports it, which is what deskew acts on.
        let tilted = LineBox {
            quad: [(0.0, 0.0), (100.0, 3.49), (100.0, 23.49), (0.0, 20.0)],
            confidence: 0.9,
        };
        assert!(
            (tilted.skew().to_degrees() - 2.0).abs() < 0.2,
            "measured {}°",
            tilted.skew().to_degrees()
        );
    }

    #[test]
    fn a_null_recogniser_lets_everything_downstream_be_built_and_tested() {
        // The point of it: no model, no download, no runtime decision, and the
        // whole pipeline below recognition still runs.
        let engine = NullRecogniser;
        let page = GreyImage::white(100, 100);

        assert!(engine.detect(&page).expect("detect").is_empty());
        assert!(engine.supported_scripts().is_empty());

        let line = engine
            .recognise(&page, &LineBox::upright(0.0, 0.0, 10.0, 10.0, 1.0), Script::Latin)
            .expect("recognise");
        assert!(line.words.is_empty());
        assert_eq!(line.script, Script::Latin);
    }

    #[test]
    fn a_recognised_word_survives_being_written_down() {
        // It travels in a Command, so it has to serialise.
        let w = word("lumens", 10.0, 20.0, 60.0, 32.0);
        let json = serde_json::to_string(&w).unwrap();
        assert!(json.contains("\"confidence\""), "field name drifted: {json}");
        assert_eq!(serde_json::from_str::<RecognisedWord>(&json).unwrap(), w);
    }
}
