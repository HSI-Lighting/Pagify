//! `PageBlocks` and how one is built: from runs and styles, the twin
//! assignment, and the snapshot entry point.
//!
//! Part of the `block_input` module — split out of the single file the
//! design review flagged (Phase 4, file splits).
use super::*;

/// Everything one click needs about one page, built once and cached by the caller under
/// `(page, epoch, generation)`. Owns its data: no PDFium handle, no lock.
#[derive(Clone, Debug)]
pub struct PageBlocks {
    pub page: usize,
    /// The caller's cache keys, stored untouched (the document's render epoch and the session's undo
    /// generation at the moment the snapshot was taken).
    pub epoch: u64,
    pub generation: u64,
    /// Every text object **with area**, excluded ones included (blank, invisible, rotated, shadowed,
    /// size-0): the click is resolved against all of them, and the single-run editor needs their run.
    /// An object without area is not here (it is in `excluded.degenerate` only).
    pub runs: HashMap<usize, TextRun>,
    pub faces: HashMap<usize, String>,
    pub styles: HashMap<usize, RunStyle>,
    /// The page's drawn paths, all nesting depths, for hyphen-mark detection.
    pub shapes: Vec<DrawnObject>,
    /// What the detector was given: the admitted objects, none of them rotated.
    pub frags: Vec<Frag>,
    pub blocks: Vec<Block>,
    /// Text object -> (block, line) in `blocks`. Holds exactly the admitted objects, each once.
    pub by_object: HashMap<usize, (usize, usize)>,
    /// Member -> the blank or shadowed objects that duplicate it (see [`assign_twins`]), ascending.
    /// Only members that have a twin are keys.
    pub twins: HashMap<usize, Vec<usize>>,
    pub excluded: Excluded,
    /// The whole of [`build_page_blocks_from`]: adapt, detect and index. The caller adds its own PDFium time.
    pub build_ms: f32,
    /// The detector alone.
    pub detect_ms: f32,
}

/// Build a page's blocks from what PDFium reported. Times itself.
pub fn build_page_blocks_from(
    page: usize,
    epoch: u64,
    generation: u64,
    runs: Vec<TextRun>,
    styles: HashMap<usize, RunStyle>,
    faces: HashMap<usize, String>,
    shapes: Vec<DrawnObject>,
) -> PageBlocks {
    let started = Instant::now();
    let adapted = adapt(&runs, &styles, &faces, &shapes);

    let detecting = Instant::now();
    let blocks = blocks::detect(&adapted.frags, &adapted.shapes);
    let detect_ms = ms(detecting);

    let mut by_object = HashMap::with_capacity(adapted.frags.len());
    for (bi, b) in blocks.iter().enumerate() {
        for (li, l) in b.lines.iter().enumerate() {
            for &o in &l.objects {
                by_object.insert(o, (bi, li));
            }
        }
    }

    let mut seen = HashSet::with_capacity(runs.len());
    let mut by_id = HashMap::with_capacity(runs.len());
    for r in runs {
        if seen.insert(r.object) && has_area(&r.rect) {
            by_id.insert(r.object, r);
        }
    }
    let shapes = shapes.into_iter().filter(|s| s.kind == DrawnKind::Shape).collect();
    let twins = assign_twins(&by_id, &adapted.excluded, &by_object);

    PageBlocks {
        page,
        epoch,
        generation,
        runs: by_id,
        faces,
        styles,
        shapes,
        frags: adapted.frags,
        blocks,
        by_object,
        twins,
        excluded: adapted.excluded,
        build_ms: ms(started),
        detect_ms,
    }
}

/// Which member each twin duplicates: member id -> its twins' ids, ascending.
///
/// A twin is a blank or shadowed object **with area** (so it is in `runs`) whose box is within
/// [`TWIN_RECT_PT`] of a member's on every edge and whose origin is within [`SAME_ORIGIN_PT`] of the
/// member's: the second layer of a faux-bold heading, or a repeated copy of a word. A twin that
/// could be either of two members' goes to the closer (sum of its four edge gaps), then to the lower
/// id, so the answer never depends on hash order, and no twin is on two members. A blank object
/// that coincides with no member is nobody's twin (an unreadable word of its own, or a space).
///
/// `members` is the page's admitted objects (the keys of [`PageBlocks::by_object`]). Pure; O(n): the
/// members are put in 1 pt cells of their origin, as [`adapt`] does, and a twin looks at nine cells.
pub fn assign_twins(runs: &HashMap<usize, TextRun>, excluded: &Excluded, members: &HashMap<usize, (usize, usize)>) -> HashMap<usize, Vec<usize>> {
    let mut out: HashMap<usize, Vec<usize>> = HashMap::new();
    if excluded.blank.is_empty() && excluded.shadowed.is_empty() {
        return out;
    }
    let cell = |x: f32, y: f32| (x.floor() as i64, y.floor() as i64);
    let mut cells: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for &m in members.keys() {
        if let Some(r) = runs.get(&m).filter(|r| r.origin.x.is_finite() && r.origin.y.is_finite() && finite_rect(&r.rect)) {
            cells.entry(cell(r.origin.x, r.origin.y)).or_default().push(m);
        }
    }
    for &t in excluded.blank.iter().chain(&excluded.shadowed) {
        let Some(tr) = runs.get(&t).filter(|r| r.origin.x.is_finite() && r.origin.y.is_finite() && finite_rect(&r.rect)) else { continue };
        let (kx, ky) = cell(tr.origin.x, tr.origin.y);
        let tb = normalised(&tr.rect);
        let mut best: Option<(f32, usize)> = None;
        for dx in -1..=1 {
            for dy in -1..=1 {
                let Some(ids) = cells.get(&(kx.saturating_add(dx), ky.saturating_add(dy))) else { continue };
                for &m in ids {
                    let Some(mr) = runs.get(&m) else { continue };
                    if (mr.origin.x - tr.origin.x).hypot(mr.origin.y - tr.origin.y) > SAME_ORIGIN_PT {
                        continue;
                    }
                    let gaps = edge_gaps(&normalised(&mr.rect), &tb);
                    if gaps.iter().all(|g| *g <= TWIN_RECT_PT) {
                        keep_min(&mut best, (gaps.iter().sum(), m));
                    }
                }
            }
        }
        if let Some((_, m)) = best {
            out.entry(m).or_default().push(t);
        }
    }
    for ids in out.values_mut() {
        ids.sort_unstable();
        ids.dedup();
    }
    out
}

/// [`build_page_blocks_from`] for a [`PageTextSnapshot`].
pub fn build_page_blocks(page: usize, epoch: u64, generation: u64, snapshot: PageTextSnapshot) -> PageBlocks {
    build_page_blocks_from(page, epoch, generation, snapshot.runs, snapshot.styles, snapshot.faces, snapshot.shapes)
}
