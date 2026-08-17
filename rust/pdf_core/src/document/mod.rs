//! The document abstraction.
//!
//! Everything above this layer (the JNI bridge, the cache, the future command
//! stack) talks to `dyn Document`, so the read-only PDFium implementation shipping
//! today and the `lopdf`-backed editable one arriving in roadmap phase 3 are
//! interchangeable without touching the bridge.

pub mod metadata;
pub mod pdfium_doc;

#[cfg(feature = "editing")]
pub mod editable_doc;

pub use metadata::DocumentMetadata;

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::render::RenderTarget;

/// Page dimensions in PostScript points (1/72"), as authored in the PDF.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageSize {
    pub width_pt: f32,
    pub height_pt: f32,
}

impl PageSize {
    /// Pixel dimensions this page occupies at the given scale, clamped to at
    /// least 1x1 so a degenerate page can never produce a zero-sized bitmap.
    pub fn pixel_size(&self, scale: f32) -> (u32, u32) {
        let w = (self.width_pt * scale).round().max(1.0) as u32;
        let h = (self.height_pt * scale).round().max(1.0) as u32;
        (w, h)
    }
}

/// Quarter-turn page rotation applied at render time (not persisted to the file).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Rotation {
    #[default]
    None,
    Clockwise90,
    Clockwise180,
    Clockwise270,
}

impl Rotation {
    /// Rotation swaps the axes for odd quarter-turns, so callers sizing a target
    /// bitmap must ask through here rather than using the raw page size.
    pub fn swaps_axes(self) -> bool {
        matches!(self, Rotation::Clockwise90 | Rotation::Clockwise270)
    }

    pub fn from_quarter_turns(turns: i32) -> Self {
        match turns.rem_euclid(4) {
            1 => Rotation::Clockwise90,
            2 => Rotation::Clockwise180,
            3 => Rotation::Clockwise270,
            _ => Rotation::None,
        }
    }
}

/// What to draw, and how big. Kept separate from `RenderTarget` (where to draw)
/// so a request can be hashed into a cache key without involving the destination.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderRequest {
    /// Points-to-pixels multiplier. 1.0 renders at 72 dpi.
    pub scale: f32,
    pub rotation: Rotation,
    /// Draw annotation appearances (highlights, form field borders, stamps).
    pub render_annotations: bool,
    /// Draw interactive form field contents on top of the page.
    pub render_form_data: bool,
}

impl Default for RenderRequest {
    fn default() -> Self {
        RenderRequest {
            scale: 1.0,
            rotation: Rotation::None,
            render_annotations: true,
            render_form_data: true,
        }
    }
}

/// A single page of an open document.
///
/// The `'_` lifetime on `Document::page` is a deviation from the original sketch:
/// PDFium page handles borrow their document, so a `Box<dyn Page>` detached from
/// the document's lifetime could not be made sound.
pub trait Page {
    fn size(&self) -> PageSize;

    /// Rasterise into a caller-owned buffer. The buffer is supplied rather than
    /// returned so the hot path can draw straight into a locked Android Bitmap
    /// with no intermediate allocation or copy.
    fn render_into(&self, request: &RenderRequest, target: &mut RenderTarget<'_>) -> Result<()>;

    /// Extracted text in reading order, as PDFium's text page reports it.
    fn text(&self) -> Result<String>;
}

pub trait Document: Send + Sync {
    fn page_count(&self) -> usize;

    fn metadata(&self) -> Result<DocumentMetadata>;

    fn page(&self, index: usize) -> Result<Box<dyn Page + '_>>;

    /**
     * Page dimensions without loading the page.
     *
     * Split out from [`Page::size`] because loading a page parses its resources
     * and content-stream references, which on a large document is orders of
     * magnitude more expensive than reading two numbers out of the page tree.
     * Sizing happens constantly — measuring placeholders, choosing a render
     * scale, prefetching — so it must not drag a page load along with it.
     */
    fn page_size(&self, index: usize) -> Result<PageSize> {
        self.validate_page_index(index)?;
        Ok(self.page(index)?.size())
    }

    /// `Some` only for implementations that can mutate and save the file.
    /// Read-only documents return `None`, which is what phase 1 always does.
    fn as_editable(&mut self) -> Option<&mut dyn EditableDocument> {
        None
    }

    /// Bounds check shared by every implementation.
    fn validate_page_index(&self, index: usize) -> Result<()> {
        let count = self.page_count();
        if index >= count {
            return Err(crate::error::PdfError::PageOutOfRange { index, count });
        }
        Ok(())
    }
}

/// Roadmap phase 3+. Declared now so the command stack in [`crate::command`] and
/// the plugin traits in [`crate::plugins`] can be written against a real type.
pub trait EditableDocument {
    fn add_annotation(&mut self, page: usize, annotation: Annotation) -> Result<()>;
    fn add_signature(&mut self, page: usize, signature: Signature) -> Result<()>;
    fn remove_annotation(&mut self, page: usize, annotation_id: u64) -> Result<()>;
    fn save(&self, path: &str) -> Result<()>;
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Annotation {
    Text {
        rect: Rect,
        content: String,
        author: String,
    },
    Highlight {
        rect: Rect,
        color: Color,
    },
    Ink {
        strokes: Vec<Vec<(f32, f32)>>,
        color: Color,
        width: f32,
    },
}

#[derive(Debug, Clone)]
pub struct Signature {
    pub rect: Rect,
    /// Rasterised visual appearance (PNG bytes).
    pub image: Vec<u8>,
    pub digital_signature: Option<DigitalSignature>,
}

#[derive(Debug, Clone)]
pub struct DigitalSignature {
    pub certificate: Vec<u8>,
    pub reason: String,
    pub location: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixel_size_scales_and_rounds() {
        let a4 = PageSize {
            width_pt: 595.0,
            height_pt: 842.0,
        };
        assert_eq!(a4.pixel_size(1.0), (595, 842));
        assert_eq!(a4.pixel_size(2.0), (1190, 1684));
    }

    #[test]
    fn pixel_size_never_collapses_to_zero() {
        let sliver = PageSize {
            width_pt: 0.4,
            height_pt: 800.0,
        };
        let (w, h) = sliver.pixel_size(0.01);
        assert_eq!(w, 1, "a zero-width bitmap would fail allocation downstream");
        assert_eq!(h, 8);
    }

    #[test]
    fn only_odd_quarter_turns_swap_axes() {
        assert!(!Rotation::None.swaps_axes());
        assert!(Rotation::Clockwise90.swaps_axes());
        assert!(!Rotation::Clockwise180.swaps_axes());
        assert!(Rotation::Clockwise270.swaps_axes());
    }

    #[test]
    fn quarter_turns_wrap_in_both_directions() {
        assert_eq!(Rotation::from_quarter_turns(0), Rotation::None);
        assert_eq!(Rotation::from_quarter_turns(5), Rotation::Clockwise90);
        assert_eq!(Rotation::from_quarter_turns(-1), Rotation::Clockwise270);
    }
}
