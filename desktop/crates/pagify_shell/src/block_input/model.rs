//! The input model: constants, small rectangle predicates, what an excluded
//! or adapted run is, and the adapt pass over a page's runs.
//!
//! Part of the `block_input` module — split out of the single file the
//! design review flagged (Phase 4, file splits).
use super::*;

pub const OUTLINED_PLACEHOLDER: &str = "[drawn text]";

/// Two objects whose baseline origins are this close (points) are one place. With coincident boxes the
/// later is a twin and is shadowed; whatever their boxes, C4 refuses both as members of one block.
pub const SAME_ORIGIN_PT: f32 = 1.0;

/// Two boxes coincide when every edge is this close (points): a blank twin and the readable member
/// beneath it, or a shadowed duplicate and the object it repeats.
pub const TWIN_RECT_PT: f32 = 1.5;

/// How far a text matrix's x axis may stray from (1, 0) and still be upright.
pub(super) const UPRIGHT_TOL: f32 = 0.02;

/// Inside one line, the objects' baselines may differ by this many em (the detector's own row
/// tolerance is 0.28 em of the first object, so two objects of one row differ by at most 0.56).
pub(super) const ROW_EM: f32 = 0.75;

/// The next line lies at least this many em below the previous one (line pitch is about 1.2 em).
pub(super) const LINE_BELOW_EM: f32 = 0.4;

/// A text line never spans a gap wider than this many em, less what the line's outlined words fill of
/// it (the detector cuts a line at 4 em unless an outlined word fills the hole; the datasheet's own
/// holes are 3.4 em, a receipt's merged cells 12.7).
pub(super) const MAX_LINE_GAP_EM: f32 = 6.0;

/// C17: a block of at most this many lines whose line has a gap of at least [`CELL_GAP_EM`] between two
/// of its objects is a row of table cells. 1.0 em is the width above which the real pages' pieces of one
/// line are mostly different cells (an em is a word space of a justified line at the most); the gaps
/// that were wrongly joined measured 1.2 to 4.9 em.
pub(super) const CELL_ROW_MAX_LINES: usize = 3;
pub(super) const CELL_GAP_EM: f32 = 1.0;

/// Whether a run's own rect looks like text set at some rotation other than upright: four or more
/// characters of ink in a box taller than it is wide, which upright text never is. The app's click
/// path calls this one (it used to keep a private copy).
///
/// **A heuristic, not a fact read off the file.** [`TextRun`] carries no rotation angle, so rotation is
/// inferred from the shape PDFium reports for the run's ink, and deliberately **not** from
/// [`TextRun::size`], which cannot be trusted for it: a quarter turn makes the effective size 0 (the
/// matrix's `d`), and some producers write a nominal `1 Tf` and put the real size in the matrix.
/// [`RunStyle::axis`] is the better witness where there is one ([`adapt`] asks it first); the box shape
/// is the second opinion, and the only one a caller holding only a run (the single-run editor) has.
///
/// **Gated on four characters.** Two or three narrow letters ("Ill") at a large size are legitimately
/// taller than wide without being rotated at all; four or more characters read left to right are
/// never taller than they are wide, at any size, in any ordinary face. The count is of the text, not
/// its padding (callers pass `text.trim().chars().count()`). A rotated label of one to three characters
/// ("5m") is therefore not caught by the box shape: its text matrix is, through the axis. The 1.2
/// factor is slack: the box has to be clearly taller than wide, not merely as tall as it is wide.
pub fn looks_rotated(rect: &Rect, char_count: usize) -> bool {
    if char_count < 4 {
        return false;
    }
    let width = (rect.right - rect.left).abs();
    let height = (rect.bottom - rect.top).abs();
    height > width * 1.2
}

// ------------------------------------------------------------------------------------------------
// Small helpers
// ------------------------------------------------------------------------------------------------

pub(super) fn finite_rect(r: &Rect) -> bool {
    r.left.is_finite() && r.top.is_finite() && r.right.is_finite() && r.bottom.is_finite()
}

/// Left <= right, top <= bottom. Only meaningful for a finite rect.
pub(super) fn normalised(r: &Rect) -> Rect {
    Rect { left: r.left.min(r.right), top: r.top.min(r.bottom), right: r.left.max(r.right), bottom: r.top.max(r.bottom) }
}

pub(super) fn left_of(r: &TextRun) -> f32 {
    r.rect.left.min(r.rect.right)
}

/// How far apart the four edges of two rects are, as left, top, right, bottom.
pub(super) fn edge_gaps(a: &Rect, b: &Rect) -> [f32; 4] {
    [a.left - b.left, a.top - b.top, a.right - b.right, a.bottom - b.bottom].map(f32::abs)
}

/// Any area at all: finite, and neither side zero (a flipped rect has the area of its normalised self).
///
/// Not `text_run_at`'s "both sides over half a point": that is a click rule (nothing to aim at), and
/// applied here it dropped real letters, the 0.4 pt wide 'l' of "reliable" and "Tunable", out of the
/// editor's text while the guard said all was well.
pub(super) fn has_area(r: &Rect) -> bool {
    finite_rect(r) && (r.right - r.left).abs() > 0.0 && (r.bottom - r.top).abs() > 0.0
}

/// A missing style cannot say the text is rotated, so it is taken as upright.
pub(super) fn upright(style: Option<&RunStyle>) -> bool {
    style.is_none_or(|s| (s.axis.0 - 1.0).abs() <= UPRIGHT_TOL && s.axis.1.abs() <= UPRIGHT_TOL)
}

/// `BCDIEE+OpenSans-Regular` -> `OpenSans-Regular`: the six-uppercase-letter subset tag and its `+`.
pub(super) fn strip_subset_tag(face: &str) -> &str {
    let f = face.trim();
    match f.split_once('+') {
        Some((tag, rest)) if tag.len() == 6 && tag.bytes().all(|c| c.is_ascii_uppercase()) => rest,
        _ => f,
    }
}

/// A line of the editor holds no line break: the buffer's own `\n` is the only one.
pub(super) fn sanitise(text: &str) -> String {
    text.replace(['\n', '\r'], " ")
}

pub(super) fn ms(since: Instant) -> f32 {
    since.elapsed().as_secs_f32() * 1000.0
}

// ------------------------------------------------------------------------------------------------
// Adapter: PDFium's types -> the detector's
// ------------------------------------------------------------------------------------------------

/// Object ids kept out of the detector, by reason. See the module doc for what each list means and
/// which wins when more than one applies. An id is in at most one list, and an id in any list is in
/// no [`Frag`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Excluded {
    pub blank: Vec<usize>,
    pub invisible: Vec<usize>,
    pub degenerate: Vec<usize>,
    pub shadowed: Vec<usize>,
    pub rotated: Vec<usize>,
}

impl Excluded {
    /// Why this object is kept out of the layout, if it is.
    pub fn reason(&self, object: usize) -> Option<&'static str> {
        [("blank", &self.blank), ("invisible", &self.invisible), ("degenerate", &self.degenerate), ("shadowed", &self.shadowed), ("rotated", &self.rotated)]
            .into_iter()
            .find(|(_, ids)| ids.contains(&object))
            .map(|(why, _)| why)
    }

    pub fn count(&self) -> usize {
        self.blank.len() + self.invisible.len() + self.degenerate.len() + self.shadowed.len() + self.rotated.len()
    }
}

/// What [`adapt`] makes of a page: the detector's input, and what it left out.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Adapted {
    pub frags: Vec<Frag>,
    pub shapes: Vec<Shape>,
    pub excluded: Excluded,
}

/// Where a run goes before the shadow test: into the layout, or out of it and why.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Fate {
    Admit,
    Invisible,
    Blank,
    Rotated,
    Degenerate,
}

/// The order of these tests is the order of [`Excluded`]'s precedence; see the module doc.
pub(super) fn fate(r: &TextRun, style: Option<&RunStyle>) -> Fate {
    if !has_area(&r.rect) {
        return Fate::Degenerate;
    }
    if r.color.a == 0 {
        return Fate::Invisible;
    }
    let text = r.text.trim();
    if text.is_empty() {
        return Fate::Blank;
    }
    if !upright(style) || looks_rotated(&r.rect, text.chars().count()) {
        return Fate::Rotated;
    }
    if !(r.origin.x.is_finite() && r.origin.y.is_finite() && r.size.is_finite() && r.size > 0.0) {
        return Fate::Degenerate;
    }
    Fate::Admit
}

/// Turn a page's runs, styles, font names and drawn objects into the detector's input.
///
/// Pure, deterministic and independent of the order of `runs` (they are taken by object id, the first
/// run of an id wins and a second run under the same id is ignored: ids are unique on a real page).
/// `shapes` may be the whole drawn list: only [`DrawnKind::Shape`] objects with a finite rect are kept,
/// with their nesting depth (the detector only looks at depth 0, and a form's children carry the
/// form's own object id).
///
/// Never panics: NaN and infinite geometry, zero and negative sizes, flipped rects, empty text.
pub fn adapt(runs: &[TextRun], styles: &HashMap<usize, RunStyle>, faces: &HashMap<usize, String>, shapes: &[DrawnObject]) -> Adapted {
    let mut order: Vec<usize> = (0..runs.len()).collect();
    order.sort_by_key(|&i| runs[i].object); // stable: the first of equal ids stays first

    let mut out = Adapted::default();
    // origin and box of every admitted member, in 1 pt cells of the origin, so "an earlier member drawn
    // under this one" looks at nine cells and not at every member
    type Cells = HashMap<(i64, i64), Vec<(f32, f32, Rect)>>;
    let mut cells: Cells = HashMap::new();
    let mut last: Option<usize> = None;
    for &i in &order {
        let r = &runs[i];
        if last == Some(r.object) {
            continue;
        }
        last = Some(r.object);
        let style = styles.get(&r.object);
        match fate(r, style) {
            Fate::Degenerate => out.excluded.degenerate.push(r.object),
            Fate::Invisible => out.excluded.invisible.push(r.object),
            Fate::Blank => out.excluded.blank.push(r.object),
            Fate::Rotated => out.excluded.rotated.push(r.object),
            Fate::Admit => {
                let (x, y) = (r.origin.x, r.origin.y);
                let (kx, ky) = (x.floor() as i64, y.floor() as i64);
                let b = normalised(&r.rect);
                let twin = (-1..=1).any(|dx| {
                    (-1..=1).any(|dy| {
                        cells.get(&(kx.saturating_add(dx), ky.saturating_add(dy))).is_some_and(|v| {
                            v.iter().any(|(ox, oy, orect)| (ox - x).hypot(oy - y) <= SAME_ORIGIN_PT && edge_gaps(orect, &b).iter().all(|g| *g <= TWIN_RECT_PT))
                        })
                    })
                });
                if twin {
                    out.excluded.shadowed.push(r.object);
                    continue;
                }
                cells.entry((kx, ky)).or_default().push((x, y, b));
                out.frags.push(Frag {
                    object: r.object,
                    left: b.left,
                    top: b.top,
                    right: b.right,
                    bottom: b.bottom,
                    baseline: r.origin.y,
                    size: r.size,
                    font: style.map_or(u32::MAX, |s| s.font),
                    stem: style.and_then(|s| s.stem_milli_em),
                    face: faces.get(&r.object).map(|f| strip_subset_tag(f).to_string()).unwrap_or_default(),
                    rgb: [r.color.r, r.color.g, r.color.b],
                    text: sanitise(&r.text),
                    rotated: false,
                });
            }
        }
    }

    out.shapes = shapes
        .iter()
        .filter(|s| s.kind == DrawnKind::Shape && finite_rect(&s.rect))
        .map(|s| {
            let b = normalised(&s.rect);
            Shape { object: s.object, left: b.left, top: b.top, right: b.right, bottom: b.bottom, depth: u32::try_from(s.depth).unwrap_or(u32::MAX) }
        })
        .collect();
    out
}
