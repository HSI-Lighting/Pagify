//! Linking a piece to the one above it: the predecessor/successor chains,
//! the rule and box checks that stop a link, and the style/leading
//! comparisons that cut one.
//!
//! Part of the `blocks` module — split out of the single file the design
//! review flagged (Phase 4, file splits).
use super::*;

pub(super) struct Link {
    pub(super) pred: Vec<Option<usize>>,
    pub(super) succ: Vec<Option<usize>>,
    /// why a piece without a predecessor starts a chain: "start", "rule" or "column"
    pub(super) head_why: Vec<&'static str>,
}

/// A horizontal rule between the two lines `q` (above) and `p` (below) that covers most of what the two
/// have in common: the rule of a table or a section, not the underline of a word or a hyperlink. A
/// rule that hugs the glyphs of `q` (within `underline_zone` em under its baseline) must cover nearly
/// all of it: a hyperlink underlines a phrase, a heading underline spans the heading.
pub(super) fn rule_between(furn: &Furniture, q: &Piece, p: &Piece, params: &Params) -> bool {
    let ov_l = q.l.max(p.l);
    let ov_r = q.r.min(p.r);
    let (y0, y1) = (q.base - 0.1 * q.size, p.base - 0.8 * p.size);
    let lo = furn.hrules.partition_point(|s| s.y <= y0);
    let hi = furn.hrules.partition_point(|s| s.y < y1);
    furn.hrules[lo..hi.max(lo)].iter().any(|s| {
        let need = if s.y - q.base <= params.underline_zone as f64 * q.size { params.underline_cover } else { params.rule_cover };
        s.r.min(ov_r) - s.l.max(ov_l) >= need as f64 * (ov_r - ov_l).max(1e-9)
    })
}

pub(super) fn link_pieces(pieces: &[Piece], furn: &Furniture, p: &Params, nrows: usize) -> Link {
    let n = pieces.len();
    // pieces of one row are contiguous and sorted by left: per-row ranges and prefix maxima of right edges
    let mut ranges: Vec<(usize, usize)> = vec![(0, 0); nrows];
    let mut i = 0;
    while i < n {
        let mut j = i;
        while j < n && pieces[j].row == pieces[i].row {
            j += 1;
        }
        ranges[pieces[i].row] = (i, j);
        i = j;
    }
    let mut pmr: Vec<f64> = vec![0.0; n];
    for &(lo, hi) in &ranges {
        let mut m = f64::MIN;
        for k in lo..hi {
            m = m.max(pieces[k].r);
            pmr[k] = m;
        }
    }
    let row_max_size: Vec<f64> = ranges.iter().map(|&(lo, hi)| pieces[lo..hi].iter().map(|q| q.size).fold(0.0, f64::max)).collect();

    let mut cand: Vec<Option<(usize, f64)>> = vec![None; n];
    let mut head_why: Vec<&'static str> = vec!["start"; n];
    let mut up_n: Vec<usize> = vec![0; n]; // pieces of the nearest row above that this piece overlaps
    let mut below: Vec<Vec<(usize, usize)>> = vec![Vec::new(); n]; // (row, piece) of the pieces under q that overlap it
    for i in 0..n {
        let pc = &pieces[i];
        let mut found: Option<(usize, f64)> = None;
        let mut overlapped: Vec<usize> = Vec::new();
        let mut ri = pc.row;
        while ri > 0 {
            ri -= 1;
            let (lo, hi) = ranges[ri];
            if lo == hi {
                continue;
            }
            let dy = pc.base - pieces[lo].base;
            if dy > p.near as f64 * pc.size.max(row_max_size[ri]) {
                break; // too far above (rows are in baseline order)
            }
            if found.is_some() {
                break; // nearest row that has an overlapping piece wins
            }
            let end = lo + pieces[lo..hi].partition_point(|q| q.l < pc.r);
            let start = lo + pmr[lo..hi].partition_point(|&m| m <= pc.l);
            for j in (start..end.max(start)).rev() {
                let q = &pieces[j];
                if dy > p.near as f64 * pc.size.max(q.size) {
                    continue;
                }
                let ov = pc.r.min(q.r) - pc.l.max(q.l);
                let narrow = (pc.r - pc.l).min(q.r - q.l).max(1e-9);
                if ov >= p.link_overlap as f64 * narrow && ov > 0.0 {
                    if !q.mark_only {
                        overlapped.push(j);
                    }
                    if found.map_or(true, |(_, o)| ov > o) {
                        found = Some((j, ov));
                    }
                }
            }
        }
        up_n[i] = overlapped.len();
        for &q in &overlapped {
            below[q].push((pc.row, i));
        }
        if let Some((q, ov)) = found {
            let above = &pieces[q];
            if p.rules && rule_between(furn, above, pc, p) {
                head_why[i] = "rule"; // a drawn rule lies between this line and the one above
                continue;
            }
            // a drawn cell that holds one of the two lines and not the other: two cells, not one paragraph
            if p.boxes && !above.opaque && !pc.opaque && box_separates(furn, [above.l, above.t, above.r, above.b], [pc.l, pc.t, pc.r, pc.b], pc.size.max(above.size)) {
                head_why[i] = "rule";
                continue;
            }
            cand[i] = Some((q, ov));
        }
    }
    // a line over several columns (or under several) is no part of any of them
    if p.spanning_lines {
        let spans_down: Vec<bool> = below
            .iter()
            .map(|b| {
                let nearest = b.iter().map(|x| x.0).min();
                nearest.map_or(false, |r| b.iter().filter(|x| x.0 == r).count() >= 2)
            })
            .collect();
        for i in 0..n {
            if let Some((q, _)) = cand[i] {
                if up_n[i] >= 2 || spans_down[q] {
                    cand[i] = None;
                    head_why[i] = "column";
                }
            }
        }
    }
    let mut pred = vec![None; n];
    let mut succ: Vec<Option<usize>> = vec![None; n];
    let mut best_ov = vec![0.0f64; n];
    for i in 0..n {
        if let Some((q, ov)) = cand[i] {
            if succ[q].is_none() || ov > best_ov[q] {
                if let Some(old) = succ[q] {
                    pred[old] = None;
                    head_why[old] = "column"; // the line above went on with a better-overlapping line
                }
                succ[q] = Some(i);
                best_ov[q] = ov;
                pred[i] = Some(q);
            } else {
                head_why[i] = "column";
            }
        }
    }
    Link { pred, succ, head_why }
}

// ---- blocks ----

/// The most common value in bins of width `bin`: (smallest value in that bin, its count). Ties go to the lower bin.
pub(super) fn mode_bin(vals: &[f64], bin: f64) -> Option<(f64, usize)> {
    let mut keyed: Vec<(i64, f64)> = vals.iter().map(|&v| ((v / bin).round() as i64, v)).collect();
    keyed.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
    let mut best: Option<(i64, usize, f64)> = None;
    let mut i = 0;
    while i < keyed.len() {
        let mut j = i;
        while j < keyed.len() && keyed[j].0 == keyed[i].0 {
            j += 1;
        }
        let (count, min_v) = (j - i, keyed[i].1);
        if best.map_or(true, |b| count > b.1) {
            best = Some((keyed[i].0, count, min_v));
        }
        i = j;
    }
    best.map(|b| (b.2, b.1))
}

pub(super) fn style_differs(a: &Piece, b: &Piece, fonts: &Fonts, p: &Params, body: f64) -> bool {
    !a.opaque
        && !b.opaque
        && (fonts.differs(a.font, b.font)
            || (a.size - b.size).abs() > (p.size_rel as f64 * a.size).max(p.size_rel as f64 * body)
            || colour_far(a.rgb, b.rgb, p.colour_delta))
}

/// Lines of which at least one holds more than one look (a bold label and a regular value, a company name and a
/// reference in two sizes) say nothing about style through their dominant look, but two lines that have no look in
/// common are not one text: no look of one is any look of the other (same font style, the size within
/// `size_rel`, the colour within `colour_delta`). "size" when every look of the one is more than `size_break`
/// off every look of the other, "style" otherwise. The lines of a specification cell (a bold label and a regular
/// value on one line, a bold label alone on the next) share their looks and stay together.
pub(super) fn looks_differ(a: &Piece, b: &Piece, fonts: &Fonts, p: &Params, body: f64) -> Option<&'static str> {
    if a.opaque || b.opaque || a.looks.is_empty() || b.looks.is_empty() {
        return None;
    }
    let (mut any_same, mut any_near_size) = (false, false);
    for x in &a.looks {
        for y in &b.looks {
            let size_off = (x.size - y.size).abs();
            if size_off <= (p.size_break as f64 * x.size.min(y.size)).max(0.075 * body) {
                any_near_size = true;
            }
            let differs = fonts.differs(x.font, y.font) || size_off > (p.size_rel as f64 * x.size).max(p.size_rel as f64 * body) || colour_far(x.rgb, y.rgb, p.colour_delta);
            any_same |= !differs;
        }
    }
    if any_same {
        None
    } else if any_near_size {
        Some("style")
    } else {
        Some("size")
    }
}
