//! Regressions found by the independent verification of the click path on the user's own documents
//! (quotations with tall stroked cells, purchase orders, receipts, a datasheet with a QR code and charts).
//! Every layout here is generated in Rust (no PDF, no external file) and states what a person would
//! call "one block": in a TABLE every CELL is its own block, a multi-line cell is one block.
//!
//! Sections: 1. boxed cells of every height, 1b. bands and one-row cells, 2a. vector art that is no text (QR code,
//! drawings, cells), 2b. chart rows (a gap that is no word space), 2c. a number over a long line, 3. mixed looks and
//! bogus stems, 4. hostile and huge input. (Tables without rules or boxes are not covered here.)

mod common;

use common::*;
use pagify_shell::blocks::*;

fn block_of<'a>(blocks: &'a [Block], object: usize) -> &'a Block {
    &blocks[block_index_of(blocks, object).unwrap_or_else(|| panic!("object {object} is in no block"))]
}

fn objects_of(blocks: &[Block], object: usize) -> Vec<usize> {
    let mut v = block_of(blocks, object).objects();
    v.sort();
    v
}

/// Every id of `ids` is alone in its block.
fn assert_alone(blocks: &[Block], ids: &[usize], what: &str) {
    for &o in ids {
        assert_eq!(objects_of(blocks, o), vec![o], "{what}: object {o} must be its own block\n");
    }
}

const BODY: Look = Look { font: 1, stem: Some(95), face: "Helvetica", size: 9.0, rgb: [0, 0, 0] };
const BOLD9: Look = Look { font: 2, stem: Some(145), face: "Helvetica-Bold", size: 9.0, rgb: [0, 0, 0] };

/// A text width at 0.5 em per character (the builder's convention).
fn tw(s: &str, size: f32) -> f32 {
    s.chars().count() as f32 * 0.5 * size
}

/// One row of cells, each drawn as its own stroked rectangle (the way the user's quotation tool draws
/// them) with one text object in it, vertically centred; `right` aligns the text to the cell's right
/// edge. Returns the text objects.
fn boxed_row(l: &mut Layout, xs: &[f32], ws: &[f32], y: f32, h: f32, cells: &[(&str, bool)], look: &Look) -> Vec<usize> {
    cells
        .iter()
        .enumerate()
        .map(|(i, &(t, right))| {
            l.shape(xs[i], y, xs[i] + ws[i], y + h);
            let w = tw(t, look.size);
            let x = if right { xs[i] + ws[i] - 4.0 - w } else { xs[i] + 4.0 };
            l.frag(x, y + h / 2.0 + 0.35 * look.size, w, look, t)
        })
        .collect()
}

/// The same row with each cell's four edges drawn as separate thin lines (two horizontal, two vertical).
fn lined_row(l: &mut Layout, xs: &[f32], ws: &[f32], y: f32, h: f32, cells: &[(&str, bool)], look: &Look) -> Vec<usize> {
    cells
        .iter()
        .enumerate()
        .map(|(i, &(t, right))| {
            l.hrule(xs[i], xs[i] + ws[i], y);
            l.hrule(xs[i], xs[i] + ws[i], y + h);
            l.vrule(xs[i], y, y + h);
            l.vrule(xs[i] + ws[i], y, y + h);
            let w = tw(t, look.size);
            let x = if right { xs[i] + ws[i] - 4.0 - w } else { xs[i] + 4.0 };
            l.frag(x, y + h / 2.0 + 0.35 * look.size, w, look, t)
        })
        .collect()
}

/// The seven columns of V3's ladder: # | Ref | Product specification | Quantity | Unit | Unit price | Total.
const COLS: [f32; 7] = [26.0, 70.0, 150.0, 46.0, 38.0, 64.0, 72.0];

fn col_xs() -> Vec<f32> {
    let mut xs = Vec::new();
    let mut x = 30.0;
    for w in COLS {
        xs.push(x);
        x += w;
    }
    xs
}

fn item_cells(n: usize) -> Vec<(String, bool)> {
    vec![(n.to_string(), false), ("HS000LC-1692".into(), false), ("Track light 15W 2700K".into(), false), ((5 + 3 * n).to_string(), true), ("PCS".into(), false), ("200.00".into(), true), (format!("{}.00", 1000 + 100 * n), true)]
}

type Ladder = (Layout, Vec<(f32, Vec<usize>)>);

/// V3's syn_box page: a header row of 22 pt and ten rows of growing height, every cell a stroked box.
fn ladder(heights: &[f32], draw: fn(&mut Layout, &[f32], &[f32], f32, f32, &[(&str, bool)], &Look) -> Vec<usize>) -> Ladder {
    let mut l = Layout::new();
    let xs = col_xs();
    let head = ["#", "Ref", "Product specification", "Quantity", "Unit", "Unit price", "Total (AED)"];
    let cells: Vec<(&str, bool)> = head.iter().map(|h| (*h, false)).collect();
    let mut rows = vec![(22.0, draw(&mut l, &xs, &COLS, 100.0, 22.0, &cells, &BOLD9))];
    let mut y = 122.0;
    for (n, &h) in heights.iter().enumerate() {
        let c = item_cells(n + 1);
        let cells: Vec<(&str, bool)> = c.iter().map(|(t, r)| (t.as_str(), *r)).collect();
        rows.push((h, draw(&mut l, &xs, &COLS, y, h, &cells, &BODY)));
        y += h;
    }
    (l, rows)
}

const LADDER: [f32; 10] = [24.0, 36.0, 48.0, 56.0, 62.0, 66.0, 70.0, 74.0, 80.0, 100.0];

// ------------------------------------------------------------------------------------------------
// 1. boxed cells of every height
// ------------------------------------------------------------------------------------------------

/// V3's ladder: rows of 24 to 100 pt at 9 pt text (2.7 to 11 em), every cell a stroked rectangle. Every cell
/// is its own block at every height; the old limit of 7.5 em (67.5 pt) merged [#|Ref|Spec] and [Qty|Unit].
#[test]
fn every_boxed_cell_is_its_own_block_at_every_row_height() {
    let (l, rows) = ladder(&LADDER, boxed_row);
    let blocks = detect(&l.frags, &l.shapes);
    assert_partition(&l.frags, &blocks);
    for (h, ids) in &rows {
        assert_alone(&blocks, ids, &format!("row {h} pt"));
    }
    assert_eq!(blocks.len(), 7 * 11);
}

/// Still true for much taller rows, up to a cell that is half a page high (66 em).
#[test]
fn a_cell_half_a_page_high_is_still_a_cell() {
    let (l, rows) = ladder(&[120.0, 200.0, 300.0, 600.0], boxed_row);
    let blocks = detect(&l.frags, &l.shapes);
    for (h, ids) in &rows {
        assert_alone(&blocks, ids, &format!("row {h} pt"));
    }
}

/// The same table with the edges of every cell drawn as four separate thin lines.
#[test]
fn cells_drawn_as_four_thin_lines_are_cells_at_every_height() {
    let (l, rows) = ladder(&LADDER, lined_row);
    let blocks = detect(&l.frags, &l.shapes);
    for (h, ids) in &rows {
        assert_alone(&blocks, ids, &format!("lined row {h} pt"));
    }
}

/// A row of boxes is a row of cells whatever the page's other furniture does: with a frame drawn around the
/// whole table (a box that holds every cell and so separates none) nothing changes.
#[test]
fn a_frame_around_the_whole_table_changes_nothing() {
    let (mut l, rows) = ladder(&LADDER, boxed_row);
    l.shape(29.0, 99.0, 497.0, 123.0 + LADDER.iter().sum::<f32>());
    let blocks = detect(&l.frags, &l.shapes);
    for (h, ids) in &rows {
        assert_alone(&blocks, ids, &format!("framed row {h} pt"));
    }
}

/// Tight rows: boxed cells 16 to 20 pt high (1.8 to 2.2 em) at equal pitch. The column of "Unit" cells would
/// read as a double-spaced paragraph without the boxes; a box that holds one line and not the next is a
/// boundary between the two lines as much as between two neighbours on a line.
#[test]
fn boxed_cells_stacked_at_equal_pitch_are_not_a_paragraph() {
    for h in [16.0f32, 18.0, 20.0, 22.0] {
        let (l, rows) = ladder(&[h; 8], boxed_row);
        let blocks = detect(&l.frags, &l.shapes);
        for (rh, ids) in &rows {
            assert_alone(&blocks, ids, &format!("tight row {h} pt (row of {rh})"));
        }
    }
}

/// The DQ-8 quotation row: nine cells 88 pt (9.8 em) high; the specification cell holds eight lines of a bold
/// label and a regular value (ONE block of 8 lines), the part reference two centred lines (one block), the
/// quantity, unit, price and total one line each with 2.6 to 3.4 em between them (each its own block).
#[test]
fn the_quotation_row_with_an_eight_line_specification_cell() {
    let mut l = Layout::new();
    let size = 6.9;
    let lab = Look { font: 2, stem: Some(145), face: "Helvetica-Bold", size, rgb: [0, 0, 0] };
    let val = Look { font: 1, stem: Some(95), face: "Helvetica", size, rgb: [0, 0, 0] };
    let norm = Look { font: 1, stem: Some(95), face: "Helvetica", size: 7.6, rgb: [0, 0, 0] };
    let xs = [29.5f32, 53.5, 131.5, 251.5, 325.5, 367.5, 395.5, 441.5, 497.5];
    let ws = [25.0f32, 79.0, 121.0, 75.0, 43.0, 29.0, 47.0, 57.0, 68.0];
    for i in 0..9 {
        l.shape(xs[i], 187.5, xs[i] + ws[i], 275.7);
    }
    let no = l.frag(40.7, 234.1, 2.0, &norm, "1");
    let reference = l.frag(65.1, 234.1, 56.2, &norm, "HS000LC-1692-T");
    let mut spec: Vec<usize> = Vec::new();
    for (i, (k, v)) in [("Wattage : ", "15W"), ("CCT : ", "2700K"), ("Beam Angle : ", ""), ("Finish : ", "Black"), ("Mounting : ", "Hanging"), ("Driver : ", ""), ("Dimming : ", "DALI"), ("IP : ", "")].iter().enumerate() {
        let base = 200.0 + 9.4 * i as f32;
        let kw = tw(k, size) * 0.9;
        spec.push(l.frag(137.0, base, kw, &lab, k));
        if !v.is_empty() {
            spec.push(l.frag(137.0 + kw + 4.4, base, tw(v, size) * 0.9, &val, v));
        }
    }
    let bold = Look { font: 2, stem: Some(145), face: "Helvetica-Bold", size: 8.4, rgb: [146, 109, 30] };
    let part_a = l.frag(256.5, 229.6, 65.0, &bold, "CAMINO ELITEE");
    let part_b = l.frag(271.4, 239.6, 35.4, &bold, "PLUS 3.0 ");
    let qty = l.frag(343.0, 234.1, 7.9, &norm, "25 ");
    let unit = l.frag(375.2, 234.1, 13.8, &norm, "PCS ");
    let price = l.frag(407.9, 234.1, 22.1, &norm, "200.00 ");
    let total = l.frag(455.2, 234.1, 29.6, &BOLD9, "5,000.00");
    let blocks = detect(&l.frags, &l.shapes);
    assert_partition(&l.frags, &blocks);
    assert_alone(&blocks, &[no, reference, qty, unit, price, total], "single-line cells");
    let mut want = spec.clone();
    want.sort();
    assert_eq!(objects_of(&blocks, spec[0]), want, "the specification cell is one block");
    assert_eq!(block_of(&blocks, spec[0]).lines.len(), 8);
    assert_eq!(objects_of(&blocks, part_a), vec![part_a, part_b], "the two-line part reference is one block");
}

/// A band (a filled rectangle) with a label at its left and an amount at its far right: two blocks.
#[test]
fn a_band_with_a_label_at_the_left_and_an_amount_at_the_right_is_two_blocks() {
    for band_h in [14.0f32, 30.0, 90.0] {
        let mut l = Layout::new();
        l.shape(30.0, 100.0, 565.0, 100.0 + band_h);
        let base = 100.0 + band_h / 2.0 + 3.0;
        let label = l.frag(34.0, base, tw("Area 1 - Living room", 9.0), &BOLD9, "Area 1 - Living room");
        let amount = l.frag(565.0 - 4.0 - tw("12,500.00", 9.0), base, tw("12,500.00", 9.0), &BOLD9, "12,500.00");
        let blocks = detect(&l.frags, &l.shapes);
        assert_alone(&blocks, &[label, amount], &format!("band {band_h} pt"));
    }
}

/// A frame (or any panel) that holds both pieces of a line separates nothing, whatever its size.
#[test]
fn a_frame_that_holds_both_pieces_separates_nothing() {
    for (w, h) in [(300.0f32, 40.0f32), (300.0, 100.0), (500.0, 400.0), (560.0, 780.0)] {
        let mut l = Layout::new();
        l.shape(20.0, 90.0, 20.0 + w, 90.0 + h);
        let a = l.frag(40.0, 120.0, 60.0, &BODY, "Wattage :");
        let b = l.frag(100.0 + 9.0, 120.0, 30.0, &BODY, "15 W");
        let blocks = detect(&l.frags, &l.shapes);
        assert_eq!(objects_of(&blocks, a), vec![a, b], "frame {w} x {h}: one line, one block");
    }
}

/// A panel that holds one piece of a line and not the other is a boundary, however large the panel is.
#[test]
fn a_large_panel_that_holds_only_one_piece_is_a_boundary() {
    for (w, h) in [(90.0f32, 40.0f32), (90.0, 100.0), (90.0, 400.0), (90.0, 780.0)] {
        let mut l = Layout::new();
        l.shape(20.0, 90.0, 20.0 + w, 90.0 + h);
        let a = l.frag(40.0, 120.0, 60.0, &BODY, "Wattage :");
        let b = l.frag(100.0 + 9.0, 120.0, 30.0, &BODY, "15 W");
        let blocks = detect(&l.frags, &l.shapes);
        assert_alone(&blocks, &[a, b], &format!("panel {w} x {h}"));
    }
}

/// A panel behind a paragraph is part of the page, not a boundary inside it: every line of the paragraph is
/// inside, and so are its pieces.
#[test]
fn a_paragraph_inside_a_big_panel_stays_one_block() {
    let mut rng = Rng(7);
    let mut l = Layout::new();
    l.shape(20.0, 80.0, 330.0, 330.0);
    let mut lines: Vec<Vec<&str>> = Vec::new();
    for i in 0..9 {
        lines.push(if i + 1 < 9 { fitted_words(&mut rng, 280.0, 9.0, 0.96) } else { words(&mut rng, 4) });
    }
    let ids = l.paragraph(35.0, 315.0, 100.0, 11.0, &BODY, &lines, 3);
    let blocks = detect(&l.frags, &l.shapes);
    let mut want: Vec<usize> = ids.iter().flatten().copied().collect();
    want.sort();
    assert_eq!(objects_of(&blocks, ids[0][0]), want);
}

/// Thousands of boxes, one of them as big as the page: the lookup must not scan them all for every gap.
#[test]
fn ten_thousand_boxed_cells_and_a_page_frame_stay_fast() {
    let mut l = Layout::new();
    l.shape(5.0, 5.0, 590.0, 3600.0);
    let xs: Vec<f32> = (0..40).map(|c| 10.0 + 14.0 * c as f32).collect();
    let mut ids = Vec::new();
    for r in 0..250 {
        let y = 20.0 + 14.0 * r as f32;
        for c in 0..40 {
            l.shape(xs[c], y, xs[c] + 14.0, y + 14.0);
            ids.push(l.frag(xs[c] + 1.0, y + 10.0, 5.0, &Look { size: 6.0, ..BODY.clone() }, "7"));
        }
    }
    let t = std::time::Instant::now();
    let blocks = detect(&l.frags, &l.shapes);
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    println!("10,000 boxed cells: {} blocks in {ms:.0} ms", blocks.len());
    assert_alone(&blocks, &ids[..40], "first row");
    assert!(ms < 3000.0, "{ms} ms");
}

// ------------------------------------------------------------------------------------------------
// 1b. bands and one-row cells
// ------------------------------------------------------------------------------------------------

/// The header of the quotation template: ONE filled band (a single rectangle, no separators) with the nine column
/// labels in it, centred over their columns. The five labels on the right are 2.2 to 2.9 em apart, a title band lies
/// directly above, and the item cells (boxes) directly below: the labels are the cells of the header row.
fn header_page() -> (Layout, Vec<usize>, Vec<usize>) {
    let mut l = Layout::new();
    let white = Look { font: 2, stem: Some(145), face: "Helvetica-Bold", size: 6.9, rgb: [255, 255, 255] };
    let title = Look { font: 1, stem: Some(95), face: "Helvetica", size: 7.6, rgb: [255, 255, 255] };
    l.shape(30.0, 158.0, 565.0, 172.0); // the title band
    l.shape(30.0, 172.0, 565.0, 188.0); // the header band
    let t = l.frag(185.0, 168.0, 225.0, &title, "Based On H.S.I Specified Fitting Requirements for Primary Lighting");
    let mut labels = Vec::new();
    for (x, w, text) in [(40.0f32, 3.7f32, "#"), (87.0, 12.0, "Ref."), (157.0, 69.0, "Product specification"), (265.0, 48.0, "Part Reference"), (333.0, 27.6, "Quantity"), (375.8, 12.8, "Unit"), (403.6, 31.1, "Unit price"), (451.4, 37.0, "Total (AED)"), (517.0, 28.3, "Remarks")] {
        labels.push(l.frag(x, 183.0, w, &white, text));
    }
    // the item cells directly under the band
    let body = Look { size: 7.4, ..BODY.clone() };
    for (x0, x1) in [(29.5f32, 54.5f32), (53.5, 132.5), (131.5, 252.5), (251.5, 326.5), (325.5, 368.5), (367.5, 396.5), (395.5, 442.5), (441.5, 498.5), (497.5, 565.5)] {
        l.shape(x0, 187.5, x1, 275.7);
    }
    let cell = l.frag(343.0, 234.1, 8.0, &body, "25");
    // the first item cells hold a text at each end of the row, so that a row 1.9 em under the header has the very
    // edges of the header: the table looks justified, and only the band itself says that the labels are cells
    let (first, last) = (l.frag(40.1, 197.0, 3.7, &body, "1"), l.frag(541.6, 197.0, 3.7, &body, "9"));
    (l, labels, vec![t, cell, first, last])
}

#[test]
fn the_labels_of_a_header_band_are_its_cells() {
    let (l, labels, others) = header_page();
    let blocks = detect(&l.frags, &l.shapes);
    assert_partition(&l.frags, &blocks);
    assert_alone(&blocks, &labels, "header label");
    assert_alone(&blocks, &others, "title band line and item cells");
    // the rule that does it: the one-row cell rule (the sparse-gap rule has the flush row under the header as veto)
    let off = detect_with(&l.frags, &l.shapes, &Params { lone_cell_gap: 1000.0, ..Params::default() });
    assert!(labels.iter().any(|&o| objects_of(&off, o).len() > 1), "without the one-row cell rule the five right-hand labels are one line");
}

/// A one-line box (a pill, a tag, a bar) with an ordinary sentence in it is not cut at its word spaces, however
/// the rule above treats wide gaps.
#[test]
fn a_sentence_in_a_one_line_bar_stays_whole() {
    let mut l = Layout::new();
    l.shape(30.0, 100.0, 330.0, 114.0);
    let a = l.line(34.0, 300.0, 110.0, &BODY, &["Based", "on", "the", "specified", "fitting", "requirements"], 1, false);
    let blocks = detect(&l.frags, &l.shapes);
    let mut want = a.clone();
    want.sort();
    assert_eq!(objects_of(&blocks, a[0]), want, "the sentence in the bar is one block");
}

/// A box that holds a second row of text is no one-row bar, even when it is as tall as one: two lines packed into a
/// box 2.4 em tall (the first with two words 1.6 em apart, a loose line) are one paragraph.
#[test]
fn a_box_that_holds_a_second_row_is_no_bar() {
    let mut l = Layout::new();
    l.shape(30.0, 94.0, 330.0, 115.6);
    let b1 = l.frag(34.0, 102.0, 60.0, &BODY, "narrow words");
    let b2 = l.frag(34.0 + 60.0 + 14.4, 102.0, 70.0, &BODY, "stretched out");
    let b3 = l.frag(34.0, 111.0, 120.0, &BODY, "and a second line here");
    let blocks = detect(&l.frags, &l.shapes);
    let mut want = vec![b1, b2, b3];
    want.sort();
    assert_eq!(objects_of(&blocks, b1), want, "{}", dump(&l.frags, &l.shapes, &blocks));
}

/// A box taller than a line and a half over one row (a panel with a single line of text and a loose gap in it) is no bar
/// either: only a box that is the row's own cell, at most 2.6 em tall, says that a gap in it separates cells.
#[test]
fn a_tall_box_over_one_row_is_no_bar() {
    let mut l = Layout::new();
    l.shape(30.0, 90.0, 330.0, 122.0); // 3.6 em tall
    let a = l.frag(34.0, 108.0, 60.0, &BODY, "narrow words");
    let b = l.frag(34.0 + 60.0 + 14.4, 108.0, 70.0, &BODY, "stretched out");
    let blocks = detect(&l.frags, &l.shapes);
    assert_eq!(objects_of(&blocks, a), vec![a, b], "{}", dump(&l.frags, &l.shapes, &blocks));
}

/// A band with a label at the left and an amount at the right 2.6 em of text away from it (closer than a hole):
/// the band is a one-row cell, so the two are two cells.
#[test]
fn a_band_whose_label_and_amount_are_close_is_still_two_cells() {
    let mut l = Layout::new();
    l.shape(30.0, 100.0, 300.0, 114.0);
    let w1 = tw("Area 1 - Hall", 9.0);
    let label = l.frag(34.0, 110.0, w1, &BOLD9, "Area 1 - Hall");
    let amount = l.frag(34.0 + w1 + 2.6 * 9.0, 110.0, tw("12,500.00", 9.0), &BOLD9, "12,500.00");
    let blocks = detect(&l.frags, &l.shapes);
    assert_alone(&blocks, &[label, amount], "band");
}

/// Neighbouring cells drawn as rectangles that share their border (each stroke 1 pt wide, so the boxes overlap by a
/// point), as the quotation tool draws them.
fn boxed_row_shared(l: &mut Layout, xs: &[f32], ws: &[f32], y: f32, h: f32, cells: &[(&str, bool)], look: &Look) -> Vec<usize> {
    let wider: Vec<f32> = ws.iter().map(|w| w + 1.0).collect();
    boxed_row(l, xs, &wider, y, h, cells, look)
}

#[test]
fn cells_that_share_their_border_are_cells_at_every_height() {
    let (l, rows) = ladder(&LADDER, boxed_row_shared);
    let blocks = detect(&l.frags, &l.shapes);
    assert_partition(&l.frags, &blocks);
    for (h, ids) in &rows {
        assert_alone(&blocks, ids, &format!("shared-border row {h} pt"));
    }
}

// ------------------------------------------------------------------------------------------------
// 2a. vector art is not outlined text
// ------------------------------------------------------------------------------------------------

/// No line of any block carries an outlined word.
fn assert_no_outlined(blocks: &[Block], what: &str) {
    for b in blocks {
        for ln in &b.lines {
            assert!(ln.outlined.is_empty(), "{what}: a line carries outlined words {:?} (block starts {})", ln.outlined, b.starts_because);
        }
    }
}

/// A QR code like the datasheet's: a 55 pt square, a few dozen path objects on a 1.67 pt lattice (3 to 7 modules
/// wide and tall, many of them word-sized and overlapping), text columns to its left and right on the same
/// baselines, and a boxed caption 15 pt under it. Every path is text-sized, and some have text on their baseline
/// within 12 em, but none is a word.
fn qr_page() -> (Layout, Vec<usize>, Vec<usize>) {
    let mut l = Layout::new();
    let mut rng = Rng(11);
    let body = Look { size: 8.0, ..BODY.clone() };
    let mut text = Vec::new();
    for k in 0..7 {
        let base = 718.0 + 9.6 * k as f32;
        text.push(l.frag(20.0, base, 70.0, &body, "text left of the code"));
        text.push(l.frag(190.0, base, 80.0, &body, "and text on the right"));
    }
    l.shape(104.0, 708.0, 160.0, 764.0); // the code's square
    let m = 1.67f32;
    let mut modules = Vec::new();
    for _ in 0..40 {
        let (cx, cy) = (rng.below(26) as f32, rng.below(26) as f32);
        let (w, h) = (3 + rng.below(5), 3 + rng.below(5));
        modules.push(l.shape(106.0 + cx * m, 710.0 + cy * m, 106.0 + (cx + w as f32) * m, 710.0 + (cy + h as f32) * m));
    }
    l.shape(103.3, 773.0, 138.4, 786.0);
    let caption = vec![l.frag(108.0, 783.0, 19.7, &body, "ROH"), l.frag(129.4, 783.0, 4.7, &body, "S")];
    (l, caption, modules)
}

#[test]
fn a_qr_code_beside_text_is_not_outlined_words() {
    let (l, caption, _) = qr_page();
    let blocks = detect(&l.frags, &l.shapes);
    assert_partition(&l.frags, &blocks);
    assert_no_outlined(&blocks, "QR code");
    assert_eq!(objects_of(&blocks, caption[0]), caption, "the caption is one block");
    assert_eq!(block_of(&blocks, caption[0]).lines.len(), 1, "of one line (no phantom line of drawn text over the code)");
}

/// The cone chart of the datasheet: a trapezoid drawn as two paths with (nearly) the same box, a number centred in
/// the line above it and another under it. Two paths on top of each other are one drawing, not two words.
fn trapezoid_page() -> (Layout, usize, usize) {
    let mut l = Layout::new();
    let small = Look { size: 8.0, ..BODY.clone() };
    let above = l.frag(505.0, 321.0, 20.0, &small, "0.85");
    l.shape(505.7, 331.3, 535.5, 337.8);
    l.shape(507.1, 331.3, 535.5, 337.8);
    let under = l.frag(505.0, 349.0, 20.0, &small, "1.06");
    (l, above, under)
}

#[test]
fn two_paths_drawn_over_each_other_are_one_drawing_not_words() {
    let (l, above, under) = trapezoid_page();
    let blocks = detect(&l.frags, &l.shapes);
    assert_no_outlined(&blocks, "trapezoid");
    assert_eq!(objects_of(&blocks, above), vec![above]);
    assert_eq!(objects_of(&blocks, under), vec![under]);
}

/// A row of table cells, each a rectangle 90 x 17 pt with one small text object inside (a receipt table): text lies
/// IN every rectangle, so no rectangle is a word, although the text covers well under a third of it.
#[test]
fn a_cell_with_a_small_text_in_it_is_a_cell_not_an_outlined_word() {
    let mut l = Layout::new();
    let st = Look { size: 12.0, ..BODY.clone() };
    let mut texts = Vec::new();
    for (i, t) in ["34.02", "10", "9.00"].iter().enumerate() {
        let x = 30.0 + 90.0 * i as f32;
        l.shape(x, 419.0, x + 90.0, 436.0);
        texts.push(l.frag(x + 30.0, 431.0, 30.0, &st, t));
    }
    // a line of text above, so that "a line of text one pitch above" supports the cells
    l.frag(30.0, 412.0, 150.0, &st, "Receipt number and date");
    let blocks = detect(&l.frags, &l.shapes);
    assert_no_outlined(&blocks, "cells");
    for &t in &texts {
        assert_eq!(objects_of(&blocks, t), vec![t]);
    }
}

/// A legend swatch (a thin coloured bar, 55 x 4.8 pt) in front of its label on the same baseline is not a word.
#[test]
fn a_legend_swatch_is_not_an_outlined_word() {
    let mut l = Layout::new();
    let small = Look { size: 7.6, ..BODY.clone() };
    l.shape(311.6, 740.5, 366.4, 745.3);
    l.frag(372.0, 745.0, 60.0, &small, "Mains power");
    l.frag(372.0, 760.0, 60.0, &small, "Dimming bus");
    let blocks = detect(&l.frags, &l.shapes);
    assert_no_outlined(&blocks, "swatch");
}

/// What the new tests must not take away: a word of the paragraph's line drawn as a path, 1.2 em of justified gap
/// on either side of it, is still bridged; so is a whole outlined line aligned with its paragraph.
#[test]
fn a_real_outlined_word_with_justified_gaps_around_it_is_still_bridged() {
    let mut rng = Rng(77);
    let mut l = Layout::new();
    let mut ids = Vec::new();
    let mut shape = 0;
    for i in 0..6 {
        let base = 100.0 + 9.6 * i as f32;
        if i == 2 {
            ids.push(l.frag(50.0, base, 70.0, &LIGHT, "text of the line"));
            shape = l.shape(129.6, base - 6.0, 167.6, base + 1.6); // 1.2 em (9.6 pt) from the text on both sides
            ids.push(l.frag(177.2, base, 72.0, &LIGHT, "goes on to the margin"));
        } else {
            let w = fitted_words(&mut rng, 200.0, 8.0, 0.93);
            ids.extend(l.line(50.0, 250.0, base, &LIGHT, &w, 1, true));
        }
    }
    let blocks = detect(&l.frags, &l.shapes);
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].lines[2].outlined, vec![shape]);
}

// ------------------------------------------------------------------------------------------------
// 2b. chart rows: a gap that is no word space
// ------------------------------------------------------------------------------------------------

const TICK: Look = Look { font: 6, stem: Some(51), face: "Montserrat-Light", size: 5.97, rgb: [0x6d, 0x6e, 0x71] };
const CAPTION: Look = Look { font: 7, stem: Some(125), face: "Montserrat-SemiBold", size: 5.97, rgb: [0x58, 0x59, 0x5b] };

/// The datasheet's wattage chart: six x tick labels on one baseline 23.4 pt apart (gaps 2.3 to 2.55 em at 5.97 pt),
/// the axis title ('Ambient temperature') 1.3 em under them in another weight, and y tick labels in a column
/// above the first one. Every label is one block; the title is one block.
fn tick_page() -> (Layout, Vec<usize>, usize, Vec<usize>) {
    let mut l = Layout::new();
    let mut y_labels = Vec::new();
    for (k, t) in ["19", "16", "13", "10", "7"].iter().enumerate() {
        y_labels.push(l.frag(28.7, 412.4 + 20.9 * k as f32, 5.2, &TICK, t));
    }
    let mut ticks = Vec::new();
    for (k, t) in ["30\u{b0} ", "35\u{b0} ", "40\u{b0} ", "45\u{b0} ", "50\u{b0} "].iter().enumerate() {
        ticks.push(l.frag(28.7 + 23.4 * k as f32, 510.66, 9.0, &TICK, t));
    }
    ticks.push(l.frag(145.6, 510.66, 21.4, &TICK, "55\u{b0} 60\u{b0}"));
    let title = l.frag(95.5, 518.39, 72.5, &CAPTION, "Ambient temperature \u{b0}C");
    (l, ticks, title, y_labels)
}

#[test]
fn x_tick_labels_with_wide_gaps_are_one_block_each() {
    let (l, ticks, title, y) = tick_page();
    let blocks = detect(&l.frags, &l.shapes);
    assert_partition(&l.frags, &blocks);
    assert_alone(&blocks, &ticks, "tick label");
    assert_alone(&blocks, &[title], "axis title");
    assert_alone(&blocks, &y, "y tick label");
    // the rule that does it
    let off = detect_with(&l.frags, &l.shapes, &Params { sparse_gap: 1000.0, ..Params::default() });
    assert!(ticks.iter().any(|&o| objects_of(&off, o).len() > 1), "without it the tick row is one line");
}

#[test]
fn a_second_axis_title_3_em_from_the_first_is_its_own_block() {
    // 'Cone diameter(M)' and 'Illuminance(lx)' on one baseline of a 4.8 pt chart, 2.9 em apart, and the legend
    // line of the second curve under the first of them
    let mut l = Layout::new();
    let tiny = Look { size: 4.81, ..TICK };
    let a = l.frag(490.3, 335.1, 35.5, &tiny, "Cone diameter(M)");
    let b = l.frag(539.9, 335.2, 28.0, &tiny, "Illuminance(lx)");
    let c = l.frag(479.3, 342.0, 63.2, &tiny, "C0 - C180 (Half value angle : 24\u{b0})");
    let blocks = detect(&l.frags, &l.shapes);
    assert_eq!(objects_of(&blocks, b), vec![b], "the second title is its own block");
    assert!(!objects_of(&blocks, a).contains(&b) && !objects_of(&blocks, c).contains(&b));
}

#[test]
fn two_short_labels_3_4_em_apart_are_two_blocks() {
    let mut l = Layout::new();
    let st = Look { size: 5.97, ..TICK };
    let a = l.frag(300.0, 200.0, 8.0, &st, "30\u{b0}");
    let b = l.frag(300.0 + 8.0 + 3.4 * 5.97, 200.0, 8.0, &st, "15\u{b0}");
    // a caption line under them that spans the gap: the pair is not isolated, the corridor is blocked
    l.frag(296.0, 207.5, 60.0, &CAPTION, "Half value angle");
    let blocks = detect(&l.frags, &l.shapes);
    assert_alone(&blocks, &[a, b], "label");
    let off = detect_with(&l.frags, &l.shapes, &Params { sparse_gap: 1000.0, ..Params::default() });
    assert_eq!(objects_of(&off, a).len(), 2, "without the sparse-gap rule a blocked 3.4 em gap is no cut");
}

/// The cost of the rule if it were careless: a justified paragraph in which one line is stretched to a 2.4 em gap
/// stays one block, in the middle of the paragraph and at its first line (justified neighbours are the proof).
#[test]
fn a_loose_justified_line_does_not_break_its_paragraph() {
    for loose_line in [0usize, 3] {
        let mut rng = Rng(21);
        let mut l = Layout::new();
        let mut ids = Vec::new();
        for i in 0..7 {
            let base = 100.0 + 9.6 * i as f32;
            if i == loose_line {
                // two groups of words with a 19 pt (2.4 em) gap between them, justified over the full column
                let a = l.frag(50.0, base, 80.0, &LIGHT, "the optics of");
                let b = l.frag(50.0 + 80.0 + 19.2, base, 200.0 - 99.2, &LIGHT, "the light");
                ids.push(a);
                ids.push(b);
            } else {
                let w = fitted_words(&mut rng, 200.0, 8.0, 0.93);
                ids.extend(l.line(50.0, 250.0, base, &LIGHT, &w, 1, i + 1 < 7));
            }
        }
        let blocks = detect(&l.frags, &l.shapes);
        assert_eq!(blocks.len(), 1, "loose line {loose_line}\n{}", dump(&l.frags, &l.shapes, &blocks));
    }
}

// ------------------------------------------------------------------------------------------------
// 3. mixed looks and bogus stems
// ------------------------------------------------------------------------------------------------

fn look(font: u32, stem: u16, face: &'static str, size: f32, rgb: [u8; 3]) -> Look {
    Look { font, stem: Some(stem), face, size, rgb }
}

/// The quotation's letterhead: a 13 pt bold title, then ONE line that holds two looks (the company name in 8.6 pt
/// black bold and the quotation number in 7.4 pt blue bold, 1.2 em apart: a line with no dominant look), then a
/// regular 7.6 pt line. Title, name line and registration line are three blocks.
#[test]
fn a_title_over_a_two_look_line_over_a_plain_line_is_three_blocks() {
    let mut l = Layout::new();
    let title = l.frag(250.8, 60.0, 93.5, &look(1, 145, "Helvetica-Bold", 13.0, [0, 0, 0]), "Draft Quotation");
    let name = l.frag(30.0, 74.0, 155.9, &look(1, 145, "Helvetica-Bold", 8.6, [0, 0, 0]), "Company Name & Lighting Equip Tr");
    let reference = l.frag(196.3, 74.0, 202.6, &look(1, 145, "Helvetica-Bold", 7.4, [0x3c, 0x46, 0x5a]), "DQ-8-ACMEFASHION-PROJECTXX-HS-2026-SF-8-REV-00");
    let trn = l.frag(30.2, 87.0, 92.8, &look(2, 95, "Helvetica", 7.6, [0, 0, 0]), "TRN No-100000000000003");
    let blocks = detect(&l.frags, &l.shapes);
    assert_eq!(objects_of(&blocks, title), vec![title], "the title");
    assert_eq!(objects_of(&blocks, name), vec![name, reference], "the company line");
    assert_eq!(objects_of(&blocks, trn), vec![trn], "the registration line");
    assert_eq!(block_of(&blocks, name).starts_because, "size");
}

/// A bold 8 pt label with a 7 pt value on one line (a line with no dominant look) over the regular 8 pt text of a
/// clause: the label line is not the first line of the clause.
#[test]
fn a_label_and_value_line_is_not_the_first_line_of_the_paragraph_under_it() {
    let mut l = Layout::new();
    let lab = look(1, 346, "Times-Bold", 8.0, [0x22, 0x22, 0x22]);
    let val = look(2, 284, "Times-Roman", 7.0, [0, 0, 0]);
    let txt = look(2, 284, "Times-Roman", 8.0, [0, 0, 0]);
    let a = l.frag(22.6, 683.1, 92.7, &lab, "Receiver name & Contact :");
    let b = l.frag(117.3, 683.1, 132.1, &val, "Amit Singh | M +971 56 993 1923 Tony Bose");
    let mut para = Vec::new();
    for (i, t) in ["The amount stated in the PO includes VAT, all other taxes, levies, imports, duties,", "charges, fees and withholdings of any nature now or hereafter imposed by any", "governmental, fiscal or other authority, as applicable as per delivery terms."].iter().enumerate() {
        para.push(l.frag(22.6, 692.0 + 9.0 * i as f32, if i < 2 { 520.0 } else { 440.0 }, &txt, t));
    }
    let blocks = detect(&l.frags, &l.shapes);
    assert_eq!(objects_of(&blocks, a), vec![a, b], "the label line");
    let mut want = para.clone();
    want.sort();
    assert_eq!(objects_of(&blocks, para[0]), want, "the clause");
}

/// What must stay whole: the 8 lines of a specification cell (each a bold label and a regular value of one size,
/// some lines all label): they share their looks, so the mixed-look rule does not cut them.
#[test]
fn lines_that_share_a_look_are_not_cut_by_the_mixed_look_rule() {
    let mut l = Layout::new();
    let lab = look(1, 145, "Helvetica-Bold", 6.9, [0, 0, 0]);
    let val = look(2, 95, "Helvetica", 6.9, [0, 0, 0]);
    let mut ids = Vec::new();
    for (i, (k, v)) in [("Wattage : ", "15W"), ("CCT : ", "2700K"), ("Beam Angle : ", ""), ("Finish : ", "Black"), ("Mounting : ", "Hanging"), ("Driver : ", "Osram 500mA constant"), ("Dimming : ", "DALI"), ("IP : ", "")].iter().enumerate() {
        let base = 200.0 + 9.4 * i as f32;
        ids.push(l.frag(137.0, base, tw(k, 6.9) * 0.9, &lab, k));
        if !v.is_empty() {
            ids.push(l.frag(137.0 + tw(k, 6.9) * 0.9 + 4.4, base, tw(v, 6.9) * 0.9, &val, v));
        }
    }
    let blocks = detect(&l.frags, &l.shapes);
    ids.sort();
    assert_eq!(objects_of(&blocks, ids[0]), ids, "the specification cell is one block of eight lines");
    assert_eq!(block_of(&blocks, ids[0]).lines.len(), 8);
}

/// The stems of a Smallpdf page: one font embedded twice (a CID TrueType resource whose stem measures 134 and a
/// WinAnsi TrueType resource that measures 476), the same face name, size and colour: a 4-line paragraph in which
/// the second resource carries the last lines is one paragraph, not two.
#[test]
fn a_bogus_stem_does_not_split_a_paragraph_of_one_face() {
    let mut rng = Rng(5);
    let mut l = Layout::new();
    let a = look(1, 134, "SourceSansPro-Regular", 14.0, [26, 26, 26]);
    let b = look(2, 476, "SourceSansPro-Regular", 14.0, [26, 26, 26]);
    let mut ids = Vec::new();
    for i in 0..4 {
        let w = fitted_words(&mut rng, 400.0, 14.0, 0.92);
        let lk = if i < 2 { &a } else { &b };
        ids.extend(l.line(72.0, 472.0, 100.0 + 17.0 * i as f32, lk, &w, 1, i < 3));
    }
    let blocks = detect(&l.frags, &l.shapes);
    ids.sort();
    assert_eq!(objects_of(&blocks, ids[0]), ids, "one paragraph\n{}", dump(&l.frags, &l.shapes, &blocks));
}

#[test]
fn the_font_style_rule_with_stems_that_cannot_be_real() {
    // (stem a, name a, stem b, name b, same style?)
    let cases: &[(Option<u16>, &str, Option<u16>, &str, bool)] = &[
        (Some(134), "SourceSansPro-Regular", Some(476), "SourceSansPro-Regular", true), // the Smallpdf resources
        (Some(500), "Arial-BoldMT", Some(145), "Arial-BoldMT", true),                   // an invoice cell
        (Some(507), "FRGHPE+Montserrat-ExtraBold", Some(198), "Montserrat-ExtraBold", true),
        (Some(456), "Calibri", Some(84), "Calibri", true),
        (Some(476), "SourceSansPro-Regular", Some(134), "Calibri", false), // a bogus stem says nothing, the names still differ
        (Some(500), "", Some(74), "Calibri", true),                         // no evidence either way
        (Some(51), "Montserrat-Thin", Some(198), "Montserrat-Thin", false), // the datasheet's trap: Light and ExtraBold, one name
        (Some(60), "Arial", Some(250), "Arial", false),                      // 4.2 times apart but possible weights (Light and Black): two styles
        (Some(74), "Arial", Some(290), "Arial", false),                      // 3.9 times apart: a regular and a black weight, two styles
    ];
    for &(sa, fa, sb, fb, same) in cases {
        assert_eq!(same_font_style(sa, fa, sb, fb, 12), same, "{sa:?} {fa} vs {sb:?} {fb}");
        assert_eq!(same_font_style(sb, fb, sa, fa, 12), same, "symmetric: {sb:?} {fb} vs {sa:?} {fa}");
    }
}

/// A hyperlink in a paragraph: blue words in the middle of a line of black text (the Smallpdf page's links split
/// over two font resources, the stems of which cannot be trusted): the line's colour is the colour of most of its
/// characters, black, not the colour of the first fragment of its dominant font resource.
#[test]
fn a_blue_link_in_a_line_does_not_make_the_line_blue() {
    let mut l = Layout::new();
    let (plain, other) = (look(1, 134, "SourceSansPro-Regular", 14.0, [26, 26, 26]), look(2, 476, "JFGNNE+SourceSansPro-Regular", 14.0, [26, 26, 26]));
    let link = look(1, 134, "SourceSansPro-Regular", 14.0, [0, 0x55, 0xff]);
    let mut ids = Vec::new();
    ids.push(l.frag(289.2, 517.9, 257.0, &other, "You can access files stored on Smallpdf from"));
    ids.push(l.frag(289.4, 537.9, 247.0, &plain, "your computer, phone, or tablet. We\u{2019}ll also"));
    // the third line: 'sync files from the ' (font 2) + the link (font 1, blue) + ' to our ' (font 1)
    ids.push(l.frag(289.6, 557.9, 105.9, &other, "sync files from the "));
    ids.push(l.frag(398.9, 557.9, 119.8, &link, "Smallpdf Mobile App "));
    ids.push(l.frag(519.4, 557.9, 37.8, &plain, " to our "));
    ids.push(l.frag(289.8, 577.9, 73.8, &plain, "online portal"));
    let blocks = detect(&l.frags, &l.shapes);
    ids.sort();
    assert_eq!(objects_of(&blocks, ids[0]), ids, "one paragraph\n{}", dump(&l.frags, &l.shapes, &blocks));
}

// ------------------------------------------------------------------------------------------------
// 2c. a label of a few characters over a long line
// ------------------------------------------------------------------------------------------------

/// The y labels of the datasheet's first chart: '16' stands 1.8 em over the row of x tick labels (one object, 27
/// characters), on the same left edge. Two characters on a line over a line 12 times as wide are not the first
/// line of a paragraph: the next word would have fitted on it many times over.
#[test]
fn a_two_character_label_over_a_long_line_is_not_its_first_line() {
    let mut l = Layout::new();
    let a = l.frag(21.9, 484.04, 5.3, &TICK, "16");
    // (the tick row shares its row with another column's text 1.4 pt higher: 9.6 pt, 1.6 em, under the label)
    let b = l.frag(21.9, 493.64, 138.3, &TICK, "30\u{b0} 35\u{b0} 40\u{b0} 45\u{b0} 50\u{b0} 55\u{b0} 60\u{b0}");
    let blocks = detect(&l.frags, &l.shapes);
    assert_alone(&blocks, &[a, b], "label and tick row");
    let off = detect_with(&l.frags, &l.shapes, &Params { tiny_line: 0, ..Params::default() });
    assert_eq!(objects_of(&off, a).len(), 2, "without the rule the two lines are one block");
}

/// ... but a line that is short because the next word is long (a ragged paragraph whose second line starts with a
/// word that did not fit on the first) stays with it.
#[test]
fn a_short_line_that_could_not_take_the_next_word_stays_in_its_paragraph() {
    let mut l = Layout::new();
    let st = look(1, 74, "Helvetica", 10.0, [0, 0, 0]);
    // measure 200 pt; line 1 is 'ID 7' (20 pt) and the next word is 190 pt wide: it did not fit
    let a = l.frag(50.0, 100.0, 20.0, &st, "ID 7");
    let b = l.frag(50.0, 112.0, 190.0, &st, "internationalization-considerations");
    let c = l.frag(50.0, 124.0, 120.0, &st, "for details of the text");
    let blocks = detect(&l.frags, &l.shapes);
    let mut want = vec![a, b, c];
    want.sort();
    assert_eq!(objects_of(&blocks, a), want, "{}", dump(&l.frags, &l.shapes, &blocks));
}

/// A short WORD on a line of its own (the end of a line the author broke by hand, in a table cell) over a longer line
/// is not cut: 'LED' over 'DRIVER (8 LIGHTS IN ONE DALI DRIVER)' is one description.
#[test]
fn a_short_word_over_a_longer_line_is_not_cut() {
    let mut l = Layout::new();
    let st = look(1, 74, "Helvetica", 7.0, [0, 0, 0]);
    let a = l.frag(100.0, 200.0, 95.0, &st, "3W 2700K LED ADJUSTABLE SPOTLIGHT WITH DALI");
    let b = l.frag(100.0, 208.4, 10.5, &st, "LED");
    let c = l.frag(100.0, 216.8, 90.0, &st, "DRIVER (8 LIGHTS IN ONE DALI DRIVER)");
    let blocks = detect(&l.frags, &l.shapes);
    let mut want = vec![a, b, c];
    want.sort();
    assert_eq!(objects_of(&blocks, a), want, "{}", dump(&l.frags, &l.shapes, &blocks));
}

/// A number over another tiny line over a long line (the quantity and the unit of an item, then its description):
/// only a tiny line over a LONG one is cut, so the stack stays one block.
#[test]
fn a_number_over_another_tiny_line_is_not_cut() {
    let mut l = Layout::new();
    let st = look(1, 74, "Helvetica", 7.0, [0, 0, 0]);
    let a = l.frag(100.0, 200.0, 4.0, &st, "5");
    let b = l.frag(100.0, 208.4, 12.0, &st, "pcs");
    let c = l.frag(100.0, 216.8, 110.0, &st, "and a longer third line of the cell");
    let blocks = detect(&l.frags, &l.shapes);
    let mut want = vec![a, b, c];
    want.sort();
    assert_eq!(objects_of(&blocks, a), want, "{}", dump(&l.frags, &l.shapes, &blocks));
}

// ------------------------------------------------------------------------------------------------
// 4. hostile and huge input
// ------------------------------------------------------------------------------------------------

/// A box has no size limit, so a path with an edge at a million points (a clip, an error) must not cut every line
/// it crosses: it is no drawn panel.
#[test]
fn a_path_far_off_the_page_cuts_nothing() {
    let mut rng = Rng(3);
    let mut l = Layout::new();
    let mut lines: Vec<Vec<&str>> = Vec::new();
    for i in 0..8 {
        lines.push(if i + 1 < 8 { fitted_words(&mut rng, 200.0, 9.0, 0.96) } else { words(&mut rng, 3) });
    }
    let ids = l.paragraph(50.0, 250.0, 100.0, 11.0, &BODY, &lines, 3);
    let mut all: Vec<usize> = ids.iter().flatten().copied().collect();
    all.sort();
    let clean = detect(&l.frags, &l.shapes);
    assert_eq!(objects_of(&clean, all[0]), all, "the paragraph is one block to begin with");
    // a page-sized frame and two paths whose edges are at 1e6 / 1e30 pt in the middle of the text
    l.shape(20.0, 60.0, 280.0, 400.0);
    l.shape(150.0, 90.0, 1.0e6, 1.0e6);
    l.shape(150.0, 90.0, 1.0e30, 1.0e30);
    let got = detect(&l.frags, &l.shapes);
    assert_eq!(canonical(&clean), canonical(&got), "the far-off paths change nothing");
}

/// Tens of thousands of text-sized paths on a page (a plot, a map): the lookups are windowed and capped, and none of
/// the paths is a word.
#[test]
fn forty_thousand_text_sized_paths_stay_fast_and_are_no_words() {
    let mut l = Layout::new();
    let body = Look { size: 8.0, ..LIGHT.clone() };
    for r in 0..30 {
        l.frag(20.0, 100.0 + 9.6 * r as f32, 200.0, &body, "some text of the page here");
    }
    for r in 0..100 {
        for c in 0..400 {
            l.shape(20.0 + c as f32 * 3.0, 400.0 + r as f32 * 2.0, 30.0 + c as f32 * 3.0, 408.0 + r as f32 * 2.0);
        }
    }
    let t = std::time::Instant::now();
    let blocks = detect(&l.frags, &l.shapes);
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    println!("40,000 overlapping text-sized paths: {} blocks in {ms:.0} ms", blocks.len());
    assert_no_outlined(&blocks, "plot");
    assert!(ms < 2000.0, "{ms} ms");
}

/// Copies `src` into `dst`, `dy` points lower, with fresh object ids.
fn append(dst: &mut Layout, src: &Layout, dy: f32) {
    let base = dst.next;
    for f in &src.frags {
        let mut g = f.clone();
        g.object = f.object + base;
        g.top += dy;
        g.bottom += dy;
        g.baseline += dy;
        dst.frags.push(g);
    }
    for s in &src.shapes {
        dst.shapes.push(Shape { object: s.object + base, top: s.top + dy, bottom: s.bottom + dy, ..*s });
    }
    dst.next = base + src.next;
}

/// One page with every layout of this file on it: boxed tables, a QR code, the chart rows, the quotation header.
fn everything_page() -> Layout {
    let mut page = Layout::new();
    let mut dy = 0.0;
    for sub in [ladder(&LADDER, boxed_row_shared).0, qr_page().0, tick_page().0, header_page().0, trapezoid_page().0] {
        append(&mut page, &sub, dy);
        dy += 900.0;
    }
    page
}

/// The blocks of a page as sorted (text objects, outlined words) pairs.
fn with_outlined(blocks: &[Block]) -> Vec<(Vec<usize>, Vec<usize>)> {
    let mut v: Vec<(Vec<usize>, Vec<usize>)> = blocks.iter().map(|b| (b.objects(), b.lines.iter().flat_map(|l| l.outlined.iter().copied()).collect())).collect();
    v.sort();
    v
}

/// Scaling the page, shuffling its objects and translating it change no block and no outlined word: every new
/// threshold is in em of the local size or of the page's body size.
#[test]
fn the_new_rules_are_scale_shuffle_and_translation_invariant() {
    let page = everything_page();
    let base = detect(&page.frags, &page.shapes);
    assert_partition(&page.frags, &base);
    let blocks = with_outlined(&base);
    for k in [0.5f32, 2.0, 3.7, 6.0] {
        let (f, s) = scaled(&page.frags, &page.shapes, k);
        assert_eq!(with_outlined(&detect(&f, &s)), blocks, "scaled by {k}");
    }
    let mut rng = Rng(99);
    let (mut f, mut s) = (page.frags.clone(), page.shapes.clone());
    rng.shuffle(&mut f);
    rng.shuffle(&mut s);
    assert_eq!(with_outlined(&detect(&f, &s)), blocks, "shuffled");
    let moved: Vec<Frag> = page.frags.iter().map(|g| Frag { left: g.left + 40.0, right: g.right + 40.0, ..g.clone() }).collect();
    let moved_shapes: Vec<Shape> = page.shapes.iter().map(|g| Shape { left: g.left + 40.0, right: g.right + 40.0, ..*g }).collect();
    assert_eq!(with_outlined(&detect(&moved, &moved_shapes)), blocks, "moved right");
}

/// The cone chart again, now in a drawing of six thousand dots (one point each, as a CAD plot has them) crowding the
/// band around it: the dots are no partner of a word-sized path, so the pair of trapezoid paths is still found.
#[test]
fn a_drawing_full_of_dots_still_shows_its_overlapping_paths() {
    let (mut l, above, under) = trapezoid_page();
    for r in 0..60 {
        for c in 0..100 {
            l.shape(100.0 + 4.0 * c as f32, 300.0 + 0.6 * r as f32, 101.0 + 4.0 * c as f32, 301.0 + 0.6 * r as f32);
        }
    }
    let blocks = detect(&l.frags, &l.shapes);
    assert_no_outlined(&blocks, "trapezoid in a drawing of dots");
    assert_eq!(objects_of(&blocks, above), vec![above]);
    assert_eq!(objects_of(&blocks, under), vec![under]);
}

/// A bold run-in lead ('Note:' and a longer one) at the start of a paragraph of regular text stays in it, whether the
/// lead is a few characters of the first line or most of it: the first line has a look in common with the rest.
#[test]
fn a_bold_run_in_lead_stays_with_its_paragraph() {
    for (lead, lead_w, rest, rest_w) in [("Note: ", 22.0f32, "the light output depends on the", 190.0f32), ("Important notice about the following clause: ", 170.0, "see below", 40.0)] {
        let mut l = Layout::new();
        let bold = look(1, 145, "Helvetica-Bold", 9.0, [0, 0, 0]);
        let body = look(2, 95, "Helvetica", 9.0, [0, 0, 0]);
        let mut ids = vec![l.frag(50.0, 100.0, lead_w, &bold, lead), l.frag(50.0 + lead_w + 2.0, 100.0, rest_w, &body, rest)];
        for (i, w) in [260.0f32, 262.0, 140.0].iter().enumerate() {
            ids.push(l.frag(50.0, 111.0 + 11.0 * i as f32, *w, &body, "the rest of the paragraph runs on in the body font"));
        }
        let blocks = detect(&l.frags, &l.shapes);
        ids.sort();
        assert_eq!(objects_of(&blocks, ids[0]), ids, "lead {lead:?}\n{}", dump(&l.frags, &l.shapes, &blocks));
    }
}

/// Every new tunable at extreme values (zero, tiny, huge, negative), alone and together, on a page that uses all the
/// new rules and on the datasheet: no panic, and every object still in exactly one block.
#[test]
fn extreme_values_of_the_new_tunables_never_panic_and_keep_the_partition() {
    let page = everything_page();
    let ds = load_page(2);
    let ds_frags = real_frags(&ds);
    let values = [0.0f32, 1e-6, 0.01, 0.5, 1.0, 5.0, 100.0, 1e9, -1.0];
    let mut rng = Rng(5);
    for round in 0..300 {
        let mut p = Params::default();
        macro_rules! pick {
            ($($f:ident),*) => { $( if rng.below(3) == 0 { p.$f = values[rng.below(values.len())]; } )* }
        }
        pick!(sparse_gap, flush_tol, lone_cell_h, lone_cell_gap, cell_inside, outlined_touch, art_overlap);
        if rng.below(3) == 0 {
            p.tiny_line = rng.below(40);
        }
        if rng.below(4) == 0 {
            p.mixed_looks = !p.mixed_looks;
        }
        if rng.below(3) == 0 {
            p.box_h.1 = values[rng.below(values.len())];
            p.box_w.1 = values[rng.below(values.len())];
        }
        let blocks = detect_with(&page.frags, &page.shapes, &p);
        assert_partition(&page.frags, &blocks);
        if round % 10 == 0 {
            let blocks = detect_with(&ds_frags, &ds.shapes, &p);
            assert_partition(&ds_frags, &blocks);
        }
    }
}
