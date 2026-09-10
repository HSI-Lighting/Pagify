//! Reading order: deciding whether a page has one, and rebuilding it when not.
//!
//! ## Why this does not simply sort by position
//!
//! `Page::text_segments` returns runs in the document's **character order**,
//! deliberately not sorted geometrically, and that default is right. A
//! two-column page has two columns sharing every y band; sorting by position
//! interleaves them line by line and produces text that reads as nonsense. The
//! content stream's own order is the only thing that tells the columns apart,
//! and well-authored PDFs emit it in reading order.
//!
//! Some producers do not — browser print-to-PDF above all, which emits
//! absolutely positioned fragments in paint order. So the question is not
//! "sort or don't sort", it is **"can this document's character order be
//! trusted?"** — and that is measurable. [`trust`] measures it; only pages that
//! fail fall through to [`reconstruct`].
//!
//! The most damaging failure available here is the trust check firing when it
//! should not, because that would degrade every well-authored document in the
//! name of fixing the broken ones. The tests treat it that way.

use serde::{Deserialize, Serialize};

/// A rectangle in page space: points, top-left origin, y down — the same
/// convention `TextSegment` and `PageCharacters` already use.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rect {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl Rect {
    pub fn width(&self) -> f32 {
        self.right - self.left
    }
    pub fn height(&self) -> f32 {
        self.bottom - self.top
    }
    pub fn union(self, other: Rect) -> Rect {
        Rect {
            left: self.left.min(other.left),
            top: self.top.min(other.top),
            right: self.right.max(other.right),
            bottom: self.bottom.max(other.bottom),
        }
    }
    /// Do these two share any vertical extent? Used for line grouping, where
    /// comparing baselines alone breaks on mixed font sizes.
    pub fn overlaps_vertically(&self, other: &Rect) -> bool {
        self.top < other.bottom && self.bottom > other.top
    }
}

/// `layout::Rect` and `document::Rect` are two distinct types with identical
/// fields, not one — a fact three separate call sites hand-copied before this
/// existed, which is the usual sign a conversion belongs somewhere once rather
/// than by hand every time it comes up.
impl From<Rect> for crate::document::Rect {
    fn from(r: Rect) -> Self {
        crate::document::Rect { left: r.left, top: r.top, right: r.right, bottom: r.bottom }
    }
}

/// One placed character, as extraction hands it back.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Glyph {
    pub ch: char,
    pub rect: Rect,
    /// Radians clockwise. Rotated text is segregated before line grouping, or a
    /// stamp at 45° merges into the body text it crosses.
    pub angle: f32,
}

impl Glyph {
    /// The em size this character was set at, inferred from its box. Used to
    /// derive tolerances, so they scale with the type rather than being fixed
    /// point values that are wrong at both 6pt and 60pt.
    fn size(&self) -> f32 {
        self.rect.height().max(0.1)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Direction {
    #[default]
    Ltr,
    Rtl,
}

impl Direction {
    /// The direction of a string, from the script of its characters.
    pub fn of(text: &str) -> Direction {
        let rtl = text.chars().filter(|c| is_rtl(*c)).count();
        let strong = text.chars().filter(|c| is_rtl(*c) || c.is_alphabetic()).count();
        if strong > 0 && rtl * 2 > strong {
            Direction::Rtl
        } else {
            Direction::Ltr
        }
    }
}

/// Where a page's reading order came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    /// The document's own character order, trusted and used unchanged.
    CharacterOrder,
    /// Rebuilt from geometry because character order could not be trusted.
    Reconstructed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextLine {
    pub rect: Rect,
    pub text: String,
    pub direction: Direction,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextBlock {
    pub rect: Rect,
    pub lines: Vec<TextLine>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageText {
    pub blocks: Vec<TextBlock>,
    pub source: Source,
    /// How confident the ordering is, 0–1. Reported rather than hidden: a page
    /// that says it could not be read reliably is more useful than one that
    /// quietly returns plausible nonsense.
    pub confidence: f32,
}

impl PageText {
    /// Everything, in reading order, blocks separated by blank lines.
    pub fn plain(&self) -> String {
        self.blocks
            .iter()
            .map(|block| {
                block
                    .lines
                    .iter()
                    .map(|l| l.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

// ---------------------------------------------------------------------------
// The trust check
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Trust {
    /// How often the sequence moves to a new line that is *above* the last one.
    pub backward_jumps: usize,
    /// How often it stays on a line but leaps backwards along it.
    pub horizontal_leaps: usize,
    /// Line transitions — **not** characters. See [`trust`].
    pub steps: usize,
}

impl Trust {
    /// A disorder score high enough to be *worth mentioning*.
    ///
    /// **Not a trigger.** Reconstruction is never applied automatically, and
    /// the measurement below is why.
    ///
    /// The plan assumed two separable populations — well-authored documents
    /// near zero, paint-ordered ones far above. Measured across real files,
    /// they are not separable at all:
    ///
    /// ```text
    ///   Apple licence, 117 pages           0.011
    ///   two-column.pdf                     0.033
    ///   HSI credentials, 7 pages           0.087
    ///   browser print, 10 pages            0.125
    ///   shredded.pdf  ← the *bad* case     0.267
    ///   lighting schematic                 0.251
    ///   lighting layout                    0.348
    ///   HSI CATALOG 2026, 149 pages        0.393   (36 pages above 0.15)
    ///   CAD drawing                        0.637
    /// ```
    ///
    /// Every document below the deliberately-scrambled fixture extracts
    /// correctly today, and four score *above* it. The reason is that the
    /// metric cannot tell paint order from **positioned** layout: a drawing's
    /// labels, dimensions and callouts have no linear reading order to begin
    /// with, and neither does a catalogue page of boxes and captions. They look
    /// exactly like a browser print to a measure of "how often does the text
    /// jump backwards".
    ///
    /// Acting on it would have reconstructed 36 pages of the flagship catalogue
    /// — a document that extracts perfectly — which is precisely the failure the
    /// plan called the most damaging available here.
    ///
    /// So the score is reported and never acted on. Reconstruction is a
    /// deliberate request (`reflow`), because on this evidence only a person
    /// looking at the page can tell a scrambled document from a drawing.
    pub const NOTEWORTHY: f32 = 0.15;

    pub fn disorder(&self) -> f32 {
        if self.steps == 0 {
            0.0
        } else {
            (self.backward_jumps + self.horizontal_leaps) as f32 / self.steps as f32
        }
    }

    /// Below this many line transitions, a ratio is not evidence.
    ///
    /// Measured on a real browser print: its first page is a dashboard of stat
    /// tiles with four legitimate backward jumps over thirty-two transitions —
    /// 0.125, most of the way to the threshold, on a page that reads perfectly.
    /// A shorter page with the same two or three honest jumps scores past it
    /// outright, purely because the denominator is small.
    ///
    /// So a small sample defers to character order. That is the right way round:
    /// reconstruction is the destructive option — it reorders a document that
    /// may not need it — and destructive options should require evidence rather
    /// than tolerate its absence.
    pub const MIN_EVIDENCE: usize = 12;

    /// Whether the score is worth telling the user about.
    ///
    /// Deliberately **not** named `is_reading_order`, and deliberately not used
    /// to choose a code path. A high score means "this page might not read in
    /// order, and reflowing it might help" — it does not mean the page is
    /// broken, and on real documents it usually does not.
    pub fn is_noteworthy(&self) -> bool {
        // A ratio over a handful of transitions is not evidence of anything.
        self.steps >= Self::MIN_EVIDENCE && self.disorder() > Self::NOTEWORTHY
    }
}

/// Measure whether a page's character order looks like reading order.
///
/// **Counted per line transition, not per character**, and that is the whole
/// difficulty. Within any fragment the characters are in perfect order — a
/// browser print lays out each run left to right like anything else — so
/// measuring every adjacent pair drowns the handful of real jumps in a sea of
/// ordered steps. A page of eight scrambled fragments of eleven characters each
/// measured 0.06 disorder that way, which is indistinguishable from a
/// well-authored document.
///
/// The question that actually separates them is: **of the times the text moves
/// to a new line, how often does it move up the page?** In reading order that
/// is close to never — a column break, a footnote. In paint order it is
/// routine.
pub fn trust(glyphs: &[Glyph]) -> Trust {
    let mut backward_jumps = 0;
    let mut horizontal_leaps = 0;
    let mut steps = 0;

    for pair in glyphs.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        let tolerance = a.size() * 0.5;

        if a.rect.overlaps_vertically(&b.rect) {
            // Same line. A big step backwards along it is a fragment placed out
            // of sequence rather than a run continuing.
            if b.rect.left < a.rect.left - tolerance {
                horizontal_leaps += 1;
                steps += 1;
            }
            continue;
        }

        // A line transition. This is what gets counted.
        steps += 1;
        if b.rect.top < a.rect.top - tolerance {
            backward_jumps += 1;
        }
    }

    Trust { backward_jumps, horizontal_leaps, steps }
}

// ---------------------------------------------------------------------------
// Reconstruction
// ---------------------------------------------------------------------------

/// Rebuild reading order from geometry alone.
pub fn reconstruct(glyphs: &[Glyph]) -> PageText {
    if glyphs.is_empty() {
        return PageText { blocks: Vec::new(), source: Source::Reconstructed, confidence: 0.0 };
    }

    // Rotated text is its own world. A 45° stamp across a drawing must not join
    // the horizontal line it happens to cross.
    let (upright, rotated): (Vec<Glyph>, Vec<Glyph>) =
        glyphs.iter().partition(|g| g.angle.abs() <= 0.01);

    let mut blocks = Vec::new();
    for column in columns(&upright) {
        let lines: Vec<TextLine> = group_lines(&column).into_iter().map(assemble_line).collect();
        if lines.is_empty() {
            continue;
        }
        let rect = lines.iter().map(|l| l.rect).reduce(Rect::union).expect("non-empty");
        blocks.push(TextBlock { rect, lines });
    }

    // Rotated runs land in their own block at the end, grouped only by angle,
    // so they are recoverable without being interleaved into the body.
    if !rotated.is_empty() {
        let lines: Vec<TextLine> = group_lines(&rotated).into_iter().map(assemble_line).collect();
        if let Some(rect) = lines.iter().map(|l| l.rect).reduce(Rect::union) {
            blocks.push(TextBlock { rect, lines });
        }
    }

    PageText { blocks, source: Source::Reconstructed, confidence: 0.75 }
}

/// Split glyphs into columns at vertical gutters.
///
/// A gutter is a band of x with no characters in it, wider than a few spaces
/// and running most of the page's height. Anything narrower is the gap between
/// words, and anything shorter is the space beside a figure.
fn columns(glyphs: &[Glyph]) -> Vec<Vec<Glyph>> {
    if glyphs.len() < 2 {
        return vec![glyphs.to_vec()];
    }

    let median_size = median(&mut glyphs.iter().map(Glyph::size).collect::<Vec<_>>());
    let min_gutter = median_size * 2.0;

    let content_top = glyphs.iter().map(|g| g.rect.top).fold(f32::MAX, f32::min);
    let content_bottom = glyphs.iter().map(|g| g.rect.bottom).fold(f32::MIN, f32::max);
    let content_height = (content_bottom - content_top).max(0.1);

    // Columns are a multi-line idea. On a single line every "side" of a
    // candidate gutter spans the full content height by definition, so the
    // depth test below passes trivially and a word gap becomes a column — which
    // is how "ACCEPT ERROR" came back as two columns reading "ACCEPT" and
    // "ERROR". One line of text has no columns in it.
    if content_height < median_size * 2.5 {
        return vec![glyphs.to_vec()];
    }

    let mut spans: Vec<(f32, f32)> = glyphs.iter().map(|g| (g.rect.left, g.rect.right)).collect();
    spans.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    // Walk the x extents, merging overlaps; whatever is left between merged
    // groups is a candidate gutter.
    let mut candidates = Vec::new();
    let mut reach = spans[0].1;
    for (left, right) in &spans[1..] {
        if *left - reach > min_gutter {
            candidates.push((reach + *left) / 2.0);
        }
        reach = reach.max(*right);
    }

    // A gutter has to **run most of the page's height**. Without that test, the
    // gap between two words on a single line is a column boundary, and
    // "ACCEPT ERROR" comes back as two columns reading "ACCEPT" and "ERROR".
    // Width alone cannot tell a word space from a column: only the fact that a
    // real gutter is empty all the way down can.
    let deep_enough = |cut: f32| {
        let span = |side: &dyn Fn(f32) -> bool| {
            let picked: Vec<&Glyph> = glyphs
                .iter()
                .filter(|g| side((g.rect.left + g.rect.right) / 2.0))
                .collect();
            if picked.is_empty() {
                return 0.0;
            }
            let top = picked.iter().map(|g| g.rect.top).fold(f32::MAX, f32::min);
            let bottom = picked.iter().map(|g| g.rect.bottom).fold(f32::MIN, f32::max);
            bottom - top
        };
        let left = span(&|centre: f32| centre <= cut);
        let right = span(&|centre: f32| centre > cut);
        left >= content_height * 0.5 && right >= content_height * 0.5
    };

    let cuts: Vec<f32> = candidates.into_iter().filter(|c| deep_enough(*c)).collect();

    if cuts.is_empty() {
        return vec![glyphs.to_vec()];
    }

    let mut buckets: Vec<Vec<Glyph>> = vec![Vec::new(); cuts.len() + 1];
    for glyph in glyphs {
        let centre = (glyph.rect.left + glyph.rect.right) / 2.0;
        let index = cuts.iter().filter(|cut| centre > **cut).count();
        buckets[index].push(*glyph);
    }
    buckets.retain(|b| !b.is_empty());
    buckets
}

/// Group a column's glyphs into lines, top to bottom.
fn group_lines(glyphs: &[Glyph]) -> Vec<Vec<Glyph>> {
    let mut sorted = glyphs.to_vec();
    sorted.sort_by(|a, b| {
        a.rect
            .top
            .partial_cmp(&b.rect.top)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.rect.left.partial_cmp(&b.rect.left).unwrap_or(std::cmp::Ordering::Equal))
    });

    let mut lines: Vec<Vec<Glyph>> = Vec::new();
    for glyph in sorted {
        // Same line when the boxes share vertical extent. Comparing tops alone
        // breaks the moment a capital or a larger word sits on the same
        // baseline — it starts higher and opens a spurious second line.
        let joined = lines
            .last_mut()
            .filter(|line| line.iter().any(|g| g.rect.overlaps_vertically(&glyph.rect)));

        match joined {
            Some(line) => line.push(glyph),
            None => lines.push(vec![glyph]),
        }
    }
    lines
}

/// Turn a line's glyphs into text, putting spaces where the advance says so.
fn assemble_line(mut glyphs: Vec<Glyph>) -> TextLine {
    glyphs.sort_by(|a, b| {
        a.rect.left.partial_cmp(&b.rect.left).unwrap_or(std::cmp::Ordering::Equal)
    });

    let direction = dominant_direction(&glyphs);
    let mut text = String::new();
    let mut previous: Option<&Glyph> = None;

    for glyph in &glyphs {
        if let Some(before) = previous {
            let gap = glyph.rect.left - before.rect.right;
            // A quarter em. This is what recovers word boundaries from a page
            // that contains no space characters at all — common in anything
            // that positions every glyph absolutely.
            let space = before.size() * 0.25;
            if gap > space && !glyph.ch.is_whitespace() && !text.ends_with(' ') {
                text.push(' ');
            }
        }
        text.push(glyph.ch);
        previous = Some(glyph);
    }

    let rect = glyphs
        .iter()
        .map(|g| g.rect)
        .reduce(Rect::union)
        .unwrap_or(Rect { left: 0.0, top: 0.0, right: 0.0, bottom: 0.0 });

    // Right-to-left runs are stored in logical order, which is what a consumer
    // wants for copy and search; the *display* reordering is the renderer's job.
    TextLine { rect, text: text.trim_end().to_string(), direction }
}

/// One word, kept as its own unit rather than joined into a line's text.
///
/// [`TextLine`] deliberately throws this away — one rect per *line* is right
/// for reading a page, and wrong for clicking on one word of it. This exists
/// for [`crate::document::outlined`], which needs exactly the box a selection
/// or a `RecognisedWord` wants, built by the same rule that already recovers
/// word boundaries for [`TextLine::text`] — see [`assemble_line`] — so the two
/// cannot disagree about where one word ends and the next begins.
pub(crate) struct Word {
    pub text: String,
    pub rect: Rect,
    /// One entry per character of `text`, in the same order. Kept rather than
    /// discarded once the string is built, so a caller with its own per-glyph
    /// data — [`crate::document::outlined`]'s match score, keyed by nothing
    /// but the glyph's own `(rect, ch)` — can recover which glyph is which
    /// after grouping has happened.
    pub glyphs: Vec<Glyph>,
}

/// Group placed characters into words, a line at a time.
///
/// Not exposed as a whole-page string the way [`reconstruct`] is: a caller
/// wanting words wants their boxes too, and a rect survives grouping only if
/// grouping never collapses into a single joined string in the first place.
pub(crate) fn words(glyphs: &[Glyph]) -> Vec<Word> {
    group_lines(glyphs).into_iter().flat_map(words_in_line).collect()
}

fn words_in_line(mut glyphs: Vec<Glyph>) -> Vec<Word> {
    glyphs.sort_by(|a, b| a.rect.left.partial_cmp(&b.rect.left).unwrap_or(std::cmp::Ordering::Equal));

    let mut out = Vec::new();
    let mut current: Vec<Glyph> = Vec::new();

    for glyph in glyphs {
        if let Some(before) = current.last() {
            // The same quarter-em rule `assemble_line` uses to decide where a
            // space belongs — this is that same decision, made explicit as a
            // boundary between two returned words instead of implicit as a
            // character pushed into one continuous string.
            let gap = glyph.rect.left - before.rect.right;
            if gap > before.size() * 0.25 {
                out.push(finish_word(std::mem::take(&mut current)));
            }
        }
        current.push(glyph);
    }
    if !current.is_empty() {
        out.push(finish_word(current));
    }
    out
}

fn finish_word(glyphs: Vec<Glyph>) -> Word {
    let text: String = glyphs.iter().map(|g| g.ch).collect();
    let rect = glyphs
        .iter()
        .map(|g| g.rect)
        .reduce(Rect::union)
        .expect("finish_word is never called with an empty run");
    Word { text, rect, glyphs }
}

/// A paragraph's direction, from the script of the characters in it.
///
/// Taken from the content rather than from the document's language field, which
/// is frequently absent and frequently wrong.
fn dominant_direction(glyphs: &[Glyph]) -> Direction {
    let rtl = glyphs.iter().filter(|g| is_rtl(g.ch)).count();
    let strong = glyphs.iter().filter(|g| is_rtl(g.ch) || g.ch.is_alphabetic()).count();
    if strong > 0 && rtl * 2 > strong {
        Direction::Rtl
    } else {
        Direction::Ltr
    }
}

/// Characters with strong right-to-left directionality: Hebrew, Arabic, Syriac,
/// Thaana, N'Ko, Samaritan, and the Arabic presentation forms.
fn is_rtl(ch: char) -> bool {
    matches!(ch as u32,
        0x0590..=0x05FF | 0x0600..=0x06FF | 0x0700..=0x074F | 0x0780..=0x07BF
        | 0x07C0..=0x07FF | 0x0800..=0x083F | 0x08A0..=0x08FF
        | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF)
}

fn median(values: &mut [f32]) -> f32 {
    if values.is_empty() {
        return 1.0;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    values[values.len() / 2]
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Build a page's text from the document's own character order.
///
/// **Always character order.** Reconstruction is available beside this, as
/// [`reconstruct`], and is never chosen automatically — see
/// [`Trust::NOTEWORTHY`] for the measurements that settled that.
///
/// `confidence` carries the disorder score so a caller can offer to reflow, or
/// say the page may not read in order. Reporting rather than deciding is the
/// whole of the change: a page that says it might be out of order is useful,
/// and a page silently reordered when it was already right is not.
pub fn page_text(glyphs: &[Glyph], character_order: &str) -> PageText {
    let measured = trust(glyphs);

    let lines: Vec<TextLine> = character_order
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| TextLine {
            rect: Rect { left: 0.0, top: 0.0, right: 0.0, bottom: 0.0 },
            text: l.to_string(),
            direction: Direction::of(l),
        })
        .collect();

    let rect = glyphs
        .iter()
        .map(|g| g.rect)
        .reduce(Rect::union)
        .unwrap_or(Rect { left: 0.0, top: 0.0, right: 0.0, bottom: 0.0 });

    let blocks = if lines.is_empty() { Vec::new() } else { vec![TextBlock { rect, lines }] };
    PageText {
        blocks,
        source: Source::CharacterOrder,
        confidence: 1.0 - measured.disorder(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glyph(ch: char, left: f32, top: f32, size: f32) -> Glyph {
        Glyph {
            ch,
            rect: Rect { left, top, right: left + size * 0.6, bottom: top + size },
            angle: 0.0,
        }
    }

    /// Lay a string out left to right on one line.
    fn line(text: &str, left: f32, top: f32, size: f32) -> Vec<Glyph> {
        let mut x = left;
        let mut out = Vec::new();
        for ch in text.chars() {
            if ch == ' ' {
                x += size * 0.4;
                continue;
            }
            out.push(glyph(ch, x, top, size));
            x += size * 0.65;
        }
        out
    }

    // -- the trust check ----------------------------------------------------

    /// The single most damaging failure available here.
    #[test]
    fn a_well_authored_two_column_page_is_trusted() {
        // Two columns, emitted the way a sane producer emits them: the whole of
        // the left column first, then the whole of the right. Firing the trust
        // check on this would send every good document through reconstruction.
        let mut glyphs = Vec::new();
        for row in 0..20 {
            glyphs.extend(line("left column text here", 50.0, 100.0 + row as f32 * 14.0, 12.0));
        }
        for row in 0..20 {
            glyphs.extend(line("right column text here", 320.0, 100.0 + row as f32 * 14.0, 12.0));
        }

        let measured = trust(&glyphs);
        assert!(
            !measured.is_noteworthy(),
            "a good two-column page was flagged (disorder {:.2})",
            measured.disorder()
        );
    }

    #[test]
    fn paint_ordered_fragments_are_distrusted() {
        // What a browser print produces: fragments placed in whatever order the
        // layout engine painted them, jumping up and down the page.
        //
        // Twenty of them, not eight. A real shredded page carries dozens of
        // line transitions, and a fixture with a handful is below the evidence
        // floor — it would be trusted, correctly, and prove nothing.
        let mut glyphs = Vec::new();
        let rows = [
            300.0, 100.0, 500.0, 200.0, 700.0, 150.0, 600.0, 250.0, 450.0, 120.0,
            650.0, 180.0, 400.0, 220.0, 550.0, 140.0, 620.0, 260.0, 480.0, 160.0,
        ];
        for (i, top) in rows.iter().enumerate() {
            glyphs.extend(line(&format!("fragment {i}"), 50.0, *top, 12.0));
        }

        let measured = trust(&glyphs);
        assert!(
            measured.is_noteworthy(),
            "paint order was not even worth mentioning (disorder {:.2})",
            measured.disorder()
        );
    }

    #[test]
    fn a_page_too_short_to_judge_is_trusted_rather_than_rebuilt() {
        // Three lines with one backward jump scores 0.33 — past any threshold —
        // on evidence that is nothing of the kind. Reconstruction reorders a
        // document, so it has to be earned.
        let mut glyphs = Vec::new();
        for top in [100.0, 300.0, 200.0] {
            glyphs.extend(line("a short run", 50.0, top, 12.0));
        }

        let measured = trust(&glyphs);
        assert!(measured.steps < Trust::MIN_EVIDENCE, "the fixture is not short");
        assert!(
            !measured.is_noteworthy(),
            "a three-line page was flagged on {} transitions",
            measured.steps
        );
    }

    #[test]
    fn an_empty_or_single_character_page_does_not_divide_by_zero() {
        assert_eq!(trust(&[]).disorder(), 0.0);
        assert!(!trust(&[]).is_noteworthy());
        assert!(!trust(&line("a", 0.0, 0.0, 12.0)).is_noteworthy());
    }

    // -- reconstruction -----------------------------------------------------

    #[test]
    fn columns_are_split_at_the_gutter_and_read_in_order() {
        let mut glyphs = Vec::new();
        for row in 0..5 {
            glyphs.extend(line("LEFT", 50.0, 100.0 + row as f32 * 14.0, 12.0));
            glyphs.extend(line("RIGHT", 320.0, 100.0 + row as f32 * 14.0, 12.0));
        }

        let page = reconstruct(&glyphs);
        assert_eq!(page.blocks.len(), 2, "the gutter was not found");

        // Left column first, and its lines must not interleave with the right's.
        let first = page.blocks[0].lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>();
        assert!(first.iter().all(|t| t.contains("LEFT")), "columns interleaved: {first:?}");
    }

    #[test]
    fn a_single_column_is_not_split() {
        let mut glyphs = Vec::new();
        for row in 0..6 {
            glyphs.extend(line("one column of ordinary prose", 50.0, 100.0 + row as f32 * 14.0, 12.0));
        }
        assert_eq!(reconstruct(&glyphs).blocks.len(), 1);
    }

    #[test]
    fn spaces_are_recovered_from_advance_gaps() {
        // A page with no space characters at all — every glyph placed
        // absolutely. This is the case that recovers "ACCEPT ERROR" from
        // "ACCEPTERROR".
        let mut glyphs = line("ACCEPT", 50.0, 100.0, 12.0);
        glyphs.extend(line("ERROR", 130.0, 100.0, 12.0));

        let page = reconstruct(&glyphs);
        let text = page.plain();
        assert!(text.contains("ACCEPT ERROR"), "no space recovered: {text:?}");
    }

    #[test]
    fn letters_of_one_word_do_not_gain_spaces() {
        let glyphs = line("together", 50.0, 100.0, 12.0);
        assert_eq!(reconstruct(&glyphs).plain(), "together");
    }

    #[test]
    fn a_taller_capital_on_the_same_baseline_stays_on_its_line() {
        // The line-grouping bug that a top-coordinate comparison would have:
        // a larger glyph on the same line starts higher and opens a new line.
        let mut glyphs = line("word", 50.0, 100.0, 12.0);
        glyphs.push(glyph('W', 100.0, 96.0, 18.0)); // taller, same baseline area

        let page = reconstruct(&glyphs);
        assert_eq!(page.blocks[0].lines.len(), 1, "a taller letter split the line");
    }

    // -- words() -------------------------------------------------------

    #[test]
    fn one_word_stays_one_word_with_its_own_box() {
        let glyphs = line("together", 50.0, 100.0, 12.0);
        let words = words(&glyphs);
        assert_eq!(words.len(), 1);
        assert_eq!(words[0].text, "together");
        assert_eq!(words[0].glyphs.len(), 8);
        // The box is the union of its own letters, not a guess: it must start
        // at the first letter's left edge and end at the last one's right.
        assert_eq!(words[0].rect.left, glyphs[0].rect.left);
        assert_eq!(words[0].rect.right, glyphs.last().unwrap().rect.right);
    }

    /// **The reason this function exists rather than reusing `reconstruct`.**
    /// A line has one rect for the whole sentence; a selection or a
    /// recognised-word layer needs a box per word, which `TextLine` throws
    /// away on purpose.
    #[test]
    fn two_words_on_one_line_get_two_boxes() {
        let mut glyphs = line("ACCEPT", 50.0, 100.0, 12.0);
        glyphs.extend(line("ERROR", 130.0, 100.0, 12.0));

        let words = words(&glyphs);
        assert_eq!(words.len(), 2);
        assert_eq!(words[0].text, "ACCEPT");
        assert_eq!(words[1].text, "ERROR");
        // Genuinely two boxes, not one spanning both — the whole point of not
        // collapsing into `TextLine`'s single rect.
        assert!(words[0].rect.right < words[1].rect.left);
    }

    #[test]
    fn words_on_different_lines_are_never_merged() {
        let mut glyphs = line("first", 50.0, 100.0, 12.0);
        glyphs.extend(line("second", 50.0, 130.0, 12.0));

        let words = words(&glyphs);
        assert_eq!(words.len(), 2);
        let texts: Vec<&str> = words.iter().map(|w| w.text.as_str()).collect();
        assert!(texts.contains(&"first") && texts.contains(&"second"));
    }

    #[test]
    fn each_words_glyphs_are_exactly_its_own_letters_in_order() {
        let glyphs = line("cat", 50.0, 100.0, 12.0);
        let words = words(&glyphs);
        let chars: String = words[0].glyphs.iter().map(|g| g.ch).collect();
        assert_eq!(chars, "cat", "a word's own glyphs did not survive grouping in order");
    }

    #[test]
    fn no_glyphs_means_no_words() {
        assert!(words(&[]).is_empty());
    }

    #[test]
    fn rotated_text_does_not_merge_into_the_body() {
        // A 45° stamp across a drawing.
        let mut glyphs = line("body text on the page", 50.0, 100.0, 12.0);
        let mut stamp = line("DRAFT", 60.0, 100.0, 24.0);
        for g in &mut stamp {
            g.angle = 0.785;
        }
        glyphs.extend(stamp);

        let page = reconstruct(&glyphs);
        let body = page.blocks[0].lines[0].text.clone();
        assert!(!body.contains("DRAFT"), "the stamp merged into the body: {body:?}");
        assert!(page.plain().contains("DRAFT"), "the stamp was lost entirely");
    }

    // -- direction ----------------------------------------------------------

    #[test]
    fn direction_comes_from_the_script_not_a_default() {
        let arabic = line("مرحبا", 50.0, 100.0, 12.0);
        assert_eq!(dominant_direction(&arabic), Direction::Rtl);

        let latin = line("hello", 50.0, 100.0, 12.0);
        assert_eq!(dominant_direction(&latin), Direction::Ltr);

        // Digits and punctuation alone are not a direction.
        let digits = line("12345", 50.0, 100.0, 12.0);
        assert_eq!(dominant_direction(&digits), Direction::Ltr);
    }

    #[test]
    fn hebrew_and_arabic_are_both_recognised() {
        assert!(is_rtl('א'));
        assert!(is_rtl('ب'));
        assert!(!is_rtl('a'));
        assert!(!is_rtl('あ'));
    }

    // -- the entry point ----------------------------------------------------

    #[test]
    fn a_trusted_page_reports_that_it_used_character_order() {
        let glyphs = line("ordinary prose on one line", 50.0, 100.0, 12.0);
        let page = page_text(&glyphs, "ordinary prose on one line");

        assert_eq!(page.source, Source::CharacterOrder);
        assert!(page.confidence > 0.9);
        assert_eq!(page.plain(), "ordinary prose on one line");
    }

    /// Reconstruction is never chosen for you.
    #[test]
    fn a_disordered_page_is_still_returned_in_character_order() {
        // Enough transitions to be evidence — see `MIN_EVIDENCE`.
        let mut glyphs = Vec::new();
        let rows = [
            300.0, 100.0, 500.0, 200.0, 700.0, 150.0, 600.0, 250.0, 450.0, 120.0,
            650.0, 180.0, 400.0, 220.0, 550.0, 140.0,
        ];
        for top in rows {
            glyphs.extend(line("fragment", 50.0, top, 12.0));
        }
        // Flagged, but not rebuilt behind the user's back: four real documents
        // score higher than this fixture and read perfectly.
        assert!(trust(&glyphs).is_noteworthy());

        let page = page_text(&glyphs, "unusable order");
        assert_eq!(page.source, Source::CharacterOrder);
        assert!(page.confidence < 0.9, "a disordered page reported full confidence");

        // And reflowing is available when asked for.
        assert_eq!(reconstruct(&glyphs).source, Source::Reconstructed);
    }
}
