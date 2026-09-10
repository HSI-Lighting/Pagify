//! Recognising a page that will not fit in memory.
//!
//! A 300 dpi A4 page is about 8.7 million pixels. That, plus a model, plus
//! whatever the app is already holding, is what gets a process killed on a
//! mid-range Android — not slowly, and not with an error anyone can catch.
//!
//! So a large page is read in tiles. The only interesting part is the overlap:
//! tiles that merely abut cut every line that crosses the seam in half, and two
//! half-lines recognise as two wrong words rather than one right one. Tiles
//! overlap by at least a line's height, and the duplicates that produces are
//! merged afterwards.

use crate::ocr::LineBox;

/// A rectangle of the page to recognise on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tile {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Tile {
    /// Move a box found inside this tile into whole-image coordinates.
    pub fn to_image(&self, line: &LineBox) -> LineBox {
        let mut moved = *line;
        for corner in &mut moved.quad {
            corner.0 += self.x as f32;
            corner.1 += self.y as f32;
        }
        moved
    }
}

/// Split an image into tiles no larger than `budget` pixels each.
///
/// `overlap` should be at least one line height — pass the tallest text you
/// expect, not an average, because it is the tall lines that get cut.
///
/// Tiles run full width where they can: text lines are horizontal, so cutting
/// vertically costs a seam through every line on the page, while cutting
/// horizontally costs a seam through one.
pub fn tiles(width: u32, height: u32, budget: usize, overlap: u32) -> Vec<Tile> {
    if width == 0 || height == 0 {
        return Vec::new();
    }

    let whole = width as usize * height as usize;
    if whole <= budget {
        return vec![Tile { x: 0, y: 0, width, height }];
    }

    // How many rows fit in the budget at full width.
    let rows = (budget / width.max(1) as usize).max(1) as u32;
    let step = rows.saturating_sub(overlap).max(1);

    let mut out = Vec::new();
    let mut y = 0u32;
    loop {
        let tile_height = rows.min(height - y);
        out.push(Tile { x: 0, y, width, height: tile_height });

        if y + tile_height >= height {
            break;
        }
        y += step;

        // The last tile is pulled back to the bottom edge rather than being
        // allowed to hang over it, so no row is read twice at a different
        // offset and none is missed.
        if y + rows >= height {
            out.push(Tile { x: 0, y: height.saturating_sub(rows), width, height: rows.min(height) });
            break;
        }
    }

    out.dedup();
    out
}

/// How much two boxes must overlap before they are treated as the same line.
///
/// Set low. The two views come from tiles that saw different amounts of the
/// line, so their boxes genuinely differ; demanding a close match leaves
/// duplicates in, and a duplicated line is worse than a slightly wrong box.
pub const SAME_LINE: f32 = 0.30;

/// Merge the lines found across overlapping tiles, keeping one of each.
///
/// Where two tiles saw the same line, the one that saw *more* of it wins —
/// a line clipped by a tile edge recognises as a fragment, and the fuller view
/// is the one worth keeping.
pub fn merge(mut found: Vec<LineBox>) -> Vec<LineBox> {
    // Widest first, so the fullest view of any line is the one already kept
    // when its clipped twin arrives.
    found.sort_by(|a, b| {
        let area = |l: &LineBox| {
            let (left, top, right, bottom) = l.bounds();
            (right - left) * (bottom - top)
        };
        area(b).partial_cmp(&area(a)).unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut kept: Vec<LineBox> = Vec::new();
    for line in found {
        if kept.iter().any(|k| k.overlap(&line) >= SAME_LINE) {
            continue;
        }
        kept.push(line);
    }

    // Back into reading order — merging is not allowed to reorder the page.
    kept.sort_by(|a, b| {
        let (_, at, _, _) = a.bounds();
        let (_, bt, _, _) = b.bounds();
        at.partial_cmp(&bt).unwrap_or(std::cmp::Ordering::Equal)
    });
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(top: f32, left: f32, right: f32) -> LineBox {
        LineBox::upright(left, top, right, top + 20.0, 0.9)
    }

    #[test]
    fn a_page_inside_the_budget_is_one_tile() {
        let cut = tiles(1000, 1000, 2_000_000, 40);
        assert_eq!(cut, vec![Tile { x: 0, y: 0, width: 1000, height: 1000 }]);
    }

    #[test]
    fn a_large_page_is_cut_into_full_width_strips() {
        // Horizontal cuts, because text lines are horizontal: a vertical cut
        // puts a seam through every line on the page.
        let cut = tiles(2480, 3508, 1_000_000, 60);
        assert!(cut.len() > 1, "an 8.7 M pixel page was not tiled");
        assert!(cut.iter().all(|t| t.width == 2480 && t.x == 0), "tiles were cut vertically");
        assert!(
            cut.iter().all(|t| t.width as usize * t.height as usize <= 1_000_000),
            "a tile exceeded the budget"
        );
    }

    #[test]
    fn tiles_overlap_so_a_line_on_a_seam_is_seen_whole_by_someone() {
        let overlap = 60;
        let cut = tiles(1000, 5000, 500_000, overlap);

        for pair in cut.windows(2) {
            let (first, second) = (pair[0], pair[1]);
            let shared = (first.y + first.height).saturating_sub(second.y);
            assert!(
                shared >= overlap || second.y + second.height == 5000,
                "tiles at {} and {} share only {shared} rows",
                first.y,
                second.y
            );
        }
    }

    #[test]
    fn every_row_of_the_page_is_covered_by_some_tile() {
        // A gap between tiles is a band of the page that is never recognised,
        // and nothing downstream would notice.
        let (width, height) = (1000u32, 4321u32);
        let cut = tiles(width, height, 400_000, 50);

        for row in (0..height).step_by(7) {
            assert!(
                cut.iter().any(|t| row >= t.y && row < t.y + t.height),
                "row {row} is in no tile"
            );
        }
    }

    #[test]
    fn a_boxs_coordinates_move_with_its_tile() {
        let tile = Tile { x: 0, y: 800, width: 1000, height: 400 };
        let moved = tile.to_image(&line(50.0, 10.0, 200.0));
        let (_, top, _, _) = moved.bounds();
        assert_eq!(top, 850.0, "the tile's offset was not applied");
    }

    #[test]
    fn the_same_line_seen_by_two_tiles_is_kept_once() {
        let full = line(100.0, 10.0, 500.0);
        let nearly_the_same = line(101.0, 12.0, 495.0);

        let merged = merge(vec![full, nearly_the_same]);
        assert_eq!(merged.len(), 1, "an overlapping line was kept twice");
    }

    #[test]
    fn the_fuller_view_of_a_clipped_line_is_the_one_kept() {
        // A line cut by a tile edge recognises as a fragment. Keeping the
        // fragment over the whole line loses half the words silently.
        let clipped = line(100.0, 10.0, 200.0);
        let whole = line(100.0, 10.0, 500.0);

        let merged = merge(vec![clipped, whole]);
        assert_eq!(merged.len(), 1);
        let (left, _, right, _) = merged[0].bounds();
        assert_eq!(right - left, 490.0, "the clipped fragment won");
    }

    #[test]
    fn different_lines_are_all_kept_and_stay_in_reading_order() {
        let merged = merge(vec![
            line(300.0, 10.0, 500.0),
            line(100.0, 10.0, 500.0),
            line(200.0, 10.0, 500.0),
        ]);

        assert_eq!(merged.len(), 3, "distinct lines were merged");
        let tops: Vec<f32> = merged.iter().map(|l| l.bounds().1).collect();
        assert_eq!(tops, vec![100.0, 200.0, 300.0], "merging reordered the page");
    }

    #[test]
    fn an_empty_page_tiles_to_nothing_rather_than_panicking() {
        assert!(tiles(0, 1000, 1000, 10).is_empty());
        assert!(tiles(1000, 0, 1000, 10).is_empty());
        assert!(merge(Vec::new()).is_empty());
    }
}
