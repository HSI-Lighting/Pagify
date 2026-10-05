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

fn first_id(b: &Block) -> usize {
    b.lines.iter().flat_map(|l| l.objects.iter().chain(l.outlined.iter())).copied().next().unwrap_or(usize::MAX)
}

fn finish_block(d: BlockDraft) -> Block {
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
fn size_class_starts(sizes: &mut Vec<f64>) -> Vec<f64> {
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

fn size_class_of(starts: &[f64], size: f64) -> u32 {
    (starts.partition_point(|&s| s <= size).max(1) - 1) as u32
}

fn layout_blocks(frags: &[Frag], layout: &[usize], shapes: &[Shape], p: &Params) -> Vec<BlockDraft> {
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

// ---- shapes ----

fn norm_rect(s: &Shape) -> Option<[f64; 4]> {
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
fn classify_shapes(shapes: &[Shape], body: f64, p: &Params) -> Furniture {
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
fn furn_words(shapes: &[Shape], body: f64, p: &Params) -> Vec<(usize, [f64; 4])> {
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
fn furn_art(shapes: &[Shape], body: f64, p: &Params) -> Vec<(usize, [f64; 4])> {
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
const ART_MAX_EM: f64 = 4.0;

/// The most paths one word-sized path is compared with (a page with tens of thousands of paths in one
/// band of height is a plot: the first few thousand are looked at).
// ponytail: the first 4096 paths of the band by top decide; a spatial index by x would see them all.
const MAX_ART_NEIGHBOURS: usize = 4096;

/// Does another path of comparable size lie (mostly) on the path `id` with box `rc`? Two paths of one
/// drawing on top of each other, the modules of a QR code. `art` is `furn_art`'s list.
fn overlapped_by_art(art: &[(usize, [f64; 4])], id: usize, rc: [f64; 4], body: f64, share: f64) -> bool {
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
fn add_outlined(items: &mut Vec<It>, words: &[(usize, [f64; 4])], shapes: &[Shape], body: f64, p: &Params) {
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
fn attach_scripts(items: &mut [It], body: f64, p: &Params) {
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

// ---- rows ----

fn make_rows(items: &mut [It], p: &Params) -> Vec<Row> {
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
fn scan(rows: &[Row], i: usize, dir: isize, a: f64, b: f64, wmin: f64, window: f64, max_dist: f64, eps: f64) -> (usize, bool) {
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
fn flush_row_near(rows: &[Row], ri: usize, em: f64, p: &Params) -> bool {
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

fn join_text(items: &[It], idx: &[usize]) -> String {
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

fn make_piece(items: &[It], fonts: &Fonts, row: usize, base: f64, idx: Vec<usize>, pure_share: f64) -> Piece {
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
fn is_marker(f: &It) -> bool {
    let t = f.text.trim();
    !f.opaque && !t.is_empty() && t.chars().count() <= 4 && starts_item(&format!("{t} x"))
}

/// Does some drawn box (a bordered cell, a tag, a panel) hold one of the two rectangles `a` and `b`
/// ([left, top, right, bottom]) and not the other? However tall or wide the box is: a frame that holds
/// both separates nothing, a cell that holds one of them is the boundary between them. A container is
/// a path with some room around the text, not a path the size of the text itself.
fn box_separates(furn: &Furniture, a: [f64; 4], b: [f64; 4], em: f64) -> bool {
    if furn.boxes.is_empty() {
        return false;
    }
    // a box that holds one of the two spans its vertical extent: top above fragment top + pad, bottom under fragment bottom - pad
    furn.boxes.any_spanning(a[3].min(b[3]) - 0.1875 * em, a[1].max(b[1]) + 0.1875 * em, &mut |s| box_holds(s, &a, em) != box_holds(s, &b, em))
}

/// Does the box hold the rectangle `f` ([left, top, right, bottom]): inside it, with some room around it
/// (a path the size of the text itself is no container)?
fn box_holds(s: &BoxR, f: &[f64; 4], em: f64) -> bool {
    let (pad, room) = (0.1875 * em, 0.25 * em);
    f[0] >= s.l - pad && f[2] <= s.r + pad && f[1] >= s.t - pad && f[3] <= s.b + pad && (s.b - s.t) >= (f[3] - f[1]) + room && (s.r - s.l) >= (f[2] - f[0]) + room
}

/// A one-row cell: a drawn box at most `lone_cell_h` em tall that holds both fragments and no other row of
/// the page's text (a header bar, a tag, a one-line cell). One row of a cell cannot be justified text, so
/// a gap of `lone_cell_gap` em in it is a boundary between two cells of the bar, not a word space.
fn lone_row_box(furn: &Furniture, rows: &[Row], ri: usize, a: &It, b: &It, em: f64, p: &Params) -> bool {
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
fn other_row_in(rows: &[Row], ri: usize, s: &BoxR) -> bool {
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
fn box_between(furn: &Furniture, a: &It, b: &It, em: f64) -> bool {
    !(a.opaque || b.opaque) && box_separates(furn, [a.l, a.t, a.r, a.b], [b.l, b.t, b.r, b.b], em)
}

/// A thin vertical rule inside the gap (or just above/below the row): a column border.
fn vrule_in_gap(furn: &Furniture, a: f64, b: f64, base: f64, em: f64) -> bool {
    let tol = 0.0625 * em;
    let lo = furn.vrules.partition_point(|s| s.l < a - tol);
    furn.vrules[lo..].iter().take_while(|s| s.l <= b + tol).any(|s| s.r <= b + tol && s.t < base + 3.0 * em && s.b > base - 3.0 * em)
}

/// A horizontal rule with y in (y0, y1] that covers at least `frac` of the span [l, r].
fn hrule_in(furn: &Furniture, y0: f64, y1: f64, l: f64, r: f64, frac: f64) -> bool {
    let span = (r - l).max(1e-9);
    let lo = furn.hrules.partition_point(|s| s.y <= y0);
    let hi = furn.hrules.partition_point(|s| s.y <= y1);
    furn.hrules[lo..hi.max(lo)].iter().any(|s| s.r.min(r) - s.l.max(l) >= frac * span)
}

/// A row between two horizontal rules (a table row): a rule covering the row at most `band_below` em
/// under its baseline and one at most `band_above` em over it. Such a row is one unit. The first line of
/// a cell of up to `cell_lines` lines (a description that wraps, its numbers beside its first line) sits
/// up to `cell_lines - 1` line pitches (1.2 em) higher over the rule under the cell.
fn ruled_row(items: &[It], row: &Row, furn: &Furniture, p: &Params) -> bool {
    if furn.hrules.is_empty() || row.items.len() < 2 {
        return false;
    }
    let em = row.items.iter().map(|&i| items[i].size).fold(0.0, f64::max);
    let (l, r) = (row.ls[0], row.pmr[row.pmr.len() - 1]);
    let below = p.band_below as f64 + 1.2 * p.cell_lines.saturating_sub(1) as f64;
    hrule_in(furn, row.base + 0.05 * em, row.base + below * em, l, r, 0.8) && hrule_in(furn, row.base - p.band_above as f64 * em, row.base - 0.3 * em, l, r, 0.8)
}

fn cut_rows(items: &[It], rows: &[Row], furn: &Furniture, fonts: &Fonts, p: &Params) -> Vec<Piece> {
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
fn starts_item(text: &str) -> bool {
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
fn dot_kind(text: &str) -> Option<usize> {
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
fn text_left(pc: &Piece, items: &[It]) -> f64 {
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
fn ends_sentence(text: &str) -> bool {
    text.trim_end().trim_end_matches(['"', '\u{201d}', '\u{2019}', ')', ']']).ends_with(['.', '!', '?', '\u{2026}'])
}

/// True when the line ends in a hyphen: the word goes on in the next line, so the line is short for a
/// reason that does not end the paragraph (U+0002 is PDFium's mark for a hyphenation at a line end).
fn ends_hyphen(text: &str) -> bool {
    text.trim_end().ends_with(['-', '\u{2}', '\u{ad}', '\u{2010}', '\u{2011}'])
}

/// Width of the first word of a line, from the fragments' widths (proportional to their characters).
fn first_word_width(items: &[It], pc: &Piece) -> f64 {
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

// ---- links ----

struct Link {
    pred: Vec<Option<usize>>,
    succ: Vec<Option<usize>>,
    /// why a piece without a predecessor starts a chain: "start", "rule" or "column"
    head_why: Vec<&'static str>,
}

/// A horizontal rule between the two lines `q` (above) and `p` (below) that covers most of what the two
/// have in common: the rule of a table or a section, not the underline of a word or a hyperlink. A
/// rule that hugs the glyphs of `q` (within `underline_zone` em under its baseline) must cover nearly
/// all of it: a hyperlink underlines a phrase, a heading underline spans the heading.
fn rule_between(furn: &Furniture, q: &Piece, p: &Piece, params: &Params) -> bool {
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

fn link_pieces(pieces: &[Piece], furn: &Furniture, p: &Params, nrows: usize) -> Link {
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
fn mode_bin(vals: &[f64], bin: f64) -> Option<(f64, usize)> {
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

fn style_differs(a: &Piece, b: &Piece, fonts: &Fonts, p: &Params, body: f64) -> bool {
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
fn looks_differ(a: &Piece, b: &Piece, fonts: &Fonts, p: &Params, body: f64) -> Option<&'static str> {
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

type Run = (Vec<usize>, &'static str);

/// Cut `run` after every position `i` (between `run[i]` and `run[i + 1]`) where `brk` names a reason.
fn split_by(run: Vec<usize>, why: &'static str, mut brk: impl FnMut(usize) -> Option<&'static str>) -> Vec<Run> {
    let mut out: Vec<Run> = Vec::new();
    let mut cur = vec![run[0]];
    let mut why = why;
    for i in 0..run.len() - 1 {
        if let Some(r) = brk(i) {
            out.push((std::mem::take(&mut cur), std::mem::replace(&mut why, r)));
        }
        cur.push(run[i + 1]);
    }
    out.push((cur, why));
    out
}

/// Margins: the left edge moves by more than `margin_shift` em from one stretch of lines to the next
/// AND the right edge moves with it, by at least half as much (both stretches at least two lines long). That is a block quote;
/// text wrapped round a figure moves one edge only, a hanging indent or a first-line indent is one
/// line long. `tl` are the left edges of the lines (past list markers), `r` their right edges.
/// Returns the positions where a new stretch (a block) starts.
fn margin_starts(tl: &[f64], r: &[f64], em: f64, p: &Params) -> Vec<usize> {
    let n = tl.len();
    let tol = 0.5 * em;
    let mut levels: Vec<(usize, usize)> = Vec::new(); // [start, end)
    let mut s = 0;
    for i in 1..n {
        if (tl[i] - tl[i - 1]).abs() > tol {
            levels.push((s, i));
            s = i;
        }
    }
    levels.push((s, n));
    let mut starts = Vec::new();
    for w in levels.windows(2) {
        let (x, y) = (w[0], w[1]);
        if x.1 - x.0 < 2 || y.1 - y.0 < 2 {
            continue;
        }
        let shift = p.margin_shift as f64 * em;
        let right = |lv: (usize, usize)| r[lv.0..lv.1].iter().cloned().fold(f64::MIN, f64::max);
        let left_move = (tl[y.0] - tl[x.0]).abs();
        let right_move = (right(y) - right(x)).abs();
        // both edges move, the right one about as far as the left one (the lines of ragged text end
        // anywhere: the longest of five lines differs by several em from the longest of the next five)
        if left_move > shift && right_move > shift && right_move >= 0.5 * left_move {
            starts.push(y.0);
        }
    }
    starts
}

/// A horizontal rule right under the line (a ruled cell ends here).
fn rule_under(furn: &Furniture, pc: &Piece, p: &Params) -> bool {
    hrule_in(furn, pc.base + 0.05 * pc.size, pc.base + p.band_below as f64 * pc.size, pc.l, pc.r, 0.5)
}

fn segment(items: &[It], pieces: &[Piece], link: &Link, furn: &Furniture, fonts: &Fonts, p: &Params, body: f64) -> Vec<BlockDraft> {
    let mut blocks: Vec<BlockDraft> = Vec::new();
    for head in 0..pieces.len() {
        if link.pred[head].is_some() {
            continue;
        }
        let mut chain = vec![head];
        while let Some(nx) = link.succ[*chain.last().unwrap()] {
            chain.push(nx);
        }
        let n = chain.len();
        let pc = |i: usize| &pieces[chain[i]]; // the piece at chain position i; the runs below hold positions

        // a ruled cell: a short stack of lines with a rule right under it (a label line and its value
        // line between two table rules) is one unit whatever its weights and colours
        let cell = p.ruled_bands && n >= 2 && n <= p.cell_lines && rule_under(furn, pc(n - 1), p);

        // list markers: "1." or "a." only count when the column has another of their kind (a wrapped
        // line that happens to start with "a." is no list); bullets, dashes and bracketed markers
        // count on their own. The continuation lines of an item hang from its text, so `tl` is the
        // left edge of the text proper.
        let mut kinds = [0usize; 3];
        for i in 0..n {
            if let Some(k) = dot_kind(&pc(i).text) {
                kinds[k] += 1;
            }
        }
        let is_item: Vec<bool> = (0..n).map(|i| !pc(i).opaque && starts_item(&pc(i).text) && dot_kind(&pc(i).text).map_or(true, |k| kinds[k] >= 2)).collect();
        let tl: Vec<f64> = (0..n).map(|i| if is_item[i] { text_left(pc(i), items) } else { pc(i).l }).collect();
        let rights: Vec<f64> = (0..n).map(|i| pc(i).r).collect();

        // margins: where a block quote starts or ends. Chain-wide, so that the statistics of the right
        // margin below see the whole column and not a handful of lines
        let margins: Vec<usize> = if p.margin_rule && n >= 4 {
            let mut sizes: Vec<f64> = (0..n).map(|i| pc(i).size).collect();
            sizes.sort_by(|a, b| a.total_cmp(b));
            margin_starts(&tl, &rights, sizes[(n - 1) / 2], p)
        } else {
            Vec::new()
        };

        // stage 1: pairwise facts that need no statistics: size, style, margin
        let runs: Vec<Run> = split_by((0..n).collect(), link.head_why[head], |i| {
            let (a, b) = (pc(i), pc(i + 1));
            let size_changes = a.pure && b.pure && (a.size - b.size).abs() > (p.size_break as f64 * a.size.min(b.size)).max(0.075 * body);
            // a line with no dominant look: judged by the looks it has (nothing in common = a different text)
            let mixed = if p.mixed_looks && !(a.pure && b.pure) { looks_differ(a, b, fonts, p, body) } else { None };
            if p.size_rule && (size_changes || mixed == Some("size")) {
                Some("size")
            } else if p.style_rule && !cell && ((a.pure && b.pure && style_differs(a, b, fonts, p, body)) || mixed == Some("style")) {
                Some("style")
            } else if margins.contains(&(i + 1)) {
                Some("margin")
            } else {
                None
            }
        });

        // stage 2: within a run of one look, the structure: line pitch, list items, indents
        let mut runs2: Vec<Run> = Vec::new();
        for (run, why) in runs {
            if run.len() < 2 {
                runs2.push((run, why));
                continue;
            }
            let em0 = pc(run[0]).size;
            let bin = (p.edge_tol as f64 * em0).max(1e-9);
            // the typical pitch of the run: the median of its line pitches (up to `leading` em: anything
            // wider is a paragraph gap), 1.2 em when it has none; a run of at least four equal pitches
            // is trusted whatever they are up to `wide_leading` em (a double-spaced manuscript)
            let median = |v: &[f64]| match v.len() {
                k if k % 2 == 1 => v[k / 2],
                k => 0.5 * (v[k / 2 - 1] + v[k / 2]),
            };
            let mut all: Vec<f64> = run.windows(2).map(|w| pc(w[1]).base - pc(w[0]).base).filter(|d| *d > 0.5 * em0 && *d <= p.wide_leading as f64 * em0).collect();
            all.sort_by(|a, b| a.total_cmp(b));
            let plain: Vec<f64> = all.iter().copied().filter(|d| *d < p.leading as f64 * em0).collect();
            let typ_pitch = if all.len() >= 3 && all[all.len() - 1] - all[0] <= 0.3 * em0 {
                median(&all)
            } else if plain.is_empty() {
                1.2 * em0
            } else {
                // (the upper middle one when the count is even: the first pitch of a run is often the
                // taller line box of a mark or a larger first word)
                plain[plain.len() / 2]
            };
            let lefts: Vec<f64> = run.iter().map(|&i| tl[i]).collect();
            let (ml, ml_count) = mode_bin(&lefts, bin).unwrap_or((lefts[0], 1));
            let flush_edge = ml_count >= p.edge_lines && ml_count as f64 >= p.edge_dominance as f64 * run.len() as f64;
            // how many lines of the run sit on the left edge `x` (within a bin)
            let mut sorted_lefts = lefts.clone();
            sorted_lefts.sort_by(|a, b| a.total_cmp(b));
            let on_edge = |x: f64| sorted_lefts.partition_point(|&v| v <= x + bin) - sorted_lefts.partition_point(|&v| v < x - bin);
            let entries = p.edge_lines.saturating_sub(1); // (three entries make a reference list)
            let len = run.len();
            runs2.extend(split_by(run.clone(), why, |i| {
                let (a, b) = (pc(run[i]), pc(run[i + 1]));
                let em = b.size.max(a.size);
                if p.pitch_rule && (b.base - a.base) - typ_pitch > p.pitch_extra as f64 * em {
                    Some("pitch")
                } else if p.list_items && (is_item[run[i + 1]] || b.lead_script) {
                    Some("item")
                } else if p.indent_rule
                    && flush_edge
                    && tl[run[i + 1]] - ml > p.indent as f64 * em
                    && (tl[run[i]] - ml).abs() <= 0.5 * em
                    && i + 2 < len
                    && (tl[run[i + 2]] - ml).abs() <= 0.5 * em
                {
                    Some("indent")
                } else if p.hanging_items
                    && i + 2 < len
                    && tl[run[i]] - tl[run[i + 1]] > p.indent as f64 * em
                    && (tl[run[i + 2]] - tl[run[i]]).abs() <= 0.5 * em
                    && on_edge(tl[run[i + 1]]) >= entries
                    && 5 * (on_edge(tl[run[i + 1]]) + on_edge(tl[run[i]])) >= 4 * len
                {
                    // a flush line between two lines that hang in as far as each other, in a run whose
                    // lines sit on these two left edges (four in five): an entry starts. Ragged-left
                    // text has many left edges, a few of them shared by chance.
                    Some("item")
                } else {
                    None
                }
            }));
        }

        // stage 3: short lines, judged against the right margin of the column the run sits in: the lines
        // of the stretch between two margin moves that are not much wider than the run's own and of about
        // its size (a heading, a title or a paragraph across two columns says nothing about the margin of
        // one column); a run is justified when half of its lines reach that margin
        let mut seg_bounds: Vec<(usize, usize)> = Vec::new();
        let mut s = 0;
        for &m in &margins {
            seg_bounds.push((s, m));
            s = m;
        }
        seg_bounds.push((s, n));
        for (run, why) in runs2 {
            let (first, last) = (run[0], run[run.len() - 1]);
            let seg = seg_bounds.iter().position(|&(lo, hi)| first >= lo && first < hi).unwrap_or(0);
            let (lo, hi) = (seg_bounds[seg].0.max(first.saturating_sub(1000)), seg_bounds[seg].1.min(last + 1001));
            let mut widths: Vec<f64> = run.iter().map(|&i| pc(i).r - pc(i).l).collect();
            widths.sort_by(|a, b| a.total_cmp(b));
            let wide = 1.3 * widths[(widths.len() - 1) / 2];
            let mut sizes: Vec<f64> = run.iter().map(|&i| pc(i).size).collect();
            sizes.sort_by(|a, b| a.total_cmp(b));
            let size = sizes[(sizes.len() - 1) / 2];
            let bin = (p.edge_tol as f64 * size).max(1e-9);
            // (lines of another size, a title over an abstract, do not tell where this text ends)
            let col: Vec<f64> = (lo..hi).filter(|&i| (first..=last).contains(&i) || (pc(i).r - pc(i).l <= wide && (pc(i).size - size).abs() <= 0.1 * size)).map(|i| rights[i]).collect();
            // the right margin: the lines that end within a bin of the longest line of the column sit on
            // it, and it is where the shortest of them ends
            let max_r = col.iter().cloned().fold(f64::MIN, f64::max);
            let flush: Vec<f64> = col.iter().cloned().filter(|&r| r >= max_r - bin).collect();
            let mr = flush.iter().cloned().fold(max_r, f64::min);
            // justified: half of the run's lines (the last one counts: a paragraph that flows into the
            // next column ends on a full line) sit on the margin, and the margin is proved by enough
            // lines (`edge_lines` of the run or `justify_evidence` of the column): ragged lines end
            // together by chance, the longest line always sits on the margin
            let reaching = run.iter().filter(|&&i| rights[i] >= max_r - bin).count();
            let justified = run.len() >= 3 && reaching * 2 >= run.len() && (reaching >= p.edge_lines || flush.len() >= p.justify_evidence);
            for (r2, why2) in split_by(run.clone(), why, |i| {
                let (a, b) = (pc(run[i]), pc(run[i + 1]));
                let em = b.size.max(a.size);
                let slack = mr - a.r;
                if ends_hyphen(&a.text) || !p.short_lines {
                    None
                } else if justified && slack > p.short_slack as f64 * em {
                    Some("short-line")
                } else if !justified && ends_sentence(&a.text) && slack > first_word_width(items, b) + p.short_margin as f64 * em {
                    Some("short-line")
                } else if !justified
                    && !a.opaque
                    && (1..=p.tiny_line).contains(&nonspace(&a.text))
                    && a.text.chars().any(|c| c.is_ascii_digit())
                    && nonspace(&b.text) > p.tiny_line
                    && slack > 2.0 * first_word_width(items, b) + p.short_margin as f64 * em
                {
                    // a number of a few characters over a long line (a tick value, an amount): the next word would
                    // have fitted on it twice over. (A short WORD may end a line the author broke on purpose, inside a
                    // table cell: "LED" over "DRIVER (8 LIGHTS ...)" is one description.)
                    Some("short-line")
                } else {
                    None
                }
            }) {
                blocks.push(draft(items, pieces, r2.iter().map(|&i| chain[i]).collect(), why2));
            }
        }
    }
    blocks
}

fn draft(items: &[It], pieces: &[Piece], run: Vec<usize>, why: &'static str) -> BlockDraft {
    let lines = run
        .iter()
        .map(|&pi| {
            let pc = &pieces[pi];
            let mut ld = LineDraft { members: Vec::new(), outlined: Vec::new(), rect: [f64::MAX, f64::MAX, f64::MIN, f64::MIN], baseline: pc.base };
            for &k in &pc.items {
                let f = &items[k];
                if f.opaque {
                    ld.outlined.push((f.l, f.obj));
                } else {
                    ld.members.push((f.l, f.obj));
                }
                ld.rect = [ld.rect[0].min(f.l), ld.rect[1].min(f.t), ld.rect[2].max(f.r), ld.rect[3].max(f.b)];
            }
            ld
        })
        .collect();
    BlockDraft { lines, why }
}

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
