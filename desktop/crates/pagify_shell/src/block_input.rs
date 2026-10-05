//! The pure, window-free half of "click a word, edit the whole paragraph it belongs to".
//!
//! [`crate::blocks`] decides which text objects of a page form one paragraph, but it speaks its own
//! plain types ([`Frag`], [`Shape`]). The app speaks PDFium's: [`TextRun`], [`RunStyle`],
//! [`DrawnObject`]. This module is the bridge, and everything about the bridge that can be tested
//! without a window lives here, because getting it wrong does not draw a wrong pixel, it corrupts the
//! page: the editor writes the typed text into the page's own objects and hides the others.
//!
//! # One click
//!
//! ```text
//! PageTextSnapshot -> build_page_blocks -> PageBlocks         (cached per page, epoch, generation)
//!
//! click -> pick_seed -> seed object -> by_object -> block -> editor_lines -> check_editor_invariants
//!                                                              |                 |  Err: the caller falls
//!                                                              v                 |  back to the single-run
//!                                                          line_texts (buffer)   v  editor and logs why
//!                                                                        format_pick_line (one log line)
//! ```
//!
//! # What the detector never sees ([`adapt`])
//!
//! Only objects that can safely be *members of an editor line* become [`Frag`]s. The rest are recorded
//! in [`Excluded`] (so a click on one can still be explained) and take no part in layout:
//!
//! | list         | what                                                                             |
//! |--------------|----------------------------------------------------------------------------------|
//! | `degenerate` | no area at all (zero width or height), non-finite geometry, size <= 0            |
//! | `invisible`  | alpha 0: the extracted-text layer over outlined artwork, edited by another path  |
//! | `blank`      | extracted text is blank (a font with no ToUnicode, or the faux-bold twin layer)  |
//! | `rotated`    | the text matrix is not upright, or the box is taller than wide ([`looks_rotated`]) |
//! | `shadowed`   | a later object drawn over an earlier member: origin within [`SAME_ORIGIN_PT`], box within [`TWIN_RECT_PT`] |
//!
//! First match wins, in the order above for `invisible`/`blank`/`rotated`; an area-less object is
//! always `degenerate`, a size-0 object that is rotated is `rotated` (rotated text really has size 0:
//! the effective size multiplies the matrix's `d`, which is 0 for a quarter turn).
//!
//! **Any positive area is area.** A visible character can be very thin: the 'l' of "Tunable" is 0.4 pt
//! wide, a hyphen 0.36 pt tall, the dot of "SATURATION . INTENSITY" 0.46 pt tall. They are members of
//! their line and part of its text, or the editor would show "Tunabe" and a retyped line would lose
//! the letter. (The engine's click rule, both sides over half a point, is not this module's rule:
//! the engine edits an object by its id and finds the thin ones.) One visible consequence: a line drawn
//! entirely as outlines that ends in a real hyphen glyph (six on the datasheet) now has that hyphen as
//! its one text object, so it reads "-" instead of the placeholder, is still frozen (it carries an
//! outlined word) and is never written.
//!
//! **A negative size stays excluded.** A Type 3 font reports its nominal size of 1 under a flipped
//! matrix (size -0.12 on a page whose glyphs are 4 to 7 pt tall), so the number is not an em, and the
//! text layer of such a page is glyph indexes (one object per glyph, reading "8 8 8 9 8" or "A B"
//! for letters that are not those). Admitting them with the absolute value would hand the detector
//! an em 35 times too small and the editor text that is not the page's.
//!
//! # Twins ([`LineSpec::twins`])
//!
//! A faux-bold heading is drawn twice: the second copy is a blank-text object (PDFium reads nothing)
//! on top of the readable one, or a second readable object at the same place (shadowed). Neither is a
//! member of a line, and neither is in the editor's text, but when the line is retyped or removed the
//! copy would go on drawing the old words: each line therefore lists the objects that duplicate one of
//! its members (`twins`), so the caller can remove them with it. A twin coincides with its member: box
//! within [`TWIN_RECT_PT`] on every edge and origin within [`SAME_ORIGIN_PT`]. Never a member, never
//! in the text, never counted by C3 to C5; checked by C16.
//!
//! **A twin needs the same box as well as the same origin.** The real datasheet has two different
//! words 1.28 pt apart in origin (a ": " object, 0.7 pt of ink, and the " Ma" that follows it), and at
//! 6 pt the same split is under 1 pt. Shadowing on origin alone would drop " Ma" from its paragraph and
//! leave it drawn when the line is rewritten. Two objects that do share an origin without sharing a
//! box stay members, and C4 refuses the block they are in.
//!
//! **Rotated objects are not given to the detector at all.** It would make each a one-line block of
//! its own that the editor can never open, and it ignores them in layout anyway, so the blocks of the
//! upright text are identical either way. A rotated seed is found through [`Excluded::rotated`].
//!
//! # The editor invariants ([`check_editor_invariants`])
//!
//! The editor box holds one buffer, one line of it per [`LineSpec`], and applying it rewrites page
//! objects by id. These must hold, and the check refuses (with the invariant's number first in the
//! message) when one does not, so a detector bug degrades to the single-run editor instead of damage:
//!
//! * **C1** at least one line; a line has a text object, or is a frozen outlined-only line with a placeholder
//! * **C2** every id is a real text object of this page with area, never a shape, never `usize::MAX`
//! * **C3** each object is in exactly one line, once
//! * **C4** distinct members have distinct baseline origins; a twin (blank or shadowed) is never a member
//! * **C5** every member has non-blank text
//! * **C6** every member is visible, upright, finite and has a size above zero
//! * **C7** inside a line the objects run left to right (`objects[0]` is the leftmost and receives the line)
//! * **C8** lines run top to bottom, one baseline row of one column each; no line text holds a newline.
//!   Two lines that both hold text are ordered by baseline. A line with no text object has no measured
//!   baseline (the detector estimates one from its rect), so it is ordered against a neighbour that holds
//!   text by rect: it starts lower and ends lower than the line above (two lines with no text are not
//!   compared: nothing in either is measured or ever written). The gap between two text objects of a
//!   line is measured on frozen lines too, less what the line's outlined words fill of it
//! * **C9** a line's text is the plain concatenation of its objects' text; a placeholder is non-empty
//!   and does not end in `-`
//! * **C10** a line that carries outlined words is frozen
//! * **C11** line rects are finite and normalised
//! * **C13** no line was dropped (the editor sizes its line pitch from the union height over the line count)
//! * **C14** vector art is not a paragraph: a block with text whose lines drawn as shapes outnumber its
//!   lines with text (a QR code under a label bridged as four phantom lines)
//! * **C15** no right-to-left text: its reading order is not the left-to-right order the editor and the
//!   detector's margin rules work in, so Farsi and Arabic never open as a paragraph
//! * **C16** twins are real: each is a blank or shadowed text object of this page, sits in exactly one
//!   line's `twins`, is nobody's member, and coincides with an object of the line that carries it
//! * **C17** a block of at most three lines is not a row of cells: no written line of it has two objects
//!   an em or more apart (retyping the line would run the cells together and delete the second)
//!
//! (C12, one look for the box, is not an invariant of apply: the caller picks the dominant look.)

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use pdf_core::document::{DrawnKind, DrawnObject, Rect, RunStyle, TextRun};

use crate::blocks::{self, Block, Frag, Shape};
use crate::session::PageTextSnapshot;

/// Stands in for the text of a line made only of outlined (path-drawn) words. Non-empty, no newline,
/// does not end in `-`: the editor shows it and apply never touches that line.
pub const OUTLINED_PLACEHOLDER: &str = "[drawn text]";

/// Two objects whose baseline origins are this close (points) are one place. With coincident boxes the
/// later is a twin and is shadowed; whatever their boxes, C4 refuses both as members of one block.
pub const SAME_ORIGIN_PT: f32 = 1.0;

/// Two boxes coincide when every edge is this close (points): a blank twin and the readable member
/// beneath it, or a shadowed duplicate and the object it repeats.
pub const TWIN_RECT_PT: f32 = 1.5;

/// How far a text matrix's x axis may stray from (1, 0) and still be upright.
const UPRIGHT_TOL: f32 = 0.02;

/// Inside one line, the objects' baselines may differ by this many em (the detector's own row
/// tolerance is 0.28 em of the first object, so two objects of one row differ by at most 0.56).
const ROW_EM: f32 = 0.75;

/// The next line lies at least this many em below the previous one (line pitch is about 1.2 em).
const LINE_BELOW_EM: f32 = 0.4;

/// A text line never spans a gap wider than this many em, less what the line's outlined words fill of
/// it (the detector cuts a line at 4 em unless an outlined word fills the hole; the datasheet's own
/// holes are 3.4 em, a receipt's merged cells 12.7).
const MAX_LINE_GAP_EM: f32 = 6.0;

/// C17: a block of at most this many lines whose line has a gap of at least [`CELL_GAP_EM`] between two
/// of its objects is a row of table cells. 1.0 em is the width above which the real pages' pieces of one
/// line are mostly different cells (an em is a word space of a justified line at the most); the gaps
/// that were wrongly joined measured 1.2 to 4.9 em.
const CELL_ROW_MAX_LINES: usize = 3;
const CELL_GAP_EM: f32 = 1.0;

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

fn finite_rect(r: &Rect) -> bool {
    r.left.is_finite() && r.top.is_finite() && r.right.is_finite() && r.bottom.is_finite()
}

/// Left <= right, top <= bottom. Only meaningful for a finite rect.
fn normalised(r: &Rect) -> Rect {
    Rect { left: r.left.min(r.right), top: r.top.min(r.bottom), right: r.left.max(r.right), bottom: r.top.max(r.bottom) }
}

fn left_of(r: &TextRun) -> f32 {
    r.rect.left.min(r.rect.right)
}

/// How far apart the four edges of two rects are, as left, top, right, bottom.
fn edge_gaps(a: &Rect, b: &Rect) -> [f32; 4] {
    [a.left - b.left, a.top - b.top, a.right - b.right, a.bottom - b.bottom].map(f32::abs)
}

/// Any area at all: finite, and neither side zero (a flipped rect has the area of its normalised self).
///
/// Not `text_run_at`'s "both sides over half a point": that is a click rule (nothing to aim at), and
/// applied here it dropped real letters, the 0.4 pt wide 'l' of "reliable" and "Tunable", out of the
/// editor's text while the guard said all was well.
fn has_area(r: &Rect) -> bool {
    finite_rect(r) && (r.right - r.left).abs() > 0.0 && (r.bottom - r.top).abs() > 0.0
}

/// A missing style cannot say the text is rotated, so it is taken as upright.
fn upright(style: Option<&RunStyle>) -> bool {
    style.is_none_or(|s| (s.axis.0 - 1.0).abs() <= UPRIGHT_TOL && s.axis.1.abs() <= UPRIGHT_TOL)
}

/// `BCDIEE+OpenSans-Regular` -> `OpenSans-Regular`: the six-uppercase-letter subset tag and its `+`.
fn strip_subset_tag(face: &str) -> &str {
    let f = face.trim();
    match f.split_once('+') {
        Some((tag, rest)) if tag.len() == 6 && tag.bytes().all(|c| c.is_ascii_uppercase()) => rest,
        _ => f,
    }
}

/// A line of the editor holds no line break: the buffer's own `\n` is the only one.
fn sanitise(text: &str) -> String {
    text.replace(['\n', '\r'], " ")
}

fn ms(since: Instant) -> f32 {
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
enum Fate {
    Admit,
    Invisible,
    Blank,
    Rotated,
    Degenerate,
}

/// The order of these tests is the order of [`Excluded`]'s precedence; see the module doc.
fn fate(r: &TextRun, style: Option<&RunStyle>) -> Fate {
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

// ------------------------------------------------------------------------------------------------
// One page, detected
// ------------------------------------------------------------------------------------------------

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

// ------------------------------------------------------------------------------------------------
// The click
// ------------------------------------------------------------------------------------------------

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
}

impl SeedRule {
    pub fn as_str(self) -> &'static str {
        match self {
            SeedRule::Exact => "exact",
            SeedRule::Near => "near",
            SeedRule::Twin => "twin",
        }
    }
}

fn keep_min(slot: &mut Option<(f32, usize)>, candidate: (f32, usize)) {
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
fn twin_member(pb: &PageBlocks, seed: usize) -> Option<usize> {
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

/// The lines of block `block`, in order, none dropped (an empty list for an unknown block).
pub fn editor_lines(pb: &PageBlocks, block: usize) -> Vec<LineSpec> {
    let Some(b) = pb.blocks.get(block) else { return Vec::new() };
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

// ------------------------------------------------------------------------------------------------
// The guard
// ------------------------------------------------------------------------------------------------

struct Ctx<'a> {
    pb: &'a PageBlocks,
    block: &'a Block,
    specs: &'a [LineSpec],
    texts: &'a [String],
}

impl Ctx<'_> {
    /// The run behind a member. C2 has proved it exists by the time anything else asks, but a guard
    /// must never panic.
    fn run(&self, object: usize) -> Result<&TextRun, String> {
        self.pb.runs.get(&object).ok_or_else(|| format!("C2: object {object} is not a text object of this page"))
    }

    /// (line, object) of every member, line by line.
    fn members(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        self.specs.iter().enumerate().flat_map(|(i, s)| s.objects.iter().map(move |&o| (i, o)))
    }
}

type Check = fn(&Ctx) -> Result<(), String>;

/// In the order they are reported: C1 and C13 first (they are about the list of lines as a whole), then
/// C15 (a right-to-left block fails half the others for the same one reason, and that reason is the
/// one worth logging), then C2 to C11 in number order, so when a block breaks several of those the
/// lowest-numbered one shows, then the judgements about what the block is (C14, C16, C17).
const CHECKS: [Check; 16] = [
    c1_lines,
    c13_no_line_dropped,
    c15_left_to_right_text,
    c2_real_objects,
    c3_each_once,
    c4_distinct_origins,
    c5_readable_text,
    c6_visible_upright,
    c7_left_to_right,
    c8_rows_and_breaks,
    c9_texts,
    c10_frozen,
    c11_rects,
    c14_not_vector_art,
    c16_twins,
    c17_not_a_row_of_cells,
];

/// Refuse a block the editor could corrupt the page with: `Err` names the broken invariant (C1..C17)
/// and the line or object, without any of the page's text. The caller falls back to the single-run
/// editor and logs the message.
pub fn check_editor_invariants(pb: &PageBlocks, block: usize) -> Result<(), String> {
    let Some(b) = pb.blocks.get(block) else {
        return Err(format!("C1: block {block} does not exist ({} blocks)", pb.blocks.len()));
    };
    let specs = editor_lines(pb, block);
    let texts = line_texts(pb, &specs);
    check_specs(pb, b, &specs, &texts)
}

/// [`check_editor_invariants`] on lines and texts already made (the seam the tests use to hand it
/// what [`editor_lines`] and [`line_texts`] would never produce).
pub fn check_specs(pb: &PageBlocks, block: &Block, specs: &[LineSpec], texts: &[String]) -> Result<(), String> {
    let cx = Ctx { pb, block, specs, texts };
    CHECKS.iter().try_for_each(|check| check(&cx))
}

fn c1_lines(cx: &Ctx) -> Result<(), String> {
    if cx.block.lines.is_empty() && cx.specs.is_empty() {
        return Err("C1: the block has no lines".into());
    }
    for (i, s) in cx.specs.iter().enumerate() {
        if s.objects.is_empty() {
            let placeholder = s.placeholder.as_deref().is_some_and(|p| !p.trim().is_empty());
            if !(s.frozen && placeholder) {
                return Err(format!("C1: line {i} has no text object and is not a frozen outlined line with a placeholder"));
            }
        } else if s.placeholder.is_some() {
            return Err(format!("C1: line {i} has text objects and also a placeholder"));
        }
    }
    Ok(())
}

fn c13_no_line_dropped(cx: &Ctx) -> Result<(), String> {
    if cx.specs.len() != cx.block.lines.len() {
        return Err(format!(
            "C13: the block has {} lines but {} were offered to the editor (frozen lines must stay in the list)",
            cx.block.lines.len(),
            cx.specs.len()
        ));
    }
    Ok(())
}

fn c2_real_objects(cx: &Ctx) -> Result<(), String> {
    for (i, o) in cx.members() {
        if o == usize::MAX {
            return Err(format!("C2: line {i} names usize::MAX, the drawn-word placeholder, not an object"));
        }
        match cx.pb.runs.get(&o) {
            None => {
                let what = if cx.pb.shapes.iter().any(|s| s.object == o) { "a drawn path or group, not text" } else { "not a text object with area on this page" };
                return Err(format!("C2: line {i} names object {o}, which is {what}"));
            }
            Some(r) if !has_area(&r.rect) => return Err(format!("C2: line {i} names object {o}, which has no area at all")),
            Some(_) => {}
        }
    }
    Ok(())
}

fn c3_each_once(cx: &Ctx) -> Result<(), String> {
    let mut seen: HashMap<usize, usize> = HashMap::new();
    for (i, o) in cx.members() {
        if let Some(first) = seen.insert(o, i) {
            return Err(if first == i { format!("C3: object {o} appears twice in line {i}") } else { format!("C3: object {o} is in line {first} and again in line {i}") });
        }
    }
    Ok(())
}

fn c4_distinct_origins(cx: &Ctx) -> Result<(), String> {
    let mut points: Vec<(f32, f32, usize)> = Vec::new();
    for (_, o) in cx.members() {
        if cx.pb.excluded.shadowed.contains(&o) {
            return Err(format!("C4: object {o} is a shadowed twin of an earlier object, and a twin is never a member"));
        }
        let r = cx.run(o)?;
        points.push((r.origin.x, r.origin.y, o));
    }
    points.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.2.cmp(&b.2)));
    for (i, a) in points.iter().enumerate() {
        for b in &points[i + 1..] {
            if b.0 - a.0 > SAME_ORIGIN_PT {
                break;
            }
            let d = (b.0 - a.0).hypot(b.1 - a.1);
            if d <= SAME_ORIGIN_PT {
                return Err(format!("C4: objects {} and {} share a baseline origin ({d:.2} pt apart): apply would resolve both to one operator", a.2, b.2));
            }
        }
    }
    Ok(())
}

fn c5_readable_text(cx: &Ctx) -> Result<(), String> {
    for (_, o) in cx.members() {
        if cx.run(o)?.text.trim().is_empty() {
            return Err(format!("C5: object {o} has blank text (a font with no ToUnicode, or a faux-bold twin)"));
        }
    }
    Ok(())
}

fn c6_visible_upright(cx: &Ctx) -> Result<(), String> {
    for (_, o) in cx.members() {
        let r = cx.run(o)?;
        if r.color.a == 0 {
            return Err(format!("C6: object {o} is invisible (alpha 0)"));
        }
        // (a non-finite rect has no area and C2 has already refused it)
        if !r.origin.x.is_finite() || !r.origin.y.is_finite() || !r.size.is_finite() {
            return Err(format!("C6: object {o} has non-finite geometry"));
        }
        if r.size <= 0.0 {
            return Err(format!("C6: object {o} has size {} (an effective size of 0 is a rotated or collapsed run)", r.size));
        }
        let style = cx.pb.styles.get(&o);
        if !upright(style) {
            let axis = style.map_or((1.0, 0.0), |s| s.axis);
            return Err(format!("C6: object {o} is rotated (text axis {:.2},{:.2})", axis.0, axis.1));
        }
        if looks_rotated(&r.rect, r.text.trim().chars().count()) {
            return Err(format!("C6: object {o} looks rotated (its box is taller than it is wide)"));
        }
    }
    Ok(())
}

fn c7_left_to_right(cx: &Ctx) -> Result<(), String> {
    for (i, s) in cx.specs.iter().enumerate() {
        for pair in s.objects.windows(2) {
            let (a, b) = (left_of(cx.run(pair[0])?), left_of(cx.run(pair[1])?));
            if b < a {
                return Err(format!("C7: line {i} is not left to right: object {} starts at {b:.1}, left of object {} at {a:.1}", pair[1], pair[0]));
            }
        }
    }
    Ok(())
}

/// How much of the horizontal stretch `lo..hi` the outlined words of line `line` fill: the length of
/// the union of their boxes' x extents, clipped to it. Only asked for when a gap is already wider than
/// a line may span, so the one pass over the page's paths is rare.
fn drawn_width_between(cx: &Ctx, line: usize, lo: f32, hi: f32) -> f32 {
    let Some(l) = cx.block.lines.get(line) else { return 0.0 };
    if l.outlined.is_empty() || hi <= lo {
        return 0.0;
    }
    let wanted: HashSet<usize> = l.outlined.iter().copied().collect();
    let mut spans: Vec<(f32, f32)> = cx
        .pb
        .shapes
        .iter()
        .filter(|s| s.depth == 0 && wanted.contains(&s.object) && finite_rect(&s.rect))
        .map(|s| (s.rect.left.min(s.rect.right).max(lo), s.rect.left.max(s.rect.right).min(hi)))
        .filter(|(a, b)| b > a)
        .collect();
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut filled = 0.0;
    let mut reach = lo;
    for (a, b) in spans {
        filled += (b - a.max(reach)).max(0.0);
        reach = reach.max(b);
    }
    filled
}

fn c8_rows_and_breaks(cx: &Ctx) -> Result<(), String> {
    // per line: the baseline of its first object and its em; `None` for a line with no text object,
    // which has no measured baseline (the detector estimates one from the rect: an estimate is not
    // something to hold a margin of 0.4 em against)
    let mut rows: Vec<Option<(f32, f32)>> = Vec::with_capacity(cx.specs.len());
    for (i, s) in cx.specs.iter().enumerate() {
        if s.objects.is_empty() {
            rows.push(None);
            continue;
        }
        let first = cx.run(s.objects[0])?;
        let mut em = 0.0f32;
        for &o in &s.objects {
            em = em.max(cx.run(o)?.size);
        }
        for &o in &s.objects[1..] {
            let d = (cx.run(o)?.origin.y - first.origin.y).abs();
            if d > ROW_EM * em {
                return Err(format!("C8: line {i} mixes baselines: object {o} sits {d:.1} pt from the line's first object"));
            }
        }
        // Frozen lines too: a currency sign drawn as a path froze a whole receipt row, and 12.7 em
        // between its cells went unmeasured. What a line's outlined words fill of a gap does not
        // count as gap: a hole where a long outlined word sits is not two columns.
        let mut right = f32::MIN;
        for pair in s.objects.windows(2) {
            let (a, b) = (cx.run(pair[0])?, cx.run(pair[1])?);
            right = right.max(a.rect.left.max(a.rect.right));
            let gap = left_of(b) - right;
            let most = MAX_LINE_GAP_EM * a.size.max(b.size);
            if gap > most {
                let open = gap - drawn_width_between(cx, i, right, left_of(b));
                if open > most {
                    return Err(format!("C8: line {i} spans a {open:.1} pt gap between objects {} and {}: two columns in one line", pair[0], pair[1]));
                }
            }
        }
        rows.push(Some((first.origin.y, em)));
    }
    for i in 1..rows.len() {
        match (rows[i - 1], rows[i]) {
            (Some(above), Some(here)) => {
                let em = here.1.max(above.1);
                if here.0 - above.0 <= LINE_BELOW_EM * em {
                    return Err(format!("C8: line {i} does not lie below line {}", i - 1));
                }
            }
            // two lines drawn as shapes: nothing about either is measured (both are the detector's
            // estimates), and nothing in them is ever written, so there is nothing to hold to an order
            (None, None) => {}
            // a line drawn as shapes beside a line of text: ordered by what is measured, the rects. A
            // line below another starts lower and ends lower; a symbol standing beside a label (the
            // same rows, or a rect reaching above it) is not a line below it. (Rects that are not
            // finite are C11's to refuse.)
            _ => {
                let (above, here) = (&cx.specs[i - 1].rect, &cx.specs[i].rect);
                if finite_rect(above) && finite_rect(here) {
                    let (above, here) = (normalised(above), normalised(here));
                    if !(here.top > above.top && here.bottom > above.bottom) {
                        return Err(format!("C8: line {i} does not lie below line {}: a line drawn as shapes has to start and end lower than the line above it", i - 1));
                    }
                }
            }
        }
    }
    if let Some(i) = cx.texts.iter().position(|t| t.contains('\n') || t.contains('\r')) {
        return Err(format!("C8: the text of line {i} holds a line break"));
    }
    let buffer_lines = cx.texts.join("\n").split('\n').count();
    if buffer_lines != cx.specs.len() {
        return Err(format!("C8: {} line texts joined by newlines make {buffer_lines} lines, not {}", cx.texts.len(), cx.specs.len()));
    }
    Ok(())
}

fn c9_texts(cx: &Ctx) -> Result<(), String> {
    if cx.texts.len() != cx.specs.len() {
        return Err(format!("C9: {} line texts for {} lines", cx.texts.len(), cx.specs.len()));
    }
    for (i, (s, t)) in cx.specs.iter().zip(cx.texts).enumerate() {
        if s.objects.is_empty() {
            // (C1 has already refused a blank placeholder)
            if s.placeholder.as_deref() != Some(t.as_str()) || t.ends_with('-') {
                return Err(format!("C9: the text of placeholder line {i} must be its placeholder and must not end in '-'"));
            }
        } else {
            let mut want = String::new();
            for &o in &s.objects {
                want.push_str(&sanitise(&cx.run(o)?.text));
            }
            if *t != want {
                return Err(format!(
                    "C9: the text of line {i} is not the plain concatenation of its {} objects' text ({} characters, expected {})",
                    s.objects.len(),
                    t.chars().count(),
                    want.chars().count()
                ));
            }
        }
    }
    Ok(())
}

fn c10_frozen(cx: &Ctx) -> Result<(), String> {
    for (i, (s, l)) in cx.specs.iter().zip(&cx.block.lines).enumerate() {
        if !l.outlined.is_empty() && !s.frozen {
            return Err(format!("C10: line {i} carries {} outlined words but is not frozen: apply would rewrite it around paths that stay drawn", l.outlined.len()));
        }
    }
    Ok(())
}

fn c11_rects(cx: &Ctx) -> Result<(), String> {
    for (i, s) in cx.specs.iter().enumerate() {
        if !finite_rect(&s.rect) {
            return Err(format!("C11: the rect of line {i} is not finite"));
        }
        if s.rect.left > s.rect.right || s.rect.top > s.rect.bottom {
            return Err(format!("C11: the rect of line {i} is not normalised (left > right or top > bottom)"));
        }
    }
    Ok(())
}

/// **Vector art is not a paragraph.** A QR code under a label, a chart, a CAD symbol: paths the size of
/// a word that the detector bridges into the label's lines as "outlined words", one phantom line per
/// row of modules. Such a block has more lines drawn as shapes than lines with text. (A real paragraph
/// can have lines drawn as outlines too, a line with a ligature in it, and the datasheet has one with
/// two of its four lines drawn as outlines: only a majority of them is vector art. A tie is not: the
/// literal "at least as many" would refuse that paragraph, and, in a reading without the thin hyphen
/// that ends one of its drawn lines, it is exactly a tie.) The detector is the first place to stop it
/// bridging them; this is the second.
///
/// A block with no text object at all is not judged: there is nothing in it to open or write, and no
/// click can reach it. "Fewer than three text objects with three or more drawn lines" is a case of
/// this rule (it has at most two lines with text), not a second rule.
fn c14_not_vector_art(cx: &Ctx) -> Result<(), String> {
    let with_text = cx.specs.iter().filter(|s| !s.objects.is_empty()).count();
    let drawn = cx.specs.len() - with_text;
    if with_text > 0 && drawn > with_text {
        return Err(format!("C14: {drawn} of the block's {} lines are drawn shapes and only {with_text} hold text: vector art, not a paragraph", cx.specs.len()));
    }
    Ok(())
}

/// Whether `c` is a strong right-to-left character: Hebrew, Arabic, Syriac, Thaana, N'Ko, Samaritan,
/// Mandaic and their extensions and presentation forms, the right-to-left scripts of the supplementary
/// planes (Phoenician, Kharoshthi, Adlam, ...), and the marks that make text run right to left.
/// Arabic-Indic digits are in the Arabic block: a block holding them comes from a right-to-left document.
fn is_rtl(c: char) -> bool {
    matches!(c as u32, 0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF | 0x10800..=0x10FFF | 0x1E800..=0x1EFFF | 0x200F | 0x202B | 0x202E | 0x2067)
}

/// **No right-to-left text.** The detector lays a right-to-left line out by geometry but its margin
/// rules (indent, short last line) read the left edge, the editor shows the visual order of the
/// objects (left to right: the words of a Farsi line in reverse), and apply writes the typed line into
/// the leftmost object, which is the line's last word. Refused, so the click opens the single run.
fn c15_left_to_right_text(cx: &Ctx) -> Result<(), String> {
    for (i, o) in cx.members() {
        // (a member that is not a run is C2's to refuse)
        if cx.pb.runs.get(&o).is_some_and(|r| r.text.chars().any(is_rtl)) {
            return Err(format!("C15: line {i} holds right-to-left text (object {o}): the editor reads lines left to right"));
        }
    }
    Ok(())
}

/// **Twins are real.** Apply removes them with their line, so a wrong one deletes something that is
/// not a copy of anything: each must be a blank or shadowed text object of this page that no block
/// uses as a member, listed once in one line, and coincide with an object of that line.
fn c16_twins(cx: &Ctx) -> Result<(), String> {
    let members: HashMap<usize, usize> = cx.members().map(|(i, o)| (o, i)).collect();
    let mut listed: HashMap<usize, usize> = HashMap::new();
    for (i, s) in cx.specs.iter().enumerate() {
        for &t in &s.twins {
            if let Some(line) = members.get(&t) {
                return Err(format!("C16: object {t} is a member of line {line} and also a twin in line {i}"));
            }
            if cx.pb.by_object.contains_key(&t) {
                return Err(format!("C16: line {i} lists object {t} as a twin, but it is a member of another block"));
            }
            let Some(twin) = cx.pb.runs.get(&t) else {
                return Err(format!("C16: line {i} lists object {t} as a twin, which is not a text object with area of this page"));
            };
            if !matches!(cx.pb.excluded.reason(t), Some("blank" | "shadowed")) {
                return Err(format!("C16: line {i} lists object {t} as a twin, but it is neither blank nor shadowed"));
            }
            if let Some(first) = listed.insert(t, i) {
                return Err(if first == i { format!("C16: twin {t} is listed twice in line {i}") } else { format!("C16: twin {t} is listed in line {first} and again in line {i}") });
            }
            let twin_box = normalised(&twin.rect);
            let coincides = s.objects.iter().filter_map(|m| cx.pb.runs.get(m)).any(|m| {
                (m.origin.x - twin.origin.x).hypot(m.origin.y - twin.origin.y) <= SAME_ORIGIN_PT && edge_gaps(&normalised(&m.rect), &twin_box).iter().all(|g| *g <= TWIN_RECT_PT)
            });
            if !coincides {
                return Err(format!("C16: line {i} lists object {t} as a twin, but it coincides with none of the line's objects"));
            }
        }
    }
    Ok(())
}

/// A list item's number or bullet standing alone as a text object: "1." "2)" "(a)" "[12]" "iv." (at most five
/// characters, closed by a full stop or a bracket, with a letter or digit in it) or one bullet glyph or dash.
fn is_list_marker(text: &str) -> bool {
    let t = text.trim();
    let n = t.chars().count();
    match t.chars().last() {
        Some(last) if (2..=5).contains(&n) => matches!(last, '.' | ')' | ']') && t.chars().any(char::is_alphanumeric),
        Some(last) if n == 1 => "•·▪◦●‣∙-–—*".contains(last),
        _ => false,
    }
}

/// **A short block with a wide gap inside a line is a row of cells, not a sentence.** Retyping a line
/// writes all of its words into the first piece and removes the others, so two table cells that the
/// detector let share a line (an amount and the date beside it, 1.9 em apart and no rule between) would
/// be run together and the second one deleted. A paragraph's justified lines have wide gaps of their own
/// (1.7 em in the datasheet's narrow columns) and every other line says they are justified, so only
/// blocks of at most [`CELL_ROW_MAX_LINES`] lines are judged; a frozen line is never written, so it is
/// not either. Measured on 55 real pages (quotations, purchase orders, invoices, a lux report, datasheets),
/// against two hand-labelled truths: of the 22 blocks on the quotation, purchase-order and invoice pages
/// that put cells of one row into one line, 19 were opened by the app and now all 22 are refused (the click
/// then opens the one word, as it always did); no block that the labelers call one block is lost to it
/// (the numbered terms are exempt, see below) and the datasheet loses no paragraph. A list item's own number ([`is_list_marker`]) stands apart from its words and is exempt.
fn c17_not_a_row_of_cells(cx: &Ctx) -> Result<(), String> {
    if cx.specs.len() > CELL_ROW_MAX_LINES {
        return Ok(());
    }
    for (i, s) in cx.specs.iter().enumerate() {
        if s.frozen {
            continue;
        }
        let mut right = f32::MIN;
        for (k, pair) in s.objects.windows(2).enumerate() {
            let (a, b) = (cx.run(pair[0])?, cx.run(pair[1])?);
            right = right.max(a.rect.left.max(a.rect.right));
            let gap = left_of(b) - right;
            // a list item's own number stands 1.5 em before its words ("1." and the clause, in the terms of every
            // quotation and order): the item is one block, and it is the number's line that gets retyped whole
            if k == 0 && is_list_marker(&a.text) {
                continue;
            }
            if gap >= CELL_GAP_EM * a.size.max(b.size) {
                return Err(format!("C17: line {i} has a {gap:.1} pt gap between objects {} and {}: cells of a row, not one sentence", pair[0], pair[1]));
            }
        }
    }
    Ok(())
}

// ------------------------------------------------------------------------------------------------
// The log line
// ------------------------------------------------------------------------------------------------

/// Which way a click went.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PickPath {
    /// The seed's whole block opened in the paragraph editor.
    Block,
    /// One run opened alone (a block of one object, an unreadable or invisible seed, a failed guard).
    Single,
    /// A paragraph the person joined by hand.
    Joined,
    /// A word drawn as outlines.
    Drawn,
    /// Rotated text, refused.
    RefusedRotated,
    /// Nothing was there.
    #[default]
    None,
}

impl PickPath {
    pub fn as_str(self) -> &'static str {
        match self {
            PickPath::Block => "block",
            PickPath::Single => "single",
            PickPath::Joined => "joined",
            PickPath::Drawn => "drawn",
            PickPath::RefusedRotated => "refused-rotated",
            PickPath::None => "none",
        }
    }
}

/// What one click decided, content-free (no word of the page): ids, counts, geometry, timings.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PickTrace {
    /// 0-based, as everywhere in the app; [`format_pick_line`] prints it 1-based like the page label.
    pub page: usize,
    pub click: (f32, f32),
    pub seed: Option<usize>,
    pub rule: Option<SeedRule>,
    pub path: PickPath,
    /// Admitted objects on the page, and blocks the detector made of them.
    pub frags: usize,
    pub blocks: usize,
    pub block: Option<usize>,
    pub lines: usize,
    /// Text objects of the block, and outlined words bridged into its lines.
    pub objects: usize,
    pub outlined: usize,
    /// Faux-bold twins the block's members carry (see [`LineSpec::twins`]): removed with their lines.
    pub twins: usize,
    /// Indices of the lines that carry outlined words.
    pub frozen: Vec<usize>,
    /// Why this block starts, and why the next block (in detector order) starts: the second says why
    /// this paragraph stopped.
    pub starts: Option<&'static str>,
    pub next_starts: Option<&'static str>,
    /// The block's rect: left, top, right, bottom.
    pub rect: Option<[f32; 4]>,
    /// The distinct font ids of the block's objects (`u32::MAX`, an object with no style, prints as `?`).
    pub fonts: Vec<u32>,
    pub cache_hit: bool,
    pub build_ms: f32,
    pub detect_ms: f32,
    pub total_ms: f32,
}

impl PickTrace {
    pub fn new(page: usize, click: (f32, f32)) -> PickTrace {
        PickTrace { page, click, ..PickTrace::default() }
    }

    /// The page-wide facts, from the cache entry the click was resolved against.
    pub fn page_facts(&mut self, pb: &PageBlocks) {
        self.page = pb.page;
        self.frags = pb.frags.len();
        self.blocks = pb.blocks.len();
        self.build_ms = pb.build_ms;
        self.detect_ms = pb.detect_ms;
    }

    /// The facts of the block the click opened (or tried to).
    pub fn block_facts(&mut self, pb: &PageBlocks, block: usize) {
        let Some(b) = pb.blocks.get(block) else {
            self.block = None;
            return;
        };
        self.block = Some(block);
        self.lines = b.lines.len();
        self.objects = b.lines.iter().map(|l| l.objects.len()).sum();
        self.outlined = b.lines.iter().map(|l| l.outlined.len()).sum();
        self.twins = b.lines.iter().flat_map(|l| l.objects.iter()).map(|o| pb.twins.get(o).map_or(0, Vec::len)).sum();
        self.frozen = b.lines.iter().enumerate().filter(|(_, l)| !l.outlined.is_empty()).map(|(i, _)| i).collect();
        self.starts = Some(b.starts_because);
        self.next_starts = pb.blocks.get(block + 1).map(|n| n.starts_because);
        self.rect = Some([b.left, b.top, b.right, b.bottom]);
        let mut fonts: Vec<u32> = b.lines.iter().flat_map(|l| l.objects.iter()).map(|o| pb.styles.get(o).map_or(u32::MAX, |s| s.font)).collect();
        fonts.sort_unstable();
        fonts.dedup();
        self.fonts = fonts;
    }
}

/// The one line the session log gets per click, kind `"pick"`. Key=value pairs, no page content:
///
/// `page 1 click=(212.4,501.2) seed=1029 rule=exact path=block frags=770 blocks=129 block=106 lines=13
/// objects=57 outlined=2 twins=0 frozen=[8,10] starts="style" next_starts="style" rect=[187.0,421.0,306.0,538.0]
/// fonts=[1] cache=miss build_ms=38.1 detect_ms=1.2 total_ms=41.0`
pub fn format_pick_line(t: &PickTrace) -> String {
    fn list<I: IntoIterator<Item = String>>(items: I) -> String {
        format!("[{}]", items.into_iter().collect::<Vec<_>>().join(","))
    }
    let some = |v: Option<usize>| v.map_or("none".to_string(), |v| v.to_string());
    let reason = |v: Option<&'static str>| v.map_or("none".to_string(), |v| format!("{v:?}"));
    let rect = t.rect.map_or("none".to_string(), |r| format!("[{:.1},{:.1},{:.1},{:.1}]", r[0], r[1], r[2], r[3]));
    format!(
        "page {} click=({:.1},{:.1}) seed={} rule={} path={} frags={} blocks={} block={} lines={} objects={} outlined={} twins={} frozen={} starts={} next_starts={} rect={} fonts={} cache={} build_ms={:.1} detect_ms={:.1} total_ms={:.1}",
        t.page + 1,
        t.click.0,
        t.click.1,
        some(t.seed),
        t.rule.map_or("none", SeedRule::as_str),
        t.path.as_str(),
        t.frags,
        t.blocks,
        some(t.block),
        t.lines,
        t.objects,
        t.outlined,
        t.twins,
        list(t.frozen.iter().map(|i| i.to_string())),
        reason(t.starts),
        reason(t.next_starts),
        rect,
        list(t.fonts.iter().map(|f| if *f == u32::MAX { "?".to_string() } else { f.to_string() })),
        if t.cache_hit { "hit" } else { "miss" },
        t.build_ms,
        t.detect_ms,
        t.total_ms,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::Line;
    use pdf_core::document::{Color, Point};

    // ---- builders -------------------------------------------------------------------------

    /// A run of `text` starting at `left` on `baseline`: 4.5 pt a character, size 8, black-ish, opaque.
    fn tr(object: usize, text: &str, left: f32, baseline: f32) -> TextRun {
        let w = 4.5 * text.chars().count().max(1) as f32;
        TextRun {
            object,
            text: text.to_string(),
            rect: Rect { left, top: baseline - 6.0, right: left + w, bottom: baseline + 1.5 },
            origin: Point { x: left, y: baseline },
            size: 8.0,
            color: Color { r: 10, g: 20, b: 30, a: 255 },
        }
    }

    fn with_rect(mut r: TextRun, left: f32, top: f32, right: f32, bottom: f32) -> TextRun {
        r.rect = Rect { left, top, right, bottom };
        r.origin = Point { x: left, y: bottom - 1.5 };
        r
    }

    fn style(font: u32) -> RunStyle {
        RunStyle { font, stem_milli_em: Some(51), axis: (1.0, 0.0) }
    }

    fn styles_of(runs: &[TextRun]) -> HashMap<usize, RunStyle> {
        runs.iter().map(|r| (r.object, style(1))).collect()
    }

    fn adapt_runs(runs: &[TextRun]) -> Adapted {
        adapt(runs, &styles_of(runs), &HashMap::new(), &[])
    }

    fn pb_of(runs: Vec<TextRun>) -> PageBlocks {
        let st = styles_of(&runs);
        build_page_blocks_from(0, 7, 9, runs, st, HashMap::new(), Vec::new())
    }

    fn drawn(object: usize, kind: DrawnKind, depth: usize, l: f32, t: f32, r: f32, b: f32) -> DrawnObject {
        DrawnObject { object, kind, rect: Rect { left: l, top: t, right: r, bottom: b }, label: String::new(), depth, opacity: 1.0, movable: depth == 0 }
    }

    fn ids(frags: &[Frag]) -> Vec<usize> {
        frags.iter().map(|f| f.object).collect()
    }

    // ---- looks_rotated: the same cases as main.rs's ---------------------------------------

    #[test]
    fn looks_rotated_matches_the_apps_own_rule() {
        let tall = Rect { left: 429.6, top: 74.6, right: 434.2, bottom: 91.6 };
        assert!(looks_rotated(&tall, "54mm".chars().count()), "the real rotated '54mm' label");
        let line = Rect { left: 194.6, top: 127.8, right: 366.6, bottom: 135.4 };
        assert!(!looks_rotated(&line, 38), "an ordinary line of text");
        let narrow = Rect { left: 0.0, top: 0.0, right: 8.0, bottom: 20.0 };
        assert!(!looks_rotated(&narrow, 3), "'Ill' is legitimately taller than wide");
        assert!(looks_rotated(&narrow, 4), "four characters in that box are not");
        let thin = Rect { left: 0.0, top: 0.0, right: 1.0, bottom: 50.0 };
        assert!(!looks_rotated(&thin, 0), "an empty run is never flagged");
        let edge = Rect { left: 0.0, top: 0.0, right: 10.0, bottom: 12.0 };
        assert!(!looks_rotated(&edge, 9), "exactly 1.2 times taller is not more than 1.2 times");
        let flipped = Rect { left: 434.2, top: 91.6, right: 429.6, bottom: 74.6 };
        assert!(looks_rotated(&flipped, 4), "a flipped rect measures the same");
    }

    // ---- adapt: admission -----------------------------------------------------------------

    #[test]
    fn an_ordinary_run_becomes_one_frag_with_everything_carried_over() {
        let mut run = tr(7, "Hello ", 10.0, 20.0);
        run.color = Color { r: 1, g: 2, b: 3, a: 255 };
        run.size = 8.5;
        let styles = HashMap::from([(7, RunStyle { font: 4, stem_milli_em: Some(124), axis: (1.0, 0.0) })]);
        let faces = HashMap::from([(7, "BCDIEE+OpenSans-Regular".to_string())]);
        let a = adapt(&[run], &styles, &faces, &[]);
        assert_eq!(
            a.frags,
            vec![Frag {
                object: 7,
                left: 10.0,
                top: 14.0,
                right: 37.0,
                bottom: 21.5,
                baseline: 20.0,
                size: 8.5,
                font: 4,
                stem: Some(124),
                face: "OpenSans-Regular".into(),
                rgb: [1, 2, 3],
                text: "Hello ".into(),
                rotated: false
            }]
        );
        assert_eq!(a.excluded, Excluded::default());
        assert!(a.shapes.is_empty());
    }

    #[test]
    fn blank_text_is_excluded_and_recorded() {
        let runs = [tr(1, "ok", 10.0, 20.0), tr(2, "", 30.0, 20.0), tr(3, "   ", 50.0, 20.0), tr(4, "\u{a0}\t\n", 70.0, 20.0)];
        let a = adapt_runs(&runs);
        assert_eq!(ids(&a.frags), vec![1]);
        assert_eq!(a.excluded.blank, vec![2, 3, 4]);
    }

    #[test]
    fn alpha_zero_is_invisible_and_any_other_alpha_is_visible() {
        let mut hidden = tr(1, "hidden", 10.0, 20.0);
        hidden.color.a = 0;
        let mut faint = tr(2, "faint", 60.0, 20.0);
        faint.color.a = 1;
        let a = adapt_runs(&[hidden, faint]);
        assert_eq!(a.excluded.invisible, vec![1]);
        assert_eq!(ids(&a.frags), vec![2]);
    }

    #[test]
    fn a_size_that_is_zero_negative_or_not_a_number_is_degenerate() {
        let mut runs = Vec::new();
        for (i, size) in [0.0, -3.0, f32::NAN, f32::INFINITY].into_iter().enumerate() {
            let mut r = tr(10 + i, "text", 10.0 + 40.0 * i as f32, 20.0);
            r.size = size;
            runs.push(r);
        }
        let mut tiny = tr(20, "tiny", 300.0, 20.0);
        tiny.size = 0.01;
        runs.push(tiny);
        let a = adapt_runs(&runs);
        assert_eq!(a.excluded.degenerate, vec![10, 11, 12, 13]);
        assert_eq!(ids(&a.frags), vec![20], "a positive size is a size");
    }

    #[test]
    fn no_area_at_all_or_a_non_finite_rect_or_origin_is_degenerate() {
        let no_width = with_rect(tr(1, "text", 0.0, 0.0), 10.0, 10.0, 10.0, 20.0); // zero wide
        let no_height = with_rect(tr(2, "text", 0.0, 0.0), 10.0, 10.0, 20.0, 10.0); // zero tall
        let nan = with_rect(tr(3, "text", 0.0, 0.0), f32::NAN, 10.0, 20.0, 20.0);
        let inf = with_rect(tr(4, "text", 0.0, 0.0), 10.0, 10.0, f32::INFINITY, 20.0);
        let mut lost = tr(5, "text", 100.0, 20.0);
        lost.origin.y = f32::NAN;
        let a = adapt_runs(&[no_width, no_height, nan, inf, lost]);
        assert_eq!(a.excluded.degenerate, vec![1, 2, 3, 4, 5]);
        assert!(a.frags.is_empty());
    }

    #[test]
    fn a_thin_glyph_has_area_however_little() {
        // the real ones from the datasheet: the 'l' of "reliable" (0.40 x 5.94 pt), a hyphen (2.06 x 0.36)
        // and the dot of "SATURATION . INTENSITY" (2.63 x 0.46): none is 0.5 pt on both sides
        let l = with_rect(tr(1, "l", 0.0, 0.0), 525.36, 391.50, 525.76, 397.44);
        let hyphen = with_rect(tr(2, "-", 0.0, 0.0), 565.54, 500.71, 567.59, 501.07);
        let dot = with_rect(tr(3, " . ", 0.0, 0.0), 469.62, 828.92, 472.26, 829.38);
        // the smallest positive area there is is still area
        let sliver = with_rect(tr(4, "x", 0.0, 0.0), 10.0, 10.0, 10.0 + f32::EPSILON * 8.0, 10.0 + 1e-3);
        let a = adapt_runs(&[l, hyphen, dot, sliver]);
        assert_eq!(ids(&a.frags), vec![1, 2, 3, 4]);
        assert!(a.excluded.degenerate.is_empty(), "{:?}", a.excluded);
        assert_eq!(a.frags[0].text, "l");
        // and the same objects are runs, so a click and the guard (C2) know them
        let pb = pb_of(vec![with_rect(tr(1, "l", 0.0, 0.0), 525.36, 391.50, 525.76, 397.44)]);
        assert!(pb.runs.contains_key(&1) && pb.by_object.contains_key(&1));
    }

    #[test]
    fn a_thin_glyph_stays_in_its_lines_text_between_its_neighbours() {
        // "reliable" cut as the datasheet cuts it: re | l | ia | b | l | e, the two l's 0.4 pt wide
        let w = 4.5;
        let at = |o, text: &str, left: f32, width: f32| {
            let mut r = with_rect(tr(o, text, 0.0, 0.0), left, 14.0, left + width, 21.5);
            r.origin = Point { x: left - 0.5, y: 20.0 };
            r
        };
        let runs = vec![at(1, "re", 10.0, 2.0 * w), at(2, "l", 20.2, 0.4), at(3, "ia", 22.0, 2.0 * w), at(4, "b", 31.0, w), at(5, "l", 35.8, 0.4), at(6, "e", 37.0, w)];
        let pb = pb_of(runs);
        assert_eq!(pb.frags.len(), 6);
        assert_eq!(pb.blocks.len(), 1, "one line");
        let texts = line_texts(&pb, &editor_lines(&pb, 0));
        assert_eq!(texts, vec!["reliable".to_string()], "the editor shows the word whole");
        assert_eq!(check_editor_invariants(&pb, 0), Ok(()));
    }

    #[test]
    fn a_flipped_rect_is_normalised() {
        let flipped = with_rect(tr(1, "text", 0.0, 0.0), 50.0, 30.0, 10.0, 20.0); // left > right, top > bottom
        let a = adapt_runs(&[flipped]);
        let f = &a.frags[0];
        assert_eq!((f.left, f.top, f.right, f.bottom), (10.0, 20.0, 50.0, 30.0));
    }

    #[test]
    fn line_breaks_in_the_text_become_spaces() {
        let a = adapt_runs(&[tr(1, "a\nb\r\nc\rd", 10.0, 20.0)]);
        assert_eq!(a.frags[0].text, "a b  c d");
    }

    #[test]
    fn a_later_object_at_the_same_origin_is_shadowed_and_the_first_stays() {
        let first = tr(5, "Color", 20.0, 700.0);
        let mut twin = tr(9, "Color", 20.0, 700.0);
        twin.origin.x += 0.9; // 0.9 pt apart: a faux-bold offset
        let mut apart = tr(11, "Color", 20.0, 700.0);
        apart.origin.x += 1.2; // 1.2 pt apart: a different object
        // handed over in reverse: the order of the slice does not decide who is first
        let a = adapt_runs(&[apart, twin, first]);
        assert_eq!(ids(&a.frags), vec![5, 11]);
        assert_eq!(a.excluded.shadowed, vec![9]);
    }

    #[test]
    fn origins_in_neighbouring_cells_are_still_compared() {
        // 0.3 pt apart across a cell border at x = 15 (cells are 1 pt wide)
        let a = tr(1, "ab", 14.9, 20.0);
        let b = tr(2, "ab", 15.2, 20.0);
        let r = adapt_runs(&[a, b]);
        assert_eq!(r.excluded.shadowed, vec![2]);
    }

    #[test]
    fn a_blank_layer_does_not_shadow_the_readable_one_after_it() {
        let blank = tr(1, "", 20.0, 700.0);
        let real = tr(2, "Color", 20.0, 700.0);
        let a = adapt_runs(&[blank, real]);
        assert_eq!(a.excluded.blank, vec![1]);
        assert_eq!(ids(&a.frags), vec![2]);
        assert!(a.excluded.shadowed.is_empty());
    }

    #[test]
    fn a_neighbour_that_shares_an_origin_but_not_a_box_is_not_a_twin() {
        // the real pair from the datasheet, ": " (0.7 pt of ink) and the " Ma" after it, squeezed to 0.9 pt apart
        let colon = with_rect(tr(1, ": ", 0.0, 0.0), 61.07, 652.99, 61.74, 657.23);
        let mut ma = with_rect(tr(2, " Ma", 0.0, 0.0), 61.89, 651.6, 74.83, 657.23);
        ma.origin = Point { x: colon.origin.x + 0.9, y: colon.origin.y };
        let a = adapt_runs(&[colon, ma]);
        assert_eq!(ids(&a.frags), vec![1, 2], "different words are not a twin however close their origins");
        assert!(a.excluded.shadowed.is_empty());
    }

    #[test]
    fn an_object_a_little_below_another_is_not_a_twin_and_twins_across_a_cell_border_are() {
        let a = tr(1, "word", 20.0, 700.0);
        let below = tr(2, "word", 20.0, 701.5); // same x, same box shape, 1.5 pt lower: not the same place
        assert_eq!(ids(&adapt_runs(&[a, below]).frags), vec![1, 2]);
        // 0.3 pt apart, either side of the 1 pt cell border at y = 701
        let high = tr(3, "word", 20.0, 700.9);
        let low = tr(4, "word", 20.0, 701.2);
        let r = adapt_runs(&[high, low]);
        assert_eq!((ids(&r.frags), r.excluded.shadowed), (vec![3], vec![4]));
    }

    #[test]
    fn an_origin_at_the_edge_of_the_float_range_does_not_overflow_the_grid() {
        // finite, so admitted, and far enough out that its cell index saturates
        let mut far = tr(1, "far away", 0.0, 0.0);
        far.rect = Rect { left: -1e30, top: -1e30, right: 1e30, bottom: 1e30 };
        far.origin = Point { x: 1e30, y: f32::MAX };
        let mut near_it = far.clone();
        near_it.object = 2;
        near_it.origin = Point { x: 1e30, y: f32::MAX };
        let a = adapt_runs(&[far, near_it]);
        assert_eq!((ids(&a.frags), a.excluded.shadowed), (vec![1], vec![2]), "the same place, however far out");
    }

    #[test]
    fn the_character_gate_counts_the_text_not_its_padding() {
        // two characters of ink in a tall box: not rotated, even padded out to four characters
        let padded = with_rect(tr(1, "ab  ", 0.0, 0.0), 10.0, 10.0, 14.0, 30.0);
        assert!(adapt_runs(&[padded]).excluded.rotated.is_empty());
        let four = with_rect(tr(2, "abcd", 0.0, 0.0), 10.0, 40.0, 14.0, 60.0);
        assert_eq!(adapt_runs(&[four]).excluded.rotated, vec![2]);
    }

    #[test]
    fn excluded_counts_and_names_every_list() {
        let e = Excluded { blank: vec![1], invisible: vec![2], degenerate: vec![3], shadowed: vec![4], rotated: vec![5] };
        assert_eq!(e.count(), 5);
        for (id, why) in [(1, "blank"), (2, "invisible"), (3, "degenerate"), (4, "shadowed"), (5, "rotated")] {
            assert_eq!(e.reason(id), Some(why));
        }
        assert_eq!(Excluded::default().count(), 0);
    }

    #[test]
    fn rotation_by_axis_by_box_and_not_by_a_short_label() {
        let mut styles = HashMap::new();
        let upright_run = tr(1, "upright", 10.0, 20.0);
        let by_axis = tr(2, "sideways", 10.0, 40.0);
        let slight = tr(3, "level", 10.0, 60.0);
        let tilted = tr(4, "tilted", 10.0, 80.0);
        let by_box = with_rect(tr(5, "54mm", 0.0, 0.0), 429.6, 74.6, 434.2, 91.6); // upright axis, tall box
        let short = with_rect(tr(6, "Ill", 0.0, 0.0), 100.0, 100.0, 108.0, 120.0); // 3 chars: not flagged
        styles.insert(1, style(1));
        styles.insert(2, RunStyle { axis: (0.0, -1.0), ..style(1) });
        styles.insert(3, RunStyle { axis: (1.0, 0.019), ..style(1) }); // within 0.02 of (1, 0)
        styles.insert(4, RunStyle { axis: (1.0, 0.021), ..style(1) }); // just outside
        styles.insert(5, style(1));
        styles.insert(6, style(1));
        let a = adapt(&[upright_run, by_axis, slight, tilted, by_box, short], &styles, &HashMap::new(), &[]);
        assert_eq!(a.excluded.rotated, vec![2, 4, 5]);
        assert_eq!(ids(&a.frags), vec![1, 3, 6]);
        assert!(a.frags.iter().all(|f| !f.rotated), "rotated objects never reach the detector");
    }

    #[test]
    fn rotated_text_whose_size_collapsed_to_zero_is_rotated_not_degenerate() {
        // a real page: the effective size of text turned a quarter is 0 (the matrix's d)
        let mut r = tr(1, "166m", 10.0, 20.0);
        r.size = 0.0;
        let styles = HashMap::from([(1, RunStyle { axis: (0.0, -1.0), ..style(1) })]);
        let a = adapt(&[r], &styles, &HashMap::new(), &[]);
        assert_eq!(a.excluded.rotated, vec![1]);
        assert!(a.excluded.degenerate.is_empty());
    }

    #[test]
    fn the_first_matching_reason_wins() {
        let mut both = tr(1, "", 10.0, 20.0);
        both.color.a = 0; // invisible and blank: invisible
        let mut blank_and_empty = tr(2, " ", 10.0, 40.0);
        blank_and_empty.size = 0.0; // blank and size 0: blank
        let mut no_area = with_rect(tr(3, "text", 0.0, 0.0), 10.0, 60.0, 10.0, 70.0);
        no_area.color.a = 0; // no area and invisible: degenerate
        let a = adapt_runs(&[both, blank_and_empty, no_area]);
        assert_eq!(a.excluded.invisible, vec![1]);
        assert_eq!(a.excluded.blank, vec![2]);
        assert_eq!(a.excluded.degenerate, vec![3]);
        assert_eq!(a.excluded.count(), 3);
        assert_eq!(a.excluded.reason(2), Some("blank"));
        assert_eq!(a.excluded.reason(99), None);
    }

    #[test]
    fn the_subset_tag_is_stripped_only_when_it_is_one() {
        assert_eq!(strip_subset_tag("BCDIEE+OpenSans-Regular"), "OpenSans-Regular");
        assert_eq!(strip_subset_tag("Montserrat-Thin"), "Montserrat-Thin");
        assert_eq!(strip_subset_tag("ABC+Foo"), "ABC+Foo", "three letters are not a tag");
        assert_eq!(strip_subset_tag("abcdef+Foo"), "abcdef+Foo", "lower case is not a tag");
        assert_eq!(strip_subset_tag("ABCDE1+Foo"), "ABCDE1+Foo", "a digit is not a letter");
        assert_eq!(strip_subset_tag("  ABCDEF+Foo "), "Foo");
        assert_eq!(strip_subset_tag("ABCDEF+"), "");
        assert_eq!(strip_subset_tag(""), "");
    }

    #[test]
    fn a_missing_style_means_font_max_no_stem_and_upright() {
        let a = adapt(&[tr(1, "text", 10.0, 20.0)], &HashMap::new(), &HashMap::new(), &[]);
        assert_eq!(a.frags[0].font, u32::MAX);
        assert_eq!(a.frags[0].stem, None);
        assert_eq!(a.frags[0].face, "");
        assert!(a.excluded.rotated.is_empty());
        // a style with no measured stem
        let styles = HashMap::from([(1, RunStyle { stem_milli_em: None, ..style(3) })]);
        let b = adapt(&[tr(1, "text", 10.0, 20.0)], &styles, &HashMap::new(), &[]);
        assert_eq!((b.frags[0].font, b.frags[0].stem), (3, None));
    }

    #[test]
    fn the_first_run_of_an_id_wins_and_nothing_panics() {
        let first = tr(1, "first", 10.0, 20.0);
        let second = tr(1, "second", 100.0, 60.0);
        let a = adapt_runs(&[first, second]);
        assert_eq!(a.frags.len(), 1);
        assert_eq!(a.frags[0].text, "first");
        assert_eq!(a.excluded.count(), 0);
        let pb = pb_of(vec![tr(1, "first", 10.0, 20.0), tr(1, "second", 100.0, 60.0)]);
        assert_eq!(pb.runs[&1].text, "first");
    }

    #[test]
    fn adapt_does_not_depend_on_the_order_of_the_runs() {
        let mut runs: Vec<TextRun> = (0..30).map(|i| tr(i, if i % 7 == 3 { "" } else { "word " }, 10.0 + 6.0 * i as f32, 20.0 + 10.0 * (i % 3) as f32)).collect();
        runs[4].size = 0.0;
        let forward = adapt_runs(&runs);
        runs.reverse();
        runs.rotate_left(11);
        assert_eq!(adapt_runs(&runs), forward);
    }

    #[test]
    fn only_paths_become_shapes_and_nesting_becomes_depth() {
        let list = vec![
            drawn(1, DrawnKind::Words, 0, 0.0, 0.0, 10.0, 10.0),
            drawn(2, DrawnKind::Picture, 0, 0.0, 0.0, 10.0, 10.0),
            drawn(3, DrawnKind::Group, 0, 0.0, 0.0, 100.0, 100.0),
            drawn(4, DrawnKind::Shape, 0, 5.0, 6.0, 7.0, 8.0),
            drawn(3, DrawnKind::Shape, 1, 5.0, 6.0, 7.0, 8.0), // a form's child carries the form's own id
            drawn(5, DrawnKind::Shape, 0, 20.0, 40.0, 10.0, 30.0), // flipped
            drawn(6, DrawnKind::Shape, 0, f32::NAN, 0.0, 1.0, 1.0),
        ];
        let a = adapt(&[], &HashMap::new(), &HashMap::new(), &list);
        let got: Vec<(usize, u32)> = a.shapes.iter().map(|s| (s.object, s.depth)).collect();
        assert_eq!(got, vec![(4, 0), (3, 1), (5, 0)]);
        let flipped = a.shapes[2];
        assert_eq!((flipped.left, flipped.top, flipped.right, flipped.bottom), (10.0, 30.0, 20.0, 40.0));
    }

    // ---- build_page_blocks ----------------------------------------------------------------

    #[test]
    fn an_empty_page_has_no_blocks_and_nothing_panics() {
        let pb = build_page_blocks_from(3, 11, 12, Vec::new(), HashMap::new(), HashMap::new(), Vec::new());
        assert!(pb.blocks.is_empty() && pb.frags.is_empty() && pb.by_object.is_empty() && pb.runs.is_empty());
        assert_eq!((pb.page, pb.epoch, pb.generation), (3, 11, 12));
        assert_eq!(pick_seed(&pb, 10.0, 10.0, 3.0), None);
        assert!(check_editor_invariants(&pb, 0).unwrap_err().starts_with("C1:"));
        assert!(editor_lines(&pb, 0).is_empty());
    }

    #[test]
    fn a_page_with_only_excluded_objects_has_no_blocks() {
        let mut hidden = tr(1, "hidden", 10.0, 20.0);
        hidden.color.a = 0;
        let pb = pb_of(vec![hidden, tr(2, " ", 10.0, 40.0)]);
        assert!(pb.blocks.is_empty());
        assert_eq!(pb.runs.len(), 2, "excluded objects with area stay clickable");
        assert_eq!(pick_seed(&pb, 12.0, 18.0, 3.0), Some((1, SeedRule::Exact)));
    }

    #[test]
    fn a_one_run_page_is_one_block_of_one_line() {
        let pb = pb_of(vec![tr(4, "Only words here", 30.0, 100.0)]);
        assert_eq!(pb.blocks.len(), 1);
        assert_eq!(pb.blocks[0].lines.len(), 1);
        assert_eq!(pb.by_object.get(&4), Some(&(0, 0)));
        assert_eq!(pb.frags.len(), 1);
        assert_eq!(check_editor_invariants(&pb, 0), Ok(()));
        assert_eq!(line_texts(&pb, &editor_lines(&pb, 0)), vec!["Only words here".to_string()]);
    }

    #[test]
    fn runs_keeps_every_object_with_area_the_excluded_ones_too_and_no_other() {
        let mut hidden = tr(1, "hidden", 10.0, 20.0);
        hidden.color.a = 0;
        let flat = with_rect(tr(5, "thin", 0.0, 0.0), 10.0, 10.0, 10.0, 20.0); // zero wide: no area
        let thin = with_rect(tr(6, "l", 0.0, 0.0), 90.0, 10.0, 90.2, 20.0); // 0.2 wide: a thin letter (one character: four in such a box would be rotated text)
        let pb = pb_of(vec![hidden, tr(2, "", 10.0, 40.0), tr(3, "real", 10.0, 60.0), tr(4, "real", 10.3, 60.0), flat, thin]);
        let mut have: Vec<usize> = pb.runs.keys().copied().collect();
        have.sort_unstable();
        assert_eq!(have, vec![1, 2, 3, 4, 6]);
        assert_eq!(pb.excluded.degenerate, vec![5], "no area: excluded but not a run");
        assert_eq!(pb.excluded.shadowed, vec![4]);
        assert!(pb.by_object.contains_key(&6), "a thin letter is a member");
    }

    #[test]
    fn the_full_shape_list_is_kept_for_hyphen_marks_at_every_depth() {
        let st = HashMap::new();
        let list = vec![drawn(1, DrawnKind::Shape, 0, 0.0, 0.0, 5.0, 1.0), drawn(2, DrawnKind::Picture, 0, 0.0, 0.0, 5.0, 5.0), drawn(3, DrawnKind::Shape, 2, 0.0, 0.0, 5.0, 1.0)];
        let pb = build_page_blocks_from(0, 0, 0, vec![tr(9, "x y z", 10.0, 20.0)], st, HashMap::new(), list);
        let got: Vec<(usize, usize)> = pb.shapes.iter().map(|s| (s.object, s.depth)).collect();
        assert_eq!(got, vec![(1, 0), (3, 2)]);
    }

    #[test]
    fn build_page_blocks_takes_a_snapshot_the_same_way() {
        let runs = vec![tr(1, "first line of a paragraph ", 10.0, 20.0), tr(2, "second line of it", 10.0, 30.0)];
        let faces = HashMap::from([(1, "ABCDEF+Face".to_string())]);
        let shapes = vec![drawn(7, DrawnKind::Shape, 0, 0.0, 0.0, 50.0, 0.4)];
        let snapshot = PageTextSnapshot { runs: runs.clone(), styles: styles_of(&runs), faces: faces.clone(), shapes };
        let a = build_page_blocks(2, 5, 6, snapshot);
        let b = pb_of(runs);
        assert_eq!((a.page, a.epoch, a.generation), (2, 5, 6));
        assert_eq!((a.frags.len(), a.frags[0].face.as_str()), (2, "Face"));
        assert_eq!(a.blocks, b.blocks);
        assert_eq!(a.by_object, b.by_object);
        assert_eq!(a.faces, faces, "the font names are kept as they came");
        assert_eq!(a.shapes.len(), 1, "the snapshot's paths are kept");
        assert!(a.build_ms >= a.detect_ms && a.detect_ms >= 0.0);
    }

    #[test]
    fn by_object_holds_every_admitted_object_exactly_once() {
        let mut runs = Vec::new();
        for line in 0..6 {
            for piece in 0..3 {
                runs.push(tr(100 + line * 3 + piece, "some words ", 20.0 + 60.0 * piece as f32, 100.0 + 12.0 * line as f32));
            }
        }
        runs.push(tr(1, "", 20.0, 100.0)); // a blank twin of 100
        let pb = pb_of(runs);
        assert_eq!(pb.frags.len(), 18);
        assert_eq!(pb.by_object.len(), 18);
        let total: usize = pb.blocks.iter().map(|b| b.objects().len()).sum();
        assert_eq!(total, 18, "no object in two lines or twice in one");
        for f in &pb.frags {
            let (bi, li) = pb.by_object[&f.object];
            assert!(pb.blocks[bi].lines[li].objects.contains(&f.object));
        }
        assert!(!pb.by_object.contains_key(&1));
        for bi in 0..pb.blocks.len() {
            assert_eq!(check_editor_invariants(&pb, bi), Ok(()), "block {bi}");
        }
    }

    // ---- pick_seed ------------------------------------------------------------------------

    /// Runs with the given rects (object, left, top, right, bottom); distinct origins, all readable.
    fn pb_rects(rects: &[(usize, f32, f32, f32, f32)]) -> PageBlocks {
        pb_of(rects.iter().map(|&(o, l, t, r, b)| with_rect(tr(o, "word", 0.0, 0.0), l, t, r, b)).collect())
    }

    #[test]
    fn the_smallest_rect_under_the_point_is_the_seed() {
        let pb = pb_rects(&[(1, 0.0, 0.0, 100.0, 20.0), (2, 40.0, 4.0, 60.0, 16.0), (3, 45.0, 6.0, 52.0, 12.0)]);
        assert_eq!(pick_seed(&pb, 48.0, 8.0, 3.0), Some((3, SeedRule::Exact)));
        assert_eq!(pick_seed(&pb, 58.0, 8.0, 3.0), Some((2, SeedRule::Exact)));
        assert_eq!(pick_seed(&pb, 10.0, 8.0, 3.0), Some((1, SeedRule::Exact)));
    }

    #[test]
    fn a_click_on_an_edge_is_inside() {
        let pb = pb_rects(&[(1, 10.0, 10.0, 50.0, 20.0)]);
        assert_eq!(pick_seed(&pb, 10.0, 10.0, 0.0), Some((1, SeedRule::Exact)));
        assert_eq!(pick_seed(&pb, 50.0, 20.0, 0.0), Some((1, SeedRule::Exact)));
        assert_eq!(pick_seed(&pb, 50.1, 20.0, 0.0), None);
    }

    #[test]
    fn exact_beats_near_even_when_the_near_rect_is_closer_to_the_edge() {
        // the click is inside the big rect and 1 pt from the small one
        let pb = pb_rects(&[(1, 0.0, 0.0, 100.0, 50.0), (2, 40.0, 10.0, 60.0, 14.0)]);
        assert_eq!(pick_seed(&pb, 50.0, 15.0, 3.0), Some((1, SeedRule::Exact)));
    }

    #[test]
    fn near_takes_the_closest_edge_and_respects_the_tolerance() {
        let pb = pb_rects(&[(1, 10.0, 10.0, 50.0, 20.0), (2, 10.0, 26.0, 50.0, 36.0)]);
        // 24 is 4 below rect 1 and 2 above rect 2
        assert_eq!(pick_seed(&pb, 30.0, 24.0, 5.0), Some((2, SeedRule::Near)));
        assert_eq!(pick_seed(&pb, 30.0, 24.0, 3.0), Some((2, SeedRule::Near)));
        assert_eq!(pick_seed(&pb, 30.0, 24.0, 1.9), None, "2 pt away is out of reach of 1.9");
        assert_eq!(pick_seed(&pb, 30.0, 24.0, 2.0), Some((2, SeedRule::Near)));
        // a tolerance can be 0, negative or nonsense: then only an exact hit counts
        assert_eq!(pick_seed(&pb, 30.0, 24.0, 0.0), None);
        assert_eq!(pick_seed(&pb, 30.0, 24.0, -4.0), None);
        assert_eq!(pick_seed(&pb, 30.0, 24.0, f32::NAN), None);
    }

    #[test]
    fn near_is_a_box_test_in_each_axis_and_ranks_by_distance() {
        let pb = pb_rects(&[(1, 10.0, 10.0, 20.0, 20.0)]);
        // level with the rect and 2.5 right of it
        assert_eq!(pick_seed(&pb, 22.5, 15.0, 3.0), Some((1, SeedRule::Near)));
        // 3 right and 3 below: each axis is within 3 though the corner is 4.24 away, and it is reachable
        // exactly as the single-run picker has always treated it
        assert_eq!(pick_seed(&pb, 23.0, 23.0, 3.0), Some((1, SeedRule::Near)));
        assert_eq!(pick_seed(&pb, 23.1, 23.0, 3.0), None);
        // two rects: the nearer corner wins even when both are within reach on both axes
        let two = pb_rects(&[(1, 10.0, 10.0, 20.0, 20.0), (2, 25.0, 25.0, 35.0, 35.0)]);
        assert_eq!(pick_seed(&two, 23.0, 23.0, 3.0), Some((2, SeedRule::Near)), "2.83 from rect 2, 4.24 from rect 1");
        assert_eq!(pick_seed(&two, 21.0, 21.0, 3.0), Some((1, SeedRule::Near)), "1.41 from rect 1");
    }

    #[test]
    fn a_click_miles_away_or_not_a_number_finds_nothing() {
        let pb = pb_rects(&[(1, 10.0, 10.0, 50.0, 20.0)]);
        assert_eq!(pick_seed(&pb, 5000.0, 5000.0, 3.0), None);
        assert_eq!(pick_seed(&pb, f32::NAN, 15.0, 3.0), None);
        assert_eq!(pick_seed(&pb, 20.0, f32::INFINITY, 3.0), None);
    }

    #[test]
    fn ties_go_to_the_lowest_id_whatever_the_hash_order() {
        for _ in 0..40 {
            // each build gets its own hash seed
            let mut a = tr(8, "same", 10.0, 20.0);
            let mut b = tr(3, "same", 10.0, 20.0);
            let mut c = tr(5, "same", 10.0, 20.0);
            // same rect, different origins so none is a shadow
            a.origin.x += 4.0;
            b.origin.x += 8.0;
            c.origin.x += 12.0;
            let pb = pb_of(vec![a, b, c]);
            assert_eq!(pick_seed(&pb, 12.0, 18.0, 3.0), Some((3, SeedRule::Exact)));
        }
    }

    /// A readable member 5 over (20, 690)-(60, 700) and a blank twin 9 over the same word with its own rect.
    fn pb_with_twin(twin_rect: (f32, f32, f32, f32)) -> PageBlocks {
        let real = with_rect(tr(5, "Color", 0.0, 0.0), 20.0, 690.0, 60.0, 700.0);
        let mut twin = with_rect(tr(9, "", 0.0, 0.0), twin_rect.0, twin_rect.1, twin_rect.2, twin_rect.3);
        twin.origin = real.origin;
        pb_of(vec![real, twin])
    }

    #[test]
    fn a_blank_twin_gives_way_to_the_readable_member_under_it() {
        let pb = pb_with_twin((20.2, 690.2, 59.8, 699.8)); // smaller: the smallest-rect rule lands on it
        assert!(pb.excluded.blank.contains(&9) && pb.by_object.contains_key(&5));
        assert_eq!(pick_seed(&pb, 40.0, 695.0, 3.0), Some((5, SeedRule::Twin)));
        // through the near rule: a twin that reaches 0.5 pt lower is the nearer of the two to a click below
        let low = pb_with_twin((20.0, 690.0, 60.0, 700.5));
        assert_eq!(pick_seed(&low, 40.0, 702.0, 3.0), Some((5, SeedRule::Twin)));
        // a twin that is not the nearer leaves the member itself the seed: no swap, so no Twin
        assert_eq!(pick_seed(&pb, 40.0, 702.0, 3.0), Some((5, SeedRule::Near)));
    }

    #[test]
    fn a_twin_stands_on_the_nearest_of_the_members_it_could_be() {
        // members 5 and 6 side by side, their boxes 1.2 pt apart; a blank object sits on 6 (a hair smaller,
        // so the smallest-rect rule lands on it) and is within reach of 5 as well
        let five = with_rect(tr(5, "Colour", 0.0, 0.0), 20.0, 690.0, 60.0, 700.0);
        let six = with_rect(tr(6, "Colour", 0.0, 0.0), 21.2, 690.0, 61.2, 700.0);
        let twin = with_rect(tr(9, "", 0.0, 0.0), 21.3, 690.1, 61.1, 699.9);
        let pb = pb_of(vec![five, six, twin]);
        assert_eq!(pb.by_object.len(), 2);
        assert_eq!(pick_seed(&pb, 40.0, 695.0, 3.0), Some((6, SeedRule::Twin)));
    }

    #[test]
    fn a_blank_object_that_does_not_coincide_stays_the_seed() {
        // a blank object 2 pt right of the member's rect: not a twin (1.5 pt is the reach)
        let pb = pb_with_twin((22.0, 690.0, 62.0, 700.0));
        assert_eq!(pick_seed(&pb, 61.0, 695.0, 0.0), Some((9, SeedRule::Exact)));
        // the same object 1.4 pt off is a twin
        let near = pb_with_twin((21.4, 690.0, 61.4, 700.0));
        assert_eq!(pick_seed(&near, 61.0, 695.0, 0.0), Some((5, SeedRule::Twin)));
    }

    #[test]
    fn a_blank_object_alone_is_still_its_own_seed() {
        let pb = pb_of(vec![tr(9, "", 20.0, 700.0)]);
        assert_eq!(pick_seed(&pb, 22.0, 698.0, 3.0), Some((9, SeedRule::Exact)));
    }

    #[test]
    fn a_rotated_seed_is_never_swapped_for_a_member() {
        let real = with_rect(tr(5, "Color", 0.0, 0.0), 20.0, 690.0, 60.0, 700.0);
        let spun = with_rect(tr(9, "Color", 0.0, 0.0), 20.2, 690.2, 59.8, 699.8);
        let mut st = styles_of(&[real.clone(), spun.clone()]);
        st.insert(9, RunStyle { axis: (0.0, -1.0), ..style(1) });
        let pb = build_page_blocks_from(0, 0, 0, vec![real, spun], st, HashMap::new(), Vec::new());
        assert!(pb.excluded.rotated.contains(&9));
        assert_eq!(pick_seed(&pb, 40.0, 695.0, 3.0), Some((9, SeedRule::Exact)));
    }

    // ---- editor_lines and line_texts ------------------------------------------------------

    fn line(objects: &[usize], outlined: &[usize], rect: (f32, f32, f32, f32), baseline: f32) -> Line {
        Line { objects: objects.to_vec(), outlined: outlined.to_vec(), left: rect.0, top: rect.1, right: rect.2, bottom: rect.3, baseline }
    }

    fn block_of(lines: Vec<Line>) -> Block {
        let l = lines.iter().map(|x| x.left).fold(f32::MAX, f32::min);
        let t = lines.iter().map(|x| x.top).fold(f32::MAX, f32::min);
        let r = lines.iter().map(|x| x.right).fold(f32::MIN, f32::max);
        let b = lines.iter().map(|x| x.bottom).fold(f32::MIN, f32::max);
        Block { lines, starts_because: "start", left: l, top: t, right: r, bottom: b }
    }

    /// Five readable runs on three baselines, and one block over them that the detector did not make:
    /// the tests below bend this block one way at a time.
    ///   line 0 (y 20): 1 "Hello " at 10, 2 "world" at 40     line 1 (y 32): 3 "second " at 10, 4 "line" at 45
    ///   line 2 (y 44): 5 "third" at 10
    fn good_pb() -> PageBlocks {
        let runs = vec![
            tr(1, "Hello ", 10.0, 20.0),
            tr(2, "world", 40.0, 20.0),
            tr(3, "second ", 10.0, 32.0),
            tr(4, "line", 45.0, 32.0),
            tr(5, "third", 10.0, 44.0),
        ];
        let mut pb = pb_of(runs);
        pb.blocks = vec![block_of(vec![
            line(&[1, 2], &[], (10.0, 14.0, 62.5, 21.5), 20.0),
            line(&[3, 4], &[], (10.0, 26.0, 63.0, 33.5), 32.0),
            line(&[5], &[], (10.0, 38.0, 32.5, 45.5), 44.0),
        ])];
        pb
    }

    #[test]
    fn editor_lines_keeps_every_line_and_freezes_the_ones_with_outlined_words() {
        let mut pb = good_pb();
        pb.blocks = vec![block_of(vec![
            line(&[1, 2], &[], (10.0, 14.0, 62.5, 21.5), 20.0),
            line(&[], &[100, 101], (10.0, 26.0, 80.0, 33.5), 32.0), // a whole line drawn as outlines
            line(&[3, 4], &[102], (10.0, 38.0, 90.0, 45.5), 44.0),  // a hole inside a text line
            line(&[5], &[], (10.0, 50.0, 32.5, 57.5), 56.0),
        ])];
        let specs = editor_lines(&pb, 0);
        assert_eq!(specs.len(), 4, "a frozen line is never dropped");
        assert_eq!((specs[0].frozen, specs[0].placeholder.clone()), (false, None));
        assert!(specs[1].objects.is_empty() && specs[1].frozen);
        assert_eq!(specs[1].placeholder.as_deref(), Some(OUTLINED_PLACEHOLDER));
        assert_eq!(specs[1].rect, Rect { left: 10.0, top: 26.0, right: 80.0, bottom: 33.5 });
        assert_eq!(specs[2].objects, vec![3, 4]);
        assert!(specs[2].frozen, "a hole in a text line freezes the line");
        assert_eq!(specs[2].placeholder, None, "its text is its objects' text");
        assert_eq!((specs[3].frozen, specs[3].placeholder.clone()), (false, None));
        assert_eq!(editor_lines(&pb, 9), Vec::new());
    }

    #[test]
    fn a_line_rect_comes_out_normalised_and_a_non_finite_one_is_left_to_be_refused() {
        let mut pb = good_pb();
        pb.blocks = vec![block_of(vec![line(&[1], &[], (62.5, 21.5, 10.0, 14.0), 20.0)])];
        assert_eq!(editor_lines(&pb, 0)[0].rect, Rect { left: 10.0, top: 14.0, right: 62.5, bottom: 21.5 });
        pb.blocks[0].lines[0].left = f32::NAN;
        assert!(editor_lines(&pb, 0)[0].rect.left.is_nan());
        assert!(check_editor_invariants(&pb, 0).unwrap_err().starts_with("C11:"));
    }

    #[test]
    fn line_text_is_the_plain_concatenation_with_no_space_from_geometry() {
        let runs = vec![
            tr(1, "lig", 10.0, 20.0),         // ends at 23.5
            tr(2, "ht mo", 24.9, 20.0),       // 1.4 pt later: inside a word
            tr(3, "re words ", 50.0, 20.0),   // 2.6 pt after "ht mo" ends: still no space added
            tr(4, "end", 120.0, 20.0),        // a wide gap: still no space added
            tr(5, "caf\u{e9}\u{2}", 10.0, 32.0),
            tr(6, "a\nb", 10.0, 44.0),
        ];
        let mut pb = pb_of(runs);
        pb.blocks = vec![block_of(vec![
            line(&[1, 2, 3, 4], &[], (10.0, 14.0, 130.0, 21.5), 20.0),
            line(&[5], &[], (10.0, 26.0, 40.0, 33.5), 32.0),
            line(&[6], &[], (10.0, 38.0, 40.0, 45.5), 44.0),
        ])];
        let texts = line_texts(&pb, &editor_lines(&pb, 0));
        assert_eq!(texts[0], "light more words end");
        assert_eq!(texts[1], "caf\u{e9}\u{2}", "PDFium's own markers are the caller's to repair");
        assert_eq!(texts[2], "a b", "a line break in a run is a space");
    }

    #[test]
    fn line_text_follows_the_stored_order_and_shows_the_placeholder() {
        let mut pb = good_pb();
        pb.blocks = vec![block_of(vec![
            line(&[2, 1], &[], (10.0, 14.0, 62.5, 21.5), 20.0), // stored order, not re-sorted
            line(&[], &[100], (10.0, 26.0, 80.0, 33.5), 32.0),
        ])];
        let texts = line_texts(&pb, &editor_lines(&pb, 0));
        assert_eq!(texts, vec!["worldHello ".to_string(), OUTLINED_PLACEHOLDER.to_string()]);
        // an object that is not on the page contributes nothing rather than panicking
        let spec = LineSpec { objects: vec![77, 1], rect: Rect { left: 0.0, top: 0.0, right: 1.0, bottom: 1.0 }, frozen: false, placeholder: None, twins: Vec::new() };
        assert_eq!(line_texts(&pb, &[spec]), vec!["Hello ".to_string()]);
    }

    // ---- check_editor_invariants: accepts ---------------------------------------------------

    #[test]
    fn a_good_block_passes_every_check() {
        let pb = good_pb();
        assert_eq!(check_editor_invariants(&pb, 0), Ok(()));
    }

    #[test]
    fn a_block_with_frozen_lines_of_both_kinds_passes() {
        let mut pb = good_pb();
        pb.blocks = vec![block_of(vec![
            line(&[1, 2], &[], (10.0, 14.0, 62.5, 21.5), 20.0),
            line(&[], &[100, 101], (10.0, 26.0, 80.0, 33.5), 32.0),
            line(&[3, 4], &[102], (10.0, 38.0, 90.0, 45.5), 44.0),
        ])];
        // line 2 now sits at baseline 44 with objects 3, 4 (baseline 32): bring them down to it
        pb.runs.get_mut(&3).unwrap().origin.y = 44.0;
        pb.runs.get_mut(&4).unwrap().origin.y = 44.0;
        assert_eq!(check_editor_invariants(&pb, 0), Ok(()));
    }

    #[test]
    fn a_justified_gap_and_a_row_at_the_detectors_tolerance_are_not_two_columns_or_two_rows() {
        let mut pb = good_pb();
        // object 2 moves right: a 1.1 em gap after object 1, as justification makes
        let two = pb.runs.get_mut(&2).unwrap();
        two.rect = Rect { left: 45.8, top: 14.0, right: 45.8 + 22.5, bottom: 21.5 };
        // the two objects of the row sit 0.56 em apart: the most the detector's 0.28 em row tolerance allows
        two.origin = Point { x: 45.8, y: 20.0 - 0.28 * 8.0 };
        pb.runs.get_mut(&1).unwrap().origin.y = 20.0 + 0.28 * 8.0;
        // (a paragraph of four lines: in a block of three or fewer, an em between two objects of a line is C17's)
        pb.runs.insert(6, tr(6, "fourth", 10.0, 56.0));
        pb.blocks[0].lines.push(line(&[6], &[], (10.0, 50.0, 37.0, 57.5), 56.0));
        assert_eq!(check_editor_invariants(&pb, 0), Ok(()));
    }

    #[test]
    fn a_wide_object_that_spans_a_later_gap_does_not_make_it_a_gap() {
        // object 2 lies inside object 1's box and object 6 follows object 1 closely: measured from object 2
        // alone the gap to 6 would be 75 pt, from the right edge reached so far it is 5
        let mut pb = good_pb();
        pb.runs.get_mut(&1).unwrap().rect = Rect { left: 10.0, top: 14.0, right: 100.0, bottom: 21.5 };
        pb.runs.get_mut(&2).unwrap().rect = Rect { left: 20.0, top: 14.0, right: 30.0, bottom: 21.5 };
        pb.runs.get_mut(&2).unwrap().origin = Point { x: 20.0, y: 20.0 };
        pb.runs.insert(6, tr(6, "tail", 105.0, 20.0));
        pb.blocks[0].lines[0].objects = vec![1, 2, 6];
        assert_eq!(check_editor_invariants(&pb, 0), Ok(()));
    }

    // ---- check_editor_invariants: refuses, one violation per invariant -------------------------

    fn refused(pb: &PageBlocks) -> String {
        check_editor_invariants(pb, 0).expect_err("this block must be refused")
    }

    #[test]
    fn c1_a_block_with_no_lines_or_a_line_with_nothing_in_it_is_refused() {
        let mut pb = good_pb();
        assert!(check_editor_invariants(&pb, 5).unwrap_err().starts_with("C1: block 5 does not exist"));
        pb.blocks[0].lines.clear();
        assert_eq!(refused(&pb), "C1: the block has no lines");
        // a line with no object that is not outlined either
        let mut pb = good_pb();
        pb.blocks[0].lines[1].objects.clear();
        let e = refused(&pb);
        assert!(e.starts_with("C1: line 1 has no text object"), "{e}");
        // outlined words but no placeholder or not frozen cannot happen through editor_lines, but the check must hold
        let pb = good_pb();
        let specs = vec![LineSpec { objects: vec![], rect: Rect { left: 0.0, top: 0.0, right: 1.0, bottom: 1.0 }, frozen: true, placeholder: None, twins: Vec::new() }];
        let b = block_of(vec![line(&[], &[100], (0.0, 0.0, 1.0, 1.0), 1.0)]);
        assert!(check_specs(&pb, &b, &specs, &[String::new()]).unwrap_err().starts_with("C1: line 0"));
        let blank = vec![LineSpec { placeholder: Some("   ".into()), ..specs[0].clone() }];
        assert!(check_specs(&pb, &b, &blank, &["   ".into()]).unwrap_err().starts_with("C1: line 0"), "a blank placeholder is no placeholder");
        let thawed = vec![LineSpec { placeholder: Some("[drawn text]".into()), frozen: false, ..specs[0].clone() }];
        assert!(check_specs(&pb, &b, &thawed, &["[drawn text]".into()]).unwrap_err().starts_with("C1: line 0 has no text object"), "a line with no text object has to be frozen");
        let specs = vec![LineSpec { objects: vec![1], placeholder: Some("[x]".into()), frozen: true, rect: Rect { left: 0.0, top: 0.0, right: 1.0, bottom: 1.0 }, twins: Vec::new() }];
        assert!(check_specs(&pb, &b, &specs, &["Hello ".into()]).unwrap_err().starts_with("C1: line 0 has text objects and also"));
    }

    #[test]
    fn c13_a_dropped_line_is_refused() {
        let pb = good_pb();
        let specs = editor_lines(&pb, 0);
        let texts = line_texts(&pb, &specs);
        let e = check_specs(&pb, &pb.blocks[0], &specs[..2], &texts[..2]).unwrap_err();
        assert!(e.starts_with("C13: the block has 3 lines but 2"), "{e}");
    }

    #[test]
    fn c2_an_id_that_is_not_a_text_object_with_area_is_refused() {
        // usize::MAX
        let mut pb = good_pb();
        pb.blocks[0].lines[0].objects = vec![usize::MAX];
        assert!(refused(&pb).starts_with("C2: line 0 names usize::MAX"));
        // a shape's id
        let mut pb = good_pb();
        pb.shapes.push(drawn(300, DrawnKind::Shape, 0, 0.0, 0.0, 5.0, 1.0));
        pb.blocks[0].lines[0].objects = vec![300];
        let e = refused(&pb);
        assert!(e.starts_with("C2: line 0 names object 300") && e.contains("drawn path"), "{e}");
        // an id from nowhere
        let mut pb = good_pb();
        pb.blocks[0].lines[1].objects = vec![3, 4242];
        let e = refused(&pb);
        assert!(e.starts_with("C2: line 1 names object 4242") && e.contains("not a text object"), "{e}");
        // an object that lost its area since the snapshot (a sliver is still area; nothing at all is not)
        let mut pb = good_pb();
        pb.runs.get_mut(&5).unwrap().text = "l".into(); // (five characters in a box that narrow would be rotated text)
        pb.runs.get_mut(&5).unwrap().rect.right = pb.runs[&5].rect.left + 0.2;
        assert_eq!(check_editor_invariants(&pb, 0), Ok(()), "0.2 pt wide is a thin letter, not no area");
        pb.runs.get_mut(&5).unwrap().rect.right = pb.runs[&5].rect.left;
        assert!(refused(&pb).contains("object 5, which has no area at all"));
    }

    #[test]
    fn c3_an_object_in_two_lines_or_twice_in_one_is_refused() {
        let mut pb = good_pb();
        pb.blocks[0].lines[2].objects = vec![5, 1];
        let e = refused(&pb);
        assert_eq!(e, "C3: object 1 is in line 0 and again in line 2");
        let mut pb = good_pb();
        pb.blocks[0].lines[0].objects = vec![1, 1, 2];
        assert_eq!(refused(&pb), "C3: object 1 appears twice in line 0");
    }

    #[test]
    fn c4_two_members_at_one_origin_are_refused_and_so_is_a_twin_member() {
        // 817-821 against 826-830: readable twins at one origin
        let mut pb = good_pb();
        pb.runs.get_mut(&2).unwrap().origin = Point { x: 10.4, y: 20.0 }; // 0.4 pt from object 1
        let e = refused(&pb);
        assert!(e.starts_with("C4: objects 1 and 2 share a baseline origin"), "{e}");
        // 1.01 pt apart is a different place
        let mut pb = good_pb();
        pb.runs.get_mut(&2).unwrap().origin = Point { x: 11.01, y: 20.0 };
        pb.runs.get_mut(&2).unwrap().rect.left = 11.01;
        assert_eq!(check_editor_invariants(&pb, 0), Ok(()));
        // a shadowed object is no member, whatever its origin
        let mut pb = good_pb();
        pb.excluded.shadowed.push(4);
        assert!(refused(&pb).starts_with("C4: object 4 is a shadowed twin"));
        // the blank layer of a faux-bold heading as a member: caught as a twin before it is caught as blank
        let mut pb = good_pb();
        pb.runs.insert(826, with_rect(tr(826, "", 0.0, 0.0), 10.0, 14.0, 20.0, 21.5));
        pb.runs.get_mut(&826).unwrap().origin = pb.runs[&1].origin;
        pb.blocks[0].lines[0].objects = vec![1, 826, 2];
        assert!(refused(&pb).starts_with("C4: objects 1 and 826 share"));
    }

    #[test]
    fn c5_a_member_with_blank_text_is_refused() {
        let mut pb = good_pb();
        pb.runs.get_mut(&4).unwrap().text = " \u{a0}".into();
        assert!(refused(&pb).starts_with("C5: object 4 has blank text"));
    }

    #[test]
    fn c6_invisible_rotated_non_finite_or_zero_sized_members_are_refused() {
        let mut pb = good_pb();
        pb.runs.get_mut(&2).unwrap().color.a = 0;
        assert_eq!(refused(&pb), "C6: object 2 is invisible (alpha 0)");

        let mut pb = good_pb();
        pb.styles.insert(3, RunStyle { axis: (0.0, 1.0), ..style(1) });
        assert!(refused(&pb).starts_with("C6: object 3 is rotated (text axis 0.00,1.00)"));

        let mut pb = good_pb();
        pb.runs.get_mut(&5).unwrap().rect = Rect { left: 10.0, top: 38.0, right: 14.6, bottom: 55.0 }; // 5 chars, tall and narrow
        assert!(refused(&pb).starts_with("C6: object 5 looks rotated"));

        let mut pb = good_pb();
        pb.runs.get_mut(&4).unwrap().size = 0.0;
        assert!(refused(&pb).starts_with("C6: object 4 has size 0"));

        let mut pb = good_pb();
        pb.runs.get_mut(&4).unwrap().origin.y = f32::NAN;
        assert_eq!(refused(&pb), "C6: object 4 has non-finite geometry");

        let mut pb = good_pb();
        pb.runs.get_mut(&1).unwrap().size = f32::INFINITY;
        assert_eq!(refused(&pb), "C6: object 1 has non-finite geometry");
    }

    #[test]
    fn c7_a_line_that_does_not_run_left_to_right_is_refused() {
        let mut pb = good_pb();
        pb.blocks[0].lines[0].objects = vec![2, 1];
        let e = refused(&pb);
        assert!(e.starts_with("C7: line 0 is not left to right: object 1 starts at 10.0, left of object 2 at 40.0"), "{e}");
    }

    #[test]
    fn c8_lines_out_of_order_two_baselines_two_columns_and_line_breaks_are_refused() {
        // line 1 above line 0
        let mut pb = good_pb();
        pb.blocks[0].lines.swap(0, 1);
        let e = refused(&pb);
        assert!(e.starts_with("C8: line 1 does not lie below line 0"), "{e}");
        // two lines on one baseline (two columns' lines, not one column), at origins that do not coincide
        let mut pb = good_pb();
        pb.runs.get_mut(&3).unwrap().origin = Point { x: 12.0, y: 20.0 };
        pb.runs.get_mut(&4).unwrap().origin.y = 20.0;
        let e = refused(&pb);
        assert!(e.starts_with("C8: line 1 does not lie below line 0"), "{e}");
        // the margin between two lines of text is 0.4 em of the larger: the next line of 8 pt text half an
        // em below is a line below, three tenths of an em below is not (clear of the boundary on both sides)
        for (delta, want_ok) in [(0.5 * 8.0, true), (0.3 * 8.0, false)] {
            let mut pb = good_pb();
            for o in [3, 4] {
                pb.runs.get_mut(&o).unwrap().origin.y = 20.0 + delta;
            }
            let verdict = check_editor_invariants(&pb, 0);
            assert_eq!(verdict.is_ok(), want_ok, "a line {delta} pt below: {verdict:?}");
            if !want_ok {
                assert!(verdict.unwrap_err().starts_with("C8: line 1 does not lie below line 0"));
            }
        }
        // two baselines in one line
        let mut pb = good_pb();
        pb.runs.get_mut(&2).unwrap().origin.y = 20.0 + 0.8 * 8.0;
        let e = refused(&pb);
        assert!(e.starts_with("C8: line 0 mixes baselines: object 2"), "{e}");
        // a 10 em gap inside a line that holds no outlined word
        let mut pb = good_pb();
        pb.runs.get_mut(&4).unwrap().rect = Rect { left: 45.0 + 70.0, top: 26.0, right: 120.0 + 18.0, bottom: 33.5 };
        let e = refused(&pb);
        assert!(e.starts_with("C8: line 1 spans a") && e.contains("two columns in one line"), "{e}");
    }

    /// `good_pb` with object 4 moved right so line 1 spans a gap of `gap` pt after object 3 (which ends at
    /// 41.5), the line frozen by outlined word 99 whose box is `shape` (left, right), or none.
    fn frozen_gap_pb(gap: f32, shape: Option<(f32, f32)>) -> PageBlocks {
        let mut pb = good_pb();
        let left = 41.5 + gap;
        pb.runs.get_mut(&4).unwrap().rect = Rect { left, top: 26.0, right: left + 18.0, bottom: 33.5 };
        pb.blocks[0].lines[1].outlined = vec![99];
        if let Some((l, r)) = shape {
            pb.shapes.push(drawn(99, DrawnKind::Shape, 0, l, 26.0, r, 33.5));
        }
        pb
    }

    #[test]
    fn c8_a_frozen_line_is_measured_for_a_gap_too_less_what_its_outlined_word_fills() {
        // the datasheet's own holes: 3.4 em (27 pt at 8 pt) where an outlined word sits: fine, with or
        // without the word's box at hand
        assert_eq!(check_editor_invariants(&frozen_gap_pb(27.0, None), 0), Ok(()));
        assert_eq!(check_editor_invariants(&frozen_gap_pb(27.0, Some((43.0, 66.0))), 0), Ok(()));
        // a 9 em hole (73.5 pt) with a long outlined word in it that fills it: a line with a long word
        // drawn as shapes is not two columns
        assert_eq!(check_editor_invariants(&frozen_gap_pb(72.0, Some((43.0, 112.0))), 0), Ok(()));
        // two outlined pieces side by side fill it as well (the union, not each one)
        let mut two = frozen_gap_pb(72.0, Some((43.0, 80.0)));
        two.blocks[0].lines[1].outlined = vec![99, 98];
        two.shapes.push(drawn(98, DrawnKind::Shape, 0, 78.0, 26.0, 112.0, 33.5));
        assert_eq!(check_editor_invariants(&two, 0), Ok(()));
        // the same 9 em with only a small symbol in it (a currency sign drawn as a path): two cells of a
        // table row merged into one frozen line, which used to go unmeasured
        let e = check_editor_invariants(&frozen_gap_pb(72.0, Some((43.0, 51.0))), 0).unwrap_err();
        assert!(e.starts_with("C8: line 1 spans a") && e.contains("two columns in one line"), "{e}");
        // and with no box for the outlined word at all (a shape that is gone) nothing is filled
        assert!(check_editor_invariants(&frozen_gap_pb(72.0, None), 0).unwrap_err().starts_with("C8: line 1 spans a"));
        // the 12.7 em of the Arabic receipt (cells 150 pt apart at 11.8 pt) with its small sign
        let e = check_editor_invariants(&frozen_gap_pb(100.0, Some((43.0, 56.0))), 0).unwrap_err();
        assert!(e.contains("two columns in one line"), "{e}");
        // a shape that reaches beyond the gap fills only the part of it inside the gap: 18.5 of 72 here
        // (it starts left of it), 13.5 of 72 (it ends right of it)
        let e = check_editor_invariants(&frozen_gap_pb(72.0, Some((0.0, 60.0))), 0).unwrap_err();
        assert!(e.contains("spans a 53.5 pt gap"), "{e}");
        let e = check_editor_invariants(&frozen_gap_pb(72.0, Some((100.0, 300.0))), 0).unwrap_err();
        assert!(e.contains("spans a 58.5 pt gap"), "{e}");
        // and two shapes over the same stretch fill it once, not twice (the union): 50 of 100, 50 left open
        let mut doubled = frozen_gap_pb(100.0, Some((43.0, 93.0)));
        doubled.blocks[0].lines[1].outlined = vec![99, 98];
        doubled.shapes.push(drawn(98, DrawnKind::Shape, 0, 43.0, 26.0, 93.0, 33.5));
        let e = check_editor_invariants(&doubled, 0).unwrap_err();
        assert!(e.contains("spans a 50.0 pt gap"), "{e}");
        // a shape nested in a form (depth > 0) carries the form's id and fills nothing
        let mut nested = frozen_gap_pb(72.0, None);
        nested.shapes.push(drawn(99, DrawnKind::Shape, 1, 43.0, 26.0, 112.0, 33.5));
        assert!(check_editor_invariants(&nested, 0).unwrap_err().starts_with("C8: line 1 spans a"));
        // a shape that is not one of this line's outlined words fills nothing of it
        let mut stranger = frozen_gap_pb(72.0, Some((43.0, 112.0)));
        stranger.blocks[0].lines[1].outlined = vec![98];
        assert!(check_editor_invariants(&stranger, 0).unwrap_err().starts_with("C8: line 1 spans a"));
        // a line with no outlined word keeps the old rule exactly
        let mut plain = frozen_gap_pb(72.0, Some((43.0, 112.0)));
        plain.blocks[0].lines[1].outlined.clear();
        assert!(check_editor_invariants(&plain, 0).unwrap_err().starts_with("C8: line 1 spans a"));
    }

    /// `good_pb` (three lines of 8 pt text) with object 4 moved right so line 1 has a gap of `gap` pt after
    /// object 3, which ends at 41.5.
    fn cell_gap_pb(gap: f32) -> PageBlocks {
        let mut pb = good_pb();
        let left = 41.5 + gap;
        pb.runs.get_mut(&4).unwrap().rect = Rect { left, top: 26.0, right: left + 18.0, bottom: 33.5 };
        pb
    }

    #[test]
    fn c17_a_short_block_with_a_wide_gap_in_a_line_is_a_row_of_cells_not_a_sentence() {
        // a gap under an em (8 pt here) is a word space, justified or not
        assert_eq!(check_editor_invariants(&cell_gap_pb(6.0), 0), Ok(()));
        // an em or more in a block of three lines: two cells of a table row ("189.00" and "02-JUL-2026",
        // 1.9 em apart). Retyping the line would write both into the first one and delete the second.
        for gap in [8.0, 9.6, 14.0, 40.0] {
            let e = check_editor_invariants(&cell_gap_pb(gap), 0).unwrap_err();
            assert!(e.starts_with("C17: line 1 has a") && e.contains("objects 3 and 4"), "{gap}: {e}");
        }
        // a block of one or two lines is judged the same way
        let mut two = cell_gap_pb(14.0);
        two.blocks[0].lines.remove(2);
        assert!(check_editor_invariants(&two, 0).unwrap_err().starts_with("C17:"));
        // a fourth line makes it a paragraph, whose justified lines have wide gaps of their own (1.7 em in
        // the datasheet's narrow columns): not refused
        let mut four = cell_gap_pb(14.0);
        four.runs.insert(6, tr(6, "fourth", 10.0, 56.0));
        four.blocks[0].lines.push(line(&[6], &[], (10.0, 50.0, 37.0, 57.5), 56.0));
        assert_eq!(check_editor_invariants(&four, 0), Ok(()));
        // a frozen line is never written, so nothing is collapsed in it
        let mut frozen = cell_gap_pb(27.0);
        frozen.blocks[0].lines[1].outlined = vec![99];
        assert_eq!(check_editor_invariants(&frozen, 0), Ok(()));
        // and a 10 em gap is still C8's, which is reported first
        assert!(check_editor_invariants(&cell_gap_pb(70.0), 0).unwrap_err().starts_with("C8: line 1 spans a"));
        // a list item's number stands 1.5 em before its words ("1." and the clause: every quotation's terms):
        // one item, not two cells
        for marker in ["1.", "2)", "(a)", "[12]", "iv.", "•", "-"] {
            let mut item = cell_gap_pb(12.0);
            item.runs.get_mut(&3).unwrap().text = marker.to_string();
            assert_eq!(check_editor_invariants(&item, 0), Ok(()), "{marker}");
        }
        // a bare number or a word before a gap is a cell, not a marker
        for cell in ["12", "99EXP020", "Total", "1.00", "a.b.c.d."] {
            let mut item = cell_gap_pb(12.0);
            item.runs.get_mut(&3).unwrap().text = cell.to_string();
            assert!(check_editor_invariants(&item, 0).unwrap_err().starts_with("C17:"), "{cell}");
        }
    }

    #[test]
    fn c8_a_line_text_with_a_break_or_a_wrong_number_of_texts_is_refused() {
        let pb = good_pb();
        let specs = editor_lines(&pb, 0);
        let mut texts = line_texts(&pb, &specs);
        texts[1] = "second\nline".into();
        let e = check_specs(&pb, &pb.blocks[0], &specs, &texts).unwrap_err();
        assert_eq!(e, "C8: the text of line 1 holds a line break");
        let mut texts = line_texts(&pb, &specs);
        texts[2].push('\r');
        assert!(check_specs(&pb, &pb.blocks[0], &specs, &texts).unwrap_err().starts_with("C8: the text of line 2"));
        let texts = line_texts(&pb, &specs);
        let e = check_specs(&pb, &pb.blocks[0], &specs, &texts[..2]).unwrap_err();
        assert!(e.starts_with("C8: 2 line texts joined by newlines make 2 lines, not 3"), "{e}");
    }

    #[test]
    fn c9_a_text_that_is_not_the_concatenation_or_a_bad_placeholder_is_refused() {
        let pb = good_pb();
        let specs = editor_lines(&pb, 0);
        let mut texts = line_texts(&pb, &specs);
        texts[0] = "Hello  world".into(); // a space invented from geometry
        let e = check_specs(&pb, &pb.blocks[0], &specs, &texts).unwrap_err();
        assert!(e.starts_with("C9: the text of line 0 is not the plain concatenation of its 2 objects' text (12 characters, expected 11)"), "{e}");

        let mut pb = good_pb();
        pb.blocks = vec![block_of(vec![line(&[1], &[], (10.0, 14.0, 62.5, 21.5), 20.0), line(&[], &[100], (10.0, 26.0, 80.0, 33.5), 32.0)])];
        let specs = editor_lines(&pb, 0);
        let good = line_texts(&pb, &specs);
        for bad in ["", "   ", "[drawn text]-", "[drawn word]"] {
            let mut texts = good.clone();
            texts[1] = bad.to_string();
            let e = check_specs(&pb, &pb.blocks[0], &specs, &texts).unwrap_err();
            assert!(e.starts_with("C9: the text of placeholder line 1"), "{bad:?}: {e}");
        }
        let hyphenated = vec![LineSpec { placeholder: Some("[drawn text]-".into()), ..specs[1].clone() }];
        let b = block_of(vec![line(&[], &[100], (10.0, 26.0, 80.0, 33.5), 32.0)]);
        assert!(check_specs(&pb, &b, &hyphenated, &["[drawn text]-".to_string()]).unwrap_err().starts_with("C9:"), "a placeholder ending in '-' is refused even when text and placeholder agree");
        // fewer texts than lines (here none for one line: the joined empty string still splits into one)
        let b1 = block_of(vec![pb.blocks[0].lines[0].clone()]);
        assert_eq!(check_specs(&pb, &b1, &specs[..1], &[]).unwrap_err(), "C9: 0 line texts for 1 lines");
    }

    #[test]
    fn c10_a_line_with_outlined_words_that_is_not_frozen_is_refused() {
        let mut pb = good_pb();
        pb.blocks[0].lines[1].outlined = vec![100, 101];
        let mut specs = editor_lines(&pb, 0);
        assert!(specs[1].frozen, "editor_lines freezes it");
        assert_eq!(check_editor_invariants(&pb, 0), Ok(()));
        specs[1].frozen = false;
        let texts = line_texts(&pb, &specs);
        let e = check_specs(&pb, &pb.blocks[0], &specs, &texts).unwrap_err();
        assert!(e.starts_with("C10: line 1 carries 2 outlined words but is not frozen"), "{e}");
    }

    #[test]
    fn c11_a_rect_that_is_not_finite_or_not_normalised_is_refused() {
        let pb = good_pb();
        let specs = editor_lines(&pb, 0);
        let texts = line_texts(&pb, &specs);
        let mut bad = specs.clone();
        bad[2].rect.bottom = f32::INFINITY;
        assert_eq!(check_specs(&pb, &pb.blocks[0], &bad, &texts).unwrap_err(), "C11: the rect of line 2 is not finite");
        let mut bad = specs.clone();
        bad[1].rect = Rect { left: 20.0, top: 5.0, right: 10.0, bottom: 9.0 };
        assert!(check_specs(&pb, &pb.blocks[0], &bad, &texts).unwrap_err().starts_with("C11: the rect of line 1 is not normalised"));
        let mut bad = specs;
        bad[0].rect = Rect { left: 10.0, top: 9.0, right: 20.0, bottom: 5.0 };
        assert!(check_specs(&pb, &pb.blocks[0], &bad, &texts).unwrap_err().starts_with("C11: the rect of line 0 is not normalised"));
    }

    #[test]
    fn the_lowest_numbered_violation_is_the_one_reported() {
        let mut pb = good_pb();
        pb.runs.get_mut(&1).unwrap().color.a = 0; // C6
        pb.runs.get_mut(&2).unwrap().text = "".into(); // C5
        assert!(refused(&pb).starts_with("C5:"));
        pb.blocks[0].lines[0].objects = vec![1, 1]; // C3 comes first
        assert!(refused(&pb).starts_with("C3:"));
    }

    #[test]
    fn a_message_never_carries_the_pages_text() {
        let mut pb = good_pb();
        pb.runs.get_mut(&2).unwrap().text = "TOPSECRET".into();
        pb.runs.get_mut(&2).unwrap().color.a = 0;
        assert!(!refused(&pb).contains("TOPSECRET"));
        let pb = good_pb();
        let specs = editor_lines(&pb, 0);
        let mut texts = line_texts(&pb, &specs);
        texts[0] = "TOPSECRET".into();
        assert!(!check_specs(&pb, &pb.blocks[0], &specs, &texts).unwrap_err().contains("TOPSECRET"));
    }

    // ---- C8 for lines drawn as shapes: the rects, not an estimated baseline --------------------

    /// A text run with its own baseline and size (what the guard reads), box given.
    fn placed(object: usize, text: &str, rect: (f32, f32, f32, f32), baseline: f32, size: f32) -> TextRun {
        let mut r = with_rect(tr(object, text, 0.0, 0.0), rect.0, rect.1, rect.2, rect.3);
        r.origin = Point { x: rect.0 - 0.4, y: baseline };
        r.size = size;
        r
    }

    #[test]
    fn c8_a_label_a_symbol_and_a_tag_stacked_in_order_is_not_refused_for_the_symbolss_estimated_baseline() {
        // orange.pdf block 70, as dumped: 'CH ' (6.6 pt, baseline 516.41), four drawn shapes (the detector's
        // estimate of their baseline is 519.00, 2.59 below the text's: 0.39 em, a hair under the 0.4 em
        // margin that is right for a measured baseline), 'LT-02 ' (3 pt, baseline 524.90)
        let mut pb = pb_of(vec![placed(55758, "CH ", (505.0, 511.55, 512.1, 516.48), 516.41, 6.6), placed(61684, "LT-02 ", (500.5, 522.74, 507.8, 524.93), 524.90, 3.0)]);
        pb.blocks = vec![block_of(vec![
            line(&[55758], &[], (505.0, 511.55, 512.1, 516.48), 516.32),
            line(&[], &[61631, 61632, 61629, 61630], (500.0, 513.68, 508.2, 521.90), 519.00),
            line(&[61684], &[], (500.5, 522.74, 507.8, 524.93), 524.84),
        ])];
        assert_eq!(check_editor_invariants(&pb, 0), Ok(()));
        // what made it pass is that a drawn line is ordered by its rect: starts lower and ends lower
        pb.blocks[0].lines[1].top = 511.55; // starts level with the text above
        assert!(refused(&pb).starts_with("C8: line 1 does not lie below line 0: a line drawn as shapes"), "level top");
        pb.blocks[0].lines[1].top = 513.68;
        pb.blocks[0].lines[1].bottom = 516.48; // ends level with it
        assert!(refused(&pb).starts_with("C8: line 1 does not lie below line 0"), "level bottom");
        // and the text below it has to start lower than the drawn line does
        pb.blocks[0].lines[1].bottom = 521.90;
        pb.blocks[0].lines[2].top = 513.0;
        let e = refused(&pb);
        assert!(e.starts_with("C8: line 2 does not lie below line 1: a line drawn as shapes"), "{e}");
    }

    #[test]
    fn c8_a_symbol_standing_beside_its_label_is_not_a_line_below_it() {
        // orange.pdf block 150: ' +2700' (with a drawn piece in its line) and a symbol whose rect reaches above it
        let mut pb = pb_of(vec![placed(55763, " +2700", (871.3, 786.08, 887.7, 792.86), 790.88, 6.6), placed(62468, "LT-02", (893.1, 794.69, 900.4, 796.88), 796.85, 3.0)]);
        pb.blocks = vec![block_of(vec![
            line(&[55763], &[62414], (871.3, 786.08, 899.9, 792.86), 789.95),
            line(&[], &[62415, 62416, 62413], (892.6, 785.63, 900.9, 793.85), 792.03),
            line(&[62468], &[], (893.1, 794.69, 900.4, 796.88), 796.72),
        ])];
        let e = refused(&pb);
        assert!(e.starts_with("C8: line 1 does not lie below line 0"), "{e}");
    }

    #[test]
    fn c8_two_lines_of_text_on_one_baseline_stay_refused_when_the_detector_splits_a_row_in_two() {
        // letterhead.pdf block 2, as dumped: two phone numbers and the bar between them are ONE visual row
        // (baseline 88.63) that the detector cut into two interleaved "lines". This is the guard being
        // right and the detector wrong: it is the case the baseline test exists for
        let runs = vec![
            placed(10, "+", (67.269, 83.005, 71.999, 88.043), 88.626, 11.0),
            placed(11, "9898 968 ", (72.813, 81.566, 113.381, 88.815), 88.727, 11.0),
            placed(13, " 8817 ", (116.075, 81.465, 137.899, 88.714), 88.626, 11.0),
            placed(43, "\u{2502}", (140.925, 67.552, 142.301, 89.680), 84.229, 18.05),
            placed(44, "+", (145.024, 83.274, 149.754, 88.312), 88.895, 11.0),
            placed(45, "968 9898 ", (150.568, 81.834, 191.136, 89.083), 88.995, 11.0),
            placed(47, "8817 ", (193.829, 81.734, 215.653, 88.983), 88.895, 11.0),
        ];
        let mut pb = pb_of(runs);
        pb.blocks = vec![block_of(vec![
            line(&[13, 43, 44], &[], (116.075, 67.552, 149.754, 89.680), 81.871),
            line(&[10, 11, 45, 47], &[], (67.269, 81.566, 215.653, 89.083), 88.626),
        ])];
        let e = refused(&pb);
        assert!(e.starts_with("C8: line 1 does not lie below line 0"), "{e}");
        assert!(!e.contains("drawn as shapes"), "both lines hold text: the baseline test, not the rect one: {e}");
    }

    // ---- C14: vector art is not a paragraph ----------------------------------------------------

    /// Text lines and drawn lines at a regular pitch of 12 pt (rect 14 + 12k to 21.5 + 12k): `plan` says
    /// for each line whether it holds text (true) or is drawn shapes (false). Text objects are 1 + 3 * line.
    fn stacked_pb(plan: &[bool]) -> PageBlocks {
        let mut runs = Vec::new();
        let mut lines = Vec::new();
        for (k, &text) in plan.iter().enumerate() {
            let y = 20.0 + 12.0 * k as f32;
            if text {
                let o = 1 + 3 * k;
                runs.push(tr(o, "some words ", 10.0, y));
                runs.push(tr(o + 1, "more words", 60.0, y));
                lines.push(line(&[o, o + 1], &[], (10.0, y - 6.0, 105.0, y + 1.5), y));
            } else {
                lines.push(line(&[], &[1000 + k], (10.0, y - 6.0, 100.0, y + 1.5), y - 1.0));
            }
        }
        let mut pb = pb_of(runs);
        pb.blocks = vec![block_of(lines)];
        pb
    }

    #[test]
    fn c14_a_qr_code_bridged_into_the_lines_of_a_label_is_refused() {
        // page 2 of the datasheet, 'ROHS': four phantom lines (rows of QR modules) over one line of text
        let pb = stacked_pb(&[false, false, false, false, true]);
        let e = refused(&pb);
        assert!(e.starts_with("C14: 4 of the block's 5 lines are drawn shapes"), "{e}");
        // fewer than three text objects with three or more drawn lines is the same rule
        let pb = stacked_pb(&[false, true, false, false]);
        assert!(refused(&pb).starts_with("C14: 3 of the block's 4 lines"), "two text objects, three drawn lines");
    }

    #[test]
    fn c14_a_paragraph_with_some_lines_drawn_as_outlines_is_not_vector_art() {
        // the datasheet's page-1 block 3: text, drawn, drawn, text. Half of its lines are drawn and it is a
        // real paragraph (a line with a ligature in it is converted to outlines): only a MAJORITY is art
        assert_eq!(check_editor_invariants(&stacked_pb(&[true, false, false, true]), 0), Ok(()));
        // the user's 19-line block: three drawn lines among sixteen
        let mut plan = vec![true; 19];
        for k in [4, 14, 18] {
            plan[k] = false;
        }
        assert_eq!(check_editor_invariants(&stacked_pb(&plan), 0), Ok(()));
        assert_eq!(check_editor_invariants(&stacked_pb(&[true, false, true]), 0), Ok(()));
        // one more drawn line than text lines is where it tips
        assert!(refused(&stacked_pb(&[true, false, false])).starts_with("C14: 2 of the block's 3 lines"));
    }

    #[test]
    fn c14_a_block_with_no_text_at_all_is_not_judged() {
        // nothing in it can be opened or written, and no click reaches it (a seed is a text object)
        assert_eq!(check_editor_invariants(&stacked_pb(&[false, false, false]), 0), Ok(()));
        assert_eq!(check_editor_invariants(&stacked_pb(&[false]), 0), Ok(()));
        // nor are two lines drawn as shapes held to an order against each other (the datasheet's page-3
        // block 116: a logo of two drawn lines, the second starting 0.02 pt above the first)
        let mut logo = stacked_pb(&[false, false]);
        logo.blocks[0].lines[1].top = logo.blocks[0].lines[0].top - 0.02;
        assert_eq!(check_editor_invariants(&logo, 0), Ok(()));
        let mut with_text = stacked_pb(&[true, true, false, false]);
        with_text.blocks[0].lines[3].top = with_text.blocks[0].lines[2].top - 0.02;
        assert_eq!(check_editor_invariants(&with_text, 0), Ok(()), "the pair of drawn lines is skipped, each is still ordered against the text above");
        with_text.blocks[0].lines[2].top = with_text.blocks[0].lines[1].top;
        assert!(refused(&with_text).starts_with("C8: line 2 does not lie below line 1"));
    }

    // ---- C15: no right-to-left text --------------------------------------------------------------

    #[test]
    fn right_to_left_characters_are_recognised_in_every_script_that_runs_that_way() {
        for (what, c) in [
            ("Arabic letter", '\u{0645}'),
            ("Arabic-Indic digit", '\u{0663}'),
            ("Persian letter", '\u{067E}'),
            ("Hebrew letter", '\u{05D0}'),
            ("Syriac", '\u{0710}'),
            ("Thaana", '\u{0780}'),
            ("Arabic presentation form A", '\u{FB51}'),
            ("Hebrew presentation form", '\u{FB2A}'),
            ("Arabic presentation form B", '\u{FEE3}'),
            ("Phoenician", '\u{10900}'),
            ("Adlam", '\u{1E900}'),
            ("right-to-left mark", '\u{200F}'),
            ("right-to-left embedding", '\u{202B}'),
            ("right-to-left override", '\u{202E}'),
            ("right-to-left isolate", '\u{2067}'),
        ] {
            assert!(is_rtl(c), "{what} U+{:04X}", c as u32);
        }
        for (what, c) in [("Latin", 'a'), ("digit", '7'), ("space", ' '), ("accented Latin", '\u{e9}'), ("Cyrillic", '\u{416}'), ("Greek", '\u{3a9}'), ("CJK", '\u{4e2d}'), ("left-to-right mark", '\u{200E}'), ("control", '\u{2}'), ("just below Hebrew", '\u{058F}'), ("just above Arabic Extended-A", '\u{0900}')] {
            assert!(!is_rtl(c), "{what}");
        }
    }

    #[test]
    fn c15_a_block_holding_arabic_hebrew_or_persian_is_refused_and_one_without_is_not() {
        for text in ["\u{645}\u{631}\u{62d}\u{628}\u{627} ", "shalom \u{5e9}\u{5dc}\u{5d5}\u{5dd}", "\u{67e}\u{627}\u{631}\u{633}\u{6cc}", "7 \u{663}\u{664}", "\u{fe8d}\u{fee3} ", "total \u{200f}12"] {
            let mut pb = good_pb();
            pb.runs.get_mut(&4).unwrap().text = text.to_string();
            let e = refused(&pb);
            assert!(e.starts_with("C15: line 1 holds right-to-left text (object 4)"), "{text:?}: {e}");
            assert!(!e.contains(text.trim()), "the message carries none of the page's text: {e}");
        }
        for text in ["plain latin ", "caf\u{e9} \u{416} \u{3a9} \u{4e2d}", "12.50 AED "] {
            let mut pb = good_pb();
            pb.runs.get_mut(&4).unwrap().text = text.to_string();
            assert_eq!(check_editor_invariants(&pb, 0), Ok(()), "{text:?}");
        }
    }

    #[test]
    fn c15_is_the_reason_shown_when_a_right_to_left_block_breaks_other_checks_too() {
        let mut pb = good_pb();
        pb.runs.get_mut(&4).unwrap().text = "\u{645}\u{631}\u{62d}\u{628}\u{627}".into();
        pb.runs.get_mut(&1).unwrap().color.a = 0; // C6
        pb.runs.get_mut(&3).unwrap().text = " ".into(); // C5
        assert!(refused(&pb).starts_with("C15:"), "{}", refused(&pb));
        pb.blocks[0].lines[2].objects = vec![5, 4242]; // C2: an object that is not on the page
        pb.blocks[0].lines[0].objects = vec![1, 1, 2]; // C3
        assert!(refused(&pb).starts_with("C15:"), "even before a block that names things that are not there: {}", refused(&pb));
    }

    // ---- Type 3 fonts: a negative size is not an em --------------------------------------------

    #[test]
    fn a_type_3_glyph_run_with_a_negative_size_and_glyph_index_text_stays_out_of_every_block() {
        // lpo02.pdf (Ghostscript, Type 3): every object has size -0.12 while its box is 3.6 to 7 pt tall,
        // and its text is the glyph's index ("8 ", "A B ", control codes), not a letter of the page
        let mut runs = Vec::new();
        for (i, text) in ["0 ", "1 ", "A B ", "\u{1} ", "8 "].into_iter().enumerate() {
            let mut r = placed(i + 1, text, (198.2 + 6.0 * i as f32, 765.3, 202.0 + 6.0 * i as f32, 769.5), 769.5, -0.12);
            r.origin.x = 198.0 + 6.0 * i as f32;
            runs.push(r);
        }
        let pb = pb_of(runs);
        assert!(pb.blocks.is_empty() && pb.frags.is_empty(), "nothing is laid out from a size that is not an em");
        assert_eq!(pb.excluded.degenerate, vec![1, 2, 3, 4, 5]);
        // the same objects with their size read the right way up would be a line
        let right_way: Vec<TextRun> = (0..5).map(|i| placed(i + 1, "word ", (198.2 + 30.0 * i as f32, 765.3, 224.0 + 30.0 * i as f32, 769.5), 769.5, 4.2)).collect();
        assert_eq!(pb_of(right_way).frags.len(), 5);
    }

    // ---- twins ----------------------------------------------------------------------------------

    /// One heading line of two members, 5 "Color " and 6 "Options", each drawn twice (9: a blank layer over
    /// 5, 10: a readable copy of 6 half a point to the right), and a second line with member 7 and no twin.
    fn twin_pb() -> PageBlocks {
        let five = placed(5, "Color ", (20.0, 690.0, 60.0, 700.0), 698.5, 8.0);
        let six = placed(6, "Options", (64.0, 690.0, 110.0, 700.0), 698.5, 8.0);
        let seven = placed(7, "Next line", (20.0, 702.0, 70.0, 712.0), 710.5, 8.0);
        let mut blank = placed(9, "", (20.2, 690.2, 59.8, 699.8), 698.5, 8.0);
        blank.origin.x = 20.3;
        let mut copy = placed(10, "Options", (64.0, 690.0, 110.0, 700.0), 698.5, 8.0);
        copy.origin.x = 64.5;
        let mut pb = pb_of(vec![five, six, seven, blank, copy]);
        pb.blocks = vec![block_of(vec![line(&[5, 6], &[], (20.0, 690.0, 110.0, 700.0), 698.5), line(&[7], &[], (20.0, 702.0, 70.0, 712.0), 710.5)])];
        pb
    }

    #[test]
    fn a_blank_layer_and_a_readable_copy_are_the_twins_of_the_members_they_duplicate() {
        let pb = twin_pb();
        assert_eq!(pb.excluded.blank, vec![9]);
        assert_eq!(pb.excluded.shadowed, vec![10]);
        assert_eq!(pb.twins, HashMap::from([(5, vec![9]), (6, vec![10])]));
        let specs = editor_lines(&pb, 0);
        assert_eq!(specs[0].twins, vec![9, 10], "both pieces' twins, the first piece's included, ascending");
        assert!(specs[1].twins.is_empty());
        assert_eq!(specs[0].objects, vec![5, 6], "a twin is never a member");
        // never in the text, whichever way they read
        assert_eq!(line_texts(&pb, &specs), vec!["Color Options".to_string(), "Next line".to_string()]);
        assert_eq!(check_editor_invariants(&pb, 0), Ok(()));
    }

    #[test]
    fn a_blank_object_that_coincides_with_no_member_is_nobodys_twin() {
        let mut pb = twin_pb();
        // a blank object 3 pt below, one 2 pt to the right of a member's box, one far away, and one at the
        // member's box with an origin 1.5 pt off: none is a layer drawn over a member
        pb.runs.insert(11, placed(11, "", (20.0, 693.0, 60.0, 703.0), 701.5, 8.0));
        // (12: the member's own origin and a box 2 pt to the right of it, beyond the 1.5 pt reach)
        let mut aside = placed(12, "", (22.0, 690.0, 62.0, 700.0), 698.5, 8.0);
        aside.origin.x = 19.8;
        pb.runs.insert(12, aside);
        pb.runs.insert(13, placed(13, "", (400.0, 690.0, 440.0, 700.0), 698.5, 8.0));
        // (14: the member's own box and an origin 1.3 pt from it, in the next cell: beyond the 1 pt reach)
        let mut off = placed(14, "", (20.0, 690.0, 60.0, 700.0), 698.5, 8.0);
        off.origin.x = 20.9;
        pb.runs.insert(14, off);
        pb.excluded.blank.extend([11, 12, 13, 14]);
        let twins = assign_twins(&pb.runs, &pb.excluded, &pb.by_object);
        assert_eq!(twins, HashMap::from([(5, vec![9]), (6, vec![10])]));
        // 0.9 pt away in origin (member 5's is 19.6) and 1.4 pt in the box is still the same place
        let mut near = placed(15, "", (21.4, 690.0, 61.4, 700.0), 698.5, 8.0);
        near.origin.x = 20.5;
        pb.runs.insert(15, near);
        pb.excluded.blank.push(15);
        assert_eq!(assign_twins(&pb.runs, &pb.excluded, &pb.by_object)[&5], vec![9, 15]);
    }

    #[test]
    fn a_twin_goes_to_the_nearest_member_and_to_one_only() {
        // members 5 and 6 side by side (origins 19.6 and 20.9: 1.3 apart, so two words, not a twin pair);
        // a blank object lies within reach of both and nearer 6
        let five = placed(5, "Colour", (20.0, 690.0, 60.0, 700.0), 698.5, 8.0);
        let six = placed(6, "Colour", (21.3, 690.0, 61.3, 700.0), 698.5, 8.0);
        let mut twin = placed(9, "", (20.9, 690.0, 60.9, 700.0), 698.5, 8.0);
        twin.origin.x = 20.4;
        let pb = pb_of(vec![five, six, twin]);
        assert_eq!(pb.by_object.len(), 2);
        assert_eq!(pb.twins, HashMap::from([(6, vec![9])]), "0.8 pt of edge gaps to 6, 1.8 to 5");
        // exactly between them: the lower id, whatever the hash order
        for _ in 0..30 {
            let mut a = placed(5, "Colour", (20.0, 690.0, 60.0, 700.0), 698.5, 8.0);
            a.origin.x = 20.0;
            let mut b = placed(6, "Colour", (21.25, 690.0, 61.25, 700.0), 698.5, 8.0);
            b.origin.x = 21.25;
            let mut t = placed(9, "", (20.625, 690.0, 60.625, 700.0), 698.5, 8.0);
            t.origin.x = 20.625;
            let pb = pb_of(vec![b, t, a]);
            assert_eq!(pb.by_object.len(), 2);
            assert_eq!(pb.twins, HashMap::from([(5, vec![9])]));
        }
    }

    #[test]
    fn twins_are_found_whatever_order_the_objects_come_in_and_whichever_is_drawn_first() {
        // the blank layer BEFORE the member in the file, and the readable copy before the original
        let mut blank = placed(1, "", (20.2, 690.2, 59.8, 699.8), 698.5, 8.0);
        blank.origin.x = 20.3;
        let real = placed(2, "Color ", (20.0, 690.0, 60.0, 700.0), 698.5, 8.0);
        let pb = pb_of(vec![real, blank]);
        assert_eq!(pb.twins, HashMap::from([(2, vec![1])]));
        // two readable copies: the first drawn is the member, the later is the twin
        let mut later = placed(8, "Color ", (20.0, 690.0, 60.0, 700.0), 698.5, 8.0);
        later.origin.x = 20.5;
        let pb = pb_of(vec![placed(4, "Color ", (20.0, 690.0, 60.0, 700.0), 698.5, 8.0), later]);
        assert_eq!((pb.by_object.keys().copied().collect::<Vec<_>>(), pb.twins), (vec![4], HashMap::from([(4, vec![8])])));
    }

    #[test]
    fn the_twins_of_one_member_are_ascending_whichever_kind_they_are() {
        // member 5 drawn three times: a readable copy (8, shadowed) and a blank layer (3, which comes first
        // in the file), listed by kind blank-then-shadowed but reported by id
        let mut blank = placed(3, "", (20.2, 690.2, 59.8, 699.8), 698.5, 8.0);
        blank.origin.x = 20.3;
        let real = placed(5, "Color ", (20.0, 690.0, 60.0, 700.0), 698.5, 8.0);
        let mut copy = placed(8, "Color ", (20.0, 690.0, 60.0, 700.0), 698.5, 8.0);
        copy.origin.x = 19.9;
        let mut layer = placed(9, "", (20.1, 690.1, 59.9, 699.9), 698.5, 8.0);
        layer.origin.x = 19.7;
        let pb = pb_of(vec![blank, real, copy, layer]);
        assert_eq!(pb.excluded.blank, vec![3, 9]);
        assert_eq!(pb.excluded.shadowed, vec![8]);
        assert_eq!(pb.twins[&5], vec![3, 8, 9]);
        assert_eq!(editor_lines(&pb, 0)[0].twins, vec![3, 8, 9]);
    }

    #[test]
    fn a_page_without_twins_has_none_and_a_hostile_one_does_not_panic() {
        assert!(pb_of(vec![tr(1, "a", 10.0, 20.0), tr(2, "b", 10.0, 40.0)]).twins.is_empty());
        let mut pb = twin_pb();
        for r in pb.runs.values_mut() {
            r.origin = Point { x: f32::NAN, y: f32::INFINITY };
        }
        assert!(assign_twins(&pb.runs, &pb.excluded, &pb.by_object).is_empty(), "no origin, no twin");
        let mut pb = twin_pb();
        pb.runs.get_mut(&9).unwrap().rect.left = f32::NAN;
        pb.runs.get_mut(&5).unwrap().origin = Point { x: 1e30, y: f32::MAX };
        let _ = assign_twins(&pb.runs, &pb.excluded, &pb.by_object);
    }

    #[test]
    fn c16_a_twin_that_is_real_passes_and_a_frozen_line_carries_its_twins_all_the_same() {
        let pb = twin_pb();
        assert_eq!(check_editor_invariants(&pb, 0), Ok(()));
        // a line with a hole (an outlined word) is frozen, is never written, and still reports its twins
        let mut frozen = twin_pb();
        frozen.blocks[0].lines[0].outlined = vec![99];
        frozen.shapes.push(drawn(99, DrawnKind::Shape, 0, 62.0, 690.0, 63.5, 700.0));
        let specs = editor_lines(&frozen, 0);
        assert!(specs[0].frozen);
        assert_eq!(specs[0].twins, vec![9, 10]);
        assert_eq!(check_editor_invariants(&frozen, 0), Ok(()));
    }

    fn twin_refused(pb: &PageBlocks, mutate: impl FnOnce(&mut Vec<LineSpec>)) -> String {
        let mut specs = editor_lines(pb, 0);
        mutate(&mut specs);
        let texts = line_texts(pb, &specs);
        check_specs(pb, &pb.blocks[0], &specs, &texts).expect_err("this block must be refused")
    }

    #[test]
    fn c16_a_twin_that_is_not_one_is_refused_before_apply_can_remove_it() {
        let pb = twin_pb();
        // a member of the same line, of another line
        assert_eq!(twin_refused(&pb, |s| s[0].twins.push(5)), "C16: object 5 is a member of line 0 and also a twin in line 0");
        assert_eq!(twin_refused(&pb, |s| s[1].twins.push(6)), "C16: object 6 is a member of line 0 and also a twin in line 1");
        // listed in two lines, and twice in one
        assert_eq!(twin_refused(&pb, |s| s[1].twins.push(9)), "C16: twin 9 is listed in line 0 and again in line 1");
        assert_eq!(twin_refused(&pb, |s| s[0].twins = vec![9, 9, 10]), "C16: twin 9 is listed twice in line 0");
        // not a text object of the page
        assert!(twin_refused(&pb, |s| s[0].twins.push(4242)).starts_with("C16: line 0 lists object 4242 as a twin, which is not a text object"));
        // a real object that is neither blank nor shadowed (an alpha-0 layer: edited by another path)
        let mut hidden = twin_pb();
        let mut ghost = placed(11, "Color ", (20.0, 690.0, 60.0, 700.0), 698.5, 8.0);
        ghost.color.a = 0;
        hidden.runs.insert(11, ghost);
        hidden.excluded.invisible.push(11);
        assert_eq!(twin_refused(&hidden, |s| s[0].twins.push(11)), "C16: line 0 lists object 11 as a twin, but it is neither blank nor shadowed");
        // an admitted member of another block
        let mut other = twin_pb();
        other.blocks[0].lines.truncate(1); // object 7 is a member of the page, of no line of this block
        let e = twin_refused(&other, |s| s[0].twins.push(7));
        assert_eq!(e, "C16: line 0 lists object 7 as a twin, but it is a member of another block");
        // blank, but over nothing of this line: removing it would delete something that is no copy
        let mut stray = twin_pb();
        stray.runs.insert(12, placed(12, "", (300.0, 690.0, 340.0, 700.0), 698.5, 8.0));
        stray.excluded.blank.push(12);
        assert_eq!(twin_refused(&stray, |s| s[0].twins.push(12)), "C16: line 0 lists object 12 as a twin, but it coincides with none of the line's objects");
        // blank, with the member's own origin but a box 3 pt to the right of it: not a layer drawn over it
        let mut shifted = twin_pb();
        let mut off_box = placed(14, "", (23.0, 690.0, 63.0, 700.0), 698.5, 8.0);
        off_box.origin.x = 19.8;
        shifted.runs.insert(14, off_box);
        shifted.excluded.blank.push(14);
        assert_eq!(twin_refused(&shifted, |s| s[0].twins.push(14)), "C16: line 0 lists object 14 as a twin, but it coincides with none of the line's objects");
        // blank, in the member's box but with an origin 2 pt away: the same
        let mut off_origin = twin_pb();
        let mut elsewhere = placed(15, "", (20.0, 690.0, 60.0, 700.0), 698.5, 8.0);
        elsewhere.origin.x = 21.6;
        off_origin.runs.insert(15, elsewhere);
        off_origin.excluded.blank.push(15);
        assert_eq!(twin_refused(&off_origin, |s| s[0].twins.push(15)), "C16: line 0 lists object 15 as a twin, but it coincides with none of the line's objects");
        // a blank object over a member of ANOTHER line is that line's twin, not this one's
        assert!(twin_refused(&pb, |s| s[1].twins = vec![9]).starts_with("C16: twin 9 is listed in line 0 and again in line 1"));
        let mut under_seven = twin_pb();
        under_seven.runs.insert(13, placed(13, "", (20.0, 702.0, 70.0, 712.0), 710.5, 8.0));
        under_seven.excluded.blank.push(13);
        let e = twin_refused(&under_seven, |s| s[0].twins.push(13));
        assert!(e.ends_with("it coincides with none of the line's objects"), "{e}");
    }

    #[test]
    fn the_pick_line_counts_the_twins_of_the_block() {
        let pb = twin_pb();
        let mut t = PickTrace::new(0, (1.0, 2.0));
        t.page_facts(&pb);
        t.block_facts(&pb, 0);
        assert_eq!(t.twins, 2);
        assert!(format_pick_line(&t).contains(" outlined=0 twins=2 frozen=[] "), "{}", format_pick_line(&t));
    }

    // ---- the log line ---------------------------------------------------------------------

    #[test]
    fn the_pick_line_is_exactly_the_documented_one() {
        let t = PickTrace {
            page: 0,
            click: (212.4, 501.2),
            seed: Some(1029),
            rule: Some(SeedRule::Exact),
            path: PickPath::Block,
            frags: 770,
            blocks: 129,
            block: Some(106),
            lines: 13,
            objects: 57,
            outlined: 2,
            twins: 5,
            frozen: vec![8, 10],
            starts: Some("style"),
            next_starts: Some("style"),
            rect: Some([187.0, 421.0, 306.0, 538.0]),
            fonts: vec![1],
            cache_hit: false,
            build_ms: 38.1,
            detect_ms: 1.2,
            total_ms: 41.0,
        };
        assert_eq!(
            format_pick_line(&t),
            "page 1 click=(212.4,501.2) seed=1029 rule=exact path=block frags=770 blocks=129 block=106 lines=13 objects=57 outlined=2 twins=5 frozen=[8,10] starts=\"style\" next_starts=\"style\" rect=[187.0,421.0,306.0,538.0] fonts=[1] cache=miss build_ms=38.1 detect_ms=1.2 total_ms=41.0"
        );
    }

    #[test]
    fn the_pick_line_of_a_click_that_found_nothing() {
        let t = PickTrace { cache_hit: true, ..PickTrace::new(2, (0.0, 0.0)) };
        assert_eq!(
            format_pick_line(&t),
            "page 3 click=(0.0,0.0) seed=none rule=none path=none frags=0 blocks=0 block=none lines=0 objects=0 outlined=0 twins=0 frozen=[] starts=none next_starts=none rect=none fonts=[] cache=hit build_ms=0.0 detect_ms=0.0 total_ms=0.0"
        );
        for (path, word) in [(PickPath::Single, "single"), (PickPath::Joined, "joined"), (PickPath::Drawn, "drawn"), (PickPath::RefusedRotated, "refused-rotated")] {
            let t = PickTrace { path, rule: Some(SeedRule::Twin), ..PickTrace::default() };
            assert!(format_pick_line(&t).contains(&format!("rule=twin path={word} ")), "{word}");
        }
        assert!(format_pick_line(&PickTrace { rule: Some(SeedRule::Near), ..PickTrace::default() }).contains("rule=near "));
    }

    #[test]
    fn a_trace_fills_itself_from_the_page_and_the_block() {
        let runs = vec![tr(1, "a", 10.0, 20.0), tr(2, "b", 10.0, 32.0), tr(3, "c", 10.0, 44.0), tr(4, "d", 10.0, 56.0), tr(5, "e", 30.0, 44.0)];
        let mut st = styles_of(&runs);
        st.insert(3, style(4));
        st.insert(5, style(4)); // the same font again: listed once
        st.remove(&4);
        let mut pb = build_page_blocks_from(1, 0, 0, runs, st, HashMap::new(), Vec::new());
        pb.blocks = vec![
            Block { starts_because: "start", ..block_of(vec![line(&[1, 2], &[], (10.0, 14.0, 20.0, 21.5), 20.0)]) },
            Block {
                starts_because: "style",
                ..block_of(vec![line(&[3, 5], &[], (10.0, 38.0, 20.0, 45.5), 44.0), line(&[], &[100, 101], (10.0, 50.0, 80.0, 57.5), 56.0), line(&[4], &[102], (10.0, 62.0, 80.0, 69.5), 68.0)])
            },
            Block { starts_because: "pitch", ..block_of(vec![line(&[], &[], (0.0, 0.0, 1.0, 1.0), 0.0)]) },
        ];
        let mut t = PickTrace::new(9, (1.0, 2.0));
        t.page_facts(&pb);
        t.block_facts(&pb, 1);
        assert_eq!((t.page, t.frags, t.blocks, t.block), (1, pb.frags.len(), 3, Some(1)));
        assert_eq!((t.lines, t.objects, t.outlined), (3, 3, 3));
        assert_eq!(t.frozen, vec![1, 2]);
        assert_eq!((t.starts, t.next_starts), (Some("style"), Some("pitch")));
        assert_eq!(t.rect, Some([10.0, 38.0, 80.0, 69.5]));
        assert_eq!(t.fonts, vec![4, u32::MAX], "an object with no style is the unknown font");
        assert!(format_pick_line(&t).contains("fonts=[4,?]"));
        t.block_facts(&pb, 2);
        assert_eq!(t.next_starts, None, "the last block has no next");
        t.block_facts(&pb, 99);
        assert_eq!(t.block, None);
    }
}
