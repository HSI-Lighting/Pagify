//! Rows and pieces: making rows from items, scanning for baselines,
//! joining and cutting rows into pieces, and the little text predicates
//! (list items, sentence ends, hyphens) pieces are measured with.
//!
//! Part of the `blocks` module — split out of the single file the design
//! review flagged (Phase 4, file splits).
use super::*;

pub(super) fn make_rows(items: &mut [It], p: &Params) -> Vec<Row> {
    let mut idx: Vec<usize> = (0..items.len()).collect();
    idx.sort_by(|&a, &b| {
        items[a].base.total_cmp(&items[b].base).then(items[a].l.total_cmp(&items[b].l)).then(items[a].obj.cmp(&items[b].obj)).then(items[a].opaque.cmp(&items[b].opaque))
    });
    let mut rows: Vec<Row> = Vec::new();
    for i in idx {
        let f = &items[i];
        if let Some(r) = rows.last_mut() {
            if (f.base - r.base).abs() <= p.row_tol as f64 * f.size.min(r.size0) {
                r.items.push(i);
                continue;
            }
        }
        rows.push(Row { base: f.base, size0: f.size, items: vec![i], ls: Vec::new(), rs: Vec::new(), pmr: Vec::new(), sl: Vec::new(), sr: Vec::new(), sp: Vec::new() });
    }
    for r in &mut rows {
        r.items.sort_by(|&a, &b| items[a].l.total_cmp(&items[b].l).then(items[a].obj.cmp(&items[b].obj)).then(items[a].opaque.cmp(&items[b].opaque)));
        r.ls = r.items.iter().map(|&i| items[i].l).collect();
        r.rs = r.items.iter().map(|&i| items[i].r).collect();
        let mut m = f64::MIN;
        r.pmr = r.rs.iter().map(|&x| {
            m = m.max(x);
            m
        }).collect();
        let solid: Vec<usize> = r.items.iter().copied().filter(|&i| !(p.stray_marks && items[i].mark)).collect();
        r.sl = solid.iter().map(|&i| items[i].l).collect();
        r.sr = solid.iter().map(|&i| items[i].r).collect();
        let mut m = f64::MIN;
        r.sp = r.sr.iter().map(|&x| {
            m = m.max(x);
            m
        }).collect();
    }
    rows
}

/// Walk away from row `i` (dir -1 up, +1 down) over the rows that have text near the gap [a,b] and
/// follow the *corridor*: the widest sub-interval of the gap that every row so far leaves empty.
/// A row proves a channel when the corridor survives it (>= wmin wide) AND that row has text on both
/// sides of the corridor (a short line proves nothing). Returns (proving rows, any neighbour exists).
#[allow(clippy::too_many_arguments)]
pub(super) fn scan(rows: &[Row], i: usize, dir: isize, a: f64, b: f64, wmin: f64, window: f64, max_dist: f64, eps: f64) -> (usize, bool) {
    let (mut pos, mut exists) = (0usize, false);
    let (mut cl, mut cr) = (a, b);
    let mut j = i as isize + dir;
    let mut prev = rows[i].base;
    while j >= 0 && (j as usize) < rows.len() {
        let r = &rows[j as usize];
        j += dir;
        if (r.base - prev).abs() > max_dist {
            break;
        }
        // another column's line at a different baseline phase says nothing about this gap
        let pe = r.sl.partition_point(|&l| l < b + window);
        if pe == 0 || r.sp[pe - 1] <= a - window {
            continue;
        }
        exists = true;
        // widest empty stretch of [cl,cr] in this row (fragments sorted by x)
        let s = r.sp.partition_point(|&m| m <= cl);
        let e = r.sl.partition_point(|&l| l < cr);
        let (mut best, mut at) = ((cl, cl), cl);
        for k in s..e.max(s) {
            if r.sr[k] <= cl || r.sl[k] >= cr {
                continue;
            }
            if r.sl[k] - at > best.1 - best.0 {
                best = (at, r.sl[k]);
            }
            at = at.max(r.sr[k]);
        }
        if cr - at > best.1 - best.0 {
            best = (at, cr);
        }
        if best.1 - best.0 < wmin {
            break; // blocked: text covers the gap
        }
        (cl, cr) = best;
        prev = r.base;
        // a fragment ending at the corridor's left edge, one starting at its right edge
        let mut left = false;
        let mut k = r.sl.partition_point(|&l| l <= cl + eps);
        while k > 0 {
            k -= 1;
            if r.sp[k] <= cl - window {
                break;
            }
            if r.sr[k] <= cl + eps && r.sr[k] > cl - window {
                left = true;
                break;
            }
        }
        let right = r.sl.partition_point(|&l| l < cr - eps) < r.sl.partition_point(|&l| l < cr + window);
        if left && right {
            pos += 1;
        }
    }
    (pos, exists)
}

// ---- pieces ----

/// Is there a row within `near` em of row `ri` whose left and right edges are this row's (within `flush_tol`
/// em)? A justified block has them: every line of it sits on both margins. Used to tell a stretched line of
/// justified text from a row of labels, which have no such neighbour.
pub(super) fn flush_row_near(rows: &[Row], ri: usize, em: f64, p: &Params) -> bool {
    let row = &rows[ri];
    let (l0, r0) = (row.ls[0], row.pmr[row.pmr.len() - 1]);
    let (tol, reach) = (p.flush_tol as f64 * em, p.near as f64 * em);
    let flush = |o: &Row| (o.ls[0] - l0).abs() <= tol && (o.pmr[o.pmr.len() - 1] - r0).abs() <= tol;
    let mut j = ri;
    while j > 0 {
        j -= 1;
        if row.base - rows[j].base > reach {
            break;
        }
        if flush(&rows[j]) {
            return true;
        }
    }
    let mut j = ri + 1;
    while j < rows.len() && rows[j].base - row.base <= reach {
        if flush(&rows[j]) {
            return true;
        }
        j += 1;
    }
    false
}

pub(super) fn join_text(items: &[It], idx: &[usize]) -> String {
    let mut s = String::new();
    let mut prev_r: Option<f64> = None;
    for &k in idx {
        let f = &items[k];
        if let Some(pr) = prev_r {
            if f.l - pr > 0.15 * f.size && !s.ends_with(' ') && !f.text.starts_with(' ') {
                s.push(' ');
            }
        }
        s.push_str(&f.text);
        prev_r = Some(f.r);
    }
    s
}

pub(super) fn make_piece(items: &[It], fonts: &Fonts, row: usize, base: f64, idx: Vec<usize>, pure_share: f64) -> Piece {
    let l = idx.iter().map(|&i| items[i].l).fold(f64::MAX, f64::min);
    let r = idx.iter().map(|&i| items[i].r).fold(f64::MIN, f64::max);
    let t = idx.iter().map(|&i| items[i].t).fold(f64::MAX, f64::min);
    let b = idx.iter().map(|&i| items[i].b).fold(f64::MIN, f64::max);
    // the dominant (font, size class) by non-space characters, never one minority fragment
    let mut tally: Vec<((u32, u32), usize, f64, Vec<([u8; 3], usize)>)> = Vec::new(); // key, chars, size x chars, chars per colour
    let mut total = 0usize;
    for &i in &idx {
        let f = &items[i];
        if f.opaque || f.nchars == 0 {
            continue;
        }
        total += f.nchars;
        let key = (f.font, f.size_class);
        let e = match tally.iter().position(|e| e.0 == key) {
            Some(k) => &mut tally[k],
            None => {
                tally.push((key, 0, 0.0, Vec::new()));
                tally.last_mut().unwrap()
            }
        };
        e.1 += f.nchars;
        e.2 += f.size * f.nchars as f64;
        match e.3.iter_mut().find(|c| c.0 == f.rgb) {
            Some(c) => c.1 += f.nchars,
            None => e.3.push((f.rgb, f.nchars)),
        }
    }
    // the colour of most of the characters of a set of entries (the first of equals: fragments come left to right)
    let colour_of = |entries: &mut dyn Iterator<Item = &((u32, u32), usize, f64, Vec<([u8; 3], usize)>)>| -> [u8; 3] {
        let mut sums: Vec<([u8; 3], usize)> = Vec::new();
        for e in entries {
            for &(c, n) in &e.3 {
                match sums.iter_mut().find(|x| x.0 == c) {
                    Some(x) => x.1 += n,
                    None => sums.push((c, n)),
                }
            }
        }
        let mut best = sums.first().copied().unwrap_or(([0, 0, 0], 0));
        for &x in &sums {
            if x.1 > best.1 {
                best = x;
            }
        }
        best.0
    };
    let text = join_text(items, &idx);
    let lead_script = idx.len() > 1 && items[idx[0]].script;
    let mark_only = idx.iter().all(|&i| items[i].mark);
    match tally.iter().max_by_key(|e| e.1) {
        Some(best) => Piece {
            row,
            l,
            r,
            t,
            b,
            base,
            size: best.2 / best.1 as f64,
            font: (best.0).0,
            // (a blue link in a line of black text does not make the line blue, whichever font resource carries it)
            rgb: colour_of(&mut tally.iter().filter(|e| e.0 .1 == (best.0).1 && !fonts.differs((best.0).0, e.0 .0))),
            // pure: the dominant style (any font that looks the same, one size) holds most of the line
            pure: {
                let same: usize = tally.iter().filter(|e| e.0 .1 == (best.0).1 && !fonts.differs((best.0).0, e.0 .0)).map(|e| e.1).sum();
                same as f64 >= pure_share * total as f64
            },
            looks: tally.iter().filter(|e| e.1 * 10 >= total).map(|e| PieceLook { font: (e.0).0, size: e.2 / e.1 as f64, rgb: colour_of(&mut std::iter::once(e)) }).collect(),
            opaque: false,
            lead_script,
            mark_only,
            text,
            items: idx,
        },
        None => Piece { row, l, r, t, b, base, size: items[idx[0]].size, font: NO_FONT, rgb: [0, 0, 0], pure: false, looks: Vec::new(), opaque: true, lead_script, mark_only, text, items: idx },
    }
}

/// A text object that is only a list marker: a bullet, a dash, "1.", "(a)".
pub(super) fn is_marker(f: &It) -> bool {
    let t = f.text.trim();
    !f.opaque && !t.is_empty() && t.chars().count() <= 4 && starts_item(&format!("{t} x"))
}

/// Does some drawn box (a bordered cell, a tag, a panel) hold one of the two rectangles `a` and `b`
/// ([left, top, right, bottom]) and not the other? However tall or wide the box is: a frame that holds
/// both separates nothing, a cell that holds one of them is the boundary between them. A container is
/// a path with some room around the text, not a path the size of the text itself.
pub(super) fn box_separates(furn: &Furniture, a: [f64; 4], b: [f64; 4], em: f64) -> bool {
    if furn.boxes.is_empty() {
        return false;
    }
    // a box that holds one of the two spans its vertical extent: top above fragment top + pad, bottom under fragment bottom - pad
    furn.boxes.any_spanning(a[3].min(b[3]) - 0.1875 * em, a[1].max(b[1]) + 0.1875 * em, &mut |s| box_holds(s, &a, em) != box_holds(s, &b, em))
}

/// Does the box hold the rectangle `f` ([left, top, right, bottom]): inside it, with some room around it
/// (a path the size of the text itself is no container)?
pub(super) fn box_holds(s: &BoxR, f: &[f64; 4], em: f64) -> bool {
    let (pad, room) = (0.1875 * em, 0.25 * em);
    f[0] >= s.l - pad && f[2] <= s.r + pad && f[1] >= s.t - pad && f[3] <= s.b + pad && (s.b - s.t) >= (f[3] - f[1]) + room && (s.r - s.l) >= (f[2] - f[0]) + room
}

/// A one-row cell: a drawn box at most `lone_cell_h` em tall that holds both fragments and no other row of
/// the page's text (a header bar, a tag, a one-line cell). One row of a cell cannot be justified text, so
/// a gap of `lone_cell_gap` em in it is a boundary between two cells of the bar, not a word space.
pub(super) fn lone_row_box(furn: &Furniture, rows: &[Row], ri: usize, a: &It, b: &It, em: f64, p: &Params) -> bool {
    if a.opaque || b.opaque || furn.boxes.is_empty() {
        return false;
    }
    let (ra, rb) = ([a.l, a.t, a.r, a.b], [b.l, b.t, b.r, b.b]);
    let max_h = p.lone_cell_h as f64 * em;
    furn.boxes.any_spanning(a.b.min(b.b) - 0.1875 * em, a.t.max(b.t) + 0.1875 * em, &mut |s| {
        s.b - s.t <= max_h && box_holds(s, &ra, em) && box_holds(s, &rb, em) && !other_row_in(rows, ri, s)
    })
}

/// Does a row other than `ri` have text that lies (also only partly) inside the box `s`?
pub(super) fn other_row_in(rows: &[Row], ri: usize, s: &BoxR) -> bool {
    let touches = |r: &Row| {
        let e = r.ls.partition_point(|&l| l < s.r); // fragments starting left of the box's right edge ...
        e > 0 && r.pmr[e - 1] > s.l // ... of which one ends right of its left edge
    };
    let reach = s.b - s.t + 2.0 * rows[ri].size0; // a row whose baseline is further off cannot show inside the box
    let mut j = ri;
    while j > 0 {
        j -= 1;
        if rows[ri].base - rows[j].base > reach {
            break;
        }
        // (a line's text reaches 0.25 em under its baseline, and 0.8 em above it)
        if rows[j].base + 0.25 * rows[j].size0 > s.t && touches(&rows[j]) {
            return true;
        }
    }
    let mut j = ri + 1;
    while j < rows.len() && rows[j].base - rows[ri].base <= reach {
        if rows[j].base - 0.8 * rows[j].size0 < s.b && touches(&rows[j]) {
            return true;
        }
        j += 1;
    }
    false
}

/// Is `a` inside a drawn box that `b` is not inside, or the other way round?
pub(super) fn box_between(furn: &Furniture, a: &It, b: &It, em: f64) -> bool {
    !(a.opaque || b.opaque) && box_separates(furn, [a.l, a.t, a.r, a.b], [b.l, b.t, b.r, b.b], em)
}

/// A thin vertical rule inside the gap (or just above/below the row): a column border.
pub(super) fn vrule_in_gap(furn: &Furniture, a: f64, b: f64, base: f64, em: f64) -> bool {
    let tol = 0.0625 * em;
    let lo = furn.vrules.partition_point(|s| s.l < a - tol);
    furn.vrules[lo..].iter().take_while(|s| s.l <= b + tol).any(|s| s.r <= b + tol && s.t < base + 3.0 * em && s.b > base - 3.0 * em)
}

/// A horizontal rule with y in (y0, y1] that covers at least `frac` of the span [l, r].
pub(super) fn hrule_in(furn: &Furniture, y0: f64, y1: f64, l: f64, r: f64, frac: f64) -> bool {
    let span = (r - l).max(1e-9);
    let lo = furn.hrules.partition_point(|s| s.y <= y0);
    let hi = furn.hrules.partition_point(|s| s.y <= y1);
    furn.hrules[lo..hi.max(lo)].iter().any(|s| s.r.min(r) - s.l.max(l) >= frac * span)
}

/// A row between two horizontal rules (a table row): a rule covering the row at most `band_below` em
/// under its baseline and one at most `band_above` em over it. Such a row is one unit. The first line of
/// a cell of up to `cell_lines` lines (a description that wraps, its numbers beside its first line) sits
/// up to `cell_lines - 1` line pitches (1.2 em) higher over the rule under the cell.
pub(super) fn ruled_row(items: &[It], row: &Row, furn: &Furniture, p: &Params) -> bool {
    if furn.hrules.is_empty() || row.items.len() < 2 {
        return false;
    }
    let em = row.items.iter().map(|&i| items[i].size).fold(0.0, f64::max);
    let (l, r) = (row.ls[0], row.pmr[row.pmr.len() - 1]);
    let below = p.band_below as f64 + 1.2 * p.cell_lines.saturating_sub(1) as f64;
    hrule_in(furn, row.base + 0.05 * em, row.base + below * em, l, r, 0.8) && hrule_in(furn, row.base - p.band_above as f64 * em, row.base - 0.3 * em, l, r, 0.8)
}

pub(super) fn cut_rows(items: &[It], rows: &[Row], furn: &Furniture, fonts: &Fonts, p: &Params) -> Vec<Piece> {
    let mut pieces: Vec<Piece> = Vec::new();
    for (ri, row) in rows.iter().enumerate() {
        let banded = p.ruled_bands && ruled_row(items, row, furn, p);
        let mut cur: Vec<usize> = Vec::new();
        let mut run_r = f64::MIN;
        let mut prev_size = items[row.items[0]].size;
        let mut prev_k = row.items[0];
        let mut cut_here = false; // the gap after a lone symbol that was cut off
        let mut solid = 0usize; // members of the piece under construction that are not blank text
        for (pos, &k) in row.items.iter().enumerate() {
            let f = &items[k];
            // blank text (a space object, an empty twin) has no look: it never makes a style or size
            // decision, it only occupies room
            let blank = !f.opaque && f.nchars == 0;
            if !cur.is_empty() {
                let (a, b) = (run_r, f.l);
                let g = b - a;
                let em = f.size.max(prev_size);
                let mut cut = false;
                let pf = &items[prev_k];
                let (word_gap, max_hole) = (p.word_gap as f64 * em, p.max_hole as f64 * em);
                if !p.corridor {
                    cut = g > p.gap_only as f64 * em; // the naive rule
                } else if std::mem::take(&mut cut_here) {
                    cut = true;
                } else if p.boxes && g > 0.3 * em && box_between(furn, pf, f, em) {
                    cut = true; // two different drawn cells
                } else if p.rules && g > 0.3 * em && vrule_in_gap(furn, a, b, row.base, em) {
                    cut = true; // a drawn column border
                } else if p.marker_merge && solid == 1 && is_marker(pf) && g <= p.marker_gap as f64 * em {
                    // a list marker on its own belongs to the text after it, whatever the gap says
                } else if p.boxes && g >= p.lone_cell_gap as f64 * em && lone_row_box(furn, rows, ri, pf, f, em, p) {
                    cut = true; // two cells of a one-row bar (the labels of a table's header band)
                } else if banded {
                    // a table row between two rules: the cells of one row belong together
                } else if g > word_gap {
                    let (wmin, window, near, eps) = (p.corridor_width as f64 * em, p.scan_window as f64 * em, p.near as f64 * em, 0.001 * em);
                    let (up, up_e) = scan(rows, ri, -1, a, b, wmin, window, near, eps);
                    let (dn, dn_e) = scan(rows, ri, 1, a, b, wmin, window, near, eps);
                    let pos = up + dn;
                    let isolated = !up_e && !dn_e;
                    cut = g > max_hole
                        || pos >= p.pos3
                        || (pos >= 2 && g >= p.pos2_gap as f64 * em)
                        || (pos >= 1 && g >= p.pos1_gap as f64 * em)
                        || (isolated && g >= p.iso_gap as f64 * em)
                        || (isolated && !blank && g >= p.iso_style_gap as f64 * em && (fonts.differs(pf.font, f.font) || colour_far(pf.rgb, f.rgb, p.colour_delta)))
                        || (!blank && g >= p.sparse_gap as f64 * em && !flush_row_near(rows, ri, em, p));
                }
                // a lone symbol between two lines that look like one: judged on the gap around it
                if !cut && p.corridor && p.stray_marks && f.mark {
                    if let Some(&nk) = row.items.get(pos + 1) {
                        let n = &items[nk];
                        let w = n.l - a;
                        // the symbol stands apart on both sides (a comma hugs its word)
                        if w > word_gap && g >= 0.3 * em && n.l - f.r >= 0.3 * em {
                            let (wmin, window, near, eps) = (p.corridor_width as f64 * em, p.scan_window as f64 * em, p.near as f64 * em, 0.001 * em);
                            let proofs = scan(rows, ri, -1, a, n.l, wmin, window, near, eps).0 + scan(rows, ri, 1, a, n.l, wmin, window, near, eps).0;
                            if proofs >= p.pos3 {
                                cut = true;
                                cut_here = true;
                            }
                        }
                    }
                }
                if cut {
                    pieces.push(make_piece(items, fonts, ri, row.base, std::mem::take(&mut cur), p.pure_share as f64));
                    run_r = f64::MIN;
                    solid = 0;
                }
            }
            cur.push(k);
            run_r = run_r.max(f.r);
            if !blank {
                solid += 1;
                prev_size = f.size;
                prev_k = k;
            }
        }
        if !cur.is_empty() {
            pieces.push(make_piece(items, fonts, ri, row.base, cur, p.pure_share as f64));
        }
    }
    pieces
}

// ---- list items ----

/// A line that opens with a list marker: a bullet glyph, a dash, "1.", "a.", "iv.", "(a)", "[12]", "A)".
/// ("(CCT)" must NOT match, it is an acronym.)
pub(super) fn starts_item(text: &str) -> bool {
    let t = text.trim_start();
    let c0 = match t.chars().next() {
        Some(c) => c,
        None => return false,
    };
    let marker: String = t.chars().take_while(|c| !c.is_whitespace()).collect();
    if t.chars().nth(marker.chars().count()).is_none() {
        return "•·▪◦●‣∙".contains(c0) && marker.chars().count() == 1; // a lone bullet fragment
    }
    if "•·▪◦●‣∙".contains(c0) && marker.chars().count() == 1 {
        return true;
    }
    if matches!(c0, '-' | '–' | '—' | '+' | '*' | '†' | '‡' | '§' | '¶') && marker.chars().count() == 1 {
        return true;
    }
    let m: Vec<char> = marker.chars().collect();
    let n = m.len();
    let digits = |s: &[char]| !s.is_empty() && s.len() <= 2 && s.iter().all(|c| c.is_ascii_digit());
    let roman = |s: &[char]| !s.is_empty() && s.len() <= 4 && s.iter().all(|c| "ivxIVX".contains(*c));
    if n >= 2 && m[n - 1] == '.' {
        let body = &m[..n - 1];
        return digits(body) || roman(body) || (body.len() == 1 && body[0].is_ascii_lowercase());
    }
    if n >= 3 && ((m[0] == '(' && m[n - 1] == ')') || (m[0] == '[' && m[n - 1] == ']')) {
        let body = &m[1..n - 1];
        return digits(body) || roman(body) || (body.len() <= 3 && body[0].is_ascii_alphanumeric() && body[1..].iter().all(|c| c.is_ascii_digit()));
    }
    if (2..=3).contains(&n) && m[n - 1] == ')' {
        let body = &m[..n - 1];
        return body[0].is_ascii_alphanumeric() && body[1..].iter().all(|c| c.is_ascii_digit());
    }
    false
}

/// The kind of a dotted list marker at the start of a line ("12.", "a.", "iv."): 0 number, 1 letter,
/// 2 roman numeral; None for every other line. A bullet, a dash, "(a)" or "[1]" is no dotted marker.
pub(super) fn dot_kind(text: &str) -> Option<usize> {
    let word: Vec<char> = text.trim_start().chars().take_while(|c| !c.is_whitespace()).collect();
    let n = word.len();
    if n < 2 || word[n - 1] != '.' || !starts_item(text) {
        return None;
    }
    let body = &word[..n - 1];
    Some(if body.iter().all(|c| c.is_ascii_digit()) {
        0
    } else if body.len() == 1 {
        1
    } else {
        2
    })
}

/// The left edge of a list item's text, past its marker: the continuation lines of a list item hang
/// from the text, not from the bullet, so only the text edge can be compared with them.
pub(super) fn text_left(pc: &Piece, items: &[It]) -> f64 {
    // (blank objects, spaces, are no part of the text: the marker is the first object with characters)
    let mut solid = pc.items.iter().copied().filter(|&k| items[k].opaque || items[k].nchars > 0);
    let Some(k0) = solid.next() else { return pc.l };
    let first = &items[k0];
    let t = first.text.trim();
    let marker_alone = t.chars().count() <= 4 && starts_item(&format!("{t} x"));
    let x = if marker_alone {
        solid.next().map_or(pc.l, |k| items[k].l)
    } else {
        // the marker is the first word of the object: the text starts after it, in proportion to the characters
        let n = first.text.chars().count().max(1) as f64;
        let lead = first.text.chars().take_while(|c| c.is_whitespace()).count();
        let rest: String = first.text.chars().skip(lead).collect();
        let m = rest.chars().take_while(|c| !c.is_whitespace()).count();
        let sp = rest.chars().skip(m).take_while(|c| c.is_whitespace()).count();
        first.l + (first.r - first.l) * (lead + m + sp) as f64 / n
    };
    x.max(pc.l).min(pc.r)
}

/// True when the text ends a sentence (closing quotes and brackets are looked through).
pub(super) fn ends_sentence(text: &str) -> bool {
    text.trim_end().trim_end_matches(['"', '\u{201d}', '\u{2019}', ')', ']']).ends_with(['.', '!', '?', '\u{2026}'])
}

/// True when the line ends in a hyphen: the word goes on in the next line, so the line is short for a
/// reason that does not end the paragraph (U+0002 is PDFium's mark for a hyphenation at a line end).
pub(super) fn ends_hyphen(text: &str) -> bool {
    text.trim_end().ends_with(['-', '\u{2}', '\u{ad}', '\u{2010}', '\u{2011}'])
}

/// Width of the first word of a line, from the fragments' widths (proportional to their characters).
pub(super) fn first_word_width(items: &[It], pc: &Piece) -> f64 {
    let (mut w, mut started) = (0.0, false);
    let mut prev_r: Option<f64> = None;
    for &k in &pc.items {
        let f = &items[k];
        if f.opaque {
            return w + (f.r - f.l).min(3.0 * f.size); // an outlined run: its first word is unknown
        }
        if started && prev_r.map_or(false, |pr| f.l - pr > 0.15 * f.size) {
            return w; // a visible gap ends the word
        }
        prev_r = Some(f.r);
        // a space is about half as wide as the average letter
        let n = f.text.chars().map(|c| if c.is_whitespace() { 0.5 } else { 1.0 }).sum::<f64>().max(1.0);
        let cw = (f.r - f.l) / n;
        for c in f.text.chars() {
            if c.is_whitespace() {
                if started {
                    return w;
                }
            } else {
                started = true;
                w += cw;
            }
        }
    }
    w
}
