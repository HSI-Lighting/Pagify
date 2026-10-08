//! Shapes told apart from text: words and art classified into `Furniture`,
//! the boxes and rules it holds, and outlined words added to the item list.
//!
//! Part of the `blocks` module — split out of the single file the design
//! review flagged (Phase 4, file splits).
use super::*;

pub(super) fn norm_rect(s: &Shape) -> Option<[f64; 4]> {
    let r = [s.left as f64, s.top as f64, s.right as f64, s.bottom as f64];
    // (no page is 1e5 pt (35 m) wide: a path that far out is a clip or an error, not a drawn panel, and a
    // box has no size limit now, so one with an edge at 1e30 would cut every line it crosses)
    if !r.iter().all(|v| v.is_finite() && v.abs() <= MAX_COORD) {
        return None;
    }
    Some([r[0].min(r[2]), r[1].min(r[3]), r[0].max(r[2]), r[1].max(r[3])])
}

/// Rules, and boxes that could hold a cell of text. Only depth 0, only finite shapes; the limits are
/// em of the page's body size.
pub(super) fn classify_shapes(shapes: &[Shape], body: f64, p: &Params) -> Furniture {
    let mut f = Furniture::default();
    let mut boxes: Vec<BoxR> = Vec::new();
    let thick = p.rule_thick as f64 * body;
    let long = p.rule_len as f64 * body;
    for s in shapes.iter().filter(|s| s.depth == 0) {
        let Some([l, t, r, b]) = norm_rect(s) else { continue };
        let (w, h) = (r - l, b - t);
        if h <= thick && w >= long {
            f.hrules.push(HRule { y: (t + b) * 0.5, l, r });
        } else if w <= thick && h >= long {
            f.vrules.push(VRule { l, r, t, b });
        } else if (p.box_w.0 as f64 * body..=p.box_w.1 as f64 * body).contains(&w) && (p.box_h.0 as f64 * body..=p.box_h.1 as f64 * body).contains(&h) {
            boxes.push(BoxR { l, t, r, b });
        }
    }
    f.hrules.sort_by(|a, b| a.y.total_cmp(&b.y).then(a.l.total_cmp(&b.l)).then(a.r.total_cmp(&b.r)));
    f.vrules.sort_by(|a, b| a.l.total_cmp(&b.l).then(a.t.total_cmp(&b.t)).then(a.r.total_cmp(&b.r)).then(a.b.total_cmp(&b.b)));
    f.boxes = BoxIndex::new(boxes);
    f
}

/// Word-sized paths (object id, rect), in object order. A shape can be both a word candidate and a box.
pub(super) fn furn_words(shapes: &[Shape], body: f64, p: &Params) -> Vec<(usize, [f64; 4])> {
    let thick = p.rule_thick as f64 * body;
    let long = p.rule_len as f64 * body;
    let mut out: Vec<(usize, [f64; 4])> = Vec::new();
    for s in shapes.iter().filter(|s| s.depth == 0) {
        let Some(rc) = norm_rect(s) else { continue };
        let (w, h) = (rc[2] - rc[0], rc[3] - rc[1]);
        if (h <= thick && w >= long) || (w <= thick && h >= long) {
            continue;
        }
        if (p.word_w.0 as f64 * body..=p.word_w.1 as f64 * body).contains(&w) && (p.word_h.0 as f64 * body..=p.word_h.1 as f64 * body).contains(&h) {
            out.push((s.object, rc));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out.dedup_by_key(|x| x.0);
    out
}

/// The paths a word-sized path could lie on top of: every drawn path that is no rule and neither much
/// bigger nor much smaller than a word (up to 4 em tall, 100 em wide, at least an eighth of the smallest
/// word's area), sorted by top. (Bigger ones are containers: a cell, a panel; a drawing's dots are no partner.)
pub(super) fn furn_art(shapes: &[Shape], body: f64, p: &Params) -> Vec<(usize, [f64; 4])> {
    let thick = p.rule_thick as f64 * body;
    let long = p.rule_len as f64 * body;
    let least_area = p.word_w.0 as f64 * body * p.word_h.0 as f64 * body / 8.0;
    let mut out: Vec<(usize, [f64; 4])> = Vec::new();
    for s in shapes.iter().filter(|s| s.depth == 0) {
        let Some(rc) = norm_rect(s) else { continue };
        let (w, h) = (rc[2] - rc[0], rc[3] - rc[1]);
        if (h <= thick && w >= long) || (w <= thick && h >= long) || w > 100.0 * body || h > ART_MAX_EM * body || w * h < least_area {
            continue;
        }
        out.push((s.object, rc));
    }
    out.sort_by(|a, b| a.1[1].total_cmp(&b.1[1]).then(a.0.cmp(&b.0)));
    out.dedup_by_key(|x| x.0);
    out
}

/// The tallest path (em of body) that counts as drawing next to a word-sized path.
pub(super) const ART_MAX_EM: f64 = 4.0;

/// The most paths one word-sized path is compared with (a page with tens of thousands of paths in one
/// band of height is a plot: the first few thousand are looked at).
// ponytail: the first 4096 paths of the band by top decide; a spatial index by x would see them all.
pub(super) const MAX_ART_NEIGHBOURS: usize = 4096;

/// Does another path of comparable size lie (mostly) on the path `id` with box `rc`? Two paths of one
/// drawing on top of each other, the modules of a QR code. `art` is `furn_art`'s list.
pub(super) fn overlapped_by_art(art: &[(usize, [f64; 4])], id: usize, rc: [f64; 4], body: f64, share: f64) -> bool {
    let [l, t, r, b] = rc;
    let area = (r - l) * (b - t);
    let lo = art.partition_point(|a| a.1[1] < t - ART_MAX_EM * body);
    let hi = art.partition_point(|a| a.1[1] < b);
    art[lo..hi.max(lo)].iter().take(MAX_ART_NEIGHBOURS).any(|(oid, o)| {
        if *oid == id {
            return false;
        }
        let oa = (o[2] - o[0]) * (o[3] - o[1]);
        if oa <= 0.0 || oa > 8.0 * area || oa * 8.0 < area {
            return false;
        }
        let inter = (r.min(o[2]) - l.max(o[0])).max(0.0) * (b.min(o[3]) - t.max(o[1])).max(0.0);
        inter >= share * area.min(oa)
    })
}

/// Turn text-sized paths into placeholder items so a hole in a line is a member of the line. A path
/// only counts as outlined text if real text supports it (text on its own baseline close beside it, or
/// lines of text one pitch above or below it in its column) and no text lies on it (a path with more
/// than `cover_max` of its area under text is a cell or an underline box: outlined words have no text
/// where they are).
pub(super) fn add_outlined(items: &mut Vec<It>, words: &[(usize, [f64; 4])], shapes: &[Shape], body: f64, p: &Params) {
    if words.is_empty() {
        return;
    }
    // text items sorted by baseline (support) and by top (coverage), for windowed lookups
    let mut by_base: Vec<usize> = (0..items.len()).collect();
    by_base.sort_by(|&a, &b| items[a].base.total_cmp(&items[b].base).then(items[a].l.total_cmp(&items[b].l)).then(items[a].obj.cmp(&items[b].obj)));
    let bases: Vec<f64> = by_base.iter().map(|&i| items[i].base).collect();
    let mut by_top: Vec<usize> = (0..items.len()).collect();
    by_top.sort_by(|&a, &b| items[a].t.total_cmp(&items[b].t).then(items[a].obj.cmp(&items[b].obj)));
    let tops: Vec<f64> = by_top.iter().map(|&i| items[i].t).collect();
    let max_h = items.iter().map(|i| i.b - i.t).fold(0.0, f64::max).min(4.0 * body);
    let mut add: Vec<It> = Vec::new();
    let mut art: Option<Vec<(usize, [f64; 4])>> = None; // the paths a word could lie on, listed when a path first needs them
    for &(id, [l, t, r, b]) in words {
        let (w, h) = (r - l, b - t);
        // text lying on the path: not outlined text
        let lo = tops.partition_point(|&y| y < t - max_h);
        let hi = tops.partition_point(|&y| y < b);
        let mut covered = 0.0;
        let mut holds_text = false;
        for &k in &by_top[lo..hi.max(lo)] {
            let f = &items[k];
            if f.nchars == 0 {
                continue;
            }
            let inter = (f.r.min(r) - f.l.max(l)).max(0.0) * (f.b.min(b) - f.t.max(t)).max(0.0);
            covered += inter;
            // a text object with this much of itself inside the path: the path is its cell
            holds_text |= inter > 0.0 && inter >= p.cell_inside as f64 * (f.r - f.l) * (f.b - f.t);
        }
        if covered > p.cover_max as f64 * w * h || holds_text {
            continue;
        }
        let est = t + 0.79 * h;
        // snap to the baseline of text that sits in the same column, when one is close enough
        let mut base = est;
        let mut best = 0.4 * body;
        let lo = bases.partition_point(|&y| y < est - 0.4 * body);
        let hi = bases.partition_point(|&y| y <= est + 0.4 * body);
        for &k in &by_base[lo..hi.max(lo)] {
            let f = &items[k];
            let ov = f.r.min(r) - f.l.max(l);
            if ov > 0.3 * w.min(f.r - f.l) && (f.base - est).abs() < best {
                best = (f.base - est).abs();
                base = f.base;
            }
        }
        let lo = bases.partition_point(|&y| y < base - 1.7 * body);
        let hi = bases.partition_point(|&y| y <= base + 1.7 * body);
        let supported = by_base[lo..hi.max(lo)].iter().any(|&k| {
            let f = &items[k];
            let gap = (f.l - r).max(l - f.r).max(0.0);
            let ov = f.r.min(r) - f.l.max(l);
            let dy = (f.base - base).abs();
            (dy < 0.35 * body && gap <= p.outlined_touch as f64 * body) || (ov > 0.4 * w.min(f.r - f.l) && dy >= 0.8 * body && dy <= 1.7 * body)
        });
        if !supported {
            continue;
        }
        // a path lying over another of its size is part of a drawing (a fill and its stroke, a QR code);
        // (asked last: most paths of a drawing have no text near them and never get here)
        if overlapped_by_art(art.get_or_insert_with(|| furn_art(shapes, body, p)), id, [l, t, r, b], body, p.art_overlap as f64) {
            continue;
        }
        let n = ((w / (0.5 * body)).ceil().max(1.0) as usize).min(256);
        add.push(It {
            obj: id,
            l,
            t,
            r,
            b,
            base,
            size: body,
            font: NO_FONT,
            size_class: 0,
            rgb: [0, 0, 0],
            nchars: 0,
            text: "\u{2592}".repeat(n),
            opaque: true,
            script: false,
            mark: false,
        });
    }
    items.extend(add);
}

// ---- marks ----

/// Super- and subscript marks. A footnote number, a trademark sign or the 2 of m2 is drawn smaller and
/// off the baseline of the text it belongs to, so row building would give it a row of its own and the
/// mark would end up a block of its own. A small fragment of at most four characters that touches a
/// larger one (gap up to `script_gap` em of the host, on either side) and sits within `script_shift`
/// em of its baseline (but not on it: that is the same row already) is moved onto the host's baseline.
pub(super) fn attach_scripts(items: &mut [It], body: f64, p: &Params) {
    let n = items.len();
    let max_size = items.iter().map(|i| i.size).fold(0.0, f64::max);
    if n < 2 || !(max_size > 0.0) {
        return;
    }
    let window = p.script_shift as f64 * max_size.min(4.0 * body);
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| items[a].base.total_cmp(&items[b].base).then(items[a].l.total_cmp(&items[b].l)).then(items[a].obj.cmp(&items[b].obj)));
    let bases: Vec<f64> = order.iter().map(|&i| items[i].base).collect();
    let mut host_of: Vec<Option<usize>> = vec![None; n];
    for f in 0..n {
        let m = &items[f];
        if m.opaque || m.nchars == 0 || m.nchars > 4 || m.size > p.script_ratio as f64 * max_size {
            continue;
        }
        let lo = bases.partition_point(|&y| y < m.base - window);
        let hi = bases.partition_point(|&y| y <= m.base + window);
        let mut best: Option<(f64, usize, usize)> = None; // gap, host object, host index
        for &g in &order[lo..hi.max(lo)] {
            let h = &items[g];
            if g == f || h.opaque || h.nchars == 0 || m.size > p.script_ratio as f64 * h.size {
                continue;
            }
            let dy = (m.base - h.base).abs();
            if dy <= p.row_tol as f64 * m.size || dy > p.script_shift as f64 * h.size {
                continue; // already on the baseline, or too far off it
            }
            let (after, before) = (m.l - h.r, h.l - m.r);
            let gap = if after >= -0.25 * h.size {
                after
            } else if before >= -0.25 * h.size {
                before
            } else {
                continue;
            };
            if gap > p.script_gap as f64 * h.size {
                continue;
            }
            if best.map_or(true, |(bg, bo, _)| gap < bg || (gap == bg && h.obj < bo)) {
                best = Some((gap, h.obj, g));
            }
        }
        host_of[f] = best.map(|b| b.2);
    }
    let original: Vec<f64> = items.iter().map(|i| i.base).collect();
    for f in 0..n {
        if let Some(mut g) = host_of[f] {
            for _ in 0..4 {
                match host_of[g] {
                    Some(h) if h != f => g = h,
                    _ => break,
                }
            }
            items[f].base = original[g];
            items[f].script = true;
        }
    }
}
