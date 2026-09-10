//! Page in, words out — the sequence that makes a scan searchable.
//!
//! Every step here already existed and was tested on its own. What did not
//! exist was the order they go in, and the order is where the mistakes live:
//! a tile's boxes are in tile coordinates until they are moved, a recognised
//! word is in image pixels until it is mapped, and a page that is rotated or
//! deskewed has moved *twice* by the time a word box reaches the document.
//!
//! Each of those is a coordinate space that looks like the previous one. None
//! of them raises an error when confused with it — the layer just lands
//! somewhere wrong, and the only symptom is a selection highlight that does not
//! sit on the ink.
//!
//! # Deskew is measured and undone, not baked in
//!
//! A skewed page is straightened before recognition, because a recogniser reads
//! a straight line far better than a tilted one. But the *document* is not
//! skewed — the scan was — so every box has to be rotated back by the same
//! angle before it becomes a text layer. Straightening the image and forgetting
//! to rotate the boxes back is the single most likely way to get this wrong,
//! which is why the angle is returned rather than discarded.

use std::collections::BTreeMap;

use crate::document::{Page, Rect, RenderRequest};
use crate::error::{PdfError, Result};
use crate::ocr::geometry::PageImage;
use crate::ocr::{preprocess, tiling, GreyImage, LineBox, RecognisedWord, Recogniser, Script};
use crate::render::{bitmap::PixelOrder, RenderTarget};

/// How to read a page.
#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// Rasterisation resolution.
    ///
    /// 300 is the number every OCR engine is tuned around, and the M0 spike
    /// measured recognition accuracy falling off below it rather than degrading
    /// gently. Raising it past 400 costs memory quadratically and buys very
    /// little.
    pub dpi: f32,

    /// Straighten the page before reading it, and rotate the boxes back after.
    pub deskew: bool,

    /// Largest tile, in pixels, before the page is split.
    ///
    /// The M0 spike peaked at 482 MB on a single 300 dpi page, which is what
    /// makes this a limit rather than a tuning knob. The default is sized for a
    /// desktop; a phone should pass something far smaller.
    pub tile_budget: usize,

    /// Leave alone the text that is already there.
    ///
    /// Without this the feature is wrong on the commonest real page it will
    /// meet. A page whose prose is drawn as outlines usually *also* carries a
    /// little real text — a header, a monospace run, a caption — and
    /// recognition reads the whole page, so those words get written a second
    /// time on top of themselves. Measured on a real report: 384 characters
    /// became 1,687, and `Component` appeared twice. Two overlapping selectable
    /// layers, and every search hit counted twice.
    pub skip_existing_text: bool,

    /// Words scoring below this are dropped rather than written.
    ///
    /// Zero by default — deliberately. A wrong word that is *findable* is
    /// usually better than a missing one, and the per-character scores travel
    /// with the layer so a review pass can still single them out. Callers who
    /// would rather have a gap than a guess can raise it.
    pub min_confidence: f32,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            dpi: 300.0,
            deskew: true,
            tile_budget: 40_000_000,
            skip_existing_text: true,
            min_confidence: 0.0,
        }
    }
}

/// What reading a page produced.
#[derive(Debug, Clone)]
pub struct PageReading {
    /// Words in **page points**, top-left origin — ready for
    /// [`Document::add_text_layer`](crate::document::Document::add_text_layer).
    pub words: Vec<RecognisedWord>,
    /// The skew that was corrected, in radians. Zero when `deskew` was off.
    pub skew: f32,
    /// Lines the detector found, before recognition. Reported because "found
    /// forty lines and read three" and "found three lines" are different
    /// failures with the same output.
    pub lines: usize,
    /// Words dropped because the page already had text there.
    ///
    /// Worth reporting rather than swallowing: on a page that is half native
    /// text this is most of what was read, and a caller that says "wrote 12
    /// words" without it looks like recognition failed.
    pub already_there: usize,
    pub image_size: (u32, u32),
}

/// Rasterise a page to greyscale at `dpi`.
///
/// Annotations and form data are drawn. A stamp or a filled field is text a
/// reader can see, and a searchable layer that omits what is visibly on the
/// page is a worse lie than one that includes it.
pub fn rasterise(page: &dyn Page, dpi: f32) -> Result<GreyImage> {
    let scale = dpi / 72.0;
    let (width, height) = page.size().pixel_size(scale);
    if width == 0 || height == 0 {
        return Err(PdfError::InvalidArgument("ocr: page has no area".into()));
    }

    let mut pixels = vec![0u8; width as usize * height as usize * 4];
    {
        let mut target =
            RenderTarget::new(width, height, width as usize * 4, PixelOrder::Rgba, &mut pixels)?;
        page.render_into(
            &RenderRequest {
                scale,
                render_annotations: true,
                render_form_data: true,
                ..Default::default()
            },
            &mut target,
        )?;
    }

    Ok(preprocess::to_grey(&pixels, width, height))
}

/// Rotate a point about the image centre, which is what `preprocess::rotate`
/// turns about.
fn rotate_about_centre(point: (f32, f32), angle: f32, size: (u32, u32)) -> (f32, f32) {
    let (cx, cy) = (size.0 as f32 / 2.0, size.1 as f32 / 2.0);
    let (sin, cos) = angle.sin_cos();
    let (dx, dy) = (point.0 - cx, point.1 - cy);
    (cx + dx * cos - dy * sin, cy + dx * sin + dy * cos)
}

/// Copy a tile out of an image.
fn crop(image: &GreyImage, tile: &tiling::Tile) -> GreyImage {
    let mut out = GreyImage::white(tile.width, tile.height);
    for y in 0..tile.height {
        for x in 0..tile.width {
            out.set(x, y, image.get(tile.x + x, tile.y + y));
        }
    }
    out
}

/// Find every text line on an image, tiling it if it is too large to hold.
///
/// The tiles overlap and are merged, so a line cut by a seam is found twice and
/// kept once. Detection runs on tiles but **recognition does not**: a line that
/// straddles a seam has to be read whole, and by the time the boxes are merged
/// they are in whole-image coordinates, so the full image is the right thing to
/// read them from.
pub fn detect_lines(
    image: &GreyImage,
    recogniser: &dyn Recogniser,
    budget: usize,
) -> Result<Vec<LineBox>> {
    detect_lines_until(image, recogniser, budget, &|| false)
}

/// As [`detect_lines`], stopping between tiles if asked.
pub fn detect_lines_until(
    image: &GreyImage,
    recogniser: &dyn Recogniser,
    budget: usize,
    stop: &dyn Fn() -> bool,
) -> Result<Vec<LineBox>> {
    // A generous overlap: it is the tall lines that get cut, and a heading is
    // several times the height of body text.
    let overlap = (image.height / 20).clamp(48, 400);
    let tiles = tiling::tiles(image.width, image.height, budget, overlap);

    let mut found = Vec::new();
    for tile in &tiles {
        if stop() {
            return Err(PdfError::Cancelled);
        }
        let piece = if tiles.len() == 1 { image.clone() } else { crop(image, tile) };
        for line in recogniser.detect(&piece)? {
            found.push(tile.to_image(&line));
        }
    }

    Ok(tiling::merge(found))
}

/// Where the page already has text, in page points.
///
/// Rows of character boxes, bucketed by their vertical middle, so a word can be
/// tested against the handful of characters near it rather than against every
/// character on the page. A dense page carries a few thousand boxes and a
/// recognised page a few hundred words; the naive product is slow enough to be
/// felt on a document.
struct Occupied {
    rows: BTreeMap<i32, Vec<Rect>>,
}

/// The bucket height, in points. About a line of body text — small enough that
/// a bucket holds one line, large enough that a word is never spread over many.
const ROW_HEIGHT: f32 = 12.0;

impl Occupied {
    fn of(page: &dyn Page) -> Self {
        let mut rows: BTreeMap<i32, Vec<Rect>> = BTreeMap::new();
        if let Ok(chars) = page.characters() {
            for b in chars.boxes.chunks_exact(4) {
                let rect =
                    Rect { left: b[0], top: b[1], right: b[2], bottom: b[3] };
                // A zero-area box is a space or a control character. It occupies
                // nothing, and treating it as occupied would punch holes in the
                // recognised text at every word gap.
                if (rect.right - rect.left).abs() < 0.01
                    || (rect.bottom - rect.top).abs() < 0.01
                {
                    continue;
                }
                let key = (((rect.top + rect.bottom) / 2.0) / ROW_HEIGHT).floor() as i32;
                rows.entry(key).or_default().push(rect);
            }
        }
        Occupied { rows }
    }

    fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// How much of this word is already covered by real text, 0–1.
    ///
    /// Sampled along the word's centre line rather than tested at its middle
    /// point. A recognised box spans a whole **word**; the boxes already on the
    /// page are single **characters**. The centre of `Component` falls in the
    /// gap between `p` and `o` as often as not, so a single-point test misses
    /// the very case this exists for.
    ///
    /// Sampling also gives a *degree* of overlap rather than a yes or no, which
    /// is what distinguishes a word written twice from a word that merely
    /// abuts one.
    fn covered_fraction(&self, word: &Rect) -> f32 {
        const SAMPLES: usize = 9;

        let (top, bottom) = (word.top.min(word.bottom), word.top.max(word.bottom));
        let (left, right) = (word.left.min(word.right), word.left.max(word.right));
        let cy = (top + bottom) / 2.0;
        let key = (cy / ROW_HEIGHT).floor() as i32;

        // The neighbouring buckets too: a line of characters may be filed on
        // either side of a boundary that falls in the middle of it.
        let nearby: Vec<&Rect> = (key - 1..=key + 1)
            .filter_map(|k| self.rows.get(&k))
            .flatten()
            .collect();
        if nearby.is_empty() {
            return 0.0;
        }

        let mut hits = 0usize;
        for i in 0..SAMPLES {
            // Inset from the ends, so a word that merely touches its neighbour
            // is not counted by its outermost sample.
            let t = (i as f32 + 0.5) / SAMPLES as f32;
            let x = left + (right - left) * t;
            if nearby.iter().any(|r| {
                x >= r.left.min(r.right)
                    && x <= r.left.max(r.right)
                    && cy >= r.top.min(r.bottom)
                    && cy <= r.top.max(r.bottom)
            }) {
                hits += 1;
            }
        }
        hits as f32 / SAMPLES as f32
    }

    /// Is this word already on the page?
    ///
    /// Half of it, rather than all: the two boxes for the same word never line
    /// up exactly, and a word whose recognised box runs a little wide would
    /// otherwise be written a second time.
    fn covers(&self, word: &Rect) -> bool {
        self.covered_fraction(word) >= 0.5
    }
}

/// Read a page, with nothing to interrupt it.
pub fn read_page(
    page: &dyn Page,
    recogniser: &dyn Recogniser,
    options: &Options,
) -> Result<PageReading> {
    read_page_until(page, recogniser, options, &|| false)
}

/// Read a page, stopping if asked.
///
/// `stop` is consulted between lines and between tiles — the only points where
/// stopping is both cheap and leaves nothing half-built. Recognition of a single
/// line is not interruptible and does not need to be: it is about twenty
/// milliseconds.
///
/// Without this, "cancel" can only mean *declining the answer* while the work
/// continues. For one page that is a second of wasted CPU nobody is waiting on.
/// For a 149-page catalogue it is two and a half minutes of a laptop's fans
/// after the user has said stop, which is not a cancellation in any sense they
/// would recognise.
pub fn read_page_until(
    page: &dyn Page,
    recogniser: &dyn Recogniser,
    options: &Options,
    stop: &dyn Fn() -> bool,
) -> Result<PageReading> {
    let size = page.size();
    let mut image = rasterise(page, options.dpi)?;
    let raster_size = (image.width, image.height);

    preprocess::normalise_contrast(&mut image);

    let skew = if options.deskew {
        let (straightened, angle) = preprocess::deskew(&image);
        image = straightened;
        angle
    } else {
        0.0
    };

    let lines = detect_lines_until(&image, recogniser, options.tile_budget, stop)?;

    // Which script to read in, decided once per page from the strongest
    // evidence available rather than per line. A page is overwhelmingly written
    // in one script, and a per-line guess on a three-word line is a guess.
    let script = recogniser.supported_scripts().first().copied().unwrap_or(Script::Latin);

    let mapping =
        PageImage::new(size.width_pt, size.height_pt, options.dpi, crate::document::Rotation::None);

    let occupied =
        if options.skip_existing_text { Occupied::of(page) } else { Occupied { rows: BTreeMap::new() } };

    let mut words = Vec::new();
    let mut already_there = 0usize;
    for line in &lines {
        if stop() {
            return Err(PdfError::Cancelled);
        }
        let read = recogniser.recognise(&image, line, script)?;
        for word in read.words {
            if word.confidence < options.min_confidence {
                continue;
            }

            // Back through every space the pixels travelled, in reverse.
            //
            // Undo the deskew first — the box is in straightened-image
            // coordinates and the page is not straightened — then map pixels to
            // points. Doing these the other way round rotates a box about the
            // wrong centre in the wrong units, and the error grows with
            // distance from the middle of the page, so it looks like a small
            // misalignment near the centre and a large one at the edges.
            let corners = [
                (word.rect.left, word.rect.top),
                (word.rect.right, word.rect.top),
                (word.rect.right, word.rect.bottom),
                (word.rect.left, word.rect.bottom),
            ]
            .map(|c| if skew == 0.0 { c } else { rotate_about_centre(c, -skew, raster_size) })
            .map(|(x, y)| mapping.to_page(x, y));

            let xs = corners.map(|c| c.0);
            let ys = corners.map(|c| c.1);
            let rect = Rect {
                left: xs.iter().copied().fold(f32::MAX, f32::min),
                top: ys.iter().copied().fold(f32::MAX, f32::min),
                right: xs.iter().copied().fold(f32::MIN, f32::max),
                bottom: ys.iter().copied().fold(f32::MIN, f32::max),
            };

            // Already on the page. Writing it again would put a second
            // selectable copy over the first.
            if !occupied.is_empty() && occupied.covers(&rect) {
                already_there += 1;
                continue;
            }

            words.push(RecognisedWord { rect, ..word });
        }
    }

    Ok(PageReading { words, skew, lines: lines.len(), already_there, image_size: raster_size })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boxed(left: f32, top: f32, right: f32, bottom: f32) -> Rect {
        Rect { left, top, right, bottom }
    }

    fn occupied_with(boxes: &[Rect]) -> Occupied {
        let mut rows: BTreeMap<i32, Vec<Rect>> = BTreeMap::new();
        for r in boxes {
            let key = (((r.top + r.bottom) / 2.0) / ROW_HEIGHT).floor() as i32;
            rows.entry(key).or_default().push(*r);
        }
        Occupied { rows }
    }

    /// A word already on the page, as a row of character boxes — which is how
    /// a PDF actually stores one.
    fn word_at(left: f32, top: f32, chars: usize) -> Vec<Rect> {
        (0..chars)
            .map(|i| {
                let x = left + i as f32 * 8.0;
                boxed(x, top, x + 7.6, top + 12.0)
            })
            .collect()
    }

    /// The defect: on a page that carries both real text and text drawn as
    /// outlines, recognition reads all of it, so the real words get a second
    /// selectable copy written on top of themselves.
    #[test]
    fn a_word_already_on_the_page_is_recognised_as_already_there() {
        let page = occupied_with(&word_at(100.0, 200.0, 9));
        // What recognition would return for the same word: near enough, never
        // exactly the same rectangle.
        assert!(page.covers(&boxed(98.5, 199.0, 173.0, 213.0)));
    }

    /// A single-point test at the word's middle lands in the gap between two
    /// letters as often as not. This is the case that made the centre rule
    /// wrong, kept so it cannot come back.
    #[test]
    fn the_gap_between_two_letters_does_not_hide_a_word() {
        let mut boxes = word_at(100.0, 200.0, 9);
        // Punch out the character the middle of the word lands on.
        boxes.remove(4);
        let page = occupied_with(&boxes);
        assert!(page.covers(&boxed(98.5, 199.0, 173.0, 213.0)));
    }

    #[test]
    fn a_word_on_bare_paper_is_kept() {
        let page = occupied_with(&word_at(100.0, 200.0, 9));
        assert!(!page.covers(&boxed(300.0, 400.0, 340.0, 414.0)));
    }

    /// A word that merely abuts existing text is not the same word, and must be
    /// written.
    #[test]
    fn a_word_touching_its_neighbour_is_still_kept() {
        let page = occupied_with(&word_at(100.0, 200.0, 9));
        // Starts where that word ends, overlapping by one character.
        assert!(!page.covers(&boxed(166.0, 199.0, 240.0, 213.0)));
    }

    /// A word whose centre sits near a bucket boundary must still be found —
    /// the character covering it may be filed on the other side.
    #[test]
    fn the_bucketing_does_not_lose_a_word_on_a_boundary() {
        let y = ROW_HEIGHT * 5.0 - 6.0;
        let page = occupied_with(&word_at(100.0, y, 9));
        assert!(page.covers(&boxed(98.5, y - 1.0, 173.0, y + 13.0)));
    }

    /// Spaces and control characters report zero-area boxes. Treating those as
    /// occupied would punch a hole in the recognised text at every word gap.
    #[test]
    fn a_zero_area_box_occupies_nothing() {
        struct Blank;
        impl Page for Blank {
            fn size(&self) -> crate::document::PageSize {
                crate::document::PageSize { width_pt: 612.0, height_pt: 792.0 }
            }
            fn render_into(
                &self,
                _: &RenderRequest,
                _: &mut crate::render::RenderTarget<'_>,
            ) -> Result<()> {
                Ok(())
            }
            fn text(&self) -> Result<String> {
                Ok(" ".into())
            }
            fn characters(&self) -> Result<crate::document::PageCharacters> {
                Ok(crate::document::PageCharacters {
                    text: " ".into(),
                    // left, top, right, bottom — all the same point.
                    boxes: vec![100.0, 200.0, 100.0, 200.0],
                })
            }
        }
        assert!(Occupied::of(&Blank).is_empty(), "a space was treated as occupied text");
    }

    /// A tile's boxes are in tile coordinates until they are moved. This is the
    /// step that moves them, and getting it wrong puts every line on the second
    /// tile at the top of the page.
    #[test]
    fn a_box_found_in_a_lower_tile_lands_lower_on_the_page() {
        let tile = tiling::Tile { x: 0, y: 1000, width: 800, height: 600 };
        let found = LineBox::upright(10.0, 20.0, 300.0, 60.0, 1.0);
        let moved = tile.to_image(&found);
        let (_, top, _, bottom) = moved.bounds();
        assert_eq!((top, bottom), (1020.0, 1060.0));
    }

    /// Deskew turns the image; the page did not turn. A box has to come back.
    #[test]
    fn undoing_the_skew_returns_a_box_to_where_the_ink_is() {
        let size = (2000u32, 3000u32);
        let angle = 0.035_f32; // about two degrees, the fixture's tilt
        let original = (1500.0f32, 400.0f32);

        let straightened = rotate_about_centre(original, angle, size);
        let back = rotate_about_centre(straightened, -angle, size);

        assert!(
            (back.0 - original.0).abs() < 0.01 && (back.1 - original.1).abs() < 0.01,
            "a box did not come back: {original:?} -> {straightened:?} -> {back:?}"
        );
    }

    /// The error a wrong rotation centre makes is small in the middle and large
    /// at the edges, which is exactly the shape that gets shipped.
    #[test]
    fn the_rotation_centre_matters_most_at_the_edges() {
        let size = (2000u32, 3000u32);
        let angle = 0.035_f32;

        let centre = rotate_about_centre((1000.0, 1500.0), angle, size);
        assert!((centre.0 - 1000.0).abs() < 0.01 && (centre.1 - 1500.0).abs() < 0.01);

        let corner = rotate_about_centre((0.0, 0.0), angle, size);
        let moved = ((corner.0 - 0.0).powi(2) + (corner.1 - 0.0).powi(2)).sqrt();
        assert!(moved > 50.0, "a corner barely moved ({moved:.1}px) — the centre is wrong");
    }

    #[test]
    fn a_tile_is_copied_out_where_it_says_it_is() {
        let mut image = GreyImage::white(100, 100);
        image.set(60, 70, 0x10);

        let piece = crop(&image, &tiling::Tile { x: 50, y: 50, width: 40, height: 40 });
        assert_eq!(piece.get(10, 20), 0x10);
        assert_eq!(piece.get(0, 0), 0xFF);
    }
}
