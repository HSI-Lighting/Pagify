//! Region export: the capture feature's engine half, against real PDFium.
//!
//! What these tests are for is *where the pixels came from*. Geometry cannot
//! answer that — a crop of the wrong part of a page is exactly as many pixels as
//! a crop of the right part, so a size assertion passes either way. `quadrants.pdf`
//! is four solid colours, one per quadrant, so the answer is readable off a single
//! pixel and a wrong crop cannot look right.
//!
//! The guarantee under test is decision 4.8: the export re-renders the region from
//! the document rather than capturing the screen, so nothing outside the crop —
//! and nothing that is not in the document at all — can appear in it.

mod harness;

use harness::{open_fixture, serial, skip_without_pdfium};
use pdf_core::document::{RegionRequest, Rect};
use pdf_core::render::Bitmap;

/// Colours as they appear in the fixture, in RGB.
const RED: (u8, u8, u8) = (255, 0, 0);
const GREEN: (u8, u8, u8) = (0, 255, 0);
const BLUE: (u8, u8, u8) = (0, 0, 255);
const YELLOW: (u8, u8, u8) = (255, 255, 0);

fn crop(left: f32, top: f32, right: f32, bottom: f32) -> Rect {
    Rect {
        left,
        top,
        right,
        bottom,
    }
}

fn pixel(bitmap: &Bitmap, x: u32, y: u32) -> (u8, u8, u8) {
    let at = y as usize * bitmap.stride + x as usize * 4;
    (bitmap.data[at], bitmap.data[at + 1], bitmap.data[at + 2])
}

/// Nearest of the fixture's colours, so anti-aliasing along a quadrant boundary
/// does not read as a fifth colour. White — the page under everything — is left
/// as itself, because "the crop came out blank" must not quietly classify as a
/// colour.
fn nearest_colour(sample: (u8, u8, u8)) -> Option<(u8, u8, u8)> {
    let distance = |c: (u8, u8, u8)| {
        let d = |a: u8, b: u8| (a as i32 - b as i32).pow(2);
        d(sample.0, c.0) + d(sample.1, c.1) + d(sample.2, c.2)
    };
    [RED, GREEN, BLUE, YELLOW, (255, 255, 255)]
        .into_iter()
        .min_by_key(|&c| distance(c))
        .filter(|&c| c != (255, 255, 255))
}

fn colours_present(bitmap: &Bitmap) -> Vec<(u8, u8, u8)> {
    let mut found = Vec::new();
    for y in 0..bitmap.height {
        for x in 0..bitmap.width {
            if let Some(colour) = nearest_colour(pixel(bitmap, x, y)) {
                if !found.contains(&colour) {
                    found.push(colour);
                }
            }
        }
    }
    found.sort();
    found
}

fn render(scale: f32, rect: Rect) -> Bitmap {
    let _lock = serial();
    let pdfium = ();
    let doc = open_fixture(&pdfium, "quadrants.pdf");
    let page = doc.page(0).expect("page 0");
    page.render_region(&RegionRequest {
        crop: rect,
        scale,
        ..RegionRequest::default()
    })
    .expect("render region")
}

#[test]
fn a_crop_holds_only_what_was_inside_it() {
    let Some(()) = skip_without_pdfium() else {
        return;
    };

    // The whole top-left quadrant, one point in from each edge so the boundary's
    // anti-aliasing is not the thing under test.
    let bitmap = render(4.0, crop(1.0, 1.0, 199.0, 199.0));

    assert_eq!(
        colours_present(&bitmap),
        vec![RED],
        "the top-left quadrant is red; any other colour is content from outside \
         the crop leaking in",
    );
}

#[test]
fn each_quadrant_crops_to_its_own_colour() {
    let Some(()) = skip_without_pdfium() else {
        return;
    };

    // Top-left origin, y down — decision 4.4. Getting this wrong is the single
    // easiest mistake in the whole feature, and it would put every capture in the
    // mirror-image part of the page.
    let cases = [
        (crop(1.0, 1.0, 199.0, 199.0), RED),
        (crop(201.0, 1.0, 399.0, 199.0), GREEN),
        (crop(1.0, 201.0, 199.0, 399.0), BLUE),
        (crop(201.0, 201.0, 399.0, 399.0), YELLOW),
    ];

    for (rect, expected) in cases {
        let bitmap = render(2.0, rect);
        assert_eq!(
            colours_present(&bitmap),
            vec![expected],
            "crop {rect:?} should be a single colour",
        );
    }
}

#[test]
fn content_immediately_outside_the_crop_never_appears() {
    let Some(()) = skip_without_pdfium() else {
        return;
    };

    // Deliberately tight: the crop stops one point short of green, blue and
    // yellow, all three of which are immediately adjacent. An off-by-one in the
    // offset, or a render that draws the page and trims afterwards, shows up here
    // as a stripe of another colour along an edge.
    let bitmap = render(4.0, crop(0.0, 0.0, 199.0, 199.0));

    let leaks: Vec<_> = colours_present(&bitmap)
        .into_iter()
        .filter(|&c| c != RED)
        .collect();
    assert!(
        leaks.is_empty(),
        "{leaks:?} appeared in a crop that stops short of every other quadrant",
    );
}

#[test]
fn the_same_crop_at_a_higher_scale_frames_the_same_content() {
    let Some(()) = skip_without_pdfium() else {
        return;
    };

    // Straddles the centre, so all four colours are in shot and their arrangement
    // is what has to survive the change of scale.
    let rect = crop(150.0, 150.0, 250.0, 250.0);
    let one = render(1.0, rect);
    let four = render(4.0, rect);

    assert_eq!((one.width, one.height), (100, 100));
    assert_eq!((four.width, four.height), (400, 400));

    // Sample the same *fractions* of each image. Equal colours mean the two
    // frames cover the same points of the page and differ only in resolution.
    for (fx, fy) in [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75), (0.5, 0.1)] {
        let at = |bitmap: &Bitmap| {
            nearest_colour(pixel(
                bitmap,
                (bitmap.width as f32 * fx) as u32,
                (bitmap.height as f32 * fy) as u32,
            ))
        };
        assert_eq!(
            at(&one),
            at(&four),
            "the colour {fx} across and {fy} down changed with the scale",
        );
    }

    // And all four really are in shot, or the assertion above is vacuous.
    assert_eq!(colours_present(&one).len(), 4);
}

#[test]
fn an_export_scale_beyond_the_ceiling_clamps_rather_than_failing() {
    let Some(()) = skip_without_pdfium() else {
        return;
    };

    // 400 x 400 pt at 100x would be 40000 px a side — past the dimension ceiling
    // and about 6 GB. The user asked for a picture of a region; the answer is the
    // sharpest one that fits, not an error.
    let bitmap = render(100.0, crop(0.0, 0.0, 400.0, 400.0));

    assert!(bitmap.width <= pdf_core::render::bitmap::MAX_DIMENSION_PX);
    assert!(
        u64::from(bitmap.width) * u64::from(bitmap.height)
            <= pdf_core::render::bitmap::MAX_PIXELS
    );
    assert_eq!(
        colours_present(&bitmap),
        vec![RED, GREEN, BLUE, YELLOW].into_iter().collect::<Vec<_>>().tap_sorted(),
        "a clamped export is still the whole crop, just at a lower resolution",
    );
}

/// Sorting inline, so the expectation above reads in quadrant order rather than
/// in whatever order `sort` happens to put four RGB triples.
trait TapSorted {
    fn tap_sorted(self) -> Self;
}

impl TapSorted for Vec<(u8, u8, u8)> {
    fn tap_sorted(mut self) -> Self {
        self.sort();
        self
    }
}
