//! Page text blocks: which text objects of one page form one paragraph, heading or row.
//! Pure geometry + style; no PDFium, no egui, std only (so it is testable from fixtures).
//!
//! # What it does
//!
//! [`detect`] takes every text object of one page ([`Frag`]) and every drawn path ([`Shape`]) and
//! returns the page's [`Block`]s: a paragraph, a heading, a table row, a label. Clicking any word of a
//! paragraph must select the whole paragraph, so the grouping has to survive what real PDFs do:
//! every visual line cut into 3 to 8 text objects, justified text whose word gaps are as wide as a
//! column gutter, three columns side by side, words drawn as vector outlines instead of text
//! (ligatures, a whole line of art), headings that differ from body text by weight only.
//!
//! # How (one pass, all geometry)
//!
//! 1. **Marks.** A super- or subscript mark (a footnote number, a trademark sign, the 2 of m2) is drawn
//!    smaller and off the baseline of the text it belongs to; it is moved onto that baseline so it joins
//!    its line instead of becoming a block of its own.
//! 2. **Rows.** Fragments with (nearly) equal baseline form a row, page-wide (tolerance
//!    [`Params::row_tol`] em).
//! 3. **Outlined words.** A path the size of a word that real text vouches for (text touching it on its
//!    own baseline, a space or a justified gap away, or lines one pitch above or below in its column)
//!    becomes a placeholder member of its line, so a hole in a line is bridged. These are the ids in
//!    [`Line::outlined`]. Vector art is not text: a path with a text object inside it is a cell, a path
//!    lying on another of its size (the fill and the stroke of one shape, the modules of a QR code) is
//!    a drawing, and text columns a few em away on the same baseline vouch for nothing.
//! 4. **Pieces.** A row is cut into pieces (the part of a line that lies in one column). A gap is cut
//!    when a drawn box or a vertical rule separates the two sides (a box of ANY size that holds one
//!    side and not the other: a cell 12 em tall, a panel; a frame that holds both separates nothing),
//!    when a gap of 1.5 em lies inside a box that holds that one row of text and nothing else (the
//!    labels of a header band are its cells), when it is 2 em wide and no row near it has the left
//!    and right edges of this one (a justified block has such rows; a chart's tick labels, a legend or
//!    a header have none), or when it is wide and *corridor
//!    evidence* says so: a gap that continues as an empty corridor through the
//!    neighbouring rows, with text on both sides of it, is a column gutter; one that does not is a
//!    justified word gap. A pure gap threshold cannot separate the two (word gaps reach 1.1 em, the
//!    gutter is 1.46 em), the corridor can. A list marker on its own ("1.", a bullet) is joined to the
//!    text after it, a row between two horizontal rules (a table row) is never cut except by a
//!    vertical rule or a box, and a lone symbol standing in a gutter (an asterisk, a divider bar) is cut
//!    off on both sides.
//! 5. **Chains.** Each piece links to the nearest row's piece above it with enough horizontal overlap,
//!    never across a horizontal rule that covers most of what the two lines share (the rule of a table,
//!    not the underline of a hyperlink: a rule hugging the glyphs of the upper line, within
//!    [`Params::underline_zone`] em of its baseline, must cover nearly all of what they share), never
//!    across a drawn box that holds one of the two lines and not the other (two cells, not one
//!    paragraph) and never to or from a line that stretches over two or more pieces of its neighbour
//!    row (a heading or a paragraph across two columns); the links form chains: the lines of one column.
//! 6. **Blocks.** A chain is cut, in this order of reasons, where the size changes, the style changes
//!    (dominant font by characters; both size and style compare only lines of one *pure* look, 85 % of
//!    the characters in one font and size: a line half in a larger size says nothing), the left and
//!    right margins both move (a block quote), the line pitch jumps above the run's typical pitch (the
//!    median of its own line pitches, 1.2 em when it has none; four equal pitches up to 2.5 em, a
//!    double-spaced manuscript, are trusted), a list item starts, a first-line indent starts, a flush
//!    line follows lines that hang in (the entries of a reference list), or the line above ended short
//!    (justified: right edge short of the column's right edge; ragged: a sentence end with room for the
//!    next word). A run counts as justified when half of its lines end within an eighth of an em of the
//!    longest line of its column (lines much wider than the run, a full-width heading, and lines of
//!    another size are not part of that column) and that margin is *proved*: two ragged lines end
//!    together by chance about one time in twenty, so [`Params::edge_lines`] lines of the run, or
//!    [`Params::justify_evidence`] lines of the column, must sit on it (a common left edge for the indent
//!    rule needs the same number). A stack of at most [`Params::cell_lines`] lines with a rule right
//!    under it is one ruled cell: its weights and colours do not cut it. A line with no dominant look
//!    (a bold label and a regular value, a company name and a reference in two sizes) is cut from a
//!    neighbour that has no look in common with it (a title over it, the clause under it), and a
//!    number of at most four characters over a long line (a chart's tick value) is cut when the next
//!    word would have fitted on it twice over.
//!
//! Everything is in em (the local font size) or in statistics of the page (the body size is the size
//! with most characters). There is no absolute point threshold: scaling a page scales its blocks.
//!
//! # Contract guarantees
//!
//! * Deterministic: the result does not depend on the order of the input; every sort has the object id
//!   as its last key. Blocks are ordered top to bottom, then left to right, by their first line; lines
//!   top to bottom; objects left to right.
//! * Never panics: empty input, NaN or infinite coordinates and sizes, zero or negative sizes, top above
//!   bottom (normalised), duplicates, empty text, rotated text.
//! * Every input [`Frag`] ends up in exactly one block (by object id; a duplicate id is dropped).
//!   A rotated fragment, and a fragment with no usable geometry (non-finite numbers, size not
//!   positive), is a one-line block of its own. A fragment with empty or whitespace-only text is
//!   tolerated: it is laid out by its rectangle like any other but has no characters, hence no vote
//!   in any style or size statistic.
//!
//! # `starts_because`
//!
//! Short stable words (they go to the session log):
//! `"start"` top of a column (nothing above links to it); `"column"` the line above belongs to another
//! column (it went on with a line that overlaps it better); `"rule"` a horizontal rule lies between
//! this line and the one above; `"size"` the type size changes; `"style"` font weight, typeface, colour
//! or a slightly different size; `"margin"` the left and the right margin both move (a block quote);
//! `"pitch"` the line spacing jumps (a paragraph gap); `"item"` a list item (bullet, number, footnote
//! mark, or the flush first line of a hanging entry) starts; `"indent"` a first-line indent;
//! `"short-line"` the line above ended short (end of a
//! paragraph without a blank line); `"rotated"` a rotated fragment; `"unplaced"` a fragment without
//! usable geometry.
//!
//! # Style (heading versus body)
//!
//! Two lines differ in style when their dominant fonts (by non-space characters, never one minority
//! fragment) differ by [`same_font_style`]: equal ids are the same font; different ids differ when
//! both measured stems differ by more than [`Params::stem_tol`] (12/1000 em), or when the stems are
//! within that (or one is unknown) and the cleaned face names are both known and differ. One font
//! embedded as two resources (equal stems, equal names) is one style, and a font with no stem and no
//! name evidence agrees with everything (no evidence never splits a block;
//! [`Params::unknown_stem_same_face`] turns that off: an unknown stem then merges nothing). The weight
//! ladder Light 51, Regular 74, Medium 100, SemiBold 124, ExtraBold 198 differs at every step, and the
//! page-1 trap of the datasheet (five weights all named "Montserrat-Thin") is told apart by stems
//! alone. A stem above 300/1000 em is no font's (subset fonts with composite glyphs and substituted fonts
//! have been measured at 357 to 507 for faces whose real stems are 84 to 198): it is unknown, and the
//! names decide. A line's colour is the colour of most of its characters, not of its first fragment (a
//! blue link in a line of black text). The comparison is pairwise on the two dominant fonts, never through clusters of fonts:
//! "unknown agrees with everything" is not transitive, and a cluster built around an unknown font
//! swallows every weight of its family.
//!
//! # Limitations (see also the tests)
//!
//! Right-to-left text is laid out by geometry like left-to-right text (rows and gaps are symmetric)
//! but the left-edge rules (indent, item markers, ragged short lines) look at the left edge, so an RTL
//! paragraph is grouped by justification and pitch only. Vertical or rotated text is never grouped.
//! Centred or right-aligned paragraphs have no common edge, so only pitch, style, size and rules can
//! end them. Tables without rules or boxes are grouped per cell column by pitch only, except where a
//! gap is 2 em wide or more (see step 4); two cells closer than that merge. A chart's legend entries
//! one pitch apart on one left edge read as the two lines of a paragraph. A paragraph that flows
//! across the gutter into the next column is two blocks (the gutter is a column boundary). A divider
//! drawn as one bar glyph per line, or a lone mark standing in a gutter, fills the corridor the column
//! test relies on; a divider drawn as a path (a vertical rule) is understood. A ragged line that ends a
//! sentence and has room left for the next word is read as the end of its paragraph (word-wrapped text
//! never does that, a forced line break and a paragraph end do), so ragged text laid out by hand with
//! random line ends is cut there. Short justified paragraphs (three or four lines) standing alone in
//! their column are read as ragged text, which loses nothing unless a line ends short without ending a
//! sentence. A block quote and centred lines of nearly equal width look alike (both edges move inwards).

use std::cmp::Ordering;

mod furniture;
mod rows;
mod link;
mod layout;
mod segment;
use segment::*;
use layout::*;
use link::*;
use rows::*;
use furniture::*;

pub const API_VERSION: u32 = 1;

/// One non-empty text object of a page. Coordinates are PDF points in the space of pdf_core TextRun::rect:
/// top-left origin, y grows downward; the detector must tolerate top > bottom (normalise).
/// (Empty or whitespace-only text is tolerated too: it is laid out like any other object but carries
/// no characters, so it has no say in any style or size decision.)
#[derive(Clone, Debug, PartialEq)]
pub struct Frag {
    pub object: usize, // PDFium page-object index = the id the app uses everywhere; also content-stream order
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub baseline: f32, // baseline y (TextRun::origin.y), same space
    pub size: f32,     // effective font size in points
    pub font: u32,     // font identity: equal ids = same font program (one per /Font resource on this page)
    pub stem: Option<u16>, // measured outline stem in 1/1000 em (weight signal: Light 51, Regular 74, Medium 100, SemiBold 124, ExtraBold 198); None = unknown
    pub face: String,  // font name with any subset tag stripped; may be empty or useless (page 1 of the datasheet says Montserrat-Thin for five weights)
    pub rgb: [u8; 3],
    pub text: String, // PDFium text of the object; U+0002 marks a hyphenation at a line end
    pub rotated: bool, // not upright left-to-right (rotated or vertical): never grouped with others
}

/// A drawn path object (not text). Used for rules, boxes and outlined (path-drawn) words.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shape {
    pub object: usize,
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub depth: u32, // form-XObject nesting depth; only depth 0 is considered
}

#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub objects: Vec<usize>,  // text objects of this visual line, left to right (text only)
    pub outlined: Vec<usize>, // Shape objects bridged into this line as outlined words, left to right
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32, // union of ALL members (text and outlined)
    pub baseline: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub lines: Vec<Line>,             // top to bottom; a line made only of outlined words has objects empty
    pub starts_because: &'static str, // why a block begins here: "start", "style", "size", "rule", "pitch", "item", "indent", "short-line", "column", ...
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl Block {
    /// Every text object, line by line, left to right.
    pub fn objects(&self) -> Vec<usize> {
        self.lines.iter().flat_map(|l| l.objects.iter().copied()).collect()
    }

    /// Text objects only.
    pub fn contains(&self, object: usize) -> bool {
        self.lines.iter().any(|l| l.objects.contains(&object))
    }
}

/// Group a page's text into blocks. Page-wide; deterministic; never panics (empty input, NaN, zero sizes, duplicates).
/// Every input Frag that is not rotated ends up in exactly one block; each rotated Frag is a one-line block of its own.
pub fn detect(frags: &[Frag], shapes: &[Shape]) -> Vec<Block> {
    detect_with(frags, shapes, &Params::default())
}

/// Index of the block that holds text object `object`.
pub fn block_index_of(blocks: &[Block], object: usize) -> Option<usize> {
    blocks.iter().position(|b| b.contains(object))
}

// ------------------------------------------------------------------------------------------------
// Parameters
// ------------------------------------------------------------------------------------------------

/// Switches and tunables of the detector. `Params::default()` is what ships. Every length is in em of
/// the local font size unless the name says "body" (the page's body size, the size with most
/// characters) or "ratio". The sensitivity of the numbers was measured against two independent
/// hand labelings (see the acceptance tests): most are flat over a wide range.
#[derive(Clone, Debug)]
pub struct Params {
    // ---- switches: each rule can be turned off by a test or the ablation ----
    /// Corridor evidence decides ambiguous gaps. Off: a pure threshold, `gap_only` em.
    pub corridor: bool,
    /// Bridge path-drawn (outlined) words into their lines.
    pub outlined: bool,
    /// Drawn rules: a vertical rule inside a gap cuts the row; a horizontal rule between two lines
    /// ends the block.
    pub rules: bool,
    /// A drawn box (cell, tag) holding one side of a gap and not the other cuts the row.
    pub boxes: bool,
    /// A type-size change between lines ends the block ("size").
    pub size_rule: bool,
    /// A style change (weight, typeface, colour, size) between lines ends the block ("style").
    pub style_rule: bool,
    /// A line pitch above the run's typical pitch ends the block ("pitch").
    pub pitch_rule: bool,
    /// A list marker (bullet, "1.", "(a)", a footnote mark) starts a block ("item").
    pub list_items: bool,
    /// A first-line indent starts a block ("indent").
    pub indent_rule: bool,
    /// A flush line between two lines that hang in (a reference list, a glossary: the first line of an
    /// entry is flush, its continuation hangs in) starts a block ("item").
    pub hanging_items: bool,
    /// A move of both the left and the right margin (a block quote) starts a block ("margin").
    pub margin_rule: bool,
    /// A short line ends a block ("short-line").
    pub short_lines: bool,
    /// Super- and subscript marks join the line of the text they belong to.
    pub script_marks: bool,
    /// A list marker on its own ("1.", a bullet) is joined to the text after it.
    pub marker_merge: bool,
    /// A lone symbol (a mark, a divider bar) that stands in a gutter on one row cuts the row on both
    /// sides, and symbols do not fill the gutter of the neighbouring rows.
    pub stray_marks: bool,
    /// Rows between two horizontal rules are never cut by a gap; a stack of up to `cell_lines` lines
    /// with a rule right under it is one cell.
    pub ruled_bands: bool,
    /// A line that stretches over two or more pieces of the line above (or under) it, a heading or a
    /// paragraph across two columns, belongs to none of those columns: it links to neither.
    pub spanning_lines: bool,
    /// One font with an unknown stem and another with equal or unknown face names look alike (no
    /// evidence never splits a block). Off: an unknown stem merges nothing.
    pub unknown_stem_same_face: bool,
    /// Lines with no dominant look (a bold label and a regular value) are told apart from their neighbours
    /// when they have no look in common with them: a title over them, the paragraph under them.
    pub mixed_looks: bool,

    // ---- rows and gaps ----
    /// Baselines closer than this (em of the smaller fragment) are one row.
    pub row_tol: f32,
    /// Gaps up to this are always word spaces.
    pub word_gap: f32,
    /// Gaps wider than this always cut a line.
    pub max_hole: f32,
    /// Narrowest empty corridor that counts as evidence of a gutter.
    pub corridor_width: f32,
    /// Rows that prove a corridor needed to cut a gap on that evidence alone (a river of aligned word
    /// gaps over three lines of justified text happens by chance now and then, over four lines hardly
    /// ever) ...
    pub pos3: usize,
    /// ... two proving rows cut a gap of at least this ...
    pub pos2_gap: f32,
    /// ... one proving row cuts a gap of at least this ...
    pub pos1_gap: f32,
    /// ... a gap with no neighbouring text at all cuts at this ...
    pub iso_gap: f32,
    /// ... and at this when the fonts or colours of the two sides also differ ...
    pub iso_style_gap: f32,
    /// ... and a gap this wide (em) is no word space whatever the rows above and under it do, unless
    /// the row is part of a justified block: a justified word gap stays within a few word spaces
    /// (1.7 em in the datasheet's narrow columns), the gaps of a chart's tick labels, a legend or a
    /// header are wider. Proof of justification: a row within `near` em whose left and right edges
    /// are those of this row (within `flush_tol`).
    pub sparse_gap: f32,
    /// ... see `sparse_gap`: how far (em) the edges of a neighbouring row may be off this row's.
    pub flush_tol: f32,
    /// Neighbour rows are walked while their baseline stays within this many em of the last one.
    pub near: f32,
    /// How far (em) beside a gap text counts as "text near the gap".
    pub scan_window: f32,
    /// Pure-threshold gap (em) used when `corridor` is off.
    pub gap_only: f32,

    // ---- marks, markers, ruled rows ----
    /// A mark is at most this fraction of the size of the text it belongs to.
    pub script_ratio: f32,
    /// A mark sits at most this far (em of its host) off the host's baseline.
    pub script_shift: f32,
    /// A mark touches its host: gap at most this (em of the host).
    pub script_gap: f32,
    /// A list marker on its own is joined to the text after it when the gap is at most this.
    pub marker_gap: f32,
    /// A ruled row has a horizontal rule at most this far (em) below its baseline ...
    pub band_below: f32,
    /// ... and one at most this far above it.
    pub band_above: f32,
    /// A stack of at most this many lines with a rule right under it is one ruled cell.
    pub cell_lines: usize,
    /// A drawn box at most this tall (em) that holds a row of text and no other row is a one-row cell
    /// (a header bar, a tag) ...
    pub lone_cell_h: f32,
    /// ... and a gap of at least this (em) inside it separates two cells of the bar (one row cannot be
    /// justified text, and a word space is a third of an em).
    pub lone_cell_gap: f32,

    // ---- chains and blocks ----
    /// Least horizontal overlap (fraction of the narrower piece) for a line to link to the line above.
    pub link_overlap: f32,
    /// Extra line spacing (em) over the run's typical pitch that starts a block.
    pub pitch_extra: f32,
    /// Line pitches below this (em) are line spacing, wider ones are paragraph gaps ...
    pub leading: f32,
    /// ... unless a run has at least four equal pitches: then its leading is trusted up to this.
    pub wide_leading: f32,
    /// First-line indent (em) that starts a block.
    pub indent: f32,
    /// Both margins moving by more than this (em) starts a block.
    pub margin_shift: f32,
    /// The left edge is "common" when this share of a run's lines sit on it (else: centred, right-
    /// aligned or ragged on the left, and the indent rule stays out) and at least `edge_lines` lines do.
    pub edge_dominance: f32,
    /// An edge is shared by chance by two lines about one time in twenty, so a margin counts as
    /// real only when this many lines sit on it: the common left edge of a run (indent rule), and the
    /// right margin of a justified run (at least half of its lines reach it, and at least this many,
    /// or `justify_evidence` lines of its column do).
    pub edge_lines: usize,
    /// A ragged line ends a block when the room left over exceeds the next word by this (em).
    pub short_margin: f32,
    /// A justified line ends a block when its right edge is short of the column's by more than this (em).
    pub short_slack: f32,
    /// A line of at most this many characters (a chart's axis label, a number) ends a ragged block when the
    /// first word of the next line would have fitted on it twice over: the break is not a wrap. 0 = never.
    pub tiny_line: usize,
    /// A column with this many lines flush on its right margin is justified (see `edge_lines`).
    pub justify_evidence: usize,
    /// Type-size change between lines that ends a block (fraction of the smaller size).
    pub size_break: f32,
    /// Type-size difference that counts as a style difference (fraction of the size).
    pub size_rel: f32,
    /// Share of a line's characters its dominant style must hold for the line to take part in style
    /// comparison (a line with a large minority fragment says nothing about style).
    pub pure_share: f32,
    /// Colour difference (per channel, 0 to 255) that counts as a style difference.
    pub colour_delta: u8,
    /// Weight difference: stems differing by more than this (1/1000 em) are different weights.
    pub stem_tol: u16,
    /// Left/right edge alignment tolerance (em of the run's first line).
    pub edge_tol: f32,

    // ---- shapes (em of the page's body size) ----
    /// A path at most this thick and at least `rule_len` long is a rule.
    pub rule_thick: f32,
    pub rule_len: f32,
    /// A rule separates two lines when it covers at least this share of what the lines have in common.
    pub rule_cover: f32,
    /// A rule this close (em) under the baseline of the upper line hugs its glyphs: it may be the
    /// underline of a hyperlink or a heading, so it separates the lines only when it covers at least
    /// `underline_cover` of what they have in common (a table rule sits 0.3 em and more under the text).
    pub underline_zone: f32,
    /// See `underline_zone`.
    pub underline_cover: f32,
    /// A path of this size (min, max width; min, max height) is an outlined word candidate.
    pub word_w: (f32, f32),
    pub word_h: (f32, f32),
    /// A text-sized path with more than this share of its area under text is a cell, not a word ...
    pub cover_max: f32,
    /// ... and so is one that holds a text object: this share of a text object's area inside the path is
    /// enough (a cell with a small amount in it covers a tenth of its own area only).
    pub cell_inside: f32,
    /// Text on the path's own baseline vouches for it only when it touches: the gap to it is at most this
    /// (em of body). A word drawn as outlines sits in its line, a space or a justified gap away from the
    /// text beside it; text columns a few em off on the same baseline vouch for nothing.
    pub outlined_touch: f32,
    /// A path whose box lies this share over the box of another path of comparable size (within a factor
    /// of eight in area) is drawing, not a word: the fill and the stroke of one shape, the modules of a QR
    /// code, the two layers of a chart's cone. Words drawn as outlines do not overlap each other.
    pub art_overlap: f32,
    /// A path of this size can be a box (cell) around text. Only the lower limits matter by default
    /// (no upper limit: a box is a boundary by what it holds, never by how large it is; a frame that
    /// holds both sides of a gap separates nothing, a page-high cell that holds one of them does).
    pub box_w: (f32, f32),
    pub box_h: (f32, f32),
}

impl Default for Params {
    fn default() -> Params {
        Params {
            corridor: true,
            outlined: true,
            rules: true,
            boxes: true,
            size_rule: true,
            style_rule: true,
            pitch_rule: true,
            list_items: true,
            indent_rule: true,
            hanging_items: true,
            margin_rule: true,
            short_lines: true,
            script_marks: true,
            marker_merge: true,
            stray_marks: true,
            ruled_bands: true,
            spanning_lines: true,
            unknown_stem_same_face: true,
            mixed_looks: true,
            row_tol: 0.28,
            word_gap: 0.8,
            max_hole: 4.0,
            corridor_width: 0.8,
            pos3: 4,
            pos2_gap: 1.2,
            pos1_gap: 2.5,
            iso_gap: 2.5,
            iso_style_gap: 1.0,
            sparse_gap: 2.0,
            flush_tol: 1.0,
            near: 3.0,
            scan_window: 8.0,
            gap_only: 1.25,
            script_ratio: 0.8,
            script_shift: 0.65,
            script_gap: 0.35,
            marker_gap: 6.0,
            band_below: 1.2,
            band_above: 2.4,
            cell_lines: 2,
            lone_cell_h: 2.6,
            lone_cell_gap: 1.5,
            link_overlap: 0.4,
            pitch_extra: 0.30,
            leading: 1.7,
            wide_leading: 2.5,
            indent: 0.9,
            margin_shift: 0.9,
            edge_dominance: 0.5,
            edge_lines: 4,
            short_margin: 0.7,
            short_slack: 0.5,
            tiny_line: 4,
            justify_evidence: 8,
            size_break: 0.15,
            size_rel: 0.05,
            pure_share: 0.85,
            colour_delta: 40,
            stem_tol: 12,
            edge_tol: 0.125,
            rule_thick: 0.2,
            rule_len: 1.25,
            rule_cover: 0.6,
            underline_zone: 0.25,
            underline_cover: 0.9,
            word_w: (0.75, 42.5),
            word_h: (0.7, 1.5),
            cover_max: 0.3,
            cell_inside: 0.5,
            outlined_touch: 1.5,
            art_overlap: 0.5,
            box_w: (0.625, f32::INFINITY),
            box_h: (0.625, f32::INFINITY),
        }
    }
}

// ------------------------------------------------------------------------------------------------
// Font style
// ------------------------------------------------------------------------------------------------

/// A font name without its subset tag ("BCDIEE+OpenSans-Regular"), trimmed and lower-cased.
fn clean_face(face: &str) -> String {
    let f = face.trim();
    let f = match f.split_once('+') {
        Some((tag, rest)) if tag.len() == 6 && tag.bytes().all(|c| c.is_ascii_uppercase()) => rest,
        _ => f,
    };
    f.to_ascii_lowercase()
}

/// Do two fonts that are not the same font program (different ids) look the same?
///
/// The rule, in this order:
/// 1. both stems known and differing by more than `stem_tol` (1/1000 em): different weights, a
///    different style, whatever the names say;
/// 2. otherwise (same weight, or a stem unknown) the cleaned face names decide: both known and
///    different means a different typeface; equal or unknown means the same style, so one font
///    embedded as two resources (equal stems, equal names) is one style.
///
/// The weight ladder Light 51 / Regular 74 / Medium 100 / SemiBold 124 / ExtraBold 198 differs at
/// every adjacent step (the smallest step is 23); the same font measured through its `I` and its `l`
/// (198 against 190) is one weight.
pub fn same_font_style(stem_a: Option<u16>, face_a: &str, stem_b: Option<u16>, face_b: &str, stem_tol: u16) -> bool {
    same_clean(stem_a, &clean_face(face_a), stem_b, &clean_face(face_b), stem_tol, true)
}

/// The largest stem (1/1000 em) a font can have: the heaviest weights measure 200 to 260, and a stem of
/// 357 to 507 has been measured for faces whose real stems are 84 to 198 (a subset font with composite
/// glyphs, a substituted font: the number belongs to some other program). Above this a stem is unknown.
/// (Two stems far apart under one face name are NOT told to be wrong by their ratio: the datasheet's five
/// weights are all named "Montserrat-Thin", Light 51 against ExtraBold 198 is 3.9 times and the substituted
/// fonts measure 20, so no ratio separates two real weights of one name from a bad measurement; only an
/// impossible stem does.)
const STEM_MAX: u16 = 300;

fn same_clean(stem_a: Option<u16>, face_a: &str, stem_b: Option<u16>, face_b: &str, tol: u16, unknown_ok: bool) -> bool {
    match (stem_a.filter(|&s| s <= STEM_MAX), stem_b.filter(|&s| s <= STEM_MAX)) {
        (Some(a), Some(b)) => {
            if a.abs_diff(b) > tol {
                return false;
            }
        }
        _ => {
            if !unknown_ok {
                return false;
            }
        }
    }
    face_a.is_empty() || face_b.is_empty() || face_a == face_b
}

// ------------------------------------------------------------------------------------------------
// Internal layout types
// ------------------------------------------------------------------------------------------------

/// The glyphs of a lone symbol that is no part of the text: bullets, bars, asterisks, footnote signs.
/// (Punctuation is not one: a comma is a text object of its own in half the PDFs and sits in every gap.)
const MARK_GLYPHS: &str = "|¦│║•·▪◦●‣∙*†‡§¶";

/// Font index of an outlined placeholder: it differs from every real font.
const NO_FONT: u32 = u32::MAX;

/// The page's fonts, by dense index (sorted by font id): the measured stem and the cleaned name.
struct Fonts {
    stem: Vec<Option<u16>>,
    face: Vec<String>,
    tol: u16,
    unknown_ok: bool,
}

impl Fonts {
    /// Do the fonts with these indices look different?
    fn differs(&self, a: u32, b: u32) -> bool {
        if a == b {
            return false;
        }
        if a == NO_FONT || b == NO_FONT {
            return true;
        }
        let (a, b) = (a as usize, b as usize);
        !same_clean(self.stem[a], &self.face[a], self.stem[b], &self.face[b], self.tol, self.unknown_ok)
    }
}

/// A fragment in layout: a text object, or the placeholder of an outlined word.
#[derive(Clone)]
struct It {
    obj: usize,
    l: f64,
    t: f64,
    r: f64,
    b: f64,
    base: f64,
    size: f64,
    font: u32,       // index into the page's font table; NO_FONT for a placeholder
    size_class: u32, // size class (sizes within 1 % are one class)
    rgb: [u8; 3],
    nchars: usize, // non-space characters; 0 for a placeholder and for blank text
    text: String,
    opaque: bool,
    script: bool, // a super- or subscript mark moved onto the baseline of its host
    mark: bool,   // a lone symbol of at most two characters (a bullet, a bar, an asterisk), not punctuation
}

struct Row {
    base: f64,
    size0: f64, // size of the row's first fragment (the tolerance of later joiners)
    items: Vec<usize>,
    // sorted by left; prefix maximum of the right edges makes "any fragment reaching x" a binary search
    ls: Vec<f64>,
    rs: Vec<f64>,
    pmr: Vec<f64>,
    // the same without the lone symbols: what the corridor walk of the neighbouring rows sees
    sl: Vec<f64>,
    sr: Vec<f64>,
    sp: Vec<f64>,
}

struct Piece {
    row: usize,
    items: Vec<usize>,
    l: f64,
    r: f64,
    t: f64,
    b: f64,
    base: f64,
    size: f64,
    font: u32, // the dominant font
    rgb: [u8; 3],
    pure: bool,
    /// every look (font, size, colour) that holds at least a tenth of the line's characters
    looks: Vec<PieceLook>,
    opaque: bool,
    /// the line starts with a super- or subscript mark and goes on: a footnote starts here
    lead_script: bool,
    /// nothing but lone symbols (a divider bar, a bullet): no column of text
    mark_only: bool,
    text: String,
}

/// One look of a line: the font, the mean size and the colour of a (font, size class) of its fragments.
#[derive(Clone, Copy)]
struct PieceLook {
    font: u32,
    size: f64,
    rgb: [u8; 3],
}

#[derive(Clone, Copy)]
struct HRule {
    y: f64,
    l: f64,
    r: f64,
}

#[derive(Clone, Copy)]
struct VRule {
    l: f64,
    r: f64,
    t: f64,
    b: f64,
}

#[derive(Clone, Copy)]
struct BoxR {
    l: f64,
    t: f64,
    r: f64,
    b: f64,
}

/// Boxes sorted by top, with a segment tree of the largest bottom over them: every box that spans a
/// vertical range is found in O(log n + found), however tall the boxes are (a page frame spans every
/// row, and must not be scanned again for every gap).
#[derive(Default)]
struct BoxIndex {
    boxes: Vec<BoxR>,
    max_b: Vec<f64>, // implicit binary tree over `boxes`, leaves at `size + k`
    size: usize,
}

impl BoxIndex {
    fn new(mut boxes: Vec<BoxR>) -> BoxIndex {
        boxes.sort_by(|a, b| a.t.total_cmp(&b.t).then(a.l.total_cmp(&b.l)).then(a.b.total_cmp(&b.b)).then(a.r.total_cmp(&b.r)));
        let size = boxes.len().next_power_of_two().max(1);
        let mut max_b = vec![f64::NEG_INFINITY; 2 * size];
        for (k, s) in boxes.iter().enumerate() {
            max_b[size + k] = s.b;
        }
        for i in (1..size).rev() {
            max_b[i] = max_b[2 * i].max(max_b[2 * i + 1]);
        }
        BoxIndex { boxes, max_b, size }
    }

    fn is_empty(&self) -> bool {
        self.boxes.is_empty()
    }

    /// Does `pred` hold for some box with top <= `y1` and bottom >= `y0`? (At most `MAX_BOX_VISITS` boxes
    /// are looked at: a page that stacks thousands of page-high panels cannot make one lookup slow.)
    fn any_spanning(&self, y0: f64, y1: f64, pred: &mut impl FnMut(&BoxR) -> bool) -> bool {
        let hi = self.boxes.partition_point(|s| s.t <= y1);
        let mut budget = MAX_BOX_VISITS;
        self.walk(1, 0, self.size, hi, y0, &mut budget, pred)
    }

    #[allow(clippy::too_many_arguments)]
    fn walk(&self, node: usize, lo: usize, end: usize, hi: usize, y0: f64, budget: &mut usize, pred: &mut impl FnMut(&BoxR) -> bool) -> bool {
        if lo >= hi || *budget == 0 || self.max_b[node] < y0 {
            return false;
        }
        if node >= self.size {
            *budget -= 1;
            return pred(&self.boxes[node - self.size]);
        }
        let mid = (lo + end) / 2;
        self.walk(2 * node, lo, mid, hi, y0, budget, pred) || self.walk(2 * node + 1, mid, end, hi, y0, budget, pred)
    }
}

/// The most boxes one lookup examines (see `BoxIndex::any_spanning`).
// ponytail: a lookup that meets more than 4096 boxes spanning one row stops looking (a page of page-high panels,
// never seen); lift it, or keep the stack of containers as a tree, if one ever matters.
const MAX_BOX_VISITS: usize = 4096;

/// The largest coordinate (pt) a path may have to count as drawn furniture.
const MAX_COORD: f64 = 1.0e5;

#[derive(Default)]
struct Furniture {
    hrules: Vec<HRule>, // sorted by y
    vrules: Vec<VRule>, // sorted by left
    boxes: BoxIndex,
}

fn nonspace(s: &str) -> usize {
    s.chars().filter(|c| !c.is_whitespace()).count()
}

fn colour_far(a: [u8; 3], b: [u8; 3], delta: u8) -> bool {
    a.iter().zip(b.iter()).any(|(x, y)| (*x as i32 - *y as i32).abs() > delta as i32)
}

// ------------------------------------------------------------------------------------------------
// detect_with
// ------------------------------------------------------------------------------------------------

/// A one-line block that stands apart from the layout: rotated or unplaceable text.
struct Special {
    obj: usize,
    rect: [f64; 4],
    baseline: f64,
    why: &'static str,
}

/// A line under construction.
struct LineDraft {
    members: Vec<(f64, usize)>, // (left, object)
    outlined: Vec<(f64, usize)>,
    rect: [f64; 4],
    baseline: f64,
}

struct BlockDraft {
    lines: Vec<LineDraft>,
    why: &'static str,
}

fn fin(v: f64) -> f64 {
    if v.is_finite() {
        v
    } else {
        0.0
    }
}

fn frag_order(a: &Frag, b: &Frag) -> Ordering {
    a.object
        .cmp(&b.object)
        .then(a.left.total_cmp(&b.left))
        .then(a.top.total_cmp(&b.top))
        .then(a.right.total_cmp(&b.right))
        .then(a.bottom.total_cmp(&b.bottom))
        .then(a.baseline.total_cmp(&b.baseline))
        .then(a.size.total_cmp(&b.size))
        .then(a.font.cmp(&b.font))
        .then(a.text.cmp(&b.text))
}

/// `detect` with explicit parameters.
pub fn detect_with(frags: &[Frag], shapes: &[Shape], p: &Params) -> Vec<Block> {
    // ---- canonical order, one fragment per object id ----
    let mut order: Vec<usize> = (0..frags.len()).collect();
    order.sort_by(|&a, &b| frag_order(&frags[a], &frags[b]));
    order.dedup_by_key(|&mut i| frags[i].object);

    let mut layout: Vec<usize> = Vec::new();
    let mut specials: Vec<Special> = Vec::new();
    for &i in &order {
        let f = &frags[i];
        let rect = [f.left as f64, f.top as f64, f.right as f64, f.bottom as f64];
        let geometry_ok = rect.iter().all(|v| v.is_finite()) && (f.baseline as f64).is_finite() && (f.size as f64).is_finite() && f.size > 0.0;
        if f.rotated || !geometry_ok {
            specials.push(Special {
                obj: f.object,
                rect: [fin(rect[0]).min(fin(rect[2])), fin(rect[1]).min(fin(rect[3])), fin(rect[0]).max(fin(rect[2])), fin(rect[1]).max(fin(rect[3]))],
                baseline: fin(f.baseline as f64),
                why: if f.rotated { "rotated" } else { "unplaced" },
            });
        } else {
            layout.push(i);
        }
    }

    let drafts: Vec<BlockDraft> = if layout.is_empty() { Vec::new() } else { layout_blocks(frags, &layout, shapes, p) };

    // ---- assemble ----
    let mut blocks: Vec<Block> = Vec::with_capacity(drafts.len() + specials.len());
    for d in drafts {
        blocks.push(finish_block(d));
    }
    for s in specials {
        let d = BlockDraft {
            lines: vec![LineDraft { members: vec![(s.rect[0], s.obj)], outlined: Vec::new(), rect: s.rect, baseline: s.baseline }],
            why: s.why,
        };
        blocks.push(finish_block(d));
    }
    blocks.sort_by(|a, b| {
        let (la, lb) = (&a.lines[0], &b.lines[0]);
        la.top
            .total_cmp(&lb.top)
            .then(la.left.total_cmp(&lb.left))
            .then(first_id(a).cmp(&first_id(b)))
    });
    blocks
}


// ---- shapes ----


// ---- rows ----


// ---- links ----


type Run = (Vec<usize>, &'static str);

/// Cut `run` after every position `i` (between `run[i]` and `run[i + 1]`) where `brk` names a reason.

#[cfg(test)]
mod tests {
    use super::*;

    fn frag(object: usize, left: f32, baseline: f32, text: &str) -> Frag {
        Frag {
            object,
            left,
            top: baseline - 7.0,
            right: left + 10.0,
            bottom: baseline + 2.0,
            baseline,
            size: 9.0,
            font: 0,
            stem: None,
            face: String::new(),
            rgb: [0, 0, 0],
            text: text.to_string(),
            rotated: false,
        }
    }

    #[test]
    fn smoke_two_rows_become_two_blocks_and_lookup_works() {
        let frags = vec![frag(5, 10.0, 100.0, "b"), frag(4, 0.0, 100.0, "a"), frag(6, 0.0, 120.0, "c")];
        let blocks = detect(&frags, &[]);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].objects(), vec![4, 5]);
        assert_eq!(blocks[1].objects(), vec![6]);
        assert_eq!(block_index_of(&blocks, 5), Some(0));
        assert_eq!(block_index_of(&blocks, 6), Some(1));
        assert_eq!(block_index_of(&blocks, 99), None);
        assert!(blocks[0].contains(4) && !blocks[0].contains(6));
    }

    #[test]
    fn smoke_empty_and_hostile_input_never_panics() {
        assert!(detect(&[], &[]).is_empty());
        let mut f = frag(1, f32::NAN, f32::INFINITY, "x");
        f.size = f32::NAN;
        let mut r = frag(2, 0.0, 5.0, "y");
        r.rotated = true;
        let blocks = detect(&[f, r], &[]);
        assert_eq!(blocks.len(), 2);
        assert_eq!(API_VERSION, 1);
    }
}
