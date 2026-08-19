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
use pdf_core::document::{Color, RegionRequest, Rect};
use pdf_core::render::{Bitmap, Tile, ViewportRequest};

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

// -------------------------------------------------------------- the viewport --

/// The gap between pages, so background pixels are recognisable as background.
///
/// Deliberately a colour neither page uses: "this pixel came from no page" has to
/// be distinguishable from "this pixel came from the wrong page".
const GAP: (u8, u8, u8) = (0, 255, 0);

/// Capture across `spread.pdf`, whose page 0 is red and page 1 blue.
fn capture_spread(tiles: Vec<Tile>, width: f32, height: f32, scale: f32) -> Bitmap {
    let _lock = serial();
    let pdfium = ();
    let doc = open_fixture(&pdfium, "spread.pdf");

    let request = ViewportRequest {
        tiles,
        width,
        height,
        scale,
        background: Color { r: GAP.0, g: GAP.1, b: GAP.2, a: 255 },
        render_annotations: true,
        render_form_data: true,
    };

    // Encoded and decoded again rather than asserted on in memory: the export is
    // what leaves the app, and a picture that is only correct before it is written
    // is not correct.
    let png = pdf_core::engine::export_viewport(
        doc.as_ref(),
        &request,
        pdf_core::render::ImageFormat::Png,
        &[],
    )
    .expect("export the viewport");

    decode(&png)
}

fn decode(png: &[u8]) -> Bitmap {
    let decoded = image::load_from_memory(png).expect("decode the export").to_rgba8();
    let (width, height) = decoded.dimensions();
    let mut bitmap = Bitmap::new(width, height, pdf_core::render::PixelOrder::Rgba).unwrap();
    bitmap.data.copy_from_slice(decoded.as_raw());
    bitmap
}

fn nearest_of(sample: (u8, u8, u8), palette: &[(u8, u8, u8)]) -> (u8, u8, u8) {
    let distance = |c: (u8, u8, u8)| {
        let d = |a: u8, b: u8| (a as i32 - b as i32).pow(2);
        d(sample.0, c.0) + d(sample.1, c.1) + d(sample.2, c.2)
    };
    palette.iter().copied().min_by_key(|&c| distance(c)).expect("a palette")
}

/// What a pixel is, out of red page / blue page / the gap between them.
fn source_of(bitmap: &Bitmap, x: u32, y: u32) -> (u8, u8, u8) {
    nearest_of(pixel(bitmap, x, y), &[RED, BLUE, GAP, (255, 255, 255)])
}

/// Two pages meeting, as the reader actually draws them: the bottom quarter of
/// page 0, a gap, then the top half of page 1.
///
/// The destinations keep each crop's aspect — 400 pt across becomes 100 units, so
/// 100 pt down becomes 25. That is not a detail of the test: the caller owns the
/// layout, and a destination that disagrees with its crop is asking for a stretch
/// (see `a_tile_is_never_stretched_to_fill_a_destination_that_does_not_match`).
fn across_the_join() -> Vec<Tile> {
    vec![
        Tile {
            page_index: 0,
            crop: crop(0.0, 300.0, 400.0, 400.0),
            dest: crop(0.0, 0.0, 100.0, 25.0),
        },
        Tile {
            page_index: 1,
            crop: crop(0.0, 0.0, 400.0, 200.0),
            dest: crop(0.0, 35.0, 100.0, 85.0),
        },
    ]
}

#[test]
fn a_capture_across_a_join_holds_both_pages() {
    let Some(()) = skip_without_pdfium() else {
        return;
    };

    // The complaint this feature answers: a box dragged across the boundary
    // between two pages used to come back holding only the page it started on.
    let bitmap = capture_spread(across_the_join(), 100.0, 100.0, 2.0);
    assert_eq!((bitmap.width, bitmap.height), (200, 200));

    assert_eq!(source_of(&bitmap, 100, 30), RED, "the upper page is missing");
    assert_eq!(source_of(&bitmap, 100, 160), BLUE, "the lower page is missing");
}

#[test]
fn the_gap_between_two_pages_is_the_background_rather_than_either_page() {
    let Some(()) = skip_without_pdfium() else {
        return;
    };

    // Between the two tiles nothing was rendered. Those pixels have to be the
    // background the caller asked for — not whatever the allocation held, and not
    // a page stretched to fill the space.
    let bitmap = capture_spread(across_the_join(), 100.0, 100.0, 2.0);

    assert_eq!(source_of(&bitmap, 100, 60), GAP, "the gap is not the background");
}

#[test]
fn each_page_lands_where_the_layout_put_it_rather_than_at_the_top() {
    let Some(()) = skip_without_pdfium() else {
        return;
    };

    let bitmap = capture_spread(across_the_join(), 100.0, 100.0, 2.0);

    // Scan the column and record where each source starts and stops. Getting the
    // order or the boundaries wrong is the failure a "both colours present" check
    // would sail past.
    let column: Vec<(u8, u8, u8)> = (0..bitmap.height).map(|y| source_of(&bitmap, 100, y)).collect();

    let first_blue = column.iter().position(|&c| c == BLUE).expect("blue somewhere");
    let last_red = column.iter().rposition(|&c| c == RED).expect("red somewhere");

    assert!(last_red < first_blue, "the pages came out in the wrong order");
    // Page 0's share is 25 units tall and the gap is 10, so at 2x red ends near
    // 50 px and blue starts near 70.
    assert!((48..=52).contains(&last_red), "red ends at {last_red}, expected ~50");
    assert!((68..=72).contains(&first_blue), "blue starts at {first_blue}, expected ~70");
}

#[test]
fn a_tile_scrolled_half_off_the_top_is_clipped_rather_than_moved() {
    let Some(()) = skip_without_pdfium() else {
        return;
    };

    // The everyday case: the page above is half out of shot. Its tile has a
    // negative top, and the part that is off-screen must be dropped instead of
    // sliding the visible part down.
    let tiles = vec![Tile {
        page_index: 0,
        crop: crop(0.0, 0.0, 400.0, 400.0),
        dest: crop(0.0, -50.0, 100.0, 50.0),
    }];

    let bitmap = capture_spread(tiles, 100.0, 100.0, 2.0);

    assert_eq!(source_of(&bitmap, 100, 10), RED, "the visible half is missing");
    assert_eq!(
        source_of(&bitmap, 100, 190),
        GAP,
        "the page was slid down instead of clipped",
    );
}

#[test]
fn a_capture_of_one_page_through_the_viewport_matches_a_plain_region_capture() {
    let Some(()) = skip_without_pdfium() else {
        return;
    };

    // The two paths have to agree where they overlap, or a capture would change
    // appearance depending on whether it happened to touch a second page.
    let tiles = vec![Tile {
        page_index: 1,
        crop: crop(0.0, 0.0, 400.0, 400.0),
        dest: crop(0.0, 0.0, 100.0, 100.0),
    }];

    let bitmap = capture_spread(tiles, 100.0, 100.0, 2.0);
    assert_eq!((bitmap.width, bitmap.height), (200, 200));
    for (x, y) in [(10, 10), (100, 100), (190, 190)] {
        assert_eq!(source_of(&bitmap, x, y), BLUE);
    }
}

#[test]
fn a_higher_export_scale_changes_the_resolution_and_nothing_else() {
    let Some(()) = skip_without_pdfium() else {
        return;
    };

    let small = capture_spread(across_the_join(), 100.0, 100.0, 1.0);
    let large = capture_spread(across_the_join(), 100.0, 100.0, 4.0);

    assert_eq!((small.width, small.height), (100, 100));
    assert_eq!((large.width, large.height), (400, 400));

    // Same fractions of each picture: the same source at both sizes means the two
    // frame the same thing.
    for fraction in [0.1f32, 0.3, 0.6, 0.9] {
        let at = |bitmap: &Bitmap| {
            source_of(
                bitmap,
                bitmap.width / 2,
                (bitmap.height as f32 * fraction) as u32,
            )
        };
        assert_eq!(
            at(&small),
            at(&large),
            "the source {fraction} of the way down changed with the scale",
        );
    }
}

#[test]
fn a_tile_is_never_stretched_to_fill_a_destination_that_does_not_match() {
    let Some(()) = skip_without_pdfium() else {
        return;
    };

    // A tile's scale comes from its width. Ask for a destination twice as tall as
    // the crop's aspect allows and the extra is background, not a stretched page.
    //
    // The alternative — scaling each axis independently — would silently distort
    // a page whenever the caller's layout arithmetic drifted, and a distorted
    // capture looks plausible enough to go unnoticed.
    let tiles = vec![Tile {
        page_index: 0,
        crop: crop(0.0, 0.0, 400.0, 100.0),
        dest: crop(0.0, 0.0, 100.0, 50.0),
    }];

    let bitmap = capture_spread(tiles, 100.0, 100.0, 2.0);

    // 400 pt wide into 200 px is 0.5 px per point, so 100 pt tall comes out 50 px.
    assert_eq!(source_of(&bitmap, 100, 40), RED, "the page is missing");
    assert_eq!(
        source_of(&bitmap, 100, 80),
        GAP,
        "the page was stretched to fill the destination",
    );
}
