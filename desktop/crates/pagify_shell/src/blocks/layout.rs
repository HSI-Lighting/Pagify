//! Turning laid-out fragment order into block drafts: the detection pass
//! over fragment runs, size classes, and finishing a draft into a `Block`.
//!
//! Part of the `blocks` module — split out of the single file the design
//! review flagged (Phase 4, file splits).
use super::*;

pub(super) fn first_id(b: &Block) -> usize {
    b.lines.iter().flat_map(|l| l.objects.iter().chain(l.outlined.iter())).copied().next().unwrap_or(usize::MAX)
}

pub(super) fn finish_block(d: BlockDraft) -> Block {
    let mut lines: Vec<Line> = Vec::with_capacity(d.lines.len());
    let (mut l, mut t, mut r, mut b) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for ld in d.lines {
        // (members arrive left to right, ties by object id: rows are built in that order)
        l = l.min(ld.rect[0]);
        t = t.min(ld.rect[1]);
        r = r.max(ld.rect[2]);
        b = b.max(ld.rect[3]);
        lines.push(Line {
            objects: ld.members.iter().map(|m| m.1).collect(),
            outlined: ld.outlined.iter().map(|m| m.1).collect(),
            left: ld.rect[0] as f32,
            top: ld.rect[1] as f32,
            right: ld.rect[2] as f32,
            bottom: ld.rect[3] as f32,
            baseline: ld.baseline as f32,
        });
    }
    Block { lines, starts_because: d.why, left: l as f32, top: t as f32, right: r as f32, bottom: b as f32 }
}

// ------------------------------------------------------------------------------------------------
// Layout
// ------------------------------------------------------------------------------------------------

/// Size classes: sizes within 1 % of the smallest of their class are one class (relative, so scaling
/// a page cannot move a size across a class boundary).
pub(super) fn size_class_starts(sizes: &mut Vec<f64>) -> Vec<f64> {
    sizes.sort_by(|a, b| a.total_cmp(b));
    sizes.dedup();
    let mut starts: Vec<f64> = Vec::new();
    for &s in sizes.iter() {
        match starts.last() {
            Some(&s0) if s <= s0 * 1.01 => {}
            _ => starts.push(s),
        }
    }
    starts
}

pub(super) fn size_class_of(starts: &[f64], size: f64) -> u32 {
    (starts.partition_point(|&s| s <= size).max(1) - 1) as u32
}

pub(super) fn layout_blocks(frags: &[Frag], layout: &[usize], shapes: &[Shape], p: &Params) -> Vec<BlockDraft> {
    // ---- the page's fonts: the first fragment (lowest object id) of each font id speaks for it ----
    let mut ids: Vec<u32> = layout.iter().map(|&i| frags[i].font).collect();
    ids.sort_unstable();
    ids.dedup();
    let mut stem: Vec<Option<u16>> = vec![None; ids.len()];
    let mut face: Vec<String> = vec![String::new(); ids.len()];
    let mut seen = vec![false; ids.len()];
    for &i in layout {
        let f = &frags[i];
        let k = ids.binary_search(&f.font).unwrap_or(0);
        if !seen[k] {
            seen[k] = true;
            stem[k] = f.stem;
            face[k] = clean_face(&f.face);
        }
    }
    let fonts = Fonts { stem, face, tol: p.stem_tol, unknown_ok: p.unknown_stem_same_face };
    let font_of = |id: u32| -> u32 { ids.binary_search(&id).map(|k| k as u32).unwrap_or(NO_FONT) };

    // ---- items ----
    let mut sizes: Vec<f64> = layout.iter().map(|&i| frags[i].size as f64).collect();
    let starts = size_class_starts(&mut sizes);
    let mut items: Vec<It> = layout
        .iter()
        .map(|&i| {
            let f = &frags[i];
            let (l, r) = ((f.left as f64).min(f.right as f64), (f.left as f64).max(f.right as f64));
            let (t, b) = ((f.top as f64).min(f.bottom as f64), (f.top as f64).max(f.bottom as f64));
            It {
                obj: f.object,
                l,
                t,
                r,
                b,
                base: f.baseline as f64,
                size: f.size as f64,
                font: font_of(f.font),
                size_class: size_class_of(&starts, f.size as f64),
                rgb: f.rgb,
                nchars: nonspace(&f.text),
                text: f.text.clone(),
                opaque: false,
                script: false,
                mark: false,
            }
        })
        .collect();
    for it in items.iter_mut() {
        let t = it.text.trim();
        it.mark = it.nchars > 0 && it.nchars <= 2 && t.chars().all(|c| MARK_GLYPHS.contains(c)) && it.r - it.l <= 1.5 * it.size;
    }

    // body size: the size class with most characters (a character-weighted mean of its sizes)
    let body = {
        let mut chars: Vec<(f64, f64)> = vec![(0.0, 0.0); starts.len()]; // (size x chars, chars)
        for it in &items {
            let e = &mut chars[it.size_class as usize];
            e.0 += it.size * it.nchars as f64;
            e.1 += it.nchars as f64;
        }
        let mut best = 0usize;
        for (k, e) in chars.iter().enumerate() {
            if e.1 > chars[best].1 {
                best = k;
            }
        }
        if chars[best].1 > 0.0 {
            chars[best].0 / chars[best].1
        } else {
            items[0].size
        }
    };

    if p.script_marks {
        attach_scripts(&mut items, body, p);
    }
    let furn = classify_shapes(shapes, body, p);
    if p.outlined {
        add_outlined(&mut items, &furn_words(shapes, body, p), shapes, body, p);
    }

    let rows = make_rows(&mut items, p);
    let pieces = cut_rows(&items, &rows, &furn, &fonts, p);
    let link = link_pieces(&pieces, &furn, p, rows.len());
    segment(&items, &pieces, &link, &furn, &fonts, p, body)
}
