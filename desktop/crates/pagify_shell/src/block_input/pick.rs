//! Picking a seed from a click: the seed rules, the twin fallback, and the
//! line specs/pieces an editor opens with.
//!
//! Part of the `block_input` module — split out of the single file the
//! design review flagged (Phase 4, file splits).
use super::*;

/// How the seed of a click was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeedRule {
    /// The smallest text rect that contains the point.
    Exact,
    /// No rect contained it: the nearest one within the tolerance.
    Near,
    /// The rule above chose an object that is not a block member (blank, shadowed, ...), and a readable
    /// member whose rect coincides with it within [`TWIN_RECT_PT`] was taken instead.
    Twin,
    /// The click was inside a box — an [`EditUnit`] — and the nearest word of that box was taken, even
    /// though no word rect was near the point (a click in the blank space of a box).
    InBox,
}

impl SeedRule {
    pub fn as_str(self) -> &'static str {
        match self {
            SeedRule::Exact => "exact",
            SeedRule::Near => "near",
            SeedRule::Twin => "twin",
            SeedRule::InBox => "in-box",
        }
    }
}

pub(super) fn keep_min(slot: &mut Option<(f32, usize)>, candidate: (f32, usize)) {
    let better = match slot {
        None => true,
        Some(cur) => candidate.0.total_cmp(&cur.0).then(candidate.1.cmp(&cur.1)).is_lt(),
    };
    if better {
        *slot = Some(candidate);
    }
}

/// The readable member that a blank twin (or a shadowed duplicate) stands on: every edge of its rect
/// within [`TWIN_RECT_PT`] of the seed's. The closest wins, then the lowest id.
pub(super) fn twin_member(pb: &PageBlocks, seed: usize) -> Option<usize> {
    let s = normalised(&pb.runs.get(&seed)?.rect);
    let mut best: Option<(f32, usize)> = None;
    for &m in pb.by_object.keys() {
        let Some(run) = pb.runs.get(&m) else { continue };
        let d = edge_gaps(&normalised(&run.rect), &s);
        if d.iter().all(|v| *v <= TWIN_RECT_PT) {
            keep_min(&mut best, (d.iter().sum(), m));
        }
    }
    best.map(|(_, m)| m)
}

/// The text object a click at page point `(x, y)` means.
///
/// Over every text object with area (excluded ones too, so a click on an unreadable word is still
/// explained): the smallest rect that contains the point; failing that the nearest rect within
/// `tolerance` points. Ties go to the lowest object id, so the answer never depends on hash order.
///
/// **A blank twin gives way to the readable object under it.** The faux-bold second layer of a heading
/// has blank text and the same rect as the real one; the smallest-rect rule lands on it half the time
/// and the click then opens the useless "unreadable" editor. A seed that is not a block member (and not
/// rotated) is replaced by the member whose rect coincides with it, and the rule is then
/// [`SeedRule::Twin`].
pub fn pick_seed(pb: &PageBlocks, x: f32, y: f32, tolerance: f32) -> Option<(usize, SeedRule)> {
    if !x.is_finite() || !y.is_finite() {
        return None;
    }
    let tol = if tolerance.is_finite() { tolerance } else { 0.0 };
    let mut exact: Option<(f32, usize)> = None;
    let mut near: Option<(f32, usize)> = None;
    for (&o, run) in &pb.runs {
        let r = normalised(&run.rect);
        let dx = (r.left - x).max(x - r.right).max(0.0);
        let dy = (r.top - y).max(y - r.bottom).max(0.0);
        if dx == 0.0 && dy == 0.0 {
            keep_min(&mut exact, ((r.right - r.left) * (r.bottom - r.top), o));
        } else if dx <= tol && dy <= tol {
            keep_min(&mut near, (dx.hypot(dy), o));
        }
    }
    let (seed, rule) = match (exact, near) {
        (Some((_, o)), _) => (o, SeedRule::Exact),
        (None, Some((_, o))) => (o, SeedRule::Near),
        (None, None) => return None,
    };
    if !pb.by_object.contains_key(&seed) && !pb.excluded.rotated.contains(&seed) {
        if let Some(member) = twin_member(pb, seed) {
            return Some((member, SeedRule::Twin));
        }
    }
    Some((seed, rule))
}

// ------------------------------------------------------------------------------------------------
// The editor's lines
// ------------------------------------------------------------------------------------------------

/// One line of the editor box, as the detector found it.
#[derive(Clone, Debug, PartialEq)]
pub struct LineSpec {
    /// The text objects of the line, left to right; empty only for a line made of outlined words.
    pub objects: Vec<usize>,
    /// The line's rect: finite and normalised, and it may exceed the objects' union (it includes the
    /// outlined words bridged into the line).
    pub rect: Rect,
    /// The line carries outlined (path-drawn) words: apply must never rewrite or hide it.
    pub frozen: bool,
    /// The text standing for a line with no text object: [`OUTLINED_PLACEHOLDER`]. `None` otherwise.
    pub placeholder: Option<String>,
    /// The objects that **duplicate** a member of this line (the faux-bold second layer: blank text,
    /// or a readable copy at the same place, see [`assign_twins`]), ascending. They are not members,
    /// are not in the line's text and count for nothing in C3 to C5, but whoever retypes or removes
    /// the line removes them too, the first piece's twin included, or they go on drawing the old words.
    /// A **frozen** line lists its twins like any other and is never written, so they are never
    /// removed with it. Empty for nearly every line.
    pub twins: Vec<usize>,
}

/// A run of one block's lines that is taken as a paragraph of its own.
///
/// **A paragraph with drawn words in it is more than one paragraph.** A line the page draws, wholly or in
/// part, as vector paths (type converted to outlines) can never be retyped — there are no characters to
/// put in a box — and used to sit inside the paragraph around it as a `[drawn text]` placeholder, so one
/// click opened a box that was part editable and part not (reported from use: `outlined=1 frozen=[2]`).
///
/// The detector's blocks are left exactly as they are: they are what two hand labelings are measured
/// against, and those count the outlined words as part of their paragraph. The cut is made here, where
/// the editor and the boxes over the page both read it: the written lines above, the drawn ones, the
/// written lines below.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Piece {
    /// Lines `from..to` of the block.
    pub from: usize,
    pub to: usize,
    /// Every line in the run holds outlined words. A drawn piece is never opened as a paragraph.
    pub drawn: bool,
}

/// The pieces of block `block`, top to bottom: a maximal run of written lines is one piece, a maximal run
/// of drawn lines another. A block with no drawn line is a single piece. Empty for an unknown block.
pub fn pieces(pb: &PageBlocks, block: usize) -> Vec<Piece> {
    let Some(b) = pb.blocks.get(block) else { return Vec::new() };
    let mut out: Vec<Piece> = Vec::new();
    for (i, line) in b.lines.iter().enumerate() {
        let drawn = !line.outlined.is_empty();
        match out.last_mut() {
            Some(piece) if piece.drawn == drawn => piece.to = i + 1,
            _ => out.push(Piece { from: i, to: i + 1, drawn }),
        }
    }
    out
}

/// Lines `from..to` of block `block` as a block of their own, its box recomputed over just them. `None`
/// when the range is empty or not the block's.
pub fn piece_block(pb: &PageBlocks, block: usize, from: usize, to: usize) -> Option<Block> {
    let b = pb.blocks.get(block)?;
    let lines: Vec<Line> = b.lines.get(from..to)?.to_vec();
    if lines.is_empty() {
        return None;
    }
    let left = lines.iter().map(|l| l.left).fold(f32::MAX, f32::min);
    let top = lines.iter().map(|l| l.top).fold(f32::MAX, f32::min);
    let right = lines.iter().map(|l| l.right).fold(f32::MIN, f32::max);
    let bottom = lines.iter().map(|l| l.bottom).fold(f32::MIN, f32::max);
    Some(Block { lines, starts_because: b.starts_because, left, top, right, bottom })
}

/// One thing a click can open: a paragraph Edit Text accepts, or one cell of a row it refused.
///
/// **The one answer the boxes and the click share.** Worked out once per page read, in
/// [`build_page_blocks_from`], so the box drawn over the page and the editor a click opens cannot
/// disagree about what is there. A cell lists the same text objects the paragraph would have, cut at
/// the wide gap C17 refuses.
#[derive(Clone, Debug, PartialEq)]
pub struct EditUnit {
    /// The unit's box, normalised and finite.
    pub rect: Rect,
    /// A run of drawn lines: boxed dashed, never opened as a paragraph.
    pub drawn: bool,
    /// The detector block this came from.
    pub block: usize,
    /// Lines `from..to` of the block; a cell is always one line, so `to == from + 1`.
    pub from: usize,
    pub to: usize,
    /// Its text objects, line by line, left to right.
    pub objects: Vec<usize>,
    /// Part of a line: a cell of a row the checks refused.
    pub cell: bool,
}

/// Every unit of the page, in block order. Rotated and unplaced text is left out: a click on it is
/// refused, and a box would promise otherwise. A drawn piece with no text of its own is still a unit,
/// so the gap in the paragraph is something to see rather than a hole.
pub fn edit_units(pb: &PageBlocks) -> Vec<EditUnit> {
    let mut out = Vec::new();
    for (index, block) in pb.blocks.iter().enumerate() {
        if matches!(block.starts_because, "rotated" | "unplaced") {
            continue;
        }
        for piece in pieces(pb, index) {
            let Some(b) = piece_block(pb, index, piece.from, piece.to) else { continue };
            let rect = Rect { left: b.left, top: b.top, right: b.right, bottom: b.bottom };
            if !finite_rect(&rect) {
                continue;
            }
            let rect = normalised(&rect);
            let objects: Vec<usize> = b.lines.iter().flat_map(|l| l.objects.iter().copied()).collect();
            if piece.drawn {
                out.push(EditUnit { rect, drawn: true, block: index, from: piece.from, to: piece.to, objects, cell: false });
                continue;
            }
            if check_block_invariants(pb, &b).is_ok() {
                out.push(EditUnit { rect, drawn: false, block: index, from: piece.from, to: piece.to, objects, cell: false });
                continue;
            }

            // Refused as a paragraph: each line's cells are units of their own, so a click still opens
            // everything the box showed rather than one letter of it.
            for (offset, line) in b.lines.iter().enumerate() {
                let at = piece.from + offset;
                for cell in cells_of_line(pb, &line.objects) {
                    let Some(cell_rect) = union_of_runs(pb, &cell) else { continue };
                    let mut unit = EditUnit {
                        rect: cell_rect,
                        drawn: false,
                        block: index,
                        from: at,
                        to: at + 1,
                        objects: cell,
                        cell: true,
                    };
                    let refused = unit.objects.len() > 1
                        && match unit_block(pb, &unit) {
                            Some(b) => check_block_invariants(pb, &b).is_err(),
                            None => true,
                        };
                    if refused {
                        // The cell is more than the editor can write safely as one line — open its
                        // words one by one instead of refusing the click outright.
                        for object in std::mem::take(&mut unit.objects) {
                            let Some(one) = union_of_runs(pb, &[object]) else { continue };
                            out.push(EditUnit { rect: one, objects: vec![object], ..unit.clone() });
                        }
                        continue;
                    }
                    out.push(unit);
                }
            }
        }
    }
    out
}

/// The union of one cell's runs' rects, normalised — `None` when none of its objects has a run.
fn union_of_runs(pb: &PageBlocks, objects: &[usize]) -> Option<Rect> {
    let mut union: Option<Rect> = None;
    for &object in objects {
        let Some(run) = pb.runs.get(&object) else { continue };
        let r = normalised(&run.rect);
        union = Some(match union {
            None => r,
            Some(u) => Rect {
                left: u.left.min(r.left),
                top: u.top.min(r.top),
                right: u.right.max(r.right),
                bottom: u.bottom.max(r.bottom),
            },
        });
    }
    union.filter(finite_rect)
}

/// The unit under a click: the smallest-area unit whose rect contains the point; failing that the
/// nearest one within `tolerance` points. Ties go to the lower index, so the answer never depends on
/// hash order — [`pick_seed`]'s shape, over boxes rather than words.
pub fn unit_at(units: &[EditUnit], x: f32, y: f32, tolerance: f32) -> Option<usize> {
    if !x.is_finite() || !y.is_finite() {
        return None;
    }
    let tol = if tolerance.is_finite() { tolerance } else { 0.0 };
    let mut exact: Option<(f32, usize)> = None;
    let mut near: Option<(f32, usize)> = None;
    for (i, unit) in units.iter().enumerate() {
        let r = &unit.rect;
        let dx = (r.left - x).max(x - r.right).max(0.0);
        let dy = (r.top - y).max(y - r.bottom).max(0.0);
        if dx == 0.0 && dy == 0.0 {
            keep_min(&mut exact, ((r.right - r.left) * (r.bottom - r.top), i));
        } else if dx <= tol && dy <= tol {
            keep_min(&mut near, (dx.hypot(dy), i));
        }
    }
    match (exact, near) {
        (Some((_, i)), _) => Some(i),
        (None, Some((_, i))) => Some(i),
        (None, None) => None,
    }
}

/// The word of `unit` a click at `(x, y)` means: the nearest of its objects' run rects, ties to the
/// lower object id. No tolerance: a click inside a box always has an answer as long as the box has
/// words, even in its blank space. `None` only when the unit has no objects with a run.
pub fn seed_in_unit(pb: &PageBlocks, unit: &EditUnit, x: f32, y: f32) -> Option<usize> {
    if !x.is_finite() || !y.is_finite() {
        return None;
    }
    let mut best: Option<(f32, usize)> = None;
    for &object in &unit.objects {
        let Some(run) = pb.runs.get(&object) else { continue };
        let r = normalised(&run.rect);
        let dx = (r.left - x).max(x - r.right).max(0.0);
        let dy = (r.top - y).max(y - r.bottom).max(0.0);
        keep_min(&mut best, (dx.hypot(dy), object));
    }
    best.map(|(_, object)| object)
}

/// The block of editor lines `unit` stands for: the piece as the detector made it, or one line over
/// just a cell's words.
pub fn unit_block(pb: &PageBlocks, unit: &EditUnit) -> Option<Block> {
    if !unit.cell {
        return piece_block(pb, unit.block, unit.from, unit.to);
    }
    let block = pb.blocks.get(unit.block)?;
    let original = block.lines.get(unit.from)?;
    if unit.to != unit.from + 1 {
        return None;
    }
    let rect = unit.rect;
    let line = Line {
        objects: unit.objects.clone(),
        outlined: Vec::new(),
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
        baseline: original.baseline,
    };
    Some(Block {
        lines: vec![line],
        starts_because: block.starts_because,
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    })
}

/// The lines of block `block`, in order, none dropped (an empty list for an unknown block).
pub fn editor_lines(pb: &PageBlocks, block: usize) -> Vec<LineSpec> {
    let Some(b) = pb.blocks.get(block) else { return Vec::new() };
    editor_lines_of(pb, b)
}

/// [`editor_lines`] for a block already in hand — a [`piece_block`] as readily as one of the page's.
pub fn editor_lines_of(pb: &PageBlocks, b: &Block) -> Vec<LineSpec> {
    b.lines
        .iter()
        .map(|l| {
            let frozen = !l.outlined.is_empty();
            let rect = Rect { left: l.left, top: l.top, right: l.right, bottom: l.bottom };
            let mut twins: Vec<usize> = l.objects.iter().flat_map(|o| pb.twins.get(o).into_iter().flatten().copied()).collect();
            twins.sort_unstable();
            twins.dedup();
            LineSpec {
                objects: l.objects.clone(),
                rect: if finite_rect(&rect) { normalised(&rect) } else { rect },
                frozen,
                placeholder: (frozen && l.objects.is_empty()).then(|| OUTLINED_PLACEHOLDER.to_string()),
                twins,
            }
        })
        .collect()
}

/// The text of each line: the plain concatenation of its objects' text in the stored order (left to
/// right), with any line break turned into a space; the placeholder for a line with no text object.
///
/// **No space is ever inserted from geometry.** Gaps of 0.4 to 1.4 pt occur inside words ("lig" + "ht"
/// is "light"); PDFium's own trailing spaces keep the words apart where they are apart.
pub fn line_texts(pb: &PageBlocks, lines: &[LineSpec]) -> Vec<String> {
    lines
        .iter()
        .map(|l| {
            if l.objects.is_empty() {
                l.placeholder.clone().unwrap_or_default()
            } else {
                l.objects.iter().map(|o| pb.runs.get(o).map(|r| sanitise(&r.text)).unwrap_or_default()).collect()
            }
        })
        .collect()
}
