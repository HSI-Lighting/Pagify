//! INDEPENDENT SYNTHETIC ORACLE CORPUS for `pagify_shell::blocks::detect` (written by agent R, finished by agent R2).
//!
//! WHY THIS FILE EXISTS
//!   The detector was tuned against one real datasheet. This corpus judges the port by layouts it was
//!   never tuned on. Every expected grouping below is what a person looking at the layout would call
//!   one editable text block (the user's red marking is the model: a whole multi-line paragraph is one
//!   block, a heading on its own line is its own block, an underlined label/value row is one block).
//!   NO expectation was derived from any algorithm's output, and none of the detector's source was read
//!   (only the public items of blocks.rs: the types and signatures).
//!
//! HOW IT IS BUILT
//!   * A small DSL (`Page`, `P`, `Para`) lays out lines of fragments with datasheet-realistic geometry:
//!     justified word gaps up to 1.2 em, 3-8 text objects per visual line cut at word gaps AND mid-word,
//!     tiny kerning gaps at mid-word cuts, glyph-dependent top/bottom (ascenders/descenders), trailing and
//!     leading spaces inside objects, one-letter objects, U+0002 hyphenation markers, outlined (path) words.
//!   * Each case is MUST (a person would not disagree) or AMBIGUOUS (reasonable people differ: nothing is
//!     asserted, the detector's answer is recorded and compared with the named competing readings).
//!   * Every MUST case runs 6 variants: scale x1, x0.5, x2, x3.7 (all coordinates and sizes), and the
//!     fragment/shape slice order shuffled with two deterministic seeds. Expectations must hold for all.
//!   * Every case (MUST and AMBIGUOUS) also gets the CONTRACT invariants checked: each non-rotated Frag in
//!     exactly one block, each rotated Frag alone in a one-line block, block/line boxes enclose members,
//!     lines top to bottom, objects left to right, `contains`/`block_index_of` agree with `objects()`, no
//!     panic, and `detect` is deterministic (two calls on the same input give the same blocks).
//!   * OTHER SEEDS (added by R2, `PAGIFY_BLOCKS_SEEDS`, default 24): the numbers of a layout (random words,
//!     object cuts, justified gaps) come from a seed derived from the case name. A detector can be tuned on
//!     those exact numbers, so every MUST case is ALSO instantiated with other seeds (same layout, other
//!     numbers) and the table says on how many of them it holds. The perfect oracle must stay perfect on them,
//!     and the generator checks its own justified lines (flush, gaps in range) on every seed.
//!   * AMBIGUOUS cases are run in all 6 variants too: nothing is asserted about the answer, but the table
//!     shows whether it is the same at every scale and slice order (`=` / `DIFF`).
//!   * `oracle_self_check_standins_mutants_and_perfect` proves the checker can fail AND pass: a perfect detector
//!     built from the expectations passes everything (also on other seeds), three mutants of it (split / merge /
//!     one fragment off) are caught in every applicable case, four contract breakers (a missing object, a block
//!     reported twice, a flaky answer, a box that does not enclose its members) are flagged in every applicable
//!     case, and the trivial stand-ins (one block per fragment, everything in one block) fail >= 75 % of the MUST
//!     cases.
//!
//! RUNNING
//!   cargo test -p pagify_shell --release --test blocks_synthetic -- --nocapture
//!   (the table is also written straight to stderr, so it shows even without --nocapture, and to
//!    `$CARGO_TARGET_TMPDIR/blocks_synthetic_report.txt`)
//!
//!   Default run: prints the per-case table, asserts nothing about the detector (the repository stays
//!   green until the detector is final). The corpus self-checks (`corpus_is_internally_consistent`,
//!   `oracle_self_check_*`) always assert: they test the TEST, not the detector.
//!
//!   Environment variables:
//!     PAGIFY_BLOCKS_ASSERT=1       MUST failures (canonical instance AND other seeds) and contract-invariant
//!                                  failures fail the test
//!     PAGIFY_BLOCKS_SKIP_HEAVY=1   skip the 30,000-fragment case
//!     PAGIFY_BLOCKS_HEAVY_SECS=N   time bound of the heavy case (default 60 s, release build)
//!     PAGIFY_BLOCKS_ONLY=text      run only the cases whose name contains `text`
//!     PAGIFY_BLOCKS_VERBOSE=1      print the problems of every failing variant, not only the first
//!     PAGIFY_BLOCKS_SVG=dir        write one SVG per case (detected blocks coloured, expected dashed)
//!     PAGIFY_BLOCKS_SEEDS=N        other seeds per case (default 24, 0 = off; 100 for a thorough run). With
//!                                  ASSERT=1 and SEEDS=0 only the canonical instance of every case is enforced.
//!     PAGIFY_BLOCKS_SEED_BASE=b    HOLDOUT: use the seeds b+1 ..= b+SEEDS instead of 1 ..= SEEDS (the failing seeds
//!                                  of the default ranges are printed in every report, so pick a base nobody has seen)
//!     PAGIFY_BLOCKS_SALT=k         run the whole table on seed k instead of the canonical instance (0): this
//!                                  reproduces, prints and (with _SVG) draws a failing seed of the seeds section
//!     PAGIFY_BLOCKS_SWEEP=1        also run every MUST case at 12 scales (0.2 .. 12) and list the cases whose
//!                                  verdict changes with the scale (a threshold written in points, not em)
//!     PAGIFY_BLOCKS_CATALOGUE=file write the case catalogue (name, strength, intent, readings) to `file`

#![allow(
    dead_code,
    clippy::too_many_arguments,
    clippy::type_complexity,
    clippy::needless_range_loop,
    clippy::manual_range_contains
)]

use pagify_shell::blocks::{block_index_of, detect, Block, Frag, Line, Shape};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::Write as _;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::time::Instant;

// =====================================================================================================
// 0. small utilities
// =====================================================================================================

/// Straight to the stderr handle: libtest only captures the print macros, so the table is always visible.
fn emit(s: &str) {
    let mut e = std::io::stderr().lock();
    let _ = e.write_all(s.as_bytes());
    let _ = e.flush();
}

fn env_flag(name: &str) -> bool {
    std::env::var(name).map(|v| !(v.is_empty() || v == "0")).unwrap_or(false)
}

fn env_str(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

fn fnv(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Deterministic xorshift64* (no external crates, identical on every machine).
#[derive(Clone)]
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        let mut s = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03;
        if s == 0 {
            s = 0x1234_5678_9ABC_DEF1;
        }
        let mut r = Rng(s);
        for _ in 0..4 {
            r.next();
        }
        r
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / 16_777_216.0
    }
    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit()
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
    fn chance(&mut self, p: f32) -> bool {
        self.unit() < p
    }
}

fn shuffle<T>(v: &mut [T], rng: &mut Rng) {
    for i in (1..v.len()).rev() {
        let j = rng.below(i + 1);
        v.swap(i, j);
    }
}

// =====================================================================================================
// 1. text metrics (proportional, Montserrat-ish) and styles
// =====================================================================================================

/// Advance width in em.
fn adv(c: char) -> f32 {
    match c {
        ' ' => 0.27,
        'i' | 'l' | 'j' | '.' | ',' | ':' | ';' | '!' | '|' | '\'' => 0.26,
        'f' | 't' | 'r' | 'I' | '-' | '\u{2}' | '(' | ')' | '/' => 0.36,
        'm' | 'w' => 0.86,
        'M' | 'W' => 0.94,
        c if c.is_ascii_uppercase() => 0.68,
        c if c.is_ascii_digit() => 0.60,
        _ => 0.57,
    }
}

/// Ink width of a string (leading/trailing spaces excluded).
fn ink_w(s: &str, size: f32) -> f32 {
    s.trim().chars().map(adv).sum::<f32>() * size
}

fn tall(c: char) -> bool {
    c.is_ascii_uppercase() || c.is_ascii_digit() || "bdfhklt\u{2022}\u{b0}*\u{b9}\u{b2}\u{b3}".contains(c)
}

fn deep(c: char) -> bool {
    "gjpqy,;()".contains(c)
}

/// (top, bottom) of the glyph ink of `text` on `base` (y grows downward): ascender/descender dependent,
/// like the real datasheet objects (top = base - 0.74 em with capitals/ascenders, 0.55 em otherwise).
fn vext(text: &str, size: f32, base: f32) -> (f32, f32) {
    let t = text.chars().any(tall);
    let d = text.chars().any(deep);
    (base - size * if t { 0.74 } else { 0.55 }, base + size * if d { 0.20 } else { 0.0 })
}

#[derive(Clone, Debug)]
struct St {
    font: u32,
    stem: Option<u16>,
    face: String,
    rgb: [u8; 3],
    size: f32,
}

impl St {
    fn new(font: u32, stem: Option<u16>, face: &str, size: f32) -> St {
        St { font, stem, face: face.to_string(), rgb: [0x6d, 0x6e, 0x70], size }
    }
    fn size(mut self, s: f32) -> St {
        self.size = s;
        self
    }
    fn rgb(mut self, c: [u8; 3]) -> St {
        self.rgb = c;
        self
    }
    fn font(mut self, f: u32) -> St {
        self.font = f;
        self
    }
    fn stem(mut self, s: Option<u16>) -> St {
        self.stem = s;
        self
    }
    fn face(mut self, f: &str) -> St {
        self.face = f.to_string();
        self
    }
}

// Datasheet page 1 styles: five weights, ONE useless face name; font ids and stems are the only signal.
fn body(size: f32) -> St {
    St::new(1, Some(51), "Montserrat-Thin", size) // TT1 Light
}
fn head(size: f32) -> St {
    St::new(0, Some(198), "Montserrat-Thin", size) // TT0 ExtraBold
}
fn medium(size: f32) -> St {
    St::new(5, Some(100), "Montserrat-Thin", size) // TT5 Medium
}
fn semibold(size: f32) -> St {
    St::new(4, Some(124), "Montserrat-Thin", size) // TT4 SemiBold
}
// A word-processor style document: honest, distinct face names.
fn doc(size: f32) -> St {
    St::new(20, Some(74), "Calibri", size).rgb([0, 0, 0])
}
fn doc_bold(size: f32) -> St {
    St::new(21, Some(130), "Calibri-Bold", size).rgb([0, 0, 0])
}
fn doc_italic(size: f32) -> St {
    St::new(22, Some(68), "Calibri-Italic", size).rgb([0, 0, 0])
}

// =====================================================================================================
// 2. the page DSL
// =====================================================================================================

const VOCAB: &[&str] = &[
    "the", "light", "source", "is", "produced", "by", "a", "compact", "module", "with", "high", "efficacy", "and",
    "long", "life", "The", "luminaire", "delivers", "uniform", "illumination", "across", "whole", "area", "of", "each",
    "room", "while", "keeping", "glare", "low", "driver", "reflector", "aluminium", "body", "finish", "colour",
    "temperature", "dimming", "control", "options", "are", "available", "for", "most", "projects", "where", "energy",
    "savings", "matter", "Installation", "requires", "no", "tools", "because", "spring", "clips", "hold", "fitting",
    "firmly", "in", "place", "Optical", "quality", "depends", "on", "lens", "design", "material", "selection",
    "Thermal", "management", "keeps", "junction", "below", "limit", "so", "output", "stays", "stable", "over", "time",
    "Tested", "according", "to", "international", "standards", "these", "products", "meet", "all", "safety",
    "requirements", "recommended", "ceiling", "recessed", "surface", "mounted", "suspended", "outdoor", "indoor",
    "wall", "washer", "linear", "spot", "flood", "medium", "narrow", "wide", "beam", "angle", "lumen", "watt",
    "kelvin", "rendering", "index", "ra", "IP44", "180", "50000", "hours", "LED", "COB", "CRI", "CCT", "it", "an",
    "as", "at", "be", "or", "up", "if", "we", "can", "has", "was", "not", "but", "from", "that", "this", "then",
    "also", "into", "less", "more", "even", "such", "each", "only", "when", "your", "have", "will", "their",
];

const LONG_WORDS: &[&str] = &[
    "manufacturers", "telecommunications", "electromagnetic", "characterisation", "specification", "approximately",
    "distribution", "photometric", "integrated", "recommended", "performance", "temperature", "installation",
    "environment", "illumination", "architectural", "luminaires", "components",
];

fn rand_word(rng: &mut Rng) -> String {
    let mut w = VOCAB[rng.below(VOCAB.len())].to_string();
    if rng.chance(0.08) {
        w.push(',');
    } else if rng.chance(0.04) {
        w.push('.');
    }
    w
}

fn split_long_word(rng: &mut Rng) -> (String, String) {
    let w = LONG_WORDS[rng.below(LONG_WORDS.len())];
    let n = w.len();
    let k = 3 + rng.below(n - 5);
    (w[..k].to_string(), w[k..].to_string())
}

fn capitalise(w: &str) -> String {
    let mut c = w.chars();
    match c.next() {
        Some(f) => f.to_ascii_uppercase().to_string() + c.as_str(),
        None => String::new(),
    }
}

/// What a line needs besides its geometry. Text flows like real text: the word that did not fit on a line is the
/// first word of the next one (`first`), a paragraph starts with a capital and its last line ends with a full stop.
struct FillSpec {
    justify: bool,
    /// carried over from the previous line: an overflow word, a hyphenation remainder or a list number
    first: Option<String>,
    /// a hyphenated piece (marker included) that closes the line
    last: Option<String>,
    /// capitalise the first freshly drawn word (start of a paragraph)
    cap_first: bool,
    /// the last word of the line ends with a full stop (end of a paragraph)
    end_period: bool,
}

/// Words that fill `avail` points; returns (words, gap, overflow word). Justified lines retry until the
/// stretched gap is within [gap_lo, gap_hi].
fn fill(rng: &mut Rng, size: f32, avail: f32, gap_lo: f32, gap_hi: f32, spec: &FillSpec) -> (Vec<String>, f32, Option<String>) {
    let last_w = spec.last.as_ref().map(|w| ink_w(w, size)).unwrap_or(0.0);
    let period_w = if spec.end_period && spec.last.is_none() { adv('.') * size } else { 0.0 };
    let tries = if spec.justify { 400 } else { 1 };
    let mut best: Option<(Vec<String>, f32, Option<String>)> = None;
    for _ in 0..tries {
        let mut ws: Vec<String> = spec.first.iter().cloned().collect();
        let mut sum: f32 = ws.iter().map(|w| ink_w(w, size)).sum();
        let mut drawn = 0usize;
        let mut redraws = 0;
        let mut overflow: Option<String> = None;
        loop {
            if ws.len() >= 40 {
                break;
            }
            let raw = rand_word(rng);
            let w = if drawn == 0 && spec.cap_first { capitalise(&raw) } else { raw.clone() };
            let ww = ink_w(&w, size);
            if ww > avail && redraws < 200 {
                // never draw a word wider than the whole line (very narrow columns)
                redraws += 1;
                continue;
            }
            let n_after = ws.len() + 1 + usize::from(spec.last.is_some());
            let need = sum + ww + last_w + period_w + (n_after as f32 - 1.0) * gap_lo;
            if need > avail {
                if ws.is_empty() && redraws < 200 {
                    redraws += 1;
                    continue;
                }
                overflow = Some(raw);
                break;
            }
            sum += ww;
            ws.push(w);
            drawn += 1;
        }
        if let Some(l) = &spec.last {
            ws.push(l.clone());
            sum += last_w;
            overflow = None;
        } else if spec.end_period {
            overflow = None;
            if let Some(lw) = ws.last_mut() {
                if lw.ends_with(',') {
                    lw.pop();
                }
                if !lw.ends_with('.') {
                    lw.push('.');
                }
                sum = ws.iter().map(|w| ink_w(w, size)).sum();
            }
        }
        if ws.is_empty() {
            ws.push("of".to_string());
            sum += ink_w("of", size);
        }
        let n = ws.len();
        let gap = if spec.justify && n >= 2 { (avail - sum) / (n as f32 - 1.0) } else { gap_lo };
        let ok = !spec.justify || n < 2 || (gap <= gap_hi && gap >= gap_lo - 1e-3);
        if ok {
            return (ws, gap, overflow);
        }
        best = Some((ws, gap, overflow));
    }
    best.unwrap()
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Align {
    Justify,
    Left,
    Centre,
    Right,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Hole {
    Inside,
    Start,
    End,
}

/// Number of letters of a word (punctuation and digits do not count).
fn letters(w: &str) -> usize {
    w.chars().filter(|c| c.is_alphabetic()).count()
}

/// Index of the word that becomes the outlined hole: the first / the last word, or for `Inside` the word of at
/// least 5 letters that is nearest to the middle of the line (None when no such word exists).
fn hole_index(words: &[String], h: Hole) -> Option<usize> {
    match h {
        Hole::Start => Some(0),
        Hole::End => words.len().checked_sub(1),
        Hole::Inside => {
            let mid = words.len() / 2;
            (1..words.len().saturating_sub(1)).filter(|&k| letters(&words[k]) >= 5).min_by_key(|&k| k.abs_diff(mid))
        }
    }
}

/// Paragraph specification (builder).
#[derive(Clone)]
struct P {
    id: String,
    st: St,
    x: f32,
    base: f32,
    w: f32,
    pitch: f32,
    n: usize,
    align: Align,
    gap: (f32, f32),
    frags: (usize, usize),
    indent: f32,
    hang: f32,
    last: Option<f32>,
    hyph: Vec<usize>,
    hyph_ch: char,
    holes: Vec<(usize, Hole)>,
    outlined_lines: Vec<usize>,
    lead: Option<String>,
    /// the last line does NOT end with a full stop (the sentence goes on in another P)
    no_period: bool,
    /// the first word is NOT capitalised (this P continues a sentence)
    no_cap: bool,
    /// one text object per word (OCR text layers, word-by-word exports); the object boxes hug the words
    word_objects: bool,
}

impl P {
    fn word_objects(mut self) -> P {
        self.word_objects = true;
        self
    }
    fn no_period(mut self) -> P {
        self.no_period = true;
        self
    }
    fn no_cap(mut self) -> P {
        self.no_cap = true;
        self
    }
    fn new(id: &str, st: &St) -> P {
        P {
            id: id.to_string(),
            st: st.clone(),
            x: 0.0,
            base: 0.0,
            w: 100.0,
            pitch: st.size * 1.2,
            n: 1,
            align: Align::Left,
            gap: (0.30, 1.20),
            frags: (3, 6),
            indent: 0.0,
            hang: 0.0,
            last: None,
            hyph: vec![],
            hyph_ch: '\u{2}',
            holes: vec![],
            outlined_lines: vec![],
            lead: None,
            no_period: false,
            no_cap: false,
            word_objects: false,
        }
    }
    /// The first word of the paragraph is fixed (a list number such as "1.", a run-in "Note:").
    fn lead(mut self, w: &str) -> P {
        self.lead = Some(w.to_string());
        self
    }
    fn at(mut self, x: f32, base: f32) -> P {
        self.x = x;
        self.base = base;
        self
    }
    fn w(mut self, w: f32) -> P {
        self.w = w;
        self
    }
    fn n(mut self, n: usize) -> P {
        self.n = n;
        self
    }
    fn pitch(mut self, p: f32) -> P {
        self.pitch = p;
        self
    }
    fn just(mut self) -> P {
        self.align = Align::Justify;
        self
    }
    fn left(mut self) -> P {
        self.align = Align::Left;
        self
    }
    fn centre(mut self) -> P {
        self.align = Align::Centre;
        self
    }
    fn right(mut self) -> P {
        self.align = Align::Right;
        self
    }
    /// Justified word-gap range in em (the visible gap between two words, natural space included).
    fn gap(mut self, lo: f32, hi: f32) -> P {
        self.gap = (lo, hi);
        self
    }
    fn frags(mut self, lo: usize, hi: usize) -> P {
        self.frags = (lo, hi);
        self
    }
    fn indent(mut self, d: f32) -> P {
        self.indent = d;
        self
    }
    fn hang(mut self, d: f32) -> P {
        self.hang = d;
        self
    }
    /// The last line is only `frac` of the available width (ragged, never stretched).
    fn last(mut self, frac: f32) -> P {
        self.last = Some(frac);
        self
    }
    fn hyph(mut self, lines: &[usize], ch: char) -> P {
        self.hyph = lines.to_vec();
        self.hyph_ch = ch;
        self
    }
    /// Line `line` carries one word as a vector OUTLINE (a path object) instead of text.
    fn hole(mut self, line: usize, h: Hole) -> P {
        self.holes.push((line, h));
        self
    }
    /// Line `line` is ONE path object as wide as the line and has no text object at all.
    fn outlined_line(mut self, line: usize) -> P {
        self.outlined_lines.push(line);
        self
    }
}

/// What `Page::para` built: labels per line (text objects only) and the outlined shapes.
#[derive(Clone)]
struct Para {
    lines: Vec<Vec<String>>,
    bases: Vec<f32>,
    outlined: Vec<(usize, String)>,
    x: f32,
    w: f32,
    pitch: f32,
}

impl Para {
    fn all(&self) -> Vec<String> {
        self.lines.iter().flatten().cloned().collect()
    }
    fn lines(&self, a: usize, b: usize) -> Vec<String> {
        self.lines[a..b].iter().flatten().cloned().collect()
    }
    fn line(&self, i: usize) -> Vec<String> {
        self.lines[i].clone()
    }
    fn first(&self) -> String {
        self.lines.iter().flatten().next().cloned().expect("paragraph without text")
    }
    fn first_base(&self) -> f32 {
        self.bases[0]
    }
    fn last_base(&self) -> f32 {
        *self.bases.last().unwrap()
    }
    /// Baseline of the next line below, `lines` pitches after the last baseline.
    fn below(&self, lines: f32) -> f32 {
        self.last_base() + lines * self.pitch
    }
    fn right_edge(&self) -> f32 {
        self.x + self.w
    }
    fn shapes(&self) -> Vec<String> {
        self.outlined.iter().map(|(_, l)| l.clone()).collect()
    }
}

#[derive(Clone, Copy, PartialEq)]
enum K {
    Letter,
    Space,
    Outline,
}

struct G {
    c: char,
    x0: f32,
    x1: f32,
    k: K,
}

struct Page {
    frags: Vec<Frag>,
    shapes: Vec<Shape>,
    /// Label of `frags[i]` / `shapes[i]` (same order as the vectors).
    frag_labels: Vec<String>,
    shape_labels: Vec<String>,
    ids: HashMap<String, usize>,
    names: HashMap<usize, String>,
    /// Baseline of the line an outlined word sits on (by object id).
    shape_base: HashMap<usize, f32>,
    next: usize,
    rng: Rng,
    /// Things the generator itself noticed (a justified line that is not flush, a gap outside the requested range):
    /// reported by `Case::consistency`, so a layout that is not what its intent says can never become a verdict.
    warnings: Vec<String>,
}

impl Page {
    fn new(seed: u64) -> Page {
        Page {
            warnings: vec![],
            frags: vec![],
            shapes: vec![],
            frag_labels: vec![],
            shape_labels: vec![],
            ids: HashMap::new(),
            names: HashMap::new(),
            shape_base: HashMap::new(),
            next: 0,
            rng: Rng::new(seed),
        }
    }

    fn alloc(&mut self, label: &str) -> usize {
        assert!(!self.ids.contains_key(label), "DSL: duplicate label {label}");
        let o = self.next;
        self.next += 1;
        self.ids.insert(label.to_string(), o);
        self.names.insert(o, label.to_string());
        o
    }

    /// Burn `n` object ids (unrelated page objects between the ones we care about).
    fn skip_ids(&mut self, n: usize) {
        self.next += n;
    }

    fn id(&self, label: &str) -> usize {
        *self.ids.get(label).unwrap_or_else(|| panic!("DSL: unknown label {label}"))
    }

    fn name_of(&self, object: usize) -> String {
        self.names.get(&object).cloned().unwrap_or_else(|| format!("#{object}"))
    }

    fn add_frag_rect(
        &mut self,
        label: &str,
        st: &St,
        text: &str,
        l: f32,
        t: f32,
        r: f32,
        b: f32,
        base: f32,
        rotated: bool,
    ) -> String {
        let object = self.alloc(label);
        self.frag_labels.push(label.to_string());
        self.frags.push(Frag {
            object,
            left: l,
            top: t,
            right: r,
            bottom: b,
            baseline: base,
            size: st.size,
            font: st.font,
            stem: st.stem,
            face: st.face.clone(),
            rgb: st.rgb,
            text: text.to_string(),
            rotated,
        });
        label.to_string()
    }

    fn add_frag(&mut self, label: &str, st: &St, text: &str, l: f32, r: f32, base: f32) -> String {
        let (t, b) = vext(text, st.size, base);
        self.add_frag_rect(label, st, text, l, t, r, b, base, false)
    }

    fn add_shape(&mut self, label: &str, l: f32, t: f32, r: f32, b: f32) -> String {
        self.add_shape_depth(label, l, t, r, b, 0)
    }

    fn add_shape_depth(&mut self, label: &str, l: f32, t: f32, r: f32, b: f32, depth: u32) -> String {
        let object = self.alloc(label);
        self.shape_labels.push(label.to_string());
        self.shapes.push(Shape { object, left: l, top: t, right: r, bottom: b, depth });
        label.to_string()
    }

    // ---- single objects -------------------------------------------------------------------------

    /// One text object starting at `x`.
    fn text(&mut self, label: &str, st: &St, x: f32, base: f32, text: &str) -> String {
        let w = ink_w(text, st.size);
        self.add_frag(label, st, text, x, x + w, base)
    }

    fn text_right(&mut self, label: &str, st: &St, right: f32, base: f32, text: &str) -> String {
        let w = ink_w(text, st.size);
        self.add_frag(label, st, text, right - w, right, base)
    }

    fn text_centre(&mut self, label: &str, st: &St, cx: f32, base: f32, text: &str) -> String {
        let w = ink_w(text, st.size);
        self.add_frag(label, st, text, cx - w / 2.0, cx + w / 2.0, base)
    }

    /// A single line of `text` (natural word gaps) cut into about `nfrag` objects. Labels `id.0 ...`.
    fn cut_line(&mut self, id: &str, st: &St, x: f32, base: f32, text: &str, nfrag: usize) -> Vec<String> {
        let words: Vec<String> = text.split(' ').map(|w| w.to_string()).collect();
        let gap = adv(' ') * st.size;
        self.put_line(id, st, x, base, &words, gap, nfrag, None, false).0
    }

    /// A line of single-letter objects (letter-spaced heading). `track` and `word_gap` in em.
    fn tracked(&mut self, id: &str, st: &St, x: f32, base: f32, text: &str, track: f32, word_gap: f32) -> Vec<String> {
        let mut labels = vec![];
        let mut cur = x;
        let mut idx = 0;
        for c in text.chars() {
            if c == ' ' {
                cur += word_gap * st.size;
                continue;
            }
            let w = adv(c) * st.size;
            let lab = format!("{id}.{idx}");
            idx += 1;
            self.add_frag(&lab, st, &c.to_string(), cur, cur + w, base);
            labels.push(lab);
            cur += w + track * st.size;
        }
        labels
    }

    // ---- shapes -----------------------------------------------------------------------------------

    fn rule_h(&mut self, label: &str, x0: f32, x1: f32, y: f32) -> String {
        self.add_shape(label, x0, y - 0.25, x1, y + 0.25)
    }

    fn rule_v(&mut self, label: &str, x: f32, y0: f32, y1: f32) -> String {
        self.add_shape(label, x - 0.25, y0, x + 0.25, y1)
    }

    fn rect(&mut self, label: &str, l: f32, t: f32, r: f32, b: f32) -> String {
        self.add_shape(label, l, t, r, b)
    }

    /// A box drawn as four thin rules: labels `label.t/.b/.l/.r`.
    fn box_four(&mut self, label: &str, l: f32, t: f32, r: f32, b: f32) -> Vec<String> {
        vec![
            self.rule_h(&format!("{label}.t"), l, r, t),
            self.rule_h(&format!("{label}.b"), l, r, b),
            self.rule_v(&format!("{label}.l"), l, t, b),
            self.rule_v(&format!("{label}.r"), r, t, b),
        ]
    }

    /// A rotated text object (never grouped): axis-aligned box of a 90-degree string reading upward.
    fn rotated_up(&mut self, label: &str, st: &St, x: f32, y_bottom: f32, text: &str) -> String {
        let len = ink_w(text, st.size);
        self.add_frag_rect(
            label,
            st,
            text,
            x - st.size * 0.74,
            y_bottom - len,
            x + st.size * 0.20,
            y_bottom,
            y_bottom,
            true,
        )
    }

    /// A rotated object with an explicit bounding box (e.g. a 45-degree watermark).
    fn rotated_box(&mut self, label: &str, st: &St, text: &str, l: f32, t: f32, r: f32, b: f32) -> String {
        self.add_frag_rect(label, st, text, l, t, r, b, b, true)
    }

    // ---- lines and paragraphs ---------------------------------------------------------------------

    /// Lay out one visual line. Words are `gap` apart. About `nfrag` text objects are cut from it, about
    /// 65 % of the cuts at a word gap (space goes to either side), the rest mid-word (with a tiny kerning
    /// gap). `hole` turns that word into an outlined path object. Returns (labels, outlined label, right x).
    fn put_line(
        &mut self,
        id: &str,
        st: &St,
        x: f32,
        base: f32,
        words: &[String],
        gap: f32,
        nfrag: usize,
        hole: Option<usize>,
        words_only: bool,
    ) -> (Vec<String>, Option<String>, f32) {
        let size = st.size;
        let mut gl: Vec<G> = Vec::new();
        let mut cur = x;
        for (wi, w) in words.iter().enumerate() {
            if wi > 0 {
                gl.push(G { c: ' ', x0: cur, x1: cur + gap, k: K::Space });
                cur += gap;
            }
            if Some(wi) == hole {
                let wd = ink_w(w, size);
                gl.push(G { c: '\u{fffc}', x0: cur, x1: cur + wd, k: K::Outline });
                cur += wd;
            } else {
                for c in w.chars() {
                    let wd = adv(c) * size;
                    gl.push(G { c, x0: cur, x1: cur + wd, k: K::Letter });
                    cur += wd;
                }
            }
        }
        let n = gl.len();
        let mut cuts: BTreeSet<usize> = BTreeSet::new();
        for i in 0..n {
            if gl[i].k == K::Outline {
                if i > 0 {
                    cuts.insert(i);
                }
                if i + 1 < n {
                    cuts.insert(i + 1);
                }
            }
        }
        let mut bound: Vec<usize> = vec![];
        let mut mid: Vec<usize> = vec![];
        for i in 0..n {
            match gl[i].k {
                K::Space => bound.push(if self.rng.chance(0.5) { i } else { i + 1 }),
                K::Letter => {
                    if i + 1 < n && gl[i + 1].k == K::Letter {
                        mid.push(i + 1);
                    }
                }
                K::Outline => {}
            }
        }
        if words_only {
            // an OCR / word-by-word export: exactly one text object per word, the spaces belong to no object
            for i in 0..n {
                if gl[i].k == K::Space {
                    cuts.insert(i);
                    cuts.insert(i + 1);
                }
            }
        }
        let want = if words_only { 0 } else { nfrag.saturating_sub(1) };
        let mut guard = 0;
        while cuts.len() < want && guard < 3000 && !(bound.is_empty() && mid.is_empty()) {
            guard += 1;
            let use_bound = !bound.is_empty() && (mid.is_empty() || self.rng.chance(0.65));
            let p = if use_bound { bound[self.rng.below(bound.len())] } else { mid[self.rng.below(mid.len())] };
            if p > 0 && p < n {
                cuts.insert(p);
            }
        }
        let mut edges: Vec<usize> = vec![0];
        edges.extend(cuts.iter().copied());
        edges.push(n);
        let mut labels = vec![];
        let mut shape_label = None;
        let mut idx = 0;
        for w in edges.windows(2) {
            let (a, b) = (w[0], w[1]);
            if a >= b {
                continue;
            }
            if b - a == 1 && gl[a].k == K::Outline {
                let lab = format!("{id}.o");
                let (t, bt) = (base - 0.72 * size, base + 0.22 * size);
                self.add_shape(&lab, gl[a].x0, t, gl[a].x1, bt);
                let o = self.ids[&lab];
                self.shape_base.insert(o, base);
                shape_label = Some(lab);
                continue;
            }
            let fi = (a..b).find(|&i| gl[i].k == K::Letter);
            let li = (a..b).rev().find(|&i| gl[i].k == K::Letter);
            let (fi, li) = match (fi, li) {
                (Some(f), Some(l)) => (f, l),
                _ => continue,
            };
            let text: String = gl[a..b].iter().map(|g| g.c).collect();
            let l = gl[fi].x0;
            let mut r = gl[li].x1;
            if b < n && gl[b - 1].k == K::Letter && gl[b].k == K::Letter {
                let d = (size * self.rng.range(0.01, 0.07)).min((r - l) * 0.4);
                r -= d;
            }
            let lab = format!("{id}.{idx}");
            idx += 1;
            self.add_frag(&lab, st, &text, l, r, base);
            labels.push(lab);
        }
        (labels, shape_label, cur)
    }

    /// A multi-line paragraph.
    fn para(&mut self, p: P) -> Para {
        let size = p.st.size;
        let mut lines: Vec<Vec<String>> = Vec::new();
        let mut bases = Vec::new();
        let mut outlined: Vec<(usize, String)> = Vec::new();
        let mut carry: Option<String> = p.lead.clone();
        for i in 0..p.n {
            let base = p.base + i as f32 * p.pitch;
            bases.push(base);
            let first = i == 0;
            let last = i + 1 == p.n;
            let x = p.x + if first { p.indent } else { p.hang };
            let avail_full = p.w - if first { p.indent } else { p.hang };
            let mut avail = avail_full;
            let short = last && p.last.is_some();
            if short {
                avail *= p.last.unwrap();
            }
            let justify = p.align == Align::Justify && !short;
            if p.outlined_lines.contains(&i) {
                let lab = format!("{}.{}.o", p.id, i);
                self.add_shape(&lab, x, base - 0.72 * size, x + avail, base + 0.22 * size);
                let o = self.ids[&lab];
                self.shape_base.insert(o, base);
                lines.push(Vec::new());
                outlined.push((i, lab));
                continue;
            }
            let hyphenate = p.hyph.contains(&i) && !last;
            let (piece, rest) = if hyphenate {
                let (a, b) = split_long_word(&mut self.rng);
                (Some(format!("{}{}", a, p.hyph_ch)), Some(b))
            } else {
                (None, None)
            };
            let gap_lo = if justify { p.gap.0 * size } else { adv(' ') * size };
            let gap_hi = p.gap.1 * size;
            // An outlined word is a word the font could not draw as text (the datasheet's 'efficacy' is 27.5 x 7.6 pt at
            // 8 pt): it is a word of at least 5 letters, never a 5 pt wide box that is indistinguishable from an icon.
            // Lines that carry a hole are drawn again (up to 80 times) until the word that becomes the hole is long enough.
            let want_hole = p.holes.iter().find(|(l, _)| *l == i).map(|(_, h)| *h);
            // (a line that must START with the hole cannot start with a short word carried over from the line above:
            // that word is dropped, the text is random anyway)
            let first_carry = carry.take().filter(|w| !(want_hole == Some(Hole::Start) && letters(w) < 5));
            let mut attempt = 0;
            let (words, gap, overflow) = loop {
                let spec = FillSpec {
                    justify,
                    first: first_carry.clone(),
                    last: piece.clone(),
                    cap_first: first && !p.no_cap,
                    end_period: last && !p.no_period,
                };
                let r = fill(&mut self.rng, size, avail, gap_lo, gap_hi, &spec);
                attempt += 1;
                let fine = match want_hole {
                    None => true,
                    Some(h) => hole_index(&r.0, h).map(|k| letters(&r.0[k]) >= 5).unwrap_or(false),
                };
                if fine || attempt >= 80 {
                    break r;
                }
            };
            carry = if hyphenate { rest } else { overflow };
            let total: f32 =
                words.iter().map(|w| ink_w(w, size)).sum::<f32>() + gap * words.len().saturating_sub(1) as f32;
            let x0 = match p.align {
                Align::Centre => x + (avail_full - total) / 2.0,
                Align::Right => x + avail_full - total,
                _ => x,
            };
            let hole = want_hole.map(|h| {
                hole_index(&words, h).unwrap_or(match h {
                    Hole::Start => 0,
                    Hole::End => words.len() - 1,
                    Hole::Inside => words.len() / 2,
                })
            });
            let nf = p.frags.0 + self.rng.below(p.frags.1 - p.frags.0 + 1);
            let lid = format!("{}.{}", p.id, i);
            let (labels, shape, cur) = self.put_line(&lid, &p.st, x0, base, &words, gap, nf, hole, p.word_objects);
            if justify {
                // a justified line must be flush on both sides with every gap inside the requested range
                if words.len() < 2 {
                    self.warnings.push(format!("justified line {lid} has {} word(s)", words.len()));
                } else if gap > gap_hi + 0.01 || gap < gap_lo - 0.01 || (cur - (x0 + avail)).abs() > 0.05 {
                    self.warnings.push(format!(
                        "justified line {lid}: gap {gap:.2} outside [{gap_lo:.2}, {gap_hi:.2}] or right edge {cur:.2} != {:.2}",
                        x0 + avail
                    ));
                }
            }
            if let Some(s) = shape {
                outlined.push((i, s));
            }
            lines.push(labels);
        }
        Para { lines, bases, outlined, x: p.x, w: p.w, pitch: p.pitch }
    }

    fn get(&self, label: &str) -> &Frag {
        let i = self
            .frag_labels
            .iter()
            .position(|l| l == label)
            .unwrap_or_else(|| panic!("DSL: unknown fragment {label}"));
        &self.frags[i]
    }

    fn frag_mut(&mut self, label: &str) -> &mut Frag {
        let i = self
            .frag_labels
            .iter()
            .position(|l| l == label)
            .unwrap_or_else(|| panic!("DSL: unknown fragment {label}"));
        &mut self.frags[i]
    }

    /// Give a fragment another font/stem/face/colour (inline emphasis); geometry is unchanged.
    fn restyle(&mut self, label: &str, st: &St) {
        let f = self.frag_mut(label);
        f.font = st.font;
        f.stem = st.stem;
        f.face = st.face.clone();
        f.rgb = st.rgb;
    }

    /// The same text object drawn a second time at the identical rect (faux bold / overprint).
    fn twin(&mut self, label: &str) -> String {
        let name = format!("{label}'");
        let mut f = self.frag_mut(label).clone();
        f.object = self.alloc(&name);
        self.frag_labels.push(name.clone());
        self.frags.push(f);
        name
    }

    /// Make a fragment bigger (same baseline, left edge kept): an inline enlarged word.
    fn enlarge(&mut self, label: &str, new_size: f32) {
        let f = self.frag_mut(label);
        let k = new_size / f.size;
        f.right = f.left + (f.right - f.left) * k;
        let (t, b) = vext(&f.text, new_size, f.baseline);
        f.top = t;
        f.bottom = b;
        f.size = new_size;
    }

    /// Random baseline noise of +-`amp_em` em per fragment (OCR-like text layers).
    fn jitter(&mut self, amp_em: f32, labels: &[String]) {
        for l in labels {
            let u = self.rng.range(-1.0, 1.0);
            let f = self.frag_mut(l);
            let dy = u * amp_em * f.size;
            f.baseline += dy;
            f.top += dy;
            f.bottom += dy;
        }
    }

    /// A scan-like skew: every fragment is shifted vertically by `slope * (left - x0)`.
    fn skew(&mut self, slope: f32, x0: f32, labels: &[String]) {
        for l in labels {
            let f = self.frag_mut(l);
            let dy = slope * (f.left - x0);
            f.baseline += dy;
            f.top += dy;
            f.bottom += dy;
        }
    }

    /// Swap top and bottom of every fragment (the detector must normalise).
    fn invert_vertical(&mut self) {
        for f in &mut self.frags {
            std::mem::swap(&mut f.top, &mut f.bottom);
        }
    }

    /// Re-assign object ids in the order given by `key(y, x)` (content-stream order unrelated to the
    /// order the DSL emitted things). Labels follow their objects.
    fn renumber_by<F: Fn(f32, f32) -> (i64, i64)>(&mut self, key: F) {
        let mut items: Vec<((i64, i64), usize)> = Vec::new();
        for f in &self.frags {
            items.push((key(f.baseline, f.left), f.object));
        }
        for s in &self.shapes {
            let y = self.shape_base.get(&s.object).copied().unwrap_or(s.bottom);
            items.push((key(y, s.left), s.object));
        }
        items.sort();
        let map: HashMap<usize, usize> = items.iter().enumerate().map(|(new, (_, old))| (*old, new)).collect();
        for f in &mut self.frags {
            f.object = map[&f.object];
        }
        for s in &mut self.shapes {
            s.object = map[&s.object];
        }
        self.ids = self.ids.iter().map(|(k, v)| (k.clone(), map[v])).collect();
        self.names = self.names.iter().map(|(k, v)| (map[k], v.clone())).collect();
        self.shape_base = self.shape_base.iter().map(|(k, v)| (map[k], *v)).collect();
    }
}

// ---- small helpers for writing expectations -------------------------------------------------------

fn lab(x: &str) -> Vec<String> {
    vec![x.to_string()]
}

fn labs(xs: &[&str]) -> Vec<String> {
    xs.iter().map(|s| s.to_string()).collect()
}

fn cat(parts: Vec<Vec<String>>) -> Vec<String> {
    parts.into_iter().flatten().collect()
}

// =====================================================================================================
// 3. cases
// =====================================================================================================

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Strength {
    Must,
    Ambiguous,
}

impl Strength {
    fn tag(self) -> &'static str {
        match self {
            Strength::Must => "MUST",
            Strength::Ambiguous => "AMBIG",
        }
    }
}

struct Case {
    name: &'static str,
    strength: Strength,
    /// What a person sees and why the expected grouping is the right one (AMBIGUOUS: the competing readings).
    intent: String,
    page: Page,
    /// Expected blocks as lists of fragment labels (text objects only). MUST cases only.
    expect: Vec<Vec<String>>,
    /// AMBIGUOUS cases: the named competing readings.
    readings: Vec<(String, Vec<Vec<String>>)>,
    /// Fragments whose block membership is not asserted (stray marks, degenerate geometry): they must still
    /// appear exactly once, but may stand alone or join any block.
    free: Vec<String>,
    /// (a text label of the block, shape label, whole-line): the shape must be bridged into that block's
    /// `Line::outlined`, the block box must enclose it, and for whole-line shapes the line has no text.
    outlines: Vec<(String, String, bool)>,
    /// (a text label of the block, shape label): the shape (an outlined HEADING above the block) must NOT be
    /// bridged into that block, and the block box must not swallow it.
    no_outlines: Vec<(String, String)>,
    /// Blocks of this case may overlap geometrically by design (twins, watermarks).
    overlap_ok: bool,
    /// false: only "no panic + contract invariants" are asserted (duplicate object ids).
    check_groups: bool,
    /// Geometry may be non-finite / inverted on purpose.
    degenerate: bool,
    dup_ids: bool,
    heavy: bool,
}

thread_local! {
    /// Seed salt of the instance being built: 0 = the canonical instance every table is about; 1, 2, ... = the
    /// same layouts with other random words, cuts and gaps (PAGIFY_BLOCKS_SEEDS). Thread-local because the
    /// tests of this file run on parallel threads.
    static SALT: std::cell::Cell<u64> = std::cell::Cell::new(0);
}

fn salt() -> u64 {
    SALT.with(|s| s.get())
}

impl Case {
    fn new(name: &'static str, strength: Strength, intent: &str) -> Case {
        Case {
            name,
            strength,
            intent: intent.to_string(),
            page: Page::new(fnv(name) ^ salt().wrapping_mul(0x9E37_79B9_7F4A_7C15)),
            expect: vec![],
            readings: vec![],
            free: vec![],
            outlines: vec![],
            no_outlines: vec![],
            overlap_ok: false,
            check_groups: true,
            degenerate: false,
            dup_ids: false,
            heavy: false,
        }
    }
    fn must(name: &'static str, intent: &str) -> Case {
        Case::new(name, Strength::Must, intent)
    }
    fn ambiguous(name: &'static str, intent: &str) -> Case {
        Case::new(name, Strength::Ambiguous, intent)
    }
    fn reading(&mut self, name: &str, groups: Vec<Vec<String>>) {
        self.readings.push((name.to_string(), groups));
    }
    fn outline(&mut self, anchor: &str, shape: &str, whole_line: bool) {
        self.outlines.push((anchor.to_string(), shape.to_string(), whole_line));
    }
    fn no_outline(&mut self, anchor: &str, shape: &str) {
        self.no_outlines.push((anchor.to_string(), shape.to_string()));
    }

    fn frag_by_label(&self) -> HashMap<&str, &Frag> {
        self.page.frag_labels.iter().map(|l| l.as_str()).zip(self.page.frags.iter()).collect()
    }

    /// Internal consistency of the expectations (they must be a partition of the page's fragments).
    fn consistency(&self) -> Vec<String> {
        let mut out: Vec<String> = vec![];
        let page = &self.page;
        for w in &page.warnings {
            out.push(format!("generator: {w}"));
        }
        if !self.dup_ids {
            let mut seen = HashSet::new();
            for f in &page.frags {
                if !seen.insert(f.object) {
                    out.push(format!("object id {} used twice", f.object));
                }
            }
            for s in &page.shapes {
                if !seen.insert(s.object) {
                    out.push(format!("object id {} (shape) used twice", s.object));
                }
            }
        }
        if !self.degenerate {
            for (l, f) in page.frag_labels.iter().zip(page.frags.iter()) {
                if !finite_frag(f) {
                    out.push(format!("fragment {l} has non-finite geometry in a non-degenerate case"));
                }
            }
        }
        for (l, f) in page.frag_labels.iter().zip(page.frags.iter()) {
            if !self.degenerate && f.size <= 0.0 {
                out.push(format!("fragment {l} has size {} in a non-degenerate case", f.size));
            }
        }
        let free: HashSet<&str> = self.free.iter().map(|s| s.as_str()).collect();
        let frag_labels: BTreeSet<&str> = page.frag_labels.iter().map(|s| s.as_str()).collect();
        for l in &self.free {
            if !frag_labels.contains(l.as_str()) {
                out.push(format!("free label {l} is not a text fragment"));
            }
        }
        let fmap = self.frag_by_label();
        let check = |name: &str, groups: &Vec<Vec<String>>, out: &mut Vec<String>| {
            let mut seen: HashMap<&str, usize> = HashMap::new();
            for (gi, g) in groups.iter().enumerate() {
                if g.is_empty() {
                    out.push(format!("{name}: group {gi} is empty"));
                }
                for l in g {
                    if free.contains(l.as_str()) {
                        out.push(format!("{name}: free label {l} also appears in a group"));
                    }
                    if !frag_labels.contains(l.as_str()) {
                        out.push(format!("{name}: label {l} is not a text fragment of the page"));
                    }
                    *seen.entry(l.as_str()).or_insert(0) += 1;
                }
                // a rotated fragment must be alone in its group
                if g.len() > 1 {
                    for l in g {
                        if fmap.get(l.as_str()).map(|f| f.rotated).unwrap_or(false) {
                            out.push(format!("{name}: rotated fragment {l} shares a group"));
                        }
                    }
                }
            }
            for (l, c) in &seen {
                if *c > 1 {
                    out.push(format!("{name}: label {l} appears {c} times"));
                }
            }
            if self.check_groups {
                for l in &frag_labels {
                    if !free.contains(l) && !seen.contains_key(l) {
                        out.push(format!("{name}: fragment {l} is in no group"));
                    }
                }
            }
        };
        match self.strength {
            Strength::Must => {
                if self.check_groups && self.expect.is_empty() && !page.frags.is_empty() {
                    out.push("MUST case without expected groups".to_string());
                }
                check("expect", &self.expect, &mut out);
                if !self.readings.is_empty() {
                    out.push("MUST case with readings".to_string());
                }
            }
            Strength::Ambiguous => {
                if self.readings.len() < 2 {
                    out.push("AMBIGUOUS case needs at least two competing readings".to_string());
                }
                if !self.expect.is_empty() {
                    out.push("AMBIGUOUS case must not assert an expectation".to_string());
                }
                for (n, g) in &self.readings {
                    check(&format!("reading '{n}'"), g, &mut out);
                }
                // the readings must really differ
                for i in 0..self.readings.len() {
                    for j in i + 1..self.readings.len() {
                        if canon_labels(&self.readings[i].1) == canon_labels(&self.readings[j].1) {
                            out.push(format!("readings '{}' and '{}' are identical", self.readings[i].0, self.readings[j].0));
                        }
                    }
                }
            }
        }
        for (anchor, shape, _) in self.outlines.iter().map(|(a, s, w)| (a, s, w)).chain(self.no_outlines.iter().map(|(a, s)| (a, s, &false))) {
            if !frag_labels.contains(anchor.as_str()) {
                out.push(format!("outline anchor {anchor} is not a text fragment"));
            }
            if !page.shape_labels.iter().any(|s| s == shape) {
                out.push(format!("outline shape {shape} is not a shape"));
            }
        }
        // geometry sanity of the expectation: expected blocks must not overlap each other, and the members
        // of one block must not overlap each other (unless the case says so)
        if !self.overlap_ok && !self.degenerate && page.frags.len() <= 3000 {
            let groups: &Vec<Vec<String>> = match self.strength {
                Strength::Must => &self.expect,
                Strength::Ambiguous => &self.readings[0].1,
            };
            let boxes: Vec<(f32, f32, f32, f32)> = groups
                .iter()
                .map(|g| {
                    let mut b = (f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY);
                    for l in g {
                        if let Some(f) = fmap.get(l.as_str()) {
                            let r = nrect(f);
                            b = (b.0.min(r.0), b.1.min(r.1), b.2.max(r.2), b.3.max(r.3));
                        }
                    }
                    b
                })
                .collect();
            for i in 0..boxes.len() {
                for j in i + 1..boxes.len() {
                    let (a, b) = (boxes[i], boxes[j]);
                    let ow = a.2.min(b.2) - a.0.max(b.0);
                    let oh = a.3.min(b.3) - a.1.max(b.1);
                    if ow > 0.2 && oh > 0.2 {
                        out.push(format!(
                            "expected blocks #{i} ({}) and #{j} ({}) overlap by {ow:.1} x {oh:.1} pt",
                            groups[i].first().map(|s| s.as_str()).unwrap_or("-"),
                            groups[j].first().map(|s| s.as_str()).unwrap_or("-")
                        ));
                    }
                }
            }
            for g in groups.iter().filter(|g| g.len() <= 400) {
                let rs: Vec<(&str, (f32, f32, f32, f32))> =
                    g.iter().filter_map(|l| fmap.get(l.as_str()).map(|f| (l.as_str(), nrect(f)))).collect();
                for i in 0..rs.len() {
                    for j in i + 1..rs.len() {
                        let (a, b) = (rs[i].1, rs[j].1);
                        let ow = a.2.min(b.2) - a.0.max(b.0);
                        let oh = a.3.min(b.3) - a.1.max(b.1);
                        if ow > 0.05 && oh > 0.05 {
                            out.push(format!("fragments {} and {} of one expected block overlap", rs[i].0, rs[j].0));
                        }
                    }
                }
            }
        }
        out
    }
}

fn canon_labels(groups: &[Vec<String>]) -> Vec<Vec<String>> {
    let mut c: Vec<Vec<String>> = groups
        .iter()
        .map(|g| {
            let mut g = g.clone();
            g.sort();
            g
        })
        .collect();
    c.sort();
    c
}

fn nrect(f: &Frag) -> (f32, f32, f32, f32) {
    (f.left.min(f.right), f.top.min(f.bottom), f.left.max(f.right), f.top.max(f.bottom))
}

fn finite_frag(f: &Frag) -> bool {
    [f.left, f.top, f.right, f.bottom, f.baseline, f.size].iter().all(|v| v.is_finite())
}

// =====================================================================================================
// 4. variants: scale x1, x0.5, x2, x3.7 and two deterministic slice-order shuffles
// =====================================================================================================

type Detector<'a> = &'a dyn Fn(&[Frag], &[Shape]) -> Vec<Block>;

#[derive(Clone, Copy)]
struct Variant {
    name: &'static str,
    scale: f32,
    shuffle: Option<u64>,
}

const V_X1: Variant = Variant { name: "x1", scale: 1.0, shuffle: None };
const V_S1: Variant = Variant { name: "shuf1", scale: 1.0, shuffle: Some(0x5EED_0001) };
const FULL_VARIANTS: [Variant; 6] = [
    V_X1,
    Variant { name: "x0.5", scale: 0.5, shuffle: None },
    Variant { name: "x2", scale: 2.0, shuffle: None },
    Variant { name: "x3.7", scale: 3.7, shuffle: None },
    V_S1,
    Variant { name: "shuf2", scale: 1.0, shuffle: Some(0x5EED_BEEF) },
];
const LIGHT_VARIANTS: [Variant; 2] = [V_X1, V_S1];

fn apply(page: &Page, v: &Variant) -> (Vec<Frag>, Vec<Shape>) {
    let k = v.scale;
    let mut frags: Vec<Frag> = page
        .frags
        .iter()
        .map(|f| {
            let mut g = f.clone();
            g.left *= k;
            g.top *= k;
            g.right *= k;
            g.bottom *= k;
            g.baseline *= k;
            g.size *= k;
            g
        })
        .collect();
    let mut shapes: Vec<Shape> = page
        .shapes
        .iter()
        .map(|s| Shape { left: s.left * k, top: s.top * k, right: s.right * k, bottom: s.bottom * k, ..*s })
        .collect();
    if let Some(seed) = v.shuffle {
        let mut rng = Rng::new(seed ^ page.frags.len() as u64);
        shuffle(&mut frags, &mut rng);
        shuffle(&mut shapes, &mut rng);
    }
    (frags, shapes)
}

// =====================================================================================================
// 5. contract invariants and grouping comparison
// =====================================================================================================

/// Collects problems, capped, so a catastrophic detector does not produce 30,000 lines.
struct Cap {
    v: Vec<String>,
    more: usize,
}

impl Cap {
    fn new() -> Cap {
        Cap { v: vec![], more: 0 }
    }
    fn add(&mut self, s: String) {
        if self.v.len() < 12 {
            self.v.push(s);
        } else {
            self.more += 1;
        }
    }
    fn finish(mut self) -> Vec<String> {
        if self.more > 0 {
            self.v.push(format!("... and {} more", self.more));
        }
        self.v
    }
}

fn invariants(frags: &[Frag], shapes: &[Shape], blocks: &[Block], eps: f32, dup_ids: bool) -> Vec<String> {
    let mut cap = Cap::new();
    let mut count: HashMap<usize, usize> = HashMap::new();
    let mut at: HashMap<usize, usize> = HashMap::new();
    for (bi, b) in blocks.iter().enumerate() {
        if b.lines.is_empty() {
            cap.add(format!("INV block {bi} has no lines"));
        }
        for l in &b.lines {
            if l.objects.is_empty() && l.outlined.is_empty() {
                cap.add(format!("INV block {bi} has a line without any member"));
            }
        }
        for o in b.objects() {
            *count.entry(o).or_default() += 1;
            at.insert(o, bi);
        }
    }
    let frag_ids: HashSet<usize> = frags.iter().map(|f| f.object).collect();
    let shape_ids: HashSet<usize> = shapes.iter().map(|s| s.object).collect();
    for f in frags {
        let c = count.get(&f.object).copied().unwrap_or(0);
        if dup_ids {
            if c == 0 {
                cap.add(format!("INV object {} is in no block", f.object));
            }
        } else if c != 1 {
            cap.add(format!("INV object {} is in {c} blocks (must be exactly 1)", f.object));
        }
    }
    for o in count.keys() {
        if !frag_ids.contains(o) {
            cap.add(format!("INV block lists object {o}, which is not a text fragment of the page"));
        }
    }
    for b in blocks {
        for l in &b.lines {
            for o in &l.outlined {
                if !shape_ids.contains(o) || frag_ids.contains(o) {
                    cap.add(format!("INV outlined object {o} is not a shape"));
                }
            }
        }
    }
    let fmap: HashMap<usize, &Frag> = frags.iter().map(|f| (f.object, f)).collect();
    // rotated fragments: alone in a one-line block
    for f in frags.iter().filter(|f| f.rotated) {
        if let Some(&bi) = at.get(&f.object) {
            let b = &blocks[bi];
            if b.lines.len() != 1 || b.objects() != vec![f.object] {
                cap.add(format!("INV rotated object {} is not alone in a one-line block", f.object));
            }
        }
    }
    // contains / block_index_of agree with objects() (sampled on big pages)
    if !dup_ids && !blocks.is_empty() {
        let step = (frags.len() / 300).max(1);
        for f in frags.iter().step_by(step) {
            if let Some(&bi) = at.get(&f.object) {
                if !blocks[bi].contains(f.object) {
                    cap.add(format!("INV Block::contains({}) is false for the block that lists it", f.object));
                }
                match block_index_of(blocks, f.object) {
                    Some(j) if j == bi => {}
                    other => cap.add(format!("INV block_index_of({}) = {:?}, expected Some({bi})", f.object, other)),
                }
                let other = (bi + 1) % blocks.len();
                if other != bi && blocks[other].contains(f.object) {
                    cap.add(format!("INV Block::contains({}) is true for a block that does not list it", f.object));
                }
            }
        }
        if block_index_of(blocks, usize::MAX - 7).is_some() {
            cap.add("INV block_index_of of an unknown object is not None".to_string());
        }
    }
    // geometry of blocks and lines (ambiguous when two fragments share an object id: skipped then)
    for (bi, b) in blocks.iter().enumerate() {
        if dup_ids {
            break;
        }
        for w in b.lines.windows(2) {
            if w[0].baseline.is_finite() && w[1].baseline.is_finite() && w[0].baseline > w[1].baseline + eps {
                cap.add(format!("INV block {bi}: lines are not top to bottom"));
                break;
            }
        }
        for l in &b.lines {
            let lefts: Vec<f32> = l
                .objects
                .iter()
                .filter_map(|o| fmap.get(o))
                .filter(|f| finite_frag(f))
                .map(|f| nrect(f).0)
                .collect();
            if lefts.windows(2).any(|w| w[0] > w[1] + eps) {
                cap.add(format!("INV block {bi}: objects of a line are not left to right"));
            }
            for o in &l.objects {
                if let Some(f) = fmap.get(o) {
                    if finite_frag(f) {
                        let r = nrect(f);
                        if l.left > r.0 + eps || l.top > r.1 + eps || l.right + eps < r.2 || l.bottom + eps < r.3 {
                            cap.add(format!("INV block {bi}: line box does not enclose object {o}"));
                        }
                        if b.left > r.0 + eps || b.top > r.1 + eps || b.right + eps < r.2 || b.bottom + eps < r.3 {
                            cap.add(format!("INV block {bi}: block box does not enclose object {o}"));
                        }
                    }
                }
            }
        }
        let has_finite_member =
            b.objects().iter().any(|o| fmap.get(o).map(|f| finite_frag(f)).unwrap_or(false));
        if has_finite_member && ![b.left, b.top, b.right, b.bottom].iter().all(|v| v.is_finite()) {
            cap.add(format!("INV block {bi} has a non-finite box although it holds finite fragments"));
        }
        if [b.left, b.top, b.right, b.bottom].iter().all(|v| v.is_finite()) && (b.left > b.right + eps || b.top > b.bottom + eps)
        {
            cap.add(format!("INV block {bi} has an inverted box"));
        }
    }
    cap.finish()
}

/// Compare a grouping given as labels with the detected blocks (free labels ignored).
fn compare(case: &Case, expect: &[Vec<String>], blocks: &[Block]) -> Vec<String> {
    let page = &case.page;
    let free: HashSet<usize> = case.free.iter().map(|l| page.id(l)).collect();
    let exp: Vec<Vec<usize>> = expect
        .iter()
        .map(|g| g.iter().map(|l| page.id(l)).filter(|o| !free.contains(o)).collect::<Vec<usize>>())
        .filter(|g| !g.is_empty())
        .collect();
    let det: Vec<Vec<usize>> = blocks
        .iter()
        .map(|b| b.objects().into_iter().filter(|o| !free.contains(o)).collect::<Vec<usize>>())
        .filter(|g| !g.is_empty())
        .collect();
    let canon = |v: &Vec<Vec<usize>>| {
        let mut c: Vec<Vec<usize>> = v
            .iter()
            .map(|g| {
                let mut g = g.clone();
                g.sort_unstable();
                g.dedup();
                g
            })
            .collect();
        c.sort();
        c
    };
    if canon(&exp) == canon(&det) {
        return vec![];
    }
    let mut out = vec![format!("GROUPING expected {} blocks, detected {}", exp.len(), det.len())];
    let mut e_of: HashMap<usize, usize> = HashMap::new();
    let mut d_of: HashMap<usize, usize> = HashMap::new();
    for (i, g) in exp.iter().enumerate() {
        for o in g {
            e_of.insert(*o, i);
        }
    }
    for (j, g) in det.iter().enumerate() {
        for o in g {
            d_of.insert(*o, j);
        }
    }
    let ename = |i: usize| {
        let first = exp[i].iter().min().copied().unwrap();
        format!("E{i}<{} x{}>", page.name_of(first), exp[i].len())
    };
    let dname = |j: usize| {
        let first = det[j].iter().min().copied().unwrap();
        format!("D{j}<{} x{}>", page.name_of(first), det[j].len())
    };
    let mut cap = Cap::new();
    for (i, g) in exp.iter().enumerate() {
        let mut pieces: BTreeMap<Option<usize>, usize> = BTreeMap::new();
        for o in g {
            *pieces.entry(d_of.get(o).copied()).or_default() += 1;
        }
        if pieces.len() > 1 {
            let sizes: Vec<String> = pieces
                .iter()
                .map(|(k, c)| match k {
                    Some(j) => format!("{}:{c}", dname(*j)),
                    None => format!("missing:{c}"),
                })
                .collect();
            cap.add(format!("SPLIT {} cut into {} pieces [{}]", ename(i), pieces.len(), brief(&sizes, 4)));
            for d in cut_diagnostics(case, g, &d_of).into_iter().take(3) {
                cap.add(d);
            }
        }
    }
    for (j, g) in det.iter().enumerate() {
        let mut touched: BTreeMap<Option<usize>, usize> = BTreeMap::new();
        for o in g {
            *touched.entry(e_of.get(o).copied()).or_default() += 1;
        }
        if touched.len() > 1 {
            let parts: Vec<String> = touched
                .iter()
                .map(|(k, c)| match k {
                    Some(i) => format!("{}:{c}", ename(*i)),
                    None => format!("foreign:{c}"),
                })
                .collect();
            cap.add(format!("MERGE {} spans {} expected blocks [{}]", dname(j), touched.len(), brief(&parts, 4)));
        }
    }
    out.extend(cap.finish());
    out
}

/// Where a SPLIT cut an expected block, in the units a detector reasons in: for every pair of consecutive text lines of the
/// expected block that ended up in different detected blocks, how far the line above ends short of the block's right edge (em),
/// how wide the first word below is (em) and whether it would have fitted into the gap (then the cut could be a deliberate
/// break; if not, the line merely wrapped), and how far the line below starts from the block's left edge (em).
fn cut_diagnostics(case: &Case, group: &[usize], d_of: &HashMap<usize, usize>) -> Vec<String> {
    let fm: HashMap<usize, &Frag> = case.page.frags.iter().map(|f| (f.object, f)).collect();
    let mut ms: Vec<&Frag> =
        group.iter().filter_map(|o| fm.get(o).copied()).filter(|f| finite_frag(f) && f.size > 0.0 && !f.rotated).collect();
    if ms.len() < 2 {
        return vec![];
    }
    ms.sort_by(|a, b| a.baseline.total_cmp(&b.baseline).then(nrect(a).0.total_cmp(&nrect(b).0)));
    let mut lines: Vec<Vec<&Frag>> = vec![];
    for f in ms {
        match lines.last_mut() {
            Some(l) if (f.baseline - l[0].baseline).abs() <= 0.3 * f.size => l.push(f),
            _ => lines.push(vec![f]),
        }
    }
    let right_max = lines.iter().flatten().map(|f| nrect(f).2).fold(f32::MIN, f32::max);
    let left_min = lines.iter().flatten().map(|f| nrect(f).0).fold(f32::MAX, f32::min);
    let mut out = vec![];
    for (k, w) in lines.windows(2).enumerate() {
        let (a, b) = (&w[0], &w[1]);
        let blocks_of = |l: &Vec<&Frag>| -> BTreeSet<Option<usize>> { l.iter().map(|f| d_of.get(&f.object).copied()).collect() };
        if !blocks_of(a).is_disjoint(&blocks_of(b)) {
            continue;
        }
        let em = a[0].size;
        let short = (right_max - a.iter().map(|f| nrect(f).2).fold(f32::MIN, f32::max)) / em;
        let mut sorted: Vec<&Frag> = b.clone();
        sorted.sort_by(|x, y| nrect(x).0.total_cmp(&nrect(y).0));
        let first = sorted[0];
        // the first WORD of the line below: glue the objects of the line (left to right) until a space or a visible gap, because
        // the first object is often only the beginning of a word (the objects are cut inside words)
        let mut word = String::new();
        let mut prev_right: Option<f32> = None;
        'glue: for f in &sorted {
            if let Some(pr) = prev_right {
                if nrect(f).0 - pr > 0.15 * f.size && !word.is_empty() {
                    break;
                }
            }
            for ch in f.text.chars() {
                if ch.is_whitespace() {
                    if !word.is_empty() {
                        break 'glue;
                    }
                } else {
                    word.push(ch);
                }
            }
            prev_right = Some(nrect(f).2);
            if f.text.ends_with(char::is_whitespace) && !word.is_empty() {
                break;
            }
        }
        let ww = ink_w(&word, first.size) / first.size;
        let indent = (nrect(first).0 - left_min) / first.size;
        let verdict = if ww + adv(' ') <= short { "WOULD have fitted: a deliberate break" } else { "does NOT fit: the line merely wrapped" };
        out.push(format!(
            "CUT after line {} of the block: it ends {short:.1} em short of the block's right edge; first word below '{}' is {ww:.1} em wide ({verdict}); that line starts {indent:.1} em from the block's left edge",
            k + 1,
            word.trim_matches('\u{2}')
        ));
    }
    out
}

fn outline_problems(case: &Case, blocks: &[Block], shapes: &[Shape], scale: f32) -> Vec<String> {
    let mut out = vec![];
    let page = &case.page;
    for (anchor, shape, whole) in &case.outlines {
        let (a, s) = (page.id(anchor), page.id(shape));
        let Some(b) = blocks.iter().find(|b| b.contains(a)) else {
            out.push(format!("OUTLINE anchor {anchor} is in no block"));
            continue;
        };
        let dims = shapes
            .iter()
            .find(|x| x.object == s)
            .map(|x| format!(" ({:.1} x {:.1} pt = {:.1} em wide)", x.right - x.left, x.bottom - x.top, (x.right - x.left) / ((x.bottom - x.top) / 0.94)))
            .unwrap_or_default();
        match b.lines.iter().find(|l| l.outlined.contains(&s)) {
            None => out.push(format!("OUTLINE shape {shape}{dims} is not bridged into the block of {anchor} (Line::outlined)")),
            Some(l) => {
                if *whole && !l.objects.is_empty() {
                    out.push(format!("OUTLINE whole-line shape {shape} shares its line with text objects"));
                }
            }
        }
        if let Some(sh) = shapes.iter().find(|x| x.object == s) {
            let eps = 0.05 * scale.max(1.0);
            if b.left > sh.left + eps || b.top > sh.top + eps || b.right + eps < sh.right || b.bottom + eps < sh.bottom {
                out.push(format!("OUTLINE block box of {anchor} does not enclose outlined shape {shape}"));
            }
        }
    }
    for (anchor, shape) in &case.no_outlines {
        let (a, s) = (page.id(anchor), page.id(shape));
        let Some(b) = blocks.iter().find(|b| b.contains(a)) else {
            out.push(format!("OUTLINE anchor {anchor} is in no block"));
            continue;
        };
        if b.lines.iter().any(|l| l.outlined.contains(&s)) {
            out.push(format!("OUTLINE shape {shape} (an outlined heading above) was bridged into the paragraph block of {anchor}"));
        }
        if let Some(sh) = shapes.iter().find(|x| x.object == s) {
            let eps = 0.05 * scale.max(1.0);
            if b.left <= sh.left + eps && b.top <= sh.top + eps && b.right + eps >= sh.right && b.bottom + eps >= sh.bottom {
                out.push(format!("OUTLINE block box of {anchor} swallows the outlined heading {shape}"));
            }
        }
    }
    out
}

// =====================================================================================================
// 6. running a detector over the corpus
// =====================================================================================================

fn guarded(det: Detector, frags: &[Frag], shapes: &[Shape]) -> (Result<Vec<Block>, String>, f64) {
    let t = Instant::now();
    let r = catch_unwind(AssertUnwindSafe(|| det(frags, shapes)));
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    match r {
        Ok(b) => (Ok(b), ms),
        Err(e) => {
            let m = if let Some(s) = e.downcast_ref::<&str>() {
                s.to_string()
            } else if let Some(s) = e.downcast_ref::<String>() {
                s.clone()
            } else {
                "non-string panic payload".to_string()
            };
            (Err(m), ms)
        }
    }
}

struct Eval {
    problems: Vec<String>,
    blocks: Vec<Block>,
    ms: f64,
}

fn heavy_limit_secs() -> f64 {
    env_str("PAGIFY_BLOCKS_HEAVY_SECS").and_then(|v| v.parse().ok()).unwrap_or(60.0)
}

fn evaluate(case: &Case, det: Detector, v: &Variant, groups: bool) -> Eval {
    let (frags, shapes) = apply(&case.page, v);
    let (res, ms) = guarded(det, &frags, &shapes);
    let blocks = match res {
        Ok(b) => b,
        Err(m) => return Eval { problems: vec![format!("PANIC {m}")], blocks: vec![], ms },
    };
    let mut problems = invariants(&frags, &shapes, &blocks, 0.02 * v.scale.max(1.0), case.dup_ids);
    // the contract says "deterministic": the same input twice must give the same blocks (Debug text, so NaN == NaN)
    if !case.heavy {
        match guarded(det, &frags, &shapes).0 {
            Ok(again) if format!("{again:?}") != format!("{blocks:?}") => {
                problems.push("INV detect is not deterministic: two calls on the same input returned different blocks".to_string())
            }
            Ok(_) => {}
            Err(m) => problems.push(format!("PANIC on the second call: {m}")),
        }
    }
    if groups && case.check_groups {
        problems.extend(compare(case, &case.expect, &blocks));
        problems.extend(outline_problems(case, &blocks, &shapes, v.scale));
    }
    if case.heavy && ms > heavy_limit_secs() * 1000.0 {
        problems.push(format!("TIME detect took {:.1} s, bound is {:.0} s", ms / 1000.0, heavy_limit_secs()));
    }
    Eval { problems, blocks, ms }
}

struct Row {
    index: usize,
    name: &'static str,
    strength: Strength,
    cells: Vec<(&'static str, bool)>,
    ms: f64,
    details: Vec<String>,
    note: String,
    /// MUST: every variant clean. AMBIGUOUS: contract invariants clean.
    pass: bool,
    invariant_failed: bool,
    /// AMBIGUOUS only: the variants (scale, shuffle) whose answer differs from the x1 answer.
    unstable: Vec<&'static str>,
    intent: String,
}

/// The blocks as a sorted list of sorted object ids (what a grouping IS, independent of block and line order).
fn canon_ids(blocks: &[Block]) -> Vec<Vec<usize>> {
    let mut c: Vec<Vec<usize>> = blocks
        .iter()
        .map(|b| {
            let mut o = b.objects();
            o.sort_unstable();
            o
        })
        .collect();
    c.sort();
    c
}

/// Word-wrap `text` to `width` columns, every line prefixed with `indent`.
fn wrap(text: &str, width: usize, indent: &str) -> String {
    let mut out = String::new();
    let mut line = String::new();
    for w in text.split_whitespace() {
        if !line.is_empty() && line.len() + 1 + w.len() > width {
            out.push_str(indent);
            out.push_str(&line);
            out.push('\n');
            line.clear();
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(w);
    }
    if !line.is_empty() {
        out.push_str(indent);
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// Expected (or reading) groups in the same compact form as `describe_blocks`: `[first..last Nf]`.
fn describe_groups(groups: &[Vec<String>]) -> String {
    let mut parts = vec![];
    for g in groups.iter().take(14) {
        parts.push(format!(
            "[{}..{} {}f]",
            g.first().map(|s| s.as_str()).unwrap_or("-"),
            g.last().map(|s| s.as_str()).unwrap_or("-"),
            g.len()
        ));
    }
    if groups.len() > 14 {
        parts.push(format!("... +{} more groups ({} in all)", groups.len() - 14, groups.len()));
    }
    parts.join(" ")
}

/// First `max` items, then "+N more".
fn brief(parts: &[String], max: usize) -> String {
    if parts.len() <= max {
        parts.join(", ")
    } else {
        format!("{}, +{} more", parts[..max].join(", "), parts.len() - max)
    }
}

fn describe_blocks(case: &Case, blocks: &[Block]) -> String {
    let mut parts = vec![];
    for b in blocks.iter().take(14) {
        let objs = b.objects();
        let first = objs.first().map(|o| case.page.name_of(*o)).unwrap_or_else(|| "(outlined only)".to_string());
        let last = objs.last().map(|o| case.page.name_of(*o)).unwrap_or_default();
        parts.push(format!("[{first}..{last} {}f/{}L]", objs.len(), b.lines.len()));
    }
    if blocks.len() > 14 {
        parts.push(format!("... +{} more blocks ({} in all)", blocks.len() - 14, blocks.len()));
    }
    parts.join(" ")
}

fn run_corpus(det: Detector, cases: &[Case], only: Option<&str>, verbose: bool, svg_dir: Option<&str>) -> Vec<Row> {
    let mut rows = vec![];
    for (index, case) in cases.iter().enumerate() {
        if let Some(o) = only {
            if !case.name.contains(o) {
                continue;
            }
        }
        // AMBIGUOUS cases run every variant too: nothing is asserted about the answer, but the contract invariants
        // must hold everywhere and the table shows whether the answer is stable under scale and slice order.
        let variants: Vec<Variant> = if case.heavy { LIGHT_VARIANTS.to_vec() } else { FULL_VARIANTS.to_vec() };
        let mut row = Row {
            index: index + 1,
            name: case.name,
            strength: case.strength,
            cells: vec![],
            ms: 0.0,
            details: vec![],
            note: String::new(),
            pass: true,
            invariant_failed: false,
            unstable: vec![],
            intent: case.intent.clone(),
        };
        let mut first_blocks: Option<Vec<Block>> = None;
        let mut first_canon: Option<Vec<Vec<usize>>> = None;
        let mut fails_in: Vec<&'static str> = vec![];
        let mut fail_blocks: Option<(&'static str, Vec<Block>)> = None;
        for v in &variants {
            let ev = evaluate(case, det, v, case.strength == Strength::Must);
            row.ms += ev.ms;
            let ok = ev.problems.is_empty();
            if ev.problems.iter().any(|p| p.starts_with("INV") || p.starts_with("PANIC")) {
                row.invariant_failed = true;
            }
            if case.strength == Strength::Ambiguous {
                let canon = canon_ids(&ev.blocks);
                let same = match &first_canon {
                    Some(f) => *f == canon,
                    None => {
                        first_canon = Some(canon);
                        true
                    }
                };
                if !same {
                    row.unstable.push(v.name);
                }
                row.cells.push((v.name, same));
            } else {
                row.cells.push((v.name, ok));
            }
            if !ok {
                row.pass = false;
                fails_in.push(v.name);
                if fail_blocks.is_none() {
                    fail_blocks = Some((v.name, ev.blocks.clone()));
                }
                if verbose || row.details.is_empty() {
                    row.details.push(format!("variant {}:", v.name));
                    row.details.extend(ev.problems.iter().map(|p| format!("    {p}")));
                }
            }
            if first_blocks.is_none() {
                first_blocks = Some(ev.blocks);
            }
        }
        let blocks = first_blocks.unwrap_or_default();
        if case.strength == Strength::Ambiguous {
            let matched: Vec<&str> = case
                .readings
                .iter()
                .filter(|(_, g)| compare(case, g, &blocks).is_empty())
                .map(|(n, _)| n.as_str())
                .collect();
            row.note = if matched.is_empty() {
                format!("no named reading ({} blocks)", blocks.len())
            } else {
                format!("= reading '{}'", matched.join("' and '"))
            };
            if !row.unstable.is_empty() {
                row.note.push_str(&format!("  [UNSTABLE: answer differs in {}]", row.unstable.join(", ")));
            }
            row.details.push(format!("recorded: {}", describe_blocks(case, &blocks)));
            for (n, g) in &case.readings {
                let hit = matched.contains(&n.as_str());
                row.details.push(format!("reading '{n}'{}: {}", if hit { " (MATCHES the recorded answer)" } else { "" }, describe_groups(g)));
            }
        } else if !row.pass {
            // self-explanatory failure: layout (intent), what a person expects, what detect returned, which variants fail
            let mut head = vec![format!("layout and intent:\n{}", wrap(&case.intent, 118, "      ").trim_end())];
            head.push(format!("expected ({} blocks): {}", case.expect.len(), describe_groups(&case.expect)));
            if let Some((vn, fb)) = &fail_blocks {
                head.push(format!("detected at {vn} ({} blocks): {}", fb.len(), describe_blocks(case, fb)));
            }
            let passes: Vec<&str> = variants.iter().map(|v| v.name).filter(|n| !fails_in.contains(n)).collect();
            head.push(format!(
                "fails in: {}{}",
                fails_in.join(", "),
                if passes.is_empty() { String::new() } else { format!("   (passes in: {})", passes.join(", ")) }
            ));
            row.details.splice(0..0, head);
        }
        if let Some(dir) = svg_dir {
            let _ = std::fs::create_dir_all(dir);
            let path = format!("{dir}/{:02}-{}.svg", index + 1, case.name);
            let _ = std::fs::write(path, svg_for(case, &blocks));
        }
        rows.push(row);
    }
    rows
}

fn render(rows: &[Row], title: &str, secs: f64) -> String {
    let mut s = String::new();
    s.push_str(&format!("\n==== {title} ====\n"));
    s.push_str(&format!(
        " {:>3}  {:<52} {:<5} {:<4} {:<4} {:<4} {:<4} {:<5} {:<5} {:>8}  {}\n",
        "#", "case", "class", "x1", "x0.5", "x2", "x3.7", "shuf1", "shuf2", "ms", "note"
    ));
    for r in rows {
        // MUST rows: ok / FAIL per variant. AMBIGUOUS rows: rec = the x1 answer is recorded, = / DIFF = the answer of
        // that variant is the same as / different from the x1 answer (scale and slice order must not change it).
        let amb = r.strength == Strength::Ambiguous;
        let cell = |name: &str| -> &str {
            match (r.cells.iter().find(|(n, _)| *n == name), amb) {
                (Some((_, true)), false) => "ok",
                (Some((_, false)), false) => "FAIL",
                (Some((_, true)), true) => "=",
                (Some((_, false)), true) => "DIFF",
                (None, _) => "-",
            }
        };
        let note = if amb {
            format!("{}{}", r.note, if r.invariant_failed { "  [CONTRACT VIOLATION]" } else { "" })
        } else if r.invariant_failed {
            "[contract violation]".to_string()
        } else {
            String::new()
        };
        s.push_str(&format!(
            " {:>3}  {:<52} {:<5} {:<4} {:<4} {:<4} {:<4} {:<5} {:<5} {:>8.1}  {}\n",
            r.index,
            r.name,
            r.strength.tag(),
            if amb { "rec" } else { cell("x1") },
            cell("x0.5"),
            cell("x2"),
            cell("x3.7"),
            cell("shuf1"),
            cell("shuf2"),
            r.ms,
            note
        ));
    }
    let must: Vec<&Row> = rows.iter().filter(|r| r.strength == Strength::Must).collect();
    let amb: Vec<&Row> = rows.iter().filter(|r| r.strength == Strength::Ambiguous).collect();
    let inv = rows.iter().filter(|r| r.invariant_failed).count();
    s.push_str(&format!(
        "\nMUST: {} cases, {} pass every variant, {} fail.   AMBIGUOUS: {} cases recorded ({} unstable under scale/shuffle).   contract-invariant failures: {}   ({:.1} s)\n",
        must.len(),
        must.iter().filter(|r| r.pass).count(),
        must.iter().filter(|r| !r.pass).count(),
        amb.len(),
        amb.iter().filter(|r| !r.unstable.is_empty()).count(),
        inv,
        secs
    ));
    let failing: Vec<&&Row> = must.iter().filter(|r| !r.pass).collect();
    if !failing.is_empty() {
        s.push_str("\n---- MUST failures (first failing variant; PAGIFY_BLOCKS_VERBOSE=1 shows every variant) ----\n");
        for r in failing {
            s.push_str(&format!("[{}] {}\n", r.index, r.name));
            for d in &r.details {
                s.push_str(&format!("  {d}\n"));
            }
        }
    }
    let bad_amb: Vec<&&Row> = amb.iter().filter(|r| !r.pass).collect();
    if !bad_amb.is_empty() {
        s.push_str("\n---- AMBIGUOUS cases that violate the contract ----\n");
        for r in bad_amb {
            s.push_str(&format!("[{}] {}\n", r.index, r.name));
            for d in &r.details {
                s.push_str(&format!("  {d}\n"));
            }
        }
    }
    if !amb.is_empty() {
        s.push_str("\n---- AMBIGUOUS: what detect returned (nothing asserted; a human should look) ----\n");
        for r in &amb {
            s.push_str(&format!("[{}] {}  {}\n", r.index, r.name, r.note));
            s.push_str(&wrap(&r.intent, 118, "    "));
            for d in r.details.iter().filter(|d| d.starts_with("recorded") || d.starts_with("reading")) {
                s.push_str(&format!("    {d}\n"));
            }
        }
    }
    s
}

// =====================================================================================================
// 7. stand-in detectors and the perfect oracle (used only by the self-check)
// =====================================================================================================

fn mk_block(members: &[&Frag], outl: &[(Shape, f32)], why: &'static str) -> Block {
    struct L<'a> {
        base: f32,
        objs: Vec<&'a Frag>,
        outl: Vec<Shape>,
    }
    let tol = |f: &Frag| 0.3 * if f.size.is_finite() && f.size > 0.0 { f.size } else { 1.0 };
    let mut ms: Vec<&Frag> = members.to_vec();
    ms.sort_by(|a, b| a.baseline.total_cmp(&b.baseline).then(nrect(a).0.total_cmp(&nrect(b).0)));
    let mut lines: Vec<L> = vec![];
    for f in &ms {
        match lines.last_mut() {
            Some(l) if (f.baseline - l.base).abs() <= tol(f) => l.objs.push(f),
            _ => lines.push(L { base: f.baseline, objs: vec![f], outl: vec![] }),
        }
    }
    let ref_size = {
        let mut sz: Vec<f32> = ms.iter().map(|f| f.size).filter(|s| s.is_finite() && *s > 0.0).collect();
        sz.sort_by(|a, b| a.total_cmp(b));
        sz.get(sz.len() / 2).copied().unwrap_or(8.0)
    };
    for (s, b) in outl {
        match lines.iter_mut().find(|l| (l.base - b).abs() <= 0.3 * ref_size) {
            Some(l) => l.outl.push(*s),
            None => lines.push(L { base: *b, objs: vec![], outl: vec![*s] }),
        }
    }
    lines.sort_by(|a, b| a.base.total_cmp(&b.base));
    let mut out_lines = vec![];
    let mut bb = (f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY);
    for l in lines {
        let mut objs = l.objs;
        objs.sort_by(|a, b| nrect(a).0.total_cmp(&nrect(b).0));
        let mut os = l.outl;
        os.sort_by(|a, b| a.left.total_cmp(&b.left));
        let mut r = (f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY);
        for f in &objs {
            let q = nrect(f);
            r = (r.0.min(q.0), r.1.min(q.1), r.2.max(q.2), r.3.max(q.3));
        }
        for s in &os {
            r = (r.0.min(s.left), r.1.min(s.top), r.2.max(s.right), r.3.max(s.bottom));
        }
        if !r.0.is_finite() || !r.2.is_finite() {
            r = (0.0, 0.0, 0.0, 0.0);
        }
        bb = (bb.0.min(r.0), bb.1.min(r.1), bb.2.max(r.2), bb.3.max(r.3));
        out_lines.push(Line {
            objects: objs.iter().map(|f| f.object).collect(),
            outlined: os.iter().map(|s| s.object).collect(),
            left: r.0,
            top: r.1,
            right: r.2,
            bottom: r.3,
            baseline: l.base,
        });
    }
    Block { lines: out_lines, starts_because: why, left: bb.0, top: bb.1, right: bb.2, bottom: bb.3 }
}

/// Stand-in (a): one block per fragment.
fn standin_one_per_fragment(frags: &[Frag], _shapes: &[Shape]) -> Vec<Block> {
    frags.iter().map(|f| mk_block(&[f], &[], "stand-in")).collect()
}

/// Stand-in (b): everything on the page in one block (rotated fragments alone, as the contract demands).
fn standin_everything_in_one(frags: &[Frag], _shapes: &[Shape]) -> Vec<Block> {
    let upright: Vec<&Frag> = frags.iter().filter(|f| !f.rotated).collect();
    let mut out = vec![];
    if !upright.is_empty() {
        out.push(mk_block(&upright, &[], "stand-in"));
    }
    for f in frags.iter().filter(|f| f.rotated) {
        out.push(mk_block(&[f], &[], "stand-in"));
    }
    out
}

/// Stand-in (c): a plausible but NAIVE detector, to show the corpus separates naive from careful handling. Lines are
/// cut at a fixed 1.0 em gap (no corridor evidence), pieces are chained vertically when the next baseline is within
/// 1.6 em and the pieces overlap horizontally and share (font, stem, size); no rules, indents, short lines, outlines.
fn standin_naive(frags: &[Frag], _shapes: &[Shape]) -> Vec<Block> {
    struct Piece {
        members: Vec<usize>,
        l: f32,
        r: f32,
        base: f32,
        size: f32,
    }
    let mut idx: Vec<usize> = (0..frags.len()).filter(|&i| !frags[i].rotated).collect();
    idx.sort_by(|&a, &b| frags[a].baseline.total_cmp(&frags[b].baseline).then(nrect(&frags[a]).0.total_cmp(&nrect(&frags[b]).0)));
    let mut rows: Vec<Vec<usize>> = vec![];
    for &i in &idx {
        let f = &frags[i];
        match rows.last_mut() {
            Some(r) if (f.baseline - frags[r[0]].baseline).abs() <= 0.3 * f.size.abs().max(1e-3) => r.push(i),
            _ => rows.push(vec![i]),
        }
    }
    let mut pieces: Vec<Piece> = vec![];
    for r in &mut rows {
        r.sort_by(|&a, &b| nrect(&frags[a]).0.total_cmp(&nrect(&frags[b]).0));
        let mut cur: Vec<usize> = vec![];
        let flush = |cur: &mut Vec<usize>, pieces: &mut Vec<Piece>| {
            if cur.is_empty() {
                return;
            }
            let l = cur.iter().map(|&i| nrect(&frags[i]).0).fold(f32::INFINITY, f32::min);
            let rr = cur.iter().map(|&i| nrect(&frags[i]).2).fold(f32::NEG_INFINITY, f32::max);
            let f = &frags[cur[0]];
            pieces.push(Piece { members: std::mem::take(cur), l, r: rr, base: f.baseline, size: f.size });
        };
        for &i in r.iter() {
            if let Some(&p) = cur.last() {
                if nrect(&frags[i]).0 - nrect(&frags[p]).2 > 1.0 * frags[i].size {
                    flush(&mut cur, &mut pieces);
                }
            }
            cur.push(i);
        }
        flush(&mut cur, &mut pieces);
    }
    let mut parent: Vec<usize> = (0..pieces.len()).collect();
    fn find(p: &mut Vec<usize>, x: usize) -> usize {
        let mut r = x;
        while p[r] != r {
            r = p[r];
        }
        let mut c = x;
        while p[c] != r {
            let n = p[c];
            p[c] = r;
            c = n;
        }
        r
    }
    for b in 0..pieces.len() {
        let mut best: Option<usize> = None;
        for a in 0..pieces.len() {
            let (pa, pb) = (&pieces[a], &pieces[b]);
            let dy = pb.base - pa.base;
            if !(dy > 0.0 && dy <= 1.6 * pb.size) {
                continue;
            }
            let ov = pa.r.min(pb.r) - pa.l.max(pb.l);
            if !(ov >= 0.5 * (pa.r - pa.l).min(pb.r - pb.l)) {
                continue;
            }
            let (fa, fb) = (&frags[pa.members[0]], &frags[pb.members[0]]);
            if fa.font != fb.font || fa.stem != fb.stem || (fa.size - fb.size).abs() > 0.05 * fb.size.abs() {
                continue;
            }
            if best.map(|x| pieces[x].base < pa.base).unwrap_or(true) {
                best = Some(a);
            }
        }
        if let Some(a) = best {
            let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
            parent[rb] = ra;
        }
    }
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for p in 0..pieces.len() {
        let root = find(&mut parent, p);
        groups.entry(root).or_default().extend(pieces[p].members.iter().copied());
    }
    let mut out: Vec<Block> = groups
        .values()
        .map(|g| {
            let ms: Vec<&Frag> = g.iter().map(|&i| &frags[i]).collect();
            mk_block(&ms, &[], "naive")
        })
        .collect();
    for f in frags.iter().filter(|f| f.rotated) {
        out.push(mk_block(&[f], &[], "naive"));
    }
    out
}

#[derive(Clone, Copy, PartialEq)]
enum Mutation {
    None,
    /// cut the largest expected block in two halves
    Split,
    /// glue the first two expected blocks together
    Merge,
    /// ONE fragment is wrong: the last fragment of the largest expected block stands alone (the finest mutation)
    Peel,
}

/// Ways of breaking the CONTRACT (not the grouping): the invariant checks must flag every one of them.
#[derive(Clone, Copy)]
enum Breaker {
    /// one text object is missing from every block
    DropOne,
    /// the first block is reported twice
    DuplicateBlock,
    /// every second call returns the blocks in the opposite order
    Flaky,
    /// a block box that does not enclose its members
    ShrinkBox,
}

/// The perfect detector for a case: returns exactly the expected grouping (optionally mutated).
fn perfect(case: &Case, mutation: Mutation) -> impl Fn(&[Frag], &[Shape]) -> Vec<Block> + '_ {
    move |frags: &[Frag], shapes: &[Shape]| {
        let by_obj: HashMap<usize, &Frag> = frags.iter().map(|f| (f.object, f)).collect();
        let shp: HashMap<usize, &Shape> = shapes.iter().map(|s| (s.object, s)).collect();
        let mut groups: Vec<Vec<usize>> =
            case.expect.iter().map(|g| g.iter().map(|l| case.page.id(l)).collect()).collect();
        match mutation {
            Mutation::None => {}
            Mutation::Split => {
                if let Some((i, _)) = groups.iter().enumerate().filter(|(_, g)| g.len() >= 2).max_by_key(|(_, g)| g.len()) {
                    let half = groups[i].len() / 2;
                    let tail = groups[i].split_off(half);
                    groups.push(tail);
                }
            }
            Mutation::Merge => {
                if groups.len() >= 2 {
                    let g1 = groups.remove(1);
                    groups[0].extend(g1);
                }
            }
            Mutation::Peel => {
                if let Some((i, _)) = groups.iter().enumerate().filter(|(_, g)| g.len() >= 2).max_by_key(|(_, g)| g.len()) {
                    let last = groups[i].pop().unwrap();
                    groups.push(vec![last]);
                }
            }
        }
        let listed: HashSet<usize> = groups.iter().flatten().copied().collect();
        for f in frags {
            if !listed.contains(&f.object) {
                groups.push(vec![f.object]);
            }
        }
        groups
            .into_iter()
            .map(|g| {
                let members: Vec<&Frag> = g.iter().filter_map(|o| by_obj.get(o).copied()).collect();
                let mut outl: Vec<(Shape, f32)> = vec![];
                for (anchor, shape, _) in &case.outlines {
                    if g.contains(&case.page.id(anchor)) {
                        let sid = case.page.id(shape);
                        if let Some(s) = shp.get(&sid) {
                            // outlined words are built as top = base - 0.72 em, bottom = base + 0.22 em:
                            // recover the baseline from the shape itself so it holds at every scale
                            let base = s.bottom - 0.22 * (s.bottom - s.top) / 0.94;
                            outl.push((**s, base));
                        }
                    }
                }
                mk_block(&members, &outl, "oracle")
            })
            .collect()
    }
}

// =====================================================================================================
// 8. SVG dump (human inspection): detected blocks coloured, expected blocks dashed
// =====================================================================================================

fn xml(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '&' => "&amp;".to_string(),
            '<' => "&lt;".to_string(),
            '>' => "&gt;".to_string(),
            '\u{2}' => "~".to_string(),
            c if (c as u32) < 32 => " ".to_string(),
            c => c.to_string(),
        })
        .collect()
}

fn svg_for(case: &Case, blocks: &[Block]) -> String {
    let page = &case.page;
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for f in page.frags.iter().filter(|f| finite_frag(f)) {
        let r = nrect(f);
        x0 = x0.min(r.0);
        y0 = y0.min(r.1);
        x1 = x1.max(r.2);
        y1 = y1.max(r.3);
    }
    for s in &page.shapes {
        x0 = x0.min(s.left);
        y0 = y0.min(s.top);
        x1 = x1.max(s.right);
        y1 = y1.max(s.bottom);
    }
    if x0 > x1 {
        (x0, y0, x1, y1) = (0.0, 0.0, 100.0, 100.0);
    }
    let pad = 10.0;
    let (w, h) = (x1 - x0 + 2.0 * pad, y1 - y0 + 2.0 * pad);
    let k = (1100.0 / w).min(4.0);
    let mut s = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' viewBox='{} {} {} {}' width='{}' height='{}' font-family='Arial,sans-serif'>\n<rect x='{}' y='{}' width='{}' height='{}' fill='white'/>\n",
        x0 - pad, y0 - pad, w, h, w * k, h * k, x0 - pad, y0 - pad, w, h
    );
    let mut block_of: HashMap<usize, usize> = HashMap::new();
    for (i, b) in blocks.iter().enumerate() {
        for o in b.objects() {
            block_of.insert(o, i);
        }
    }
    let hue = |i: usize| (i * 47) % 360;
    for sh in &page.shapes {
        s.push_str(&format!(
            "<rect x='{}' y='{}' width='{}' height='{}' fill='none' stroke='#888' stroke-width='0.3'/>\n",
            sh.left,
            sh.top,
            sh.right - sh.left,
            sh.bottom - sh.top
        ));
    }
    for f in page.frags.iter().filter(|f| finite_frag(f)) {
        let r = nrect(f);
        let i = block_of.get(&f.object).copied().unwrap_or(0);
        s.push_str(&format!(
            "<rect x='{}' y='{}' width='{}' height='{}' fill='hsl({},70%,50%)' fill-opacity='0.18' stroke='hsl({},70%,35%)' stroke-width='0.15'/>\n",
            r.0, r.1, r.2 - r.0, r.3 - r.1, hue(i), hue(i)
        ));
        if f.size > 0.0 && r.2 > r.0 {
            s.push_str(&format!(
                "<text x='{}' y='{}' font-size='{}' textLength='{}' lengthAdjust='spacingAndGlyphs' fill='black' xml:space='preserve'>{}</text>\n",
                r.0,
                f.baseline,
                f.size,
                r.2 - r.0,
                xml(f.text.trim())
            ));
        }
    }
    for (i, b) in blocks.iter().enumerate() {
        if [b.left, b.top, b.right, b.bottom].iter().all(|v| v.is_finite()) {
            s.push_str(&format!(
                "<rect x='{}' y='{}' width='{}' height='{}' fill='none' stroke='hsl({},80%,40%)' stroke-width='0.8'/>\n",
                b.left - 0.8,
                b.top - 0.8,
                b.right - b.left + 1.6,
                b.bottom - b.top + 1.6,
                hue(i)
            ));
        }
    }
    let fmap = case.frag_by_label();
    let groups: &Vec<Vec<String>> = match case.strength {
        Strength::Must => &case.expect,
        Strength::Ambiguous => &case.readings[0].1,
    };
    for g in groups {
        let mut b = (f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY);
        for l in g {
            if let Some(f) = fmap.get(l.as_str()) {
                if finite_frag(f) {
                    let r = nrect(f);
                    b = (b.0.min(r.0), b.1.min(r.1), b.2.max(r.2), b.3.max(r.3));
                }
            }
        }
        if b.0.is_finite() {
            s.push_str(&format!(
                "<rect x='{}' y='{}' width='{}' height='{}' fill='none' stroke='black' stroke-width='0.35' stroke-dasharray='2 1.5'/>\n",
                b.0 - 1.6,
                b.1 - 1.6,
                b.2 - b.0 + 3.2,
                b.3 - b.1 + 3.2
            ));
        }
    }
    s.push_str("</svg>\n");
    s
}

// =====================================================================================================
// 9. the tests
// =====================================================================================================

fn report_path() -> String {
    format!("{}/blocks_synthetic_report.txt", env!("CARGO_TARGET_TMPDIR"))
}

fn write_report(text: &str) {
    emit(text);
    let _ = std::fs::write(report_path(), text);
}

fn must_cases(cases: &[Case]) -> Vec<&Case> {
    cases.iter().filter(|c| c.strength == Strength::Must).collect()
}

/// The 30,000-fragment case runs in release builds (a debug build of a superlinear pass could take minutes) unless
/// PAGIFY_BLOCKS_FORCE_HEAVY=1; PAGIFY_BLOCKS_SKIP_HEAVY=1 always skips it.
fn heavy_enabled() -> bool {
    !env_flag("PAGIFY_BLOCKS_SKIP_HEAVY") && (!cfg!(debug_assertions) || env_flag("PAGIFY_BLOCKS_FORCE_HEAVY"))
}

/// How many OTHER seeds every case is instantiated with besides the canonical one (PAGIFY_BLOCKS_SEEDS, default 24,
/// 0 = off). The layouts are identical, the random words, object cuts and justified gaps are not.
fn seeds_count() -> usize {
    env_str("PAGIFY_BLOCKS_SEEDS").and_then(|v| v.parse().ok()).unwrap_or(24)
}

/// Re-runs every MUST case with `n` other seeds (all 6 variants each) and reports how many seeds each case survives.
/// Returns (text, names of the MUST cases that fail on some seed, number of contract violations on other seeds).
fn seeds_report(rows: &[Row], only: Option<&str>, n: usize) -> (String, Vec<&'static str>, usize) {
    let mut tally: BTreeMap<&'static str, (usize, usize, Vec<u64>)> = BTreeMap::new();
    let mut violations = 0;
    // PAGIFY_BLOCKS_SEED_BASE=b runs the seeds b+1 ..= b+n instead of 1 ..= n: a HOLDOUT. The seeds 1..24 (the default) and
    // 1..100 are printed in every report, so a detector can have been tuned on them; a base nobody has seen is the fair test.
    let base: u64 = env_str("PAGIFY_BLOCKS_SEED_BASE").and_then(|v| v.parse().ok()).unwrap_or(0);
    for k in 1..=n as u64 {
        let salt_value = base + k;
        let cases = all_cases_seeded(false, salt_value);
        for r in run_corpus(&detect, &cases, only, false, None) {
            violations += usize::from(r.invariant_failed);
            if r.strength == Strength::Must {
                let e = tally.entry(r.name).or_insert((0, 0, vec![]));
                e.1 += 1;
                if r.pass {
                    e.0 += 1;
                } else {
                    e.2.push(salt_value);
                }
            }
        }
    }
    let mut s = format!(
        "\n---- other seeds: every case re-instantiated with {n} other seeds, salts {}..{} (same layouts, other words, cuts and gaps; each seed run in all 6 variants) ----\n",
        base + 1,
        base + n as u64
    );
    let mut bad = vec![];
    let mut shown = false;
    for r in rows.iter().filter(|r| r.strength == Strength::Must) {
        let Some((ok, total, failed)) = tally.get(r.name) else { continue };
        if ok < total {
            bad.push(r.name);
        }
        if ok < total || !r.pass {
            if !shown {
                s.push_str(" #   case                                                 canonical   other seeds ok   failing seeds\n");
                shown = true;
            }
            s.push_str(&format!(
                " {:>3} {:<52} {:<11} {:>2}/{:<2}            {}\n",
                r.index,
                r.name,
                if r.pass { "ok" } else { "FAIL" },
                ok,
                total,
                if failed.is_empty() { "-".to_string() } else { failed.iter().map(|k| k.to_string()).collect::<Vec<_>>().join(",") }
            ));
        }
    }
    let always = tally.values().filter(|(ok, total, _)| ok == total).count();
    s.push_str(&format!(
        "{} of {} MUST cases pass on every one of the {n} other seeds; {} fail on at least one; contract violations on other seeds: {violations}\n",
        always,
        tally.len(),
        bad.len()
    ));
    (s, bad, violations)
}

/// Scale sweep (PAGIFY_BLOCKS_SWEEP=1): every MUST case at many scales, to expose a threshold that is written in
/// absolute points instead of em. Only cases whose verdict changes with the scale are listed.
fn sweep_report(cases: &[Case], only: Option<&str>) -> String {
    const SCALES: [f32; 12] = [0.2, 0.35, 0.5, 0.7, 1.0, 1.4, 2.0, 2.8, 3.7, 5.0, 8.0, 12.0];
    let mut s = String::from("\n---- scale sweep (PAGIFY_BLOCKS_SWEEP=1): MUST cases whose verdict depends on the scale ----\n      case                                                 ");
    for k in SCALES {
        s.push_str(&format!("{:>6}", format!("x{k}")));
    }
    s.push('\n');
    let mut listed = 0;
    let mut total = 0;
    for (i, c) in cases.iter().enumerate() {
        if c.strength != Strength::Must || c.heavy || only.map(|o| !c.name.contains(o)).unwrap_or(false) {
            continue;
        }
        total += 1;
        let verdicts: Vec<bool> = SCALES
            .iter()
            .map(|&k| evaluate(c, &detect, &Variant { name: "sweep", scale: k, shuffle: None }, true).problems.is_empty())
            .collect();
        if verdicts.iter().all(|v| *v) || verdicts.iter().all(|v| !*v) {
            continue;
        }
        listed += 1;
        s.push_str(&format!(" {:>3}  {:<52} ", i + 1, c.name));
        for v in verdicts {
            s.push_str(&format!("{:>6}", if v { "ok" } else { "FAIL" }));
        }
        s.push('\n');
    }
    s.push_str(&format!("{listed} of {total} MUST cases change verdict with the scale (the others pass at every scale or fail at every scale)\n"));
    s
}

/// Prints every case with its strength, intent and (AMBIGUOUS) competing readings; no detector involved.
#[test]
fn corpus_catalogue() {
    let cases = all_cases(true);
    let mut s = String::from("\n==== corpus catalogue ====\n");
    for (i, c) in cases.iter().enumerate() {
        s.push_str(&format!("{:>3} {:<5} {}\n      {}\n", i + 1, c.strength.tag(), c.name, c.intent));
        for (n, g) in &c.readings {
            s.push_str(&format!("      reading '{n}': {} block(s)\n", g.len()));
        }
        s.push_str(&format!(
            "      {} text objects, {} shapes, {} expected block(s){}\n",
            c.page.frags.len(),
            c.page.shapes.len(),
            c.expect.len(),
            if c.free.is_empty() { String::new() } else { format!(", {} free object(s)", c.free.len()) }
        ));
    }
    if let Some(path) = env_str("PAGIFY_BLOCKS_CATALOGUE") {
        let _ = std::fs::write(path, &s);
    }
    if env_flag("PAGIFY_BLOCKS_VERBOSE") {
        emit(&s);
    }
}

/// The expectations themselves: partitions of the page, no overlapping expected blocks, enough cases.
#[test]
fn corpus_is_internally_consistent() {
    let cases = all_cases(true);
    let mut problems: Vec<String> = vec![];
    let mut names = HashSet::new();
    for c in &cases {
        if !names.insert(c.name) {
            problems.push(format!("{}: duplicate case name", c.name));
        }
        if c.intent.trim().len() < 40 {
            problems.push(format!("{}: intent is missing or too short", c.name));
        }
        for p in c.consistency() {
            problems.push(format!("{}: {p}", c.name));
        }
    }
    // the other seeds must be just as consistent (the generator, not luck, makes the expectations a partition)
    let extra_seeds = seeds_count().clamp(4, 200) as u64;
    for k in 1..=extra_seeds {
        for c in &all_cases_seeded(false, k) {
            for p in c.consistency() {
                problems.push(format!("{} [seed {k}]: {p}", c.name));
            }
        }
    }
    let must = must_cases(&cases).len();
    let amb = cases.len() - must;
    emit(&format!(
        "\ncorpus: {} cases ({} MUST, {} AMBIGUOUS), {} problems (consistency also checked on {extra_seeds} other seeds)\n",
        cases.len(),
        must,
        amb,
        problems.len()
    ));
    assert!(problems.is_empty(), "corpus inconsistencies:\n{}", problems.join("\n"));
    assert!(cases.len() >= 40, "the corpus must hold at least 40 cases, has {}", cases.len());
    assert!(amb >= 5, "the corpus must record at least 5 AMBIGUOUS cases, has {amb}");
}

/// Proof that the corpus is not vacuous and the checker can both pass and fail:
///  * a perfect detector built from the expectations passes every MUST case in every variant,
///  * "split the biggest block" and "merge the first two blocks" mutants fail every case they apply to,
///  * the two trivial stand-ins (one block per fragment / everything in one block) fail most MUST cases.
#[test]
fn oracle_self_check_standins_mutants_and_perfect() {
    let cases = all_cases(true);
    let must: Vec<&Case> = must_cases(&cases);
    let mut report = String::new();
    let mut problems: Vec<String> = vec![];

    // perfect detector
    let mut perfect_fail = vec![];
    for c in &must {
        let det = perfect(c, Mutation::None);
        let variants: Vec<Variant> = if c.heavy { LIGHT_VARIANTS.to_vec() } else { FULL_VARIANTS.to_vec() };
        for v in &variants {
            let ev = evaluate(c, &det, v, true);
            if !ev.problems.is_empty() {
                perfect_fail.push(format!("{} [{}]: {}", c.name, v.name, ev.problems.join(" | ")));
            }
        }
    }
    report.push_str(&format!(
        "\nSELF-CHECK perfect detector (built from the expectations): passes {}/{} MUST cases in all variants\n",
        must.len() - perfect_fail.iter().map(|s| s.split(" [").next().unwrap()).collect::<HashSet<_>>().len(),
        must.len()
    ));
    problems.extend(perfect_fail);
    // ... and on other seeds of the same layouts (a different random instance must be judged just as cleanly)
    report.push_str(&self_check_seeded(&must, &mut problems));

    // mutants
    report.push_str(&self_check_mutants(&must, &mut problems));

    // the CONTRACT checks must be able to fail too: four ways of breaking the contract, each derived from the perfect answer
    report.push_str(&self_check_breakers(&must, &mut problems));

    // stand-ins: (a), (b) trivial; (c) plausible but naive (reported, and must stay clearly below the perfect score)
    let mut summary = vec![];
    for (name, det) in [
        ("a: one block per fragment", &standin_one_per_fragment as &dyn Fn(&[Frag], &[Shape]) -> Vec<Block>),
        ("b: everything in one block", &standin_everything_in_one),
        ("c: naive row-gap + vertical-chain detector", &standin_naive),
    ] {
        let (mut pass_x1, mut fail_x1, mut pass_all) = (vec![], vec![], 0usize);
        for c in &must {
            let ev = evaluate(c, det, &V_X1, true);
            if ev.problems.is_empty() {
                pass_x1.push(c.name);
            } else {
                fail_x1.push(c.name);
            }
            let variants: Vec<Variant> = if c.heavy { LIGHT_VARIANTS.to_vec() } else { FULL_VARIANTS.to_vec() };
            if variants.iter().all(|v| evaluate(c, det, v, true).problems.is_empty()) {
                pass_all += 1;
            }
        }
        report.push_str(&format!(
            "SELF-CHECK stand-in ({name}): passes {}/{} MUST cases at x1, {}/{} in all variants. Passing at x1: {}\n",
            pass_x1.len(),
            must.len(),
            pass_all,
            must.len(),
            if pass_x1.is_empty() { "none".to_string() } else { pass_x1.join(", ") }
        ));
        report.push_str(&format!("    stand-in ({name}) FAILS {} MUST cases at x1: {}\n", fail_x1.len(), fail_x1.join(", ")));
        summary.push((name, pass_x1.len()));
    }
    emit(&report);
    for (name, n) in &summary {
        // the trivial stand-ins must fail at least 75 % of the MUST cases, the naive one at least 40 % of them (it is
        // meant to pass the many cases that are traps for MORE elaborate logic: indents, short lines, outlines, ...)
        let limit = if name.starts_with('c') { must.len() * 3 / 5 } else { must.len() / 4 };
        if *n > limit {
            problems.push(format!("stand-in ({name}) passes {n} of {} MUST cases: the corpus is too easy", must.len()));
        }
    }
    assert!(problems.is_empty(), "oracle self-check failed:\n{}", problems.join("\n"));
}

/// The real detector against the corpus. Prints the table; with PAGIFY_BLOCKS_ASSERT=1 MUST failures and
/// contract violations fail the test.
#[test]
fn detect_against_synthetic_corpus() {
    let t0 = Instant::now();
    let include_heavy = heavy_enabled();
    // PAGIFY_BLOCKS_SALT=k runs the whole table on another instance of the layouts (the canonical one is 0): this is
    // how a failing seed of the "other seeds" section is reproduced, rendered (PAGIFY_BLOCKS_SVG) and looked at.
    let salt_value: u64 = env_str("PAGIFY_BLOCKS_SALT").and_then(|v| v.parse().ok()).unwrap_or(0);
    let cases = all_cases_seeded(include_heavy, salt_value);
    let only = env_str("PAGIFY_BLOCKS_ONLY");
    let svg = env_str("PAGIFY_BLOCKS_SVG");
    let rows = run_corpus(&detect, &cases, only.as_deref(), env_flag("PAGIFY_BLOCKS_VERBOSE"), svg.as_deref());
    let title = format!(
        "pagify_shell::blocks::detect against the independent synthetic corpus (instance {})",
        if salt_value == 0 { "0 = canonical".to_string() } else { format!("{salt_value} = another seed") }
    );
    let mut text = render(&rows, &title, t0.elapsed().as_secs_f64());
    let failing_must = rows.iter().filter(|r| r.strength == Strength::Must && !r.pass).count();
    let violations = rows.iter().filter(|r| r.invariant_failed).count();
    let enforce = env_flag("PAGIFY_BLOCKS_ASSERT");
    let n_seeds = seeds_count();
    let (mut seed_bad, mut seed_violations) = (vec![], 0);
    if n_seeds > 0 {
        let (t, bad, v) = seeds_report(&rows, only.as_deref(), n_seeds);
        text.push_str(&t);
        seed_bad = bad;
        seed_violations = v;
    }
    if env_flag("PAGIFY_BLOCKS_SWEEP") {
        text.push_str(&sweep_report(&cases, only.as_deref()));
    }
    text.push_str(&format!(
        "\n{} (PAGIFY_BLOCKS_ASSERT={}). heavy 30,000-fragment case: {}. other seeds: {}. report file: {}\n",
        if enforce { "ENFORCING" } else { "report only, nothing is asserted" },
        if enforce { 1 } else { 0 },
        if include_heavy { "run" } else { "SKIPPED (debug build or PAGIFY_BLOCKS_SKIP_HEAVY)" },
        n_seeds,
        report_path()
    ));
    write_report(&text);
    if enforce {
        let names: Vec<&str> = rows.iter().filter(|r| r.strength == Strength::Must && !r.pass).map(|r| r.name).collect();
        assert!(
            failing_must == 0 && violations == 0 && seed_bad.is_empty() && seed_violations == 0,
            "{failing_must} MUST case(s) fail ({}) and {violations} case(s) violate the contract; {} MUST case(s) fail on other seeds ({}), {seed_violations} contract violation(s) on other seeds; see the table above",
            names.join(", "),
            seed_bad.len(),
            seed_bad.join(", ")
        );
    }
}

// =====================================================================================================
// 10. the cases
// =====================================================================================================

/// Datasheet column geometry at 8 pt: column width 118.8, gutter 12.0 (= 1.5 em; the real one is 11.7).
const COL_W: f32 = 118.8;
const GUT: f32 = 12.0;
const X1: f32 = 187.0;
const X2: f32 = X1 + COL_W + GUT;
const X3: f32 = X2 + COL_W + GUT;

type CaseFn = fn() -> Case;

/// The canonical instance of every case (the one the tables and the case numbers are about).
fn all_cases(include_heavy: bool) -> Vec<Case> {
    all_cases_seeded(include_heavy, 0)
}

/// The same layouts instantiated with another seed: other random words, other object cuts, other justified gaps.
/// Guards against a detector that was tuned on the exact numbers of the canonical instance.
fn all_cases_seeded(include_heavy: bool, salt_value: u64) -> Vec<Case> {
    SALT.with(|s| s.set(salt_value));
    let v = all_cases_inner(include_heavy);
    SALT.with(|s| s.set(0));
    v
}

fn all_cases_inner(include_heavy: bool) -> Vec<Case> {
    let fns: &[CaseFn] = &[
        // --- prose ---
        c_ragged_blank_lines,
        c_ragged_spacing_6pt,
        c_justified_indents,
        c_ragged_indent_after_full_line,
        c_hanging_entries,
        // --- columns ---
        c_justified_two_columns,
        c_justified_two_columns_short_last_lines,
        c_justified_three_columns,
        c_justified_three_columns_mixed,
        c_ragged_two_columns,
        c_columns_out_of_phase,
        c_three_columns_mixed_pitch,
        c_gutter_stray_fragment,
        c_gutter_divider_bars,
        // --- headings ---
        c_heading_font_and_stem_only,
        c_heading_no_gap_either_side,
        c_heading_after_flush_last_line,
        c_heading_unknown_stem_face_names,
        c_heading_font_id_only,
        c_heading_larger_extra_space,
        c_heading_wrapped_two_lines,
        c_heading_under_heading,
        c_stacked_bold_lines_same_style,
        // --- lists ---
        c_bulleted_list,
        c_numbered_list_hanging,
        c_numbered_list_number_in_text,
        c_nested_bullets,
        c_single_line_bullets,
        c_address_lines,
        // --- tables, label/value ---
        c_table_rows_between_rules,
        c_table_two_line_rows,
        c_table_grid,
        c_table_no_rules,
        c_label_value_no_rules,
        c_three_part_header_line,
        // --- figures, boxes, rules ---
        c_figure_labels_and_caption,
        c_chart_tick_labels,
        c_text_in_path_box_beside_text,
        c_text_in_four_rule_box_below_text,
        c_underlined_line_same_style,
        c_rule_between_paragraphs,
        // --- outlined words and lines ---
        c_outlined_word_hole,
        c_outlined_whole_line,
        c_outlined_words_at_line_ends,
        c_blank_line_without_outline,
        c_outlined_heading_above,
        c_datasheet_heading_paragraph_outlines,
        // --- marks, footnotes, drop caps, run-in leads, emphasis ---
        c_superscript_marks,
        c_footnote_star_marker,
        c_footnotes_with_numerals,
        c_drop_cap,
        c_run_in_lead_single,
        c_run_in_leads_consecutive,
        c_inline_emphasis_minority,
        c_inline_larger_word,
        c_same_face_and_stem_different_font_ids,
        c_colour_only_change,
        // --- hyphenation, alignment, spacing, size ---
        c_hyphen_u0002,
        c_hyphen_plain,
        c_centred_title,
        c_right_aligned_block,
        c_size_change_only,
        c_line_spacing_neighbours,
        // --- rotation, stream order, twins ---
        c_rotated_text,
        c_stream_order_row_major,
        c_stream_order_reversed,
        c_stream_order_random,
        c_twin_fragments,
        // --- degenerate and extreme ---
        c_one_fragment,
        c_empty_page,
        c_shapes_only_page,
        c_degenerate_fragments,
        c_inverted_rects,
        c_duplicate_object_ids,
        c_single_long_line,
        // --- more realistic traps ---
        c_narrow_column,
        c_natural_short_line,
        c_widow_lines,
        c_single_letter_objects,
        c_letter_spaced_heading,
        c_text_wrap_around_figure,
        c_indented_block_quote,
        c_nested_form_shapes_ignored,
        c_toc_leader_lines,
        c_baseline_jitter,
        c_skewed_scan,
        // --- added by R2: layouts that real documents have and the first 87 cases did not cover ---
        c_one_object_per_line,
        c_word_per_object_columns,
        c_full_width_heading_and_footer_paragraph,
        c_right_aligned_paragraphs,
        c_double_spaced_manuscript,
        c_tight_leading,
        c_two_columns_tight_gutter,
        c_inline_underlines,
        c_highlight_boxes,
        // --- composite pages: several cues at once, each of which is covered alone above ---
        c_business_letter,
        c_two_column_paper,
        c_quotation_table,
        c_datasheet_left_column_twin,
        // @@CASE-LIST@@
    ];
    let mut v: Vec<Case> = fns.iter().map(|f| f()).collect();
    if include_heavy {
        v.push(c_heavy_30000());
        v.push(c_heavy_one_column());
        v.push(c_heavy_one_row());
    }
    v
}

// ---------------------------------------------------------------------------------------------------
// prose
// ---------------------------------------------------------------------------------------------------

fn c_ragged_blank_lines() -> Case {
    let mut c = Case::must(
        "ragged-paragraphs-blank-line-gaps",
        "One column of left-aligned (ragged-right) prose, 10 pt on a 12 pt pitch, three paragraphs separated by ONE blank \
         line (24 pt from the last baseline of one to the first of the next). The first paragraph ends in a nearly full \
         line, so the blank line is the only cue there. A person sees three paragraphs: three editable blocks.",
    );
    let st = doc(10.0);
    let a = c.page.para(P::new("a", &st).at(72.0, 100.0).w(400.0).n(4).pitch(12.0).frags(3, 6).last(0.97));
    let b = c.page.para(P::new("b", &st).at(72.0, a.below(2.0)).w(400.0).n(5).pitch(12.0).frags(3, 6).last(0.52));
    let d = c.page.para(P::new("c", &st).at(72.0, b.below(2.0)).w(400.0).n(3).pitch(12.0).frags(3, 6).last(0.88));
    c.expect = vec![a.all(), b.all(), d.all()];
    c
}

fn c_ragged_spacing_6pt() -> Case {
    let mut c = Case::must(
        "ragged-paragraph-spacing-6pt-no-blank-line",
        "Word-processor layout: 11 pt text on a 13.2 pt pitch, paragraphs separated only by 6 pt of paragraph spacing \
         (19.2 pt between the baselines instead of 13.2). The middle paragraph ends in a nearly full line. A person sees \
         three paragraphs: the extra air between them is the standard 'space after'.",
    );
    let st = doc(11.0);
    let a = c.page.para(P::new("a", &st).at(72.0, 100.0).w(420.0).n(5).pitch(13.2).frags(3, 6).last(0.55));
    let b = c.page.para(P::new("b", &st).at(72.0, a.below(1.0) + 6.0).w(420.0).n(4).pitch(13.2).frags(3, 6).last(0.93));
    let d = c.page.para(P::new("c", &st).at(72.0, b.below(1.0) + 6.0).w(420.0).n(6).pitch(13.2).frags(3, 6).last(0.40));
    c.expect = vec![a.all(), b.all(), d.all()];
    c
}

fn c_justified_indents() -> Case {
    let mut c = Case::must(
        "justified-first-line-indents-no-gaps",
        "Book-style justified text: three paragraphs, every first line indented 18 pt (1.6 em), constant 13.2 pt pitch, no \
         blank line and no extra spacing. Each paragraph ends with a short last line, as justified text always does. A \
         person reads the indents (and the short lines) as paragraph starts: three blocks.",
    );
    let st = doc(11.0);
    let a = c.page.para(P::new("a", &st).at(72.0, 100.0).w(420.0).n(6).pitch(13.2).just().indent(18.0).last(0.35));
    let b = c.page.para(P::new("b", &st).at(72.0, a.below(1.0)).w(420.0).n(4).pitch(13.2).just().indent(18.0).last(0.60));
    let d = c.page.para(P::new("c", &st).at(72.0, b.below(1.0)).w(420.0).n(7).pitch(13.2).just().indent(18.0).last(0.25));
    c.expect = vec![a.all(), b.all(), d.all()];
    c
}

fn c_ragged_indent_after_full_line() -> Case {
    let mut c = Case::must(
        "ragged-indent-is-the-only-cue",
        "Left-aligned text, 10 pt on a 12 pt pitch, three paragraphs with a 22 pt (2 em) first-line indent and NO extra \
         space; the last line of each paragraph is 90-96 % of the measure, so nothing but the indent says a new paragraph \
         begins. A person reads the classic typographic convention: three paragraphs.",
    );
    let st = doc(10.0);
    let a = c.page.para(P::new("a", &st).at(72.0, 100.0).w(400.0).n(5).pitch(12.0).frags(3, 6).indent(22.0).last(0.96));
    let b = c.page.para(P::new("b", &st).at(72.0, a.below(1.0)).w(400.0).n(5).pitch(12.0).frags(3, 6).indent(22.0).last(0.92));
    let d = c.page.para(P::new("c", &st).at(72.0, b.below(1.0)).w(400.0).n(4).pitch(12.0).frags(3, 6).indent(22.0).last(0.90));
    c.expect = vec![a.all(), b.all(), d.all()];
    c
}

fn c_hanging_entries() -> Case {
    let mut c = Case::must(
        "hanging-indent-entries-no-gaps",
        "Reference-list layout: three entries, first line flush left, continuation lines hang 18 pt in, constant pitch, no \
         blank lines. A person sees three entries (the flush line starts each one): three blocks.",
    );
    let st = doc(10.0);
    let a = c.page.para(P::new("a", &st).at(72.0, 100.0).w(380.0).n(3).pitch(12.0).frags(2, 5).hang(18.0).last(0.50));
    let b = c.page.para(P::new("b", &st).at(72.0, a.below(1.0)).w(380.0).n(3).pitch(12.0).frags(2, 5).hang(18.0).last(0.80));
    let d = c.page.para(P::new("c", &st).at(72.0, b.below(1.0)).w(380.0).n(2).pitch(12.0).frags(2, 5).hang(18.0).last(0.60));
    c.expect = vec![a.all(), b.all(), d.all()];
    c
}

// ---------------------------------------------------------------------------------------------------
// columns (datasheet geometry: 8 pt, 9.6 pt pitch, 118.8 pt columns, 12 pt gutter, justified, 3-8 objects/line)
// ---------------------------------------------------------------------------------------------------

fn c_justified_two_columns() -> Case {
    let mut c = Case::must(
        "justified-two-columns-datasheet-geometry",
        "Two justified columns of 8 pt text (118.8 pt wide, 12 pt = 1.5 em gutter, 9.6 pt pitch), every visual line cut \
         into 3-8 text objects, justified word gaps from 0.3 to 1.2 em (so some gaps at object boundaries are nearly as \
         wide as the gutter), both columns on the same baselines. A person sees two columns, each one paragraph: two blocks.",
    );
    let st = body(8.0);
    let l = c.page.para(P::new("l", &st).at(X1, 359.0).w(COL_W).n(12).pitch(9.6).just().gap(0.3, 1.2).frags(3, 8).last(0.55));
    let r = c.page.para(P::new("r", &st).at(X2, 359.0).w(COL_W).n(9).pitch(9.6).just().gap(0.3, 1.2).frags(3, 8).last(0.40));
    c.expect = vec![l.all(), r.all()];
    c
}

fn c_justified_two_columns_short_last_lines() -> Case {
    let mut c = Case::must(
        "justified-two-columns-break-only-by-short-last-line",
        "Two justified columns, each holding TWO paragraphs. There is no blank line and no indent: the only mark of a \
         paragraph break is that a justified line stops short of the margin (45-62 % of the measure) and the next line \
         starts at the left margin again. A person knows a justified line that stops short is a paragraph end: four blocks.",
    );
    let st = body(8.0);
    let a = c.page.para(P::new("a", &st).at(X1, 359.0).w(COL_W).n(7).pitch(9.6).just().frags(3, 8).last(0.45));
    let b = c.page.para(P::new("b", &st).at(X1, a.below(1.0)).w(COL_W).n(6).pitch(9.6).just().frags(3, 8).last(0.62));
    let d = c.page.para(P::new("c", &st).at(X2, 359.0).w(COL_W).n(5).pitch(9.6).just().frags(3, 8).last(0.33));
    let e = c.page.para(P::new("d", &st).at(X2, d.below(1.0)).w(COL_W).n(8).pitch(9.6).just().frags(3, 8).last(0.50));
    c.expect = vec![a.all(), b.all(), d.all(), e.all()];
    c
}

fn c_justified_three_columns() -> Case {
    let mut c = Case::must(
        "justified-three-columns-one-paragraph-each",
        "Three justified columns exactly like the datasheet (118.8 pt measure, 12 pt gutter, 8 pt text, 3-8 objects per \
         line, gaps up to 1.2 em), each column one 14-line paragraph. Only the gutters separate the paragraphs: a justified \
         word gap (<= 1.2 em) must not be mistaken for a gutter (1.5 em), nor the other way round. Three blocks.",
    );
    let st = body(8.0);
    let a = c.page.para(P::new("a", &st).at(X1, 359.0).w(COL_W).n(14).pitch(9.6).just().frags(3, 8).last(0.50));
    let b = c.page.para(P::new("b", &st).at(X2, 359.0).w(COL_W).n(14).pitch(9.6).just().frags(3, 8).last(0.70));
    let d = c.page.para(P::new("c", &st).at(X3, 359.0).w(COL_W).n(14).pitch(9.6).just().frags(3, 8).last(0.35));
    c.expect = vec![a.all(), b.all(), d.all()];
    c
}

fn c_justified_three_columns_mixed() -> Case {
    let mut c = Case::must(
        "justified-three-columns-mixed-paragraph-breaks",
        "Three justified columns with a different number of paragraphs in each (2, 1 and 3), the breaks marked only by \
         short last lines at constant pitch. Six paragraphs, six blocks; no block may cross a gutter and none may swallow \
         the next paragraph of its own column.",
    );
    let st = body(8.0);
    let a = c.page.para(P::new("a", &st).at(X1, 359.0).w(COL_W).n(8).pitch(9.6).just().frags(3, 8).last(0.50));
    let b = c.page.para(P::new("b", &st).at(X1, a.below(1.0)).w(COL_W).n(5).pitch(9.6).just().frags(3, 8).last(0.70));
    let d = c.page.para(P::new("c", &st).at(X2, 359.0).w(COL_W).n(13).pitch(9.6).just().frags(3, 8).last(0.40));
    let e = c.page.para(P::new("d", &st).at(X3, 359.0).w(COL_W).n(4).pitch(9.6).just().frags(3, 8).last(0.60));
    let f = c.page.para(P::new("e", &st).at(X3, e.below(1.0)).w(COL_W).n(4).pitch(9.6).just().frags(3, 8).last(0.45));
    let g = c.page.para(P::new("f", &st).at(X3, f.below(1.0)).w(COL_W).n(5).pitch(9.6).just().frags(3, 8).last(0.30));
    c.expect = vec![a.all(), b.all(), d.all(), e.all(), f.all(), g.all()];
    c
}

fn c_ragged_two_columns() -> Case {
    let mut c = Case::must(
        "ragged-two-columns-uneven-right-edges",
        "Two left-aligned columns of 10 pt text (140 pt wide, 15 pt = 1.5 em gutter). The left column's lines end anywhere \
         from 70 to 100 % of the measure, so the white space to the right column varies line by line from 15 pt to 55 pt. \
         A person sees two columns: two blocks.",
    );
    let st = doc(10.0);
    let a = c.page.para(P::new("a", &st).at(72.0, 100.0).w(140.0).n(11).pitch(12.0).frags(2, 5).last(0.55));
    let b = c.page.para(P::new("b", &st).at(72.0 + 140.0 + 15.0, 100.0).w(140.0).n(10).pitch(12.0).frags(2, 5).last(0.75));
    c.expect = vec![a.all(), b.all()];
    c
}

fn c_columns_out_of_phase() -> Case {
    let mut c = Case::must(
        "columns-baselines-half-pitch-out-of-phase",
        "Two justified columns of the same style whose baselines are out of phase by half a pitch (4.8 pt): no baseline of \
         the left column coincides with one of the right column, so a line of one column sits exactly between two lines \
         of the other. A person sees two columns: two blocks.",
    );
    let st = body(8.0);
    let a = c.page.para(P::new("a", &st).at(X1, 359.0).w(COL_W).n(12).pitch(9.6).just().frags(3, 8).last(0.55));
    let b = c.page.para(P::new("b", &st).at(X2, 359.0 + 4.8).w(COL_W).n(12).pitch(9.6).just().frags(3, 8).last(0.45));
    c.expect = vec![a.all(), b.all()];
    c
}

fn c_three_columns_mixed_pitch() -> Case {
    let mut c = Case::must(
        "three-columns-different-sizes-and-pitches",
        "Three justified columns that share nothing but the page: 8 pt on a 9.6 pt pitch, then 10 pt on a 12.5 pt pitch \
         starting 7 pt lower, then 8 pt on a 9.6 pt pitch starting 3 pt lower. Baselines of neighbouring columns drift \
         through each other. A person sees three columns: three blocks.",
    );
    let (s8, s10) = (body(8.0), body(10.0));
    let a = c.page.para(P::new("a", &s8).at(50.0, 100.0).w(118.8).n(14).pitch(9.6).just().frags(3, 8).last(0.5));
    let b = c.page.para(P::new("b", &s10).at(183.8, 107.0).w(140.0).n(11).pitch(12.5).just().frags(3, 7).last(0.6));
    let d = c.page.para(P::new("c", &s8).at(338.8, 103.0).w(118.8).n(14).pitch(9.6).just().frags(3, 8).last(0.4));
    c.expect = vec![a.all(), b.all(), d.all()];
    c
}

fn c_gutter_stray_fragment() -> Case {
    let mut c = Case::must(
        "gutter-stray-fragment-between-two-columns",
        "Two justified columns with a lone mark (an asterisk object) sitting in the middle of the 12 pt gutter on one text \
         line, 4.5 pt from each column. On that row the left line, the mark and the right line look like one long line with \
         tight gaps. A person sees two columns and a stray mark: the two columns must stay two blocks (the mark may stand \
         alone or join one of them: it is not asserted).",
    );
    let st = body(8.0);
    let a = c.page.para(P::new("a", &st).at(X1, 359.0).w(COL_W).n(12).pitch(9.6).just().frags(3, 8).last(0.55));
    let b = c.page.para(P::new("b", &st).at(X2, 359.0).w(COL_W).n(12).pitch(9.6).just().frags(3, 8).last(0.45));
    let x = X1 + COL_W + (GUT - ink_w("*", 8.0)) / 2.0;
    c.page.text("stray", &st, x, a.bases[5], "*");
    c.free = vec!["stray".to_string()];
    c.expect = vec![a.all(), b.all()];
    c
}

fn c_gutter_divider_bars() -> Case {
    let mut c = Case::must(
        "gutter-text-divider-made-of-bar-glyphs",
        "Two justified columns separated by a vertical divider drawn as text: one '|' object per line, centred in the \
         gutter (so the gutter corridor is filled on every row). A person sees two columns and a divider: the two columns \
         are two blocks (the bars may form their own block, stand alone or join a column: not asserted).",
    );
    let st = body(8.0);
    let a = c.page.para(P::new("a", &st).at(X1, 359.0).w(COL_W).n(12).pitch(9.6).just().frags(3, 8).last(0.55));
    let b = c.page.para(P::new("b", &st).at(X2, 359.0).w(COL_W).n(12).pitch(9.6).just().frags(3, 8).last(0.45));
    let x = X1 + COL_W + (GUT - ink_w("|", 8.0)) / 2.0;
    for i in 0..12 {
        let l = format!("bar{i}");
        c.page.text(&l, &st, x, a.bases[i], "|");
        c.free.push(l);
    }
    c.expect = vec![a.all(), b.all()];
    c
}

// ---------------------------------------------------------------------------------------------------
// headings
// ---------------------------------------------------------------------------------------------------

/// intro paragraph + sections (heading, justified paragraph); heading 14 pt below the previous text, body at the
/// body pitch. Returns the expected blocks. Style, not pitch, is what separates heading and body here.
fn sections(c: &mut Case, h: &St, b: &St, sections: &[(&str, usize, f32)]) -> Vec<Vec<String>> {
    let mut expect = vec![];
    let intro = c.page.para(P::new("i", b).at(X1, 359.0).w(COL_W).n(5).pitch(9.6).just().frags(3, 8).last(0.45));
    expect.push(intro.all());
    let mut last = intro.last_base();
    for (k, (title, n, frac)) in sections.iter().enumerate() {
        let hb = last + 14.0;
        let hl = c.page.cut_line(&format!("h{k}"), h, X1, hb, title, 4);
        let p = c.page.para(P::new(&format!("p{k}"), b).at(X1, hb + 9.6).w(COL_W).n(*n).pitch(9.6).just().frags(3, 8).last(*frac));
        last = p.last_base();
        expect.push(hl);
        expect.push(p.all());
    }
    expect
}

fn c_heading_font_and_stem_only() -> Case {
    let mut c = Case::must(
        "heading-differs-only-by-font-id-and-stem",
        "The datasheet pattern. A one-line heading and its body paragraph have the SAME size (8 pt), colour, face name \
         ('Montserrat-Thin' for every weight, as on page 1) and pitch (heading to first body line = 9.6 pt, the body \
         pitch); only the font id and the measured stem (ExtraBold 198 vs Light 51) differ. 14 pt above each heading, \
         nothing below. A person sees bold headings: every heading is its own block, every paragraph its own block.",
    );
    let e = sections(&mut c, &head(8.0), &body(8.0), &[("The Light Source - COB", 6, 0.5), ("Light Quality", 9, 0.62), ("Optic Component", 4, 0.4)]);
    c.expect = e;
    c
}

fn c_heading_no_gap_either_side() -> Case {
    let mut c = Case::must(
        "heading-between-paragraphs-no-gap-either-side",
        "A bold heading sits between two paragraphs at EXACTLY the body pitch above and below (9.6 pt), same size and \
         colour; the paragraph above ends in a short line. Nothing but the weight (font id and stem) marks the heading. \
         A person sees paragraph, heading, paragraph: three blocks.",
    );
    let (h, b) = (head(8.0), body(8.0));
    let a = c.page.para(P::new("a", &b).at(X1, 359.0).w(COL_W).n(5).pitch(9.6).just().frags(3, 8).last(0.5));
    let hl = c.page.cut_line("h", &h, X1, a.below(1.0), "Thermal management", 3);
    let d = c.page.para(P::new("b", &b).at(X1, a.below(2.0)).w(COL_W).n(5).pitch(9.6).just().frags(3, 8).last(0.45));
    c.expect = vec![a.all(), hl, d.all()];
    c
}

fn c_heading_after_flush_last_line() -> Case {
    let mut c = Case::must(
        "heading-after-flush-last-line-no-gap",
        "As the previous case, but the paragraph above ends in a FULL justified line (it happens when the last line \
         fills the measure), so the only cue between that line and the bold heading below it is the weight. A person \
         sees the heading by its weight: paragraph, heading, paragraph.",
    );
    let (h, b) = (head(8.0), body(8.0));
    let a = c.page.para(P::new("a", &b).at(X1, 359.0).w(COL_W).n(5).pitch(9.6).just().frags(3, 8));
    let hl = c.page.cut_line("h", &h, X1, a.below(1.0), "Fixing Device", 3);
    let d = c.page.para(P::new("b", &b).at(X1, a.below(2.0)).w(COL_W).n(5).pitch(9.6).just().frags(3, 8).last(0.45));
    c.expect = vec![a.all(), hl, d.all()];
    c
}

fn c_heading_unknown_stem_face_names() -> Case {
    let mut c = Case::must(
        "heading-unknown-stem-different-face-names",
        "The datasheet pattern with the weight signal unavailable: stems are unknown (None) for every object, but the \
         face names differ honestly ('Montserrat-ExtraBold' vs 'Montserrat-Light', different font ids). Same size, \
         colour and pitch. A person sees bold headings: every heading its own block, every paragraph its own block.",
    );
    let h = St::new(0, None, "Montserrat-ExtraBold", 8.0);
    let b = St::new(1, None, "Montserrat-Light", 8.0);
    let e = sections(&mut c, &h, &b, &[("Electrical power connections", 4, 0.55), ("Fixing Device", 5, 0.4)]);
    c.expect = e;
    c
}

fn c_heading_font_id_only() -> Case {
    let mut c = Case::ambiguous(
        "heading-differs-only-by-font-id",
        "Weak evidence: heading and body have the same size, colour, pitch, an unknown stem (None) and the same useless \
         face name; ONLY the font id differs. Reading 'heading-split': a different font program on one line is a \
         heading, so heading and paragraph are separate blocks. Reading 'heading-merged': one font resource can be \
         embedded twice by some producers, so nothing here proves the line is a heading and the whole thing is one \
         block.",
    );
    let h = St::new(0, None, "Montserrat-Thin", 8.0);
    let b = St::new(1, None, "Montserrat-Thin", 8.0);
    let intro = c.page.para(P::new("i", &b).at(X1, 359.0).w(COL_W).n(5).pitch(9.6).just().frags(3, 8).last(0.45));
    let hl = c.page.cut_line("h", &h, X1, intro.last_base() + 14.0, "Light Quality", 3);
    let p = c.page.para(P::new("p", &b).at(X1, intro.last_base() + 14.0 + 9.6).w(COL_W).n(7).pitch(9.6).just().frags(3, 8).last(0.5));
    c.reading("heading-split", vec![intro.all(), hl.clone(), p.all()]);
    c.reading("heading-merged", vec![intro.all(), cat(vec![hl, p.all()])]);
    c
}

fn c_heading_larger_extra_space() -> Case {
    let mut c = Case::must(
        "heading-larger-with-extra-space-above",
        "A conventional document heading: 13 pt bold between 10 pt paragraphs, 30 pt of baseline distance above it \
         (about a blank line and a half) and 17 pt below. A person sees paragraph, heading, paragraph: three blocks.",
    );
    let a = c.page.para(P::new("a", &doc(10.0)).at(72.0, 100.0).w(400.0).n(5).pitch(12.0).frags(3, 6).last(0.90));
    let hb = a.last_base() + 30.0;
    let hl = c.page.cut_line("h", &doc_bold(13.0), 72.0, hb, "Installation and maintenance", 3);
    let b = c.page.para(P::new("b", &doc(10.0)).at(72.0, hb + 17.0).w(400.0).n(6).pitch(12.0).frags(3, 6).last(0.50));
    c.expect = vec![a.all(), hl, b.all()];
    c
}

fn c_heading_wrapped_two_lines() -> Case {
    let mut c = Case::must(
        "heading-wrapped-over-two-lines",
        "A long 14 pt bold heading wrapped over two lines (16.8 pt pitch, second line short), followed 26 pt later by a \
         10 pt paragraph. A person sees one two-line heading and one paragraph: two blocks, the heading's two lines \
         together.",
    );
    let h = c.page.para(P::new("h", &doc_bold(14.0)).at(72.0, 100.0).w(300.0).n(2).pitch(16.8).frags(2, 4).last(0.45));
    let b = c.page.para(P::new("b", &doc(10.0)).at(72.0, h.last_base() + 26.0).w(400.0).n(5).pitch(12.0).frags(3, 6).last(0.70));
    c.expect = vec![h.all(), b.all()];
    c
}

fn c_heading_under_heading() -> Case {
    let mut c = Case::must(
        "heading-directly-under-heading-different-style",
        "A 16 pt bold title, 15.5 pt below it an italic 10.5 pt subtitle, 16 pt below that a 10 pt paragraph. Size, weight \
         and posture all change from line to line. A person sees title, subtitle and paragraph: three blocks.",
    );
    let t = c.page.cut_line("t", &doc_bold(16.0), 72.0, 100.0, "Photometric Data", 2);
    let s = c.page.cut_line("s", &doc_italic(10.5), 72.0, 115.5, "Luminaire performance summary", 3);
    let b = c.page.para(P::new("b", &doc(10.0)).at(72.0, 131.5).w(400.0).n(4).pitch(12.0).frags(3, 6).last(0.60));
    c.expect = vec![t, s, b.all()];
    c
}

fn c_stacked_bold_lines_same_style() -> Case {
    let mut c = Case::ambiguous(
        "stacked-bold-lines-same-style-title-and-model",
        "The datasheet's 'BRIEF INTRODUCTION:' / 'HS150LC-2006-A' pair: two bold lines of the same size at the body \
         pitch, the second one underlined by a rule that opens a table, then a body paragraph. Reading 'two-headings': \
         a section title and a model number are two different things. Reading 'one-two-line-heading': two stacked \
         lines of identical style are one heading.",
    );
    let (h, b) = (head(8.0), body(8.0));
    let l1 = c.page.cut_line("l1", &h, 20.0, 200.0, "BRIEF INTRODUCTION:", 2);
    let l2 = c.page.cut_line("l2", &h, 20.0, 209.6, "HS150LC-2006-A", 1);
    c.page.rule_h("rule", 17.0, 172.0, 212.4);
    let p = c.page.para(P::new("p", &b).at(20.0, 223.0).w(150.0).n(3).pitch(9.6).frags(2, 4).last(0.5));
    c.reading("two-headings", vec![l1.clone(), l2.clone(), p.all()]);
    c.reading("one-two-line-heading", vec![cat(vec![l1, l2]), p.all()]);
    c
}

// ---------------------------------------------------------------------------------------------------
// lists
// ---------------------------------------------------------------------------------------------------

fn c_bulleted_list() -> Case {
    let mut c = Case::must(
        "bulleted-list-bullet-as-its-own-object",
        "Five bullet items of 10 pt text on a 12 pt pitch with NO space between items; every bullet is a separate text \
         object 18 pt left of its text, continuation lines align with the text. Items 2, 3 and 5 end in nearly full \
         lines, so only the bullets say where an item starts. A person sees five items (bullet and text together): five \
         blocks.",
    );
    let st = doc(10.0);
    let spec = [(2usize, 0.50f32), (2, 0.95), (3, 0.90), (1, 0.35), (2, 0.97)];
    let mut base = 100.0;
    for (k, (n, last)) in spec.iter().enumerate() {
        let id = format!("i{k}");
        let bl = format!("{id}.b");
        c.page.text(&bl, &st, 72.0, base, "\u{2022}");
        let p = c.page.para(P::new(&id, &st).at(90.0, base).w(380.0).n(*n).pitch(12.0).frags(2, 5).last(*last));
        base = p.below(1.0);
        c.expect.push(cat(vec![lab(&bl), p.all()]));
    }
    c
}

fn c_numbered_list_hanging() -> Case {
    let mut c = Case::must(
        "numbered-list-number-object-and-hanging-lines",
        "Four numbered items ('1.' ... '4.' as separate objects at the left margin, text indented 18 pt, continuation lines \
         hanging under the text), constant 12 pt pitch, no blank lines; items 2 and 4 end in nearly full lines. A person \
         sees four items: four blocks.",
    );
    let st = doc(10.0);
    let spec = [(3usize, 0.50f32), (2, 0.95), (3, 0.40), (2, 0.92)];
    let mut base = 100.0;
    for (k, (n, last)) in spec.iter().enumerate() {
        let id = format!("n{k}");
        let nl = format!("{id}.b");
        c.page.text(&nl, &st, 72.0, base, &format!("{}.", k + 1));
        let p = c.page.para(P::new(&id, &st).at(90.0, base).w(380.0).n(*n).pitch(12.0).frags(2, 5).last(*last));
        base = p.below(1.0);
        c.expect.push(cat(vec![lab(&nl), p.all()]));
    }
    c
}

fn c_numbered_list_number_in_text() -> Case {
    let mut c = Case::must(
        "numbered-list-number-inside-the-text-object",
        "Three numbered items where the number is part of the first words of the first line ('1. The ...'), continuation \
         lines hanging 18 pt in, constant pitch, no blank lines; item 2 ends in a nearly full line. A person sees three \
         items: three blocks.",
    );
    let st = doc(10.0);
    let a = c.page.para(P::new("a", &st).at(72.0, 100.0).w(400.0).n(3).pitch(12.0).frags(2, 5).hang(18.0).lead("1.").last(0.5));
    let b = c.page.para(P::new("b", &st).at(72.0, a.below(1.0)).w(400.0).n(2).pitch(12.0).frags(2, 5).hang(18.0).lead("2.").last(0.96));
    let d = c.page.para(P::new("c", &st).at(72.0, b.below(1.0)).w(400.0).n(3).pitch(12.0).frags(2, 5).hang(18.0).lead("3.").last(0.4));
    c.expect = vec![a.all(), b.all(), d.all()];
    c
}

fn c_nested_bullets() -> Case {
    let mut c = Case::must(
        "nested-bullets-each-item-is-a-block",
        "A two-level bullet list: level-1 items ('\u{2022}', text at 90 pt) and level-2 items ('-', text at 114 pt) \
         interleaved, constant pitch, no blank lines, mixed one- and two-line items. A person sees one item per bullet, \
         sub-items not swallowed by their parent: six blocks.",
    );
    let st = doc(10.0);
    let spec = [(1u8, 2usize, 0.5f32), (2, 1, 0.6), (2, 2, 0.4), (1, 1, 0.9), (2, 1, 0.5), (1, 2, 0.45)];
    let mut base = 100.0;
    for (k, (lvl, n, last)) in spec.iter().enumerate() {
        let id = format!("i{k}");
        let bl = format!("{id}.b");
        let (bx, tx, bullet) = if *lvl == 1 { (72.0, 90.0, "\u{2022}") } else { (100.0, 114.0, "-") };
        c.page.text(&bl, &st, bx, base, bullet);
        let p = c.page.para(P::new(&id, &st).at(tx, base).w(470.0 - tx).n(*n).pitch(12.0).frags(2, 5).last(*last));
        base = p.below(1.0);
        c.expect.push(cat(vec![lab(&bl), p.all()]));
    }
    c
}

fn c_single_line_bullets() -> Case {
    let mut c = Case::must(
        "single-line-bullet-items",
        "Six one-line bullet items on a constant 12 pt pitch (a feature list). Every line starts with its own bullet \
         object. A person sees six items: six blocks, not one six-line paragraph.",
    );
    let st = doc(10.0);
    let items = [
        "Recessed mounting",
        "IP44 rated housing",
        "Dimmable driver included",
        "Five year warranty",
        "Tool-free installation",
        "Available in three colours",
    ];
    for (k, t) in items.iter().enumerate() {
        let base = 100.0 + 12.0 * k as f32;
        let id = format!("i{k}");
        let bl = format!("{id}.b");
        c.page.text(&bl, &st, 72.0, base, "\u{2022}");
        let l = c.page.cut_line(&id, &st, 90.0, base, t, 2 + k % 2);
        c.expect.push(cat(vec![lab(&bl), l]));
    }
    c
}

fn c_address_lines() -> Case {
    let mut c = Case::ambiguous(
        "address-lines-without-bullets",
        "Four short left-aligned lines on a constant 12 pt pitch, no bullets, no rules (an address / contact block). \
         Reading 'one-block': a person calls the block of lines one unit. Reading 'four-lines': every line is its own \
         little item (each can be edited alone).",
    );
    let st = doc(10.0);
    let lines = ["HSI Lighting FZE", "Office 12, Business Bay", "Dubai, United Arab Emirates", "+971 4 555 0123"];
    let mut ls = vec![];
    for (k, t) in lines.iter().enumerate() {
        ls.push(c.page.cut_line(&format!("l{k}"), &st, 72.0, 100.0 + 12.0 * k as f32, t, 2));
    }
    c.reading("one-block", vec![ls.concat()]);
    c.reading("four-lines", ls);
    c
}

// ---------------------------------------------------------------------------------------------------
// tables and label/value pairs
// ---------------------------------------------------------------------------------------------------

fn c_table_rows_between_rules() -> Case {
    let mut c = Case::must(
        "table-label-value-rows-between-horizontal-rules",
        "The datasheet's specification table: a bold title row and eight label/value rows, each 11 pt high, a thin \
         horizontal rule under every row, NO vertical rules. Labels start at x=20, values at x=112, so the gap between \
         label and value runs from 20 pt to 80 pt (3-10 em). A person sees every underlined row as one unit: nine \
         blocks, label and value together.",
    );
    let (ls, vs) = (semibold(7.5), body(7.5));
    let rows = [
        ("Power Input", "40W"),
        ("System Lumen", "4000lm"),
        ("COB Efficacy", "Max 180 lm/Watt"),
        ("CCT", "3500K"),
        ("CRI", ">90"),
        ("Driver Power Factor", ">90"),
        ("Protection", "Short Circuit, Overload"),
        ("Sound Rating", "Class A"),
    ];
    let mut top = 211.0;
    let hdr = c.page.cut_line("hdr", &head(8.0), 20.0, top + 8.0, "HS150LC-2006-A", 2);
    c.page.rule_h("hdr.rule", 17.0, 172.0, top + 11.0);
    c.expect.push(hdr);
    top += 11.0;
    for (k, (l, v)) in rows.iter().enumerate() {
        let base = top + 8.0;
        let mut row = c.page.cut_line(&format!("r{k}l"), &ls, 20.0, base, l, 2);
        row.extend(c.page.cut_line(&format!("r{k}v"), &vs, 112.0, base, v, 2));
        c.page.rule_h(&format!("r{k}.rule"), 17.0, 172.0, top + 11.0);
        c.expect.push(row);
        top += 11.0;
    }
    c
}

fn c_table_two_line_rows() -> Case {
    let mut c = Case::must(
        "table-two-line-rows-between-rules",
        "Specification rows whose label is on one line and whose value is on the line BELOW it ('Power Input:' / '40W'), \
         one rule under the value, 21 pt from row to row. A person sees every ruled two-line row as one unit: five \
         blocks of two lines each.",
    );
    let (ls, vs) = (semibold(7.5), body(7.5));
    let rows = [
        ("Power Input:", "40W"),
        ("Current Input:", "1050mA respectively for power"),
        ("Operation temperature:", "< 60 degrees C"),
        ("Optional integrated dimmable driver:", "1-10v / DMX / Dali"),
        ("Tunable function:", "2700K - 6500K"),
    ];
    let mut y = 219.0;
    for (k, (l, v)) in rows.iter().enumerate() {
        let mut row = c.page.cut_line(&format!("r{k}l"), &ls, 20.0, y, l, 3);
        row.extend(c.page.cut_line(&format!("r{k}v"), &vs, 20.0, y + 9.6, v, 3));
        c.page.rule_h(&format!("r{k}.rule"), 17.0, 172.0, y + 9.6 + 3.4);
        c.expect.push(row);
        y += 21.0;
    }
    c
}

fn c_table_grid() -> Case {
    let mut c = Case::must(
        "table-grid-vertical-and-horizontal-rules-cells-separate",
        "A 3 x 4 grid table: four vertical and five horizontal rules, 120 pt wide, 30 pt high cells, 8 pt text padded only \
         2.5 pt from the rules (text of neighbouring cells can be just 5 pt = 0.6 em apart, closer than a justified word \
         gap), one or two lines per cell on shared baselines, the top row bold. The vertical rules are the only cue. A \
         person sees twelve cells: twelve blocks.",
    );
    let xs = [40.0f32, 160.0, 280.0, 400.0];
    let ys = [100.0f32, 130.0, 160.0, 190.0, 220.0];
    for (i, x) in xs.iter().enumerate() {
        c.page.rule_v(&format!("vr{i}"), *x, ys[0], ys[4]);
    }
    for (j, y) in ys.iter().enumerate() {
        c.page.rule_h(&format!("hr{j}"), xs[0], xs[3], *y);
    }
    for r in 0..4 {
        for cc in 0..3 {
            let st = if r == 0 { head(8.0) } else { body(8.0) };
            let n = if (r + cc) % 2 == 0 { 2 } else { 1 };
            let p = c.page.para(
                P::new(&format!("t{r}{cc}"), &st).at(xs[cc] + 2.5, ys[r] + 6.0 + 5.9).w(115.0).n(n).pitch(9.6).frags(1, 3).last(if n == 1 { 0.92 } else { 0.55 }),
            );
            c.expect.push(p.all());
        }
    }
    c
}

fn c_table_no_rules() -> Case {
    let mut c = Case::ambiguous(
        "table-aligned-columns-without-any-rule",
        "Three aligned columns of short entries (a header row and three data rows, 130 pt apart, 12 pt pitch) with no \
         rules at all. Reading 'rows': every table row is one unit. Reading 'cells': every cell is its own block. \
         Reading 'columns': every column of stacked cells is one block.",
    );
    let xs = [72.0f32, 202.0, 332.0];
    let data = [
        ["Model", "Power", "Flux"],
        ["VEGA 100", "20 W", "2400 lm"],
        ["VEGA 150", "30 W", "3600 lm"],
        ["VEGA 200", "40 W", "4800 lm"],
    ];
    let mut cells: Vec<Vec<Vec<String>>> = vec![];
    for (r, row) in data.iter().enumerate() {
        let st = if r == 0 { head(9.0) } else { body(9.0) };
        let mut rr = vec![];
        for (cc, t) in row.iter().enumerate() {
            rr.push(c.page.cut_line(&format!("c{r}{cc}"), &st, xs[cc], 100.0 + 12.0 * r as f32, t, 1 + (r + cc) % 2));
        }
        cells.push(rr);
    }
    c.reading("rows", cells.iter().map(|r| r.concat()).collect());
    c.reading("cells", cells.iter().flatten().cloned().collect());
    c.reading("columns", (0..3).map(|cc| cells.iter().map(|r| r[cc].clone()).collect::<Vec<_>>().concat()).collect());
    c
}

fn c_label_value_no_rules() -> Case {
    let mut c = Case::ambiguous(
        "label-value-rows-without-rules",
        "Five 'Label: value' rows, label in semibold at x=72, value in regular at x=190 on the same baseline, 12 pt pitch, \
         no rules or underlines. Reading 'rows': each label with its value is one block. Reading 'columns': the labels \
         form one block and the values another. Reading 'cells': every label and every value is a block of its own.",
    );
    let (ls, vs) = (semibold(9.0), body(9.0));
    let rows = [("Power:", "40 W"), ("Voltage:", "230 V"), ("Colour temperature:", "3000 K"), ("CRI:", ">90"), ("IP rating:", "44")];
    let (mut lab_l, mut val_l) = (vec![], vec![]);
    for (k, (l, v)) in rows.iter().enumerate() {
        let base = 100.0 + 12.0 * k as f32;
        lab_l.push(c.page.cut_line(&format!("l{k}"), &ls, 72.0, base, l, 2));
        val_l.push(c.page.cut_line(&format!("v{k}"), &vs, 190.0, base, v, 1));
    }
    c.reading("rows", (0..5).map(|k| cat(vec![lab_l[k].clone(), val_l[k].clone()])).collect());
    c.reading("columns", vec![lab_l.concat(), val_l.concat()]);
    c.reading("cells", lab_l.iter().chain(val_l.iter()).cloned().collect());
    c
}

fn c_three_part_header_line() -> Case {
    let mut c = Case::ambiguous(
        "three-part-running-header-line",
        "A page header: a left-aligned name, a centred title and a right-aligned page number on one baseline, 150-200 pt \
         apart. Reading 'three-blocks': three independent items. Reading 'one-line': one header line.",
    );
    let st = body(9.0);
    let l = c.page.cut_line("hl", &st, 72.0, 40.0, "HSI Lighting", 2);
    let wc = ink_w("VEGA series datasheet", 9.0);
    let m = c.page.cut_line("hc", &st, 297.5 - wc / 2.0, 40.0, "VEGA series datasheet", 3);
    let wr = ink_w("Page 3 of 12", 9.0);
    let r = c.page.cut_line("hr", &st, 523.0 - wr, 40.0, "Page 3 of 12", 2);
    c.reading("three-blocks", vec![l.clone(), m.clone(), r.clone()]);
    c.reading("one-line", vec![cat(vec![l, m, r])]);
    c
}

// ---------------------------------------------------------------------------------------------------
// figures, boxes, rules
// ---------------------------------------------------------------------------------------------------

fn c_figure_labels_and_caption() -> Case {
    let mut c = Case::must(
        "figure-of-shapes-with-labels-and-caption",
        "A polar-diagram figure: a 300 x 150 pt frame, two arc shapes inside and ten tiny (5 pt) angle labels placed round \
         it (two columns of four labels 25 pt apart, a bottom row of three 50 pt apart); under the frame, 12 pt below \
         its edge, an italic two-line caption. A person sees ten independent labels and one two-line caption: eleven blocks; \
         the labels do not form a paragraph and the caption does not join them.",
    );
    c.page.rect("frame", 72.0, 100.0, 372.0, 250.0);
    c.page.rect("arc0", 122.0, 120.0, 322.0, 235.0);
    c.page.rect("arc1", 147.0, 135.0, 297.0, 220.0);
    let st = doc(5.0);
    for (k, t) in ["90\u{b0}", "75\u{b0}", "60\u{b0}", "45\u{b0}"].iter().enumerate() {
        let y = 122.0 + 25.0 * k as f32;
        let (l, r) = (format!("L{k}"), format!("R{k}"));
        c.page.text(&l, &st, 80.0, y, t);
        c.page.text(&r, &st, 345.0, y, t);
        c.expect.push(lab(&l));
        c.expect.push(lab(&r));
    }
    for (k, t) in ["30\u{b0}", "15\u{b0}", "0\u{b0}"].iter().enumerate() {
        let l = format!("B{k}");
        c.page.text(&l, &st, 140.0 + 50.0 * k as f32, 244.0, t);
        c.expect.push(lab(&l));
    }
    let cap = c.page.para(P::new("cap", &doc_italic(8.0)).at(72.0, 268.0).w(300.0).n(2).pitch(9.6).frags(2, 4).last(0.60));
    c.expect.push(cap.all());
    c
}

fn c_chart_tick_labels() -> Case {
    let mut c = Case::must(
        "chart-axis-tick-labels-each-its-own-block",
        "A chart's axes: seven y tick labels right-aligned in a column 17.6 pt apart (3.5 em), an axis title above them, \
         and seven x tick labels in a row 30 pt apart (6 em), all 5 pt text of one style. A person sees every label as \
         an independent item: fifteen blocks.",
    );
    let st = doc(5.0);
    c.page.rect("plot", 34.0, 96.0, 250.0, 230.0);
    let t = c.page.cut_line("title", &doc_bold(6.0), 20.0, 88.0, "Watt", 2);
    c.expect.push(t);
    for (k, v) in ["46", "40", "34", "28", "22", "16", "10"].iter().enumerate() {
        let l = format!("y{k}");
        c.page.text_right(&l, &st, 30.0, 102.0 + 17.6 * k as f32, v);
        c.expect.push(lab(&l));
    }
    for k in 0..7 {
        let l = format!("x{k}");
        c.page.text(&l, &st, 50.0 + 30.0 * k as f32, 242.0, &format!("{}\u{b0}", 30 + 5 * k));
        c.expect.push(lab(&l));
    }
    c
}

fn c_text_in_path_box_beside_text() -> Case {
    let mut c = Case::must(
        "text-in-drawn-box-beside-same-style-text",
        "A justified 8 pt paragraph (9 lines, flush at x=252) and, 3 pt to its right on the same baselines, a stroked \
         rectangle (one path object) holding 8 pt text of the SAME font, size and colour (four lines, 3 pt padding): the \
         paragraph's flush right edge and the boxed text are only 6 pt (0.75 em) apart, closer than a justified word gap. \
         The box edge is the only cue. A person sees a paragraph and a boxed note: two blocks.",
    );
    let st = body(8.0);
    let a = c.page.para(P::new("a", &st).at(72.0, 100.0).w(180.0).n(9).pitch(9.6).just().frags(3, 6).last(0.6));
    c.page.rect("box", 255.0, 91.0, 455.0, 133.4);
    let b = c.page.para(P::new("b", &st).at(258.0, 100.0).w(194.0).n(4).pitch(9.6).frags(2, 5).last(0.7));
    c.expect = vec![a.all(), b.all()];
    c
}

fn c_text_in_four_rule_box_below_text() -> Case {
    let mut c = Case::must(
        "text-in-four-rule-box-below-same-style-text",
        "A justified 8 pt paragraph whose last line is full, and directly under it (9.6 pt pitch, no extra space) a box \
         drawn as four thin rules, 3 pt under the last baseline, holding three lines of the same font, size and colour, \
         4 pt inside the box. The box edge is the only cue. A person sees a paragraph and a boxed note: two blocks.",
    );
    let st = body(8.0);
    let a = c.page.para(P::new("a", &st).at(72.0, 100.0).w(300.0).n(6).pitch(9.6).just().frags(3, 7));
    let top = a.last_base() + 2.9;
    let b = c.page.para(P::new("b", &st).at(76.0, a.below(1.0)).w(292.0).n(3).pitch(9.6).frags(2, 6).last(0.6));
    c.page.box_four("bx", 72.0, top, 372.0, b.last_base() + 3.2);
    c.expect = vec![a.all(), b.all()];
    c
}

fn c_underlined_line_same_style() -> Case {
    let mut c = Case::must(
        "underlined-line-in-body-style-is-a-heading",
        "A column of 8 pt text that starts with one underlined line in the SAME font and size as the body (a rule 2.6 pt \
         under its baseline, nothing else different), then a 6-line paragraph 11.5 pt below. A person sees an underlined \
         label followed by a paragraph: two blocks.",
    );
    let st = body(8.0);
    let h = c.page.cut_line("h", &st, 72.0, 100.0, "FIXTURE DATA:", 3);
    c.page.rule_h("ul", 72.0, 190.8, 102.6);
    let p = c.page.para(P::new("p", &st).at(72.0, 111.5).w(118.8).n(6).pitch(9.6).just().frags(3, 7).last(0.5));
    c.expect = vec![h, p.all()];
    c
}

fn c_rule_between_paragraphs() -> Case {
    let mut c = Case::must(
        "hairline-rule-between-two-paragraphs",
        "Two 8 pt paragraphs in one column, the first ending in a nearly full justified line; the baseline distance \
         between them is only 2.4 pt more than the pitch, and a hairline rule across the column runs between the two \
         lines. A person sees the rule as a divider: two blocks.",
    );
    let st = body(8.0);
    let a = c.page.para(P::new("a", &st).at(X1, 359.0).w(COL_W).n(4).pitch(9.6).just().frags(3, 8).last(0.97));
    c.page.rule_h("rule", X1, X1 + COL_W, a.last_base() + 3.9);
    let b = c.page.para(P::new("b", &st).at(X1, a.last_base() + 12.0).w(COL_W).n(4).pitch(9.6).just().frags(3, 8).last(0.5));
    c.expect = vec![a.all(), b.all()];
    c
}

// ---------------------------------------------------------------------------------------------------
// outlined (path-drawn) words and lines: the datasheet's ligature problem
// ---------------------------------------------------------------------------------------------------

/// Anchors every outlined shape of `p` to the block of `p`.
fn bridge(c: &mut Case, p: &Para, whole_line: bool) {
    let anchor = p.first();
    for (_, s) in &p.outlined {
        c.outline(&anchor, s, whole_line);
    }
}

fn c_outlined_word_hole() -> Case {
    let mut c = Case::must(
        "outlined-word-hole-inside-justified-lines",
        "A 13-line justified paragraph in which two words are vector outlines (path objects, as 'efficacy' is in the \
         datasheet), one in line 6 and one in line 10: a text-only walk sees a 30-60 pt hole in those lines. A second \
         column sits 12 pt to the right on the same baselines. A person sees one paragraph per column: two blocks, the \
         outlined words belonging to the first paragraph (the block must enclose them).",
    );
    let st = body(8.0);
    let a = c.page.para(P::new("a", &st).at(X1, 421.04).w(COL_W).n(13).pitch(9.6).just().frags(3, 8).last(0.4).hole(5, Hole::Inside).hole(9, Hole::Inside));
    let b = c.page.para(P::new("b", &st).at(X2, 421.04).w(COL_W).n(11).pitch(9.6).just().frags(3, 8).last(0.5));
    bridge(&mut c, &a, false);
    c.expect = vec![a.all(), b.all()];
    c
}

fn c_outlined_whole_line() -> Case {
    let mut c = Case::must(
        "outlined-whole-line-inside-a-paragraph",
        "A 9-line justified paragraph whose fifth line is ONE path object as wide as the column and has no text object \
         at all (a line of ligature words drawn as outlines): the text baselines jump by two pitches there. A second \
         column stands next to it. A person sees one paragraph: two blocks; the outlined line belongs to the first \
         paragraph (a line without text objects inside its block, and the block box encloses it).",
    );
    let st = body(8.0);
    let a = c.page.para(P::new("a", &st).at(X1, 421.04).w(COL_W).n(9).pitch(9.6).just().frags(3, 8).last(0.55).outlined_line(4));
    let b = c.page.para(P::new("b", &st).at(X2, 421.04).w(COL_W).n(9).pitch(9.6).just().frags(3, 8).last(0.45));
    bridge(&mut c, &a, true);
    c.expect = vec![a.all(), b.all()];
    c
}

fn c_outlined_words_at_line_ends() -> Case {
    let mut c = Case::must(
        "outlined-words-at-line-start-and-line-end",
        "A 10-line justified paragraph: line 3 STARTS with an outlined word (so its first text object begins 25-50 pt \
         right of the margin, like a deep indent) and line 7 ENDS with an outlined word (so its text stops 25-50 pt short \
         of the margin, like a paragraph's short last line). A second column stands next to it. A person sees flush, full \
         lines and one paragraph: two blocks.",
    );
    let st = body(8.0);
    let a = c.page.para(P::new("a", &st).at(X1, 421.04).w(COL_W).n(10).pitch(9.6).just().frags(3, 8).last(0.5).hole(2, Hole::Start).hole(6, Hole::End));
    let b = c.page.para(P::new("b", &st).at(X2, 421.04).w(COL_W).n(10).pitch(9.6).just().frags(3, 8).last(0.4));
    bridge(&mut c, &a, false);
    c.expect = vec![a.all(), b.all()];
    c
}

fn c_blank_line_without_outline() -> Case {
    let mut c = Case::must(
        "blank-line-gap-without-outline-is-a-paragraph-break",
        "The control for the outlined-line case: the same justified column with a two-pitch gap in the text baselines, but \
         NOTHING is drawn in the gap, and the line above it is a full justified line. A person sees a blank line: two \
         paragraphs, two blocks.",
    );
    let st = body(8.0);
    let a = c.page.para(P::new("a", &st).at(X1, 421.04).w(COL_W).n(4).pitch(9.6).just().frags(3, 8));
    let b = c.page.para(P::new("b", &st).at(X1, 421.04 + 5.0 * 9.6).w(COL_W).n(4).pitch(9.6).just().frags(3, 8).last(0.55));
    c.expect = vec![a.all(), b.all()];
    c
}

fn c_outlined_heading_above() -> Case {
    let mut c = Case::must(
        "outlined-heading-above-a-paragraph-is-not-bridged",
        "A bold heading that is itself a vector outline (a 36 x 7.6 pt path on its own line, 14 pt below a paragraph and \
         9.6 pt above the next one, as 'Reflector' is in the datasheet). A person sees heading, then a paragraph; the \
         paragraph's editor box must start at its first text line: the heading's outline must not be pulled into the \
         paragraph block (and the box must not cover it). The paragraph above is a separate block.",
    );
    let st = body(8.0);
    let a = c.page.para(P::new("a", &st).at(X1, 359.0).w(COL_W).n(5).pitch(9.6).just().frags(3, 8).last(0.45));
    let hb = a.last_base() + 14.0;
    c.page.add_shape("heading.o", X1, hb - 0.72 * 8.0, X1 + 36.0, hb + 0.22 * 8.0);
    let ho = c.page.ids["heading.o"];
    c.page.shape_base.insert(ho, hb);
    let b = c.page.para(P::new("b", &st).at(X1, hb + 9.6).w(COL_W).n(7).pitch(9.6).just().frags(3, 8).last(0.5));
    c.no_outline(&b.first(), "heading.o");
    c.expect = vec![a.all(), b.all()];
    c
}

fn c_datasheet_heading_paragraph_outlines() -> Case {
    let mut c = Case::must(
        "datasheet-heading-13-line-paragraph-two-outlined-words-heading",
        "The user's red-marked paragraph rebuilt from the datasheet's numbers: a 5-line paragraph (last baseline 397.43), \
         14 pt later the 5-object heading 'The Light Source - COB' (baseline 411.43), 9.6 pt later a 13-line justified \
         paragraph (baselines 421.04 .. 536.24, pitch 9.6) holding two outlined words (lines 9 and 11), 14 pt after it the \
         heading 'Light Quality' (550.24), then an 8-line paragraph. 8 pt, 118.8 pt measure, Light vs ExtraBold with one \
         face name. A person marks heading, paragraph, heading, paragraph: five blocks; the 13-line paragraph is ONE block \
         that encloses both outlined words.",
    );
    let (h, b) = (head(8.0), body(8.0));
    let intro = c.page.para(P::new("i", &b).at(X1, 397.43 - 4.0 * 9.6).w(COL_W).n(5).pitch(9.6).just().frags(3, 8).last(0.40));
    let h1 = c.page.cut_line("h1", &h, X1, 411.43, "The Light Source - COB", 5);
    let p = c.page.para(P::new("p", &b).at(X1, 421.04).w(COL_W).n(13).pitch(9.6).just().frags(3, 8).last(0.40).hole(8, Hole::Inside).hole(10, Hole::Inside));
    let h2 = c.page.cut_line("h2", &h, X1, 550.24, "Light Quality", 4);
    let q = c.page.para(P::new("q", &b).at(X1, 559.84).w(COL_W).n(8).pitch(9.6).just().frags(3, 8).last(0.55));
    bridge(&mut c, &p, false);
    c.expect = vec![intro.all(), h1, p.all(), h2, q.all()];
    c
}

// ---------------------------------------------------------------------------------------------------
// marks, footnotes, drop caps, run-in leads, inline emphasis
// ---------------------------------------------------------------------------------------------------

fn c_superscript_marks() -> Case {
    let mut c = Case::must(
        "superscript-and-subscript-marks-inline",
        "A 7-line ragged 10 pt paragraph carrying three footnote marks (6 pt superscript digits raised 3.5 pt, glued to the \
         end of three lines) and one subscript digit (6 pt, lowered 2.5 pt); every mark is its own text object on a shifted \
         baseline. A person sees one paragraph with its marks: one block; the paragraph after the blank line is another.",
    );
    let st = doc(10.0);
    let small = doc(6.0);
    let p = c.page.para(P::new("p", &st).at(72.0, 100.0).w(400.0).n(7).pitch(12.0).frags(3, 6).last(0.55));
    let q = c.page.para(P::new("q", &st).at(72.0, p.below(2.0)).w(400.0).n(3).pitch(12.0).frags(3, 6).last(0.60));
    let mut block = p.all();
    for (k, line) in [1usize, 3, 5].iter().enumerate() {
        let r = c.page.get(p.lines[*line].last().unwrap()).right;
        let l = format!("sup{k}");
        c.page.text(&l, &small, r + 0.5, p.bases[*line] - 3.5, &format!("{}", k + 1));
        block.push(l);
    }
    let r = c.page.get(p.lines[4].last().unwrap()).right;
    c.page.text("sub0", &small, r + 0.5, p.bases[4] + 2.5, "2");
    block.push("sub0".to_string());
    c.expect = vec![block, q.all()];
    c
}

fn c_footnote_star_marker() -> Case {
    let mut c = Case::must(
        "footnote-bold-star-marker-then-text",
        "The datasheet's footnote: a paragraph, 14 pt later a lone bold '*' object followed on the same baseline by the \
         first words of a 3-line 7 pt note (the other two lines start at the left margin). A person sees the star as part \
         of the footnote: paragraph and footnote are two blocks.",
    );
    let st = body(8.0);
    let a = c.page.para(P::new("a", &st).at(X1, 359.0).w(COL_W).n(4).pitch(9.6).just().frags(3, 8).last(0.5));
    let base = a.last_base() + 14.0;
    c.page.text("star", &semibold(7.0), X1, base, "*");
    let f = c.page.para(P::new("fn", &body(7.0)).at(X1, base).w(COL_W).n(3).pitch(8.4).frags(2, 4).indent(8.0).last(0.6));
    c.expect = vec![a.all(), cat(vec![lab("star"), f.all()])];
    c
}

fn c_footnotes_with_numerals() -> Case {
    let mut c = Case::must(
        "footnotes-each-start-with-a-lone-superscript-numeral",
        "A 10 pt paragraph, 16 pt under it a short rule, then three 8 pt footnotes at the 9.6 pt pitch, NO space between \
         them: each starts with its own tiny raised numeral object ('1', '2', '3'), followed on the line by the text; \
         footnote 1 and 2 end in nearly full lines. A person sees the numeral as the start of each footnote: the \
         paragraph and three footnotes are four blocks.",
    );
    let a = c.page.para(P::new("a", &doc(10.0)).at(72.0, 100.0).w(400.0).n(5).pitch(12.0).frags(3, 6).last(0.5));
    let mut base = a.last_base() + 28.0;
    c.page.rule_h("rule", 72.0, 172.0, a.last_base() + 16.0);
    c.expect.push(a.all());
    for (k, (n, last)) in [(2usize, 0.95f32), (1, 0.90), (2, 0.50)].iter().enumerate() {
        let sup = format!("f{k}.s");
        c.page.text(&sup, &doc(5.0), 72.0, base - 3.0, &format!("{}", k + 1));
        let p = c.page.para(P::new(&format!("f{k}"), &doc(8.0)).at(72.0, base).w(400.0).n(*n).pitch(9.6).frags(2, 5).indent(5.0).last(*last));
        base = p.below(1.0);
        c.expect.push(cat(vec![lab(&sup), p.all()]));
    }
    c
}

fn c_drop_cap() -> Case {
    let mut c = Case::must(
        "drop-cap-starts-a-paragraph",
        "A paragraph that starts with a 26 pt drop cap 'T' spanning two lines (its baseline on line 2); the first two text \
         lines are indented 20 pt to wrap round it, the following lines start at the margin; the paragraph's last line is \
         short. A person sees the cap as part of the paragraph: one block, then (after a blank line) another paragraph.",
    );
    c.page.text("cap", &doc_bold(26.0), 72.0, 112.0, "T");
    let p1 = c.page.para(P::new("d1", &doc(10.0)).at(92.0, 100.0).w(380.0).n(2).pitch(12.0).frags(2, 5).no_cap().no_period());
    let p2 = c.page.para(P::new("d2", &doc(10.0)).at(72.0, 124.0).w(400.0).n(5).pitch(12.0).frags(2, 5).last(0.6).no_cap());
    let q = c.page.para(P::new("q", &doc(10.0)).at(72.0, p2.below(2.0)).w(400.0).n(3).pitch(12.0).frags(2, 5).last(0.5));
    c.expect = vec![cat(vec![lab("cap"), p1.all(), p2.all()]), q.all()];
    c
}

fn c_run_in_lead_single() -> Case {
    let mut c = Case::must(
        "bold-run-in-lead-then-normal-text-on-one-line",
        "A paragraph that opens with a bold run-in lead 'Note:' followed on the same line by regular 10 pt text (the lead is \
         its own object, a different font and weight), 4 lines with a short last line; a blank line; another paragraph. A \
         person sees one paragraph per block: two blocks (the lead is not a heading).",
    );
    let lead_w = ink_w("Note:", 10.0);
    c.page.text("lead", &doc_bold(10.0), 72.0, 100.0, "Note:");
    let p = c.page.para(P::new("p", &doc(10.0)).at(72.0, 100.0).w(400.0).n(4).pitch(12.0).frags(2, 5).indent(lead_w + 3.5).last(0.6));
    let q = c.page.para(P::new("q", &doc(10.0)).at(72.0, p.below(2.0)).w(400.0).n(3).pitch(12.0).frags(2, 5).last(0.5));
    c.expect = vec![cat(vec![lab("lead"), p.all()]), q.all()];
    c
}

fn c_run_in_leads_consecutive() -> Case {
    let mut c = Case::must(
        "run-in-leads-start-consecutive-paragraphs",
        "Three paragraphs at a constant 12 pt pitch with no blank lines, each opening with its own bold run-in lead ('Warning:', \
         'Note:', 'Tip:') followed by regular text on the same line; the first two end in short lines. A person sees three \
         paragraphs: three blocks.",
    );
    let mut base = 100.0;
    for (k, (t, n, last)) in [("Warning:", 3usize, 0.45f32), ("Note:", 3, 0.55), ("Tip:", 2, 0.50)].iter().enumerate() {
        let ll = format!("lead{k}");
        c.page.text(&ll, &doc_bold(10.0), 72.0, base, t);
        let p = c.page.para(P::new(&format!("p{k}"), &doc(10.0)).at(72.0, base).w(400.0).n(*n).pitch(12.0).frags(2, 5).indent(ink_w(t, 10.0) + 3.5).last(*last));
        base = p.below(1.0);
        c.expect.push(cat(vec![lab(&ll), p.all()]));
    }
    c
}

fn c_inline_emphasis_minority() -> Case {
    let mut c = Case::must(
        "inline-emphasis-words-are-a-minority-of-their-line",
        "A 7-line ragged paragraph in which seven words are bold or italic (other font ids and stems; at most one per line, one \
         at the start of a line, one at the end, one on the first line and one on the last), as 'HSI ' is Medium inside the \
         Light body in the datasheet (14 % of its line). A blank line, then a second paragraph. A person sees the emphasis as \
         part of the sentence: two blocks.",
    );
    let st = doc(10.0);
    let p = c.page.para(P::new("p", &st).at(72.0, 100.0).w(400.0).n(7).pitch(12.0).frags(4, 7).last(0.5));
    let q = c.page.para(P::new("q", &st).at(72.0, p.below(2.0)).w(400.0).n(3).pitch(12.0).frags(3, 6).last(0.6));
    let (b, i) = (doc_bold(10.0), doc_italic(10.0));
    let picks: [(usize, usize, &St); 7] = [(0, 1, &b), (1, 2, &i), (2, 0, &b), (3, 99, &b), (4, 1, &i), (5, 2, &b), (6, 0, &i)];
    for (line, idx, sty) in picks {
        let l = p.lines[line].clone();
        let name = l[idx.min(l.len() - 1)].clone();
        c.page.restyle(&name, sty);
    }
    c.expect = vec![p.all(), q.all()];
    c
}

fn c_inline_larger_word() -> Case {
    let mut c = Case::must(
        "inline-larger-word-inside-a-paragraph",
        "A 6-line ragged 10 pt paragraph in which the last object of line 3 and the last object of the final line are set \
         at 13 pt (a word emphasised by size) on the same baseline. A blank line, then another paragraph. A person sees \
         bigger words inside a paragraph: two blocks.",
    );
    let st = doc(10.0);
    let p = c.page.para(P::new("p", &st).at(72.0, 100.0).w(400.0).n(6).pitch(12.0).frags(4, 6).last(0.5));
    let q = c.page.para(P::new("q", &st).at(72.0, p.below(2.0)).w(400.0).n(3).pitch(12.0).frags(3, 6).last(0.6));
    for line in [2usize, 5] {
        let l = p.lines[line].last().unwrap().clone();
        c.page.enlarge(&l, 13.0);
    }
    c.expect = vec![p.all(), q.all()];
    c
}

fn c_same_face_and_stem_different_font_ids() -> Case {
    let mut c = Case::ambiguous(
        "same-face-same-stem-but-a-new-font-id-on-every-line",
        "An 8-line ragged 10 pt paragraph in which every line uses a different font id (as a producer that embeds a fresh \
         subset per chunk would) but the face name ('Calibri'), the measured stem (74), size and colour are identical: it \
         looks identical. Reading 'one-paragraph': what the eye sees is one paragraph. Reading 'block-per-font-id': a \
         different font program starts a new block on every line.",
    );
    let st = doc(10.0);
    let p = c.page.para(P::new("p", &st).at(72.0, 100.0).w(400.0).n(8).pitch(12.0).frags(3, 6).last(0.5));
    for (i, line) in p.lines.iter().enumerate() {
        let sty = st.clone().font(30 + i as u32);
        for l in line {
            c.page.restyle(l, &sty);
        }
    }
    c.reading("one-paragraph", vec![p.all()]);
    c.reading("block-per-font-id", p.lines.clone());
    c
}

fn c_colour_only_change() -> Case {
    let mut c = Case::ambiguous(
        "colour-is-the-only-change-between-two-paragraphs",
        "Two 4-line paragraphs of one font, size and pitch, directly one under the other, the first ending in a 96 % line, \
         the second set in red instead of black. Reading 'split-at-colour': a colour change starts a new block. Reading \
         'one-block': nothing but a colour differs, so it is eight lines of one text.",
    );
    let a = c.page.para(P::new("a", &doc(10.0)).at(72.0, 100.0).w(400.0).n(4).pitch(12.0).frags(3, 6).last(0.96));
    let b = c.page.para(P::new("b", &doc(10.0).rgb([200, 0, 0])).at(72.0, a.below(1.0)).w(400.0).n(4).pitch(12.0).frags(3, 6).last(0.5));
    c.reading("split-at-colour", vec![a.all(), b.all()]);
    c.reading("one-block", vec![cat(vec![a.all(), b.all()])]);
    c
}

// ---------------------------------------------------------------------------------------------------
// hyphenation, alignment, spacing, size
// ---------------------------------------------------------------------------------------------------

fn c_hyphen_u0002() -> Case {
    let mut c = Case::must(
        "hyphenated-line-ends-with-u0002-marker",
        "A 10-line justified 8 pt paragraph in which five lines end in a hyphenated word (the text object ends with U+0002 and \
         the next line starts with the rest of the word), followed 9.6 pt x 2 lower by a second paragraph. A person reads \
         every hyphenated word as running on: the first paragraph is one block, the second another.",
    );
    let st = body(8.0);
    let a = c.page.para(P::new("a", &st).at(X1, 359.0).w(COL_W).n(10).pitch(9.6).just().frags(3, 7).last(0.5).hyph(&[1, 3, 4, 6, 8], '\u{2}'));
    let b = c.page.para(P::new("b", &st).at(X1, a.below(2.0)).w(COL_W).n(6).pitch(9.6).just().frags(3, 7).last(0.45).hyph(&[2], '\u{2}'));
    c.expect = vec![a.all(), b.all()];
    c
}

fn c_hyphen_plain() -> Case {
    let mut c = Case::must(
        "hyphenated-line-ends-with-a-plain-hyphen",
        "A 9-line ragged 10 pt paragraph in which five lines end in a hyphenated word with an ordinary '-' character, then a \
         blank line and a second paragraph. A person reads every hyphenated word as running on: two blocks.",
    );
    let st = doc(10.0);
    let a = c.page.para(P::new("a", &st).at(72.0, 100.0).w(400.0).n(9).pitch(12.0).frags(3, 6).last(0.55).hyph(&[0, 2, 3, 5, 7], '-'));
    let b = c.page.para(P::new("b", &st).at(72.0, a.below(2.0)).w(400.0).n(3).pitch(12.0).frags(3, 6).last(0.6));
    c.expect = vec![a.all(), b.all()];
    c
}

fn c_centred_title() -> Case {
    let mut c = Case::must(
        "centred-title-lines",
        "A centred 14 pt bold title of three lines (every line a different width, left edges all different, centre at 297.5), \
         a centred italic subtitle 24 pt below it, and 36 pt lower a left-aligned 10 pt paragraph. A person sees a title \
         block, a subtitle and a paragraph: three blocks, the three centred title lines together.",
    );
    let t = c.page.para(P::new("t", &doc_bold(14.0)).at(147.5, 100.0).w(300.0).n(3).pitch(17.0).centre().frags(1, 3).last(0.60));
    let sb = t.last_base() + 24.0;
    c.page.text_centre("sub", &doc_italic(10.0), 297.5, sb, "Datasheet revision 4");
    let b = c.page.para(P::new("b", &doc(10.0)).at(72.0, sb + 36.0).w(450.0).n(5).pitch(12.0).frags(3, 6).last(0.6));
    c.expect = vec![t.all(), lab("sub"), b.all()];
    c
}

fn c_right_aligned_block() -> Case {
    let mut c = Case::must(
        "right-aligned-address-block",
        "A four-line right-aligned 10 pt block in the top-right corner (right edge 522, left edges all different), and 40 pt \
         below it a left-aligned 10 pt paragraph. A person sees an address block and a paragraph: two blocks.",
    );
    let r = c.page.para(P::new("r", &doc(10.0)).at(372.0, 100.0).w(150.0).n(4).pitch(12.0).right().frags(1, 3));
    let b = c.page.para(P::new("b", &doc(10.0)).at(72.0, r.last_base() + 40.0).w(450.0).n(5).pitch(12.0).frags(3, 6).last(0.6));
    c.expect = vec![r.all(), b.all()];
    c
}

fn c_size_change_only() -> Case {
    let mut c = Case::must(
        "two-paragraphs-separated-only-by-a-size-change",
        "A 4-line 10 pt paragraph ending in a 97 % line, directly followed (10.8 pt later, between the two pitches) by a \
         4-line 8 pt paragraph in the same font and colour. Nothing but the size changes. A person sees smaller text start: \
         two blocks.",
    );
    let a = c.page.para(P::new("a", &doc(10.0)).at(72.0, 100.0).w(400.0).n(4).pitch(12.0).frags(3, 6).last(0.97));
    let b = c.page.para(P::new("b", &doc(8.0)).at(72.0, a.last_base() + 10.8).w(400.0).n(4).pitch(9.6).frags(3, 7).last(0.5));
    c.expect = vec![a.all(), b.all()];
    c
}

fn c_line_spacing_neighbours() -> Case {
    let mut c = Case::must(
        "line-spacing-1.2-and-1.5-in-neighbouring-paragraphs",
        "Three 10 pt paragraphs directly under each other: 12 pt pitch (5 lines), 15 pt pitch (5 lines), 12 pt pitch (4 lines), \
         the transitions 13.5 pt (the mean), each paragraph but the last ending in a short line. A person sees three \
         paragraphs: three blocks; the 15 pt paragraph is not cut at its wider pitch.",
    );
    let st = doc(10.0);
    let a = c.page.para(P::new("a", &st).at(72.0, 100.0).w(400.0).n(5).pitch(12.0).frags(3, 6).last(0.40));
    let b = c.page.para(P::new("b", &st).at(72.0, a.last_base() + 13.5).w(400.0).n(5).pitch(15.0).frags(3, 6).last(0.45));
    let d = c.page.para(P::new("c", &st).at(72.0, b.last_base() + 13.5).w(400.0).n(4).pitch(12.0).frags(3, 6).last(0.60));
    c.expect = vec![a.all(), b.all(), d.all()];
    c
}

// ---------------------------------------------------------------------------------------------------
// rotation, stream order, twins
// ---------------------------------------------------------------------------------------------------

fn c_rotated_text() -> Case {
    let mut c = Case::must(
        "rotated-text-beside-and-across-a-paragraph",
        "An 8-line justified paragraph with three rotated margin stamps running upward at x=52 ('CONFIDENTIAL', 'DRAFT', 'v2') \
         and a big 45-degree watermark whose box covers the whole paragraph. Rotated objects are never grouped: the paragraph \
         is one block and each rotated object is a one-line block of its own (five blocks).",
    );
    c.overlap_ok = true;
    let p = c.page.para(P::new("p", &doc(10.0)).at(72.0, 100.0).w(300.0).n(8).pitch(12.0).just().frags(3, 6).last(0.5));
    c.page.rotated_up("rot0", &doc_bold(9.0), 52.0, 300.0, "CONFIDENTIAL");
    c.page.rotated_up("rot1", &doc(9.0), 52.0, 200.0, "DRAFT");
    c.page.rotated_up("rot2", &doc(9.0), 52.0, 150.0, "v2");
    c.page.rotated_box("wm", &doc_bold(60.0), "SAMPLE", 100.0, 120.0, 400.0, 380.0);
    c.expect = vec![p.all(), lab("rot0"), lab("rot1"), lab("rot2"), lab("wm")];
    c
}

fn c_stream_order_row_major() -> Case {
    let mut c = Case::must(
        "stream-order-row-major-across-three-columns",
        "The three-column datasheet layout, but the content stream is written ROW BY ROW across the columns (object ids go \
         line 1 of column 1, line 1 of column 2, line 1 of column 3, line 2 ...), as a table-oriented generator would. What \
         a person sees does not depend on the stream order: three paragraphs, three blocks.",
    );
    let st = body(8.0);
    let a = c.page.para(P::new("a", &st).at(X1, 359.0).w(COL_W).n(14).pitch(9.6).just().frags(3, 8).last(0.50));
    let b = c.page.para(P::new("b", &st).at(X2, 359.0).w(COL_W).n(14).pitch(9.6).just().frags(3, 8).last(0.70));
    let d = c.page.para(P::new("c", &st).at(X3, 359.0).w(COL_W).n(14).pitch(9.6).just().frags(3, 8).last(0.35));
    c.page.renumber_by(|y, x| ((y * 4.0) as i64, (x * 10.0) as i64));
    c.expect = vec![a.all(), b.all(), d.all()];
    c
}

fn c_stream_order_reversed() -> Case {
    let mut c = Case::must(
        "stream-order-bottom-to-top",
        "The heading/paragraph column of the datasheet with the content stream written BOTTOM TO TOP and right to left (object \
         ids decrease down the page). The grouping a person sees is unchanged: intro, three headings and three paragraphs.",
    );
    let e = sections(&mut c, &head(8.0), &body(8.0), &[("The Light Source - COB", 6, 0.5), ("Light Quality", 9, 0.62), ("Optic Component", 4, 0.4)]);
    c.page.renumber_by(|y, x| (-((y * 4.0) as i64), -((x * 10.0) as i64)));
    c.expect = e;
    c
}

fn c_stream_order_random() -> Case {
    let mut c = Case::must(
        "stream-order-random-permutation",
        "Mixed three-column layout (six paragraphs) with the object ids assigned in a pseudo-random order, as a generator that \
         emits text in hash order would. The page looks exactly the same: six blocks. Geometry alone decides here; the stream \
         order carries no information.",
    );
    let st = body(8.0);
    let a = c.page.para(P::new("a", &st).at(X1, 359.0).w(COL_W).n(8).pitch(9.6).just().frags(3, 8).last(0.50));
    let b = c.page.para(P::new("b", &st).at(X1, a.below(1.0)).w(COL_W).n(5).pitch(9.6).just().frags(3, 8).last(0.70));
    let d = c.page.para(P::new("c", &st).at(X2, 359.0).w(COL_W).n(13).pitch(9.6).just().frags(3, 8).last(0.40));
    let e = c.page.para(P::new("d", &st).at(X3, 359.0).w(COL_W).n(4).pitch(9.6).just().frags(3, 8).last(0.60));
    let f = c.page.para(P::new("e", &st).at(X3, e.below(1.0)).w(COL_W).n(4).pitch(9.6).just().frags(3, 8).last(0.45));
    let g = c.page.para(P::new("f", &st).at(X3, f.below(1.0)).w(COL_W).n(5).pitch(9.6).just().frags(3, 8).last(0.30));
    c.page.renumber_by(|y, x| {
        let h = (y.to_bits() as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (x.to_bits() as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
        ((h >> 8) as i64, 0)
    });
    c.expect = vec![a.all(), b.all(), d.all(), e.all(), f.all(), g.all()];
    c
}

fn c_twin_fragments() -> Case {
    let mut c = Case::must(
        "twin-fragments-with-identical-rects",
        "A heading and the first line of the paragraph under it are each drawn TWICE at the identical position (faux bold / \
         overprint, like the datasheet's 'Color Options'): every object has a twin with the same rect and text and its own \
         object id. A person sees one heading and one paragraph: two blocks, each twin in the block of its original.",
    );
    c.overlap_ok = true;
    let h = c.page.cut_line("h", &head(8.0), X1, 100.0, "Color Options", 3);
    let p = c.page.para(P::new("p", &body(8.0)).at(X1, 109.6).w(COL_W).n(7).pitch(9.6).just().frags(3, 7).last(0.5));
    let mut hb = h.clone();
    for l in &h {
        hb.push(c.page.twin(l));
    }
    let mut pb = p.all();
    for l in p.line(0) {
        pb.push(c.page.twin(&l));
    }
    c.expect = vec![hb, pb];
    c
}

// ---------------------------------------------------------------------------------------------------
// degenerate and extreme inputs
// ---------------------------------------------------------------------------------------------------

fn c_one_fragment() -> Case {
    let mut c = Case::must(
        "one-fragment-page",
        "A page that holds a single text object. A person sees one word: one block with one line.",
    );
    c.page.text("only", &doc(10.0), 100.0, 100.0, "Hello");
    c.expect = vec![lab("only")];
    c
}

fn c_empty_page() -> Case {
    Case::must(
        "empty-page",
        "A page with no text object and no shape at all. A person sees a blank page: detect must return no block and not panic.",
    )
}

fn c_shapes_only_page() -> Case {
    let mut c = Case::must(
        "page-with-only-rules-and-boxes",
        "A page that holds drawings but no text object (a frame, two rules, a box of four rules). A person sees no text: no \
         text block, no panic.",
    );
    c.page.rect("frame", 40.0, 40.0, 500.0, 700.0);
    c.page.rule_h("r0", 60.0, 480.0, 200.0);
    c.page.rule_v("r1", 270.0, 60.0, 680.0);
    c.page.box_four("bx", 80.0, 300.0, 240.0, 380.0);
    c
}

fn c_degenerate_fragments() -> Case {
    let mut c = Case::must(
        "nan-zero-size-infinite-and-huge-fragments-in-a-page",
        "A clean 6-line justified paragraph next to eight broken text objects (all NaN, NaN left edge, size 0, negative size, an \
         infinite right edge, a zero-area rect, coordinates near 1e30, a rect sitting inside the paragraph with size 0). They \
         must not panic detect, must each end up in exactly one block, and must not stop the clean paragraph from being one \
         block (the broken objects may stand alone or join a block: not asserted); a block that holds finite objects keeps \
         a finite box.",
    );
    c.degenerate = true;
    let st = body(8.0);
    let p = c.page.para(P::new("p", &st).at(X1, 359.0).w(COL_W).n(6).pitch(9.6).just().frags(3, 7).last(0.5));
    let nan = f32::NAN;
    let z = st.clone().size(0.0);
    let neg = st.clone().size(-8.0);
    c.page.add_frag_rect("nan_all", &st, "x", nan, nan, nan, nan, nan, false);
    c.page.add_frag_rect("nan_left", &st, "x", nan, 400.0, 410.0, 406.0, 405.0, false);
    c.page.add_frag_rect("zero_size", &z, "x", 450.0, 400.0, 456.0, 406.0, 405.0, false);
    c.page.add_frag_rect("neg_size", &neg, "x", 460.0, 400.0, 466.0, 406.0, 405.0, false);
    c.page.add_frag_rect("inf_right", &st, "x", 470.0, 400.0, f32::INFINITY, 406.0, 405.0, false);
    c.page.add_frag_rect("zero_area", &st, "x", 480.0, 405.0, 480.0, 405.0, 405.0, false);
    c.page.add_frag_rect("huge", &st, "x", 1.0e30, 1.0e30, 1.0e30 + 1.0e24, 1.0e30 + 1.0e24, 1.0e30, false);
    c.page.add_frag_rect("zero_inside", &z, "ab", X1 + 30.0, 385.0, X1 + 40.0, 391.0, 390.0, false);
    for l in ["nan_all", "nan_left", "zero_size", "neg_size", "inf_right", "zero_area", "huge", "zero_inside"] {
        c.free.push(l.to_string());
    }
    c.expect = vec![p.all()];
    c
}

fn c_inverted_rects() -> Case {
    let mut c = Case::must(
        "top-greater-than-bottom-on-every-fragment",
        "Two justified columns exactly as in the plain two-column case, but every text object reports top > bottom (a flipped \
         y axis). The contract says the detector normalises: the page looks the same, two blocks.",
    );
    let st = body(8.0);
    let a = c.page.para(P::new("a", &st).at(X1, 359.0).w(COL_W).n(12).pitch(9.6).just().frags(3, 8).last(0.55));
    let b = c.page.para(P::new("b", &st).at(X2, 359.0).w(COL_W).n(9).pitch(9.6).just().frags(3, 8).last(0.40));
    c.page.invert_vertical();
    c.expect = vec![a.all(), b.all()];
    c
}

fn c_duplicate_object_ids() -> Case {
    let mut c = Case::must(
        "duplicate-object-ids-no-panic",
        "A small paragraph in which two text objects carry the SAME object id (a corrupt or merged input). The contract only \
         promises: no panic, and every object id is reported. Nothing about the grouping is asserted.",
    );
    c.page.para(P::new("p", &body(8.0)).at(X1, 359.0).w(COL_W).n(3).pitch(9.6).just().frags(3, 5).last(0.6));
    let id = c.page.frags[2].object;
    c.page.frags[3].object = id;
    c.dup_ids = true;
    c.check_groups = false;
    c
}

fn c_single_long_line() -> Case {
    let mut c = Case::must(
        "single-very-long-line-of-600-words",
        "One text line of 600 words cut into 300 objects (about 19,000 pt wide: a log or ticker line), with a normal paragraph \
         40 pt below it. A person sees one line and one paragraph: two blocks, the long line whole.",
    );
    let st = body(8.0);
    let words: Vec<String> = (0..600).map(|_| rand_word(&mut c.page.rng)).collect();
    let (long, _, _) = c.page.put_line("long", &st, 20.0, 100.0, &words, adv(' ') * 8.0, 300, None, false);
    let p = c.page.para(P::new("p", &st).at(20.0, 140.0).w(300.0).n(4).pitch(9.6).frags(2, 4).last(0.6));
    c.expect = vec![long, p.all()];
    c
}

// ---------------------------------------------------------------------------------------------------
// more realistic traps
// ---------------------------------------------------------------------------------------------------

fn c_narrow_column() -> Case {
    let mut c = Case::must(
        "narrow-ragged-column-of-one-or-two-words-per-line",
        "A very narrow (7 em) left-aligned column: 14 lines of one or two words each, next to a normal column 58 pt to its \
         right. Most lines are 'short' relative to a normal measure. A person sees a narrow paragraph and a normal paragraph: \
         two blocks.",
    );
    let a = c.page.para(P::new("a", &doc(10.0)).at(72.0, 100.0).w(70.0).n(14).pitch(12.0).frags(1, 2).last(0.7));
    let b = c.page.para(P::new("b", &doc(10.0)).at(200.0, 100.0).w(300.0).n(8).pitch(12.0).frags(3, 5).last(0.6));
    c.expect = vec![a.all(), b.all()];
    c
}

fn c_natural_short_line() -> Case {
    let wlong = "telecommunications";
    let w_long = ink_w(wlong, 10.0);
    for attempt in 0..600u64 {
        let mut c = Case::must(
            "ragged-short-line-because-the-next-word-does-not-fit",
            "A 160 pt wide ragged 10 pt paragraph whose fourth line ends at about 45 % of the measure because the next word \
             ('telecommunications', 95 pt) does not fit in the remaining 88 pt and is NOT hyphenated; the paragraph \
             goes on with that word at the start of line 5. A blank line, then another paragraph. A person sees an ordinary \
             ragged paragraph: two blocks (a short line is a paragraph end only when the next word would have fitted).",
        );
        c.page = Page::new(fnv(c.name) ^ attempt.wrapping_mul(0x9E37_79B9) ^ salt().wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let st = doc(10.0);
        let a = c.page.para(P::new("a", &st).at(72.0, 100.0).w(160.0).n(4).pitch(12.0).frags(2, 4).last(0.45).no_period());
        let r = c.page.get(&a.lines[3].iter().cloned().max_by(|x, y| c.page.get(x).right.total_cmp(&c.page.get(y).right)).unwrap()).right;
        let remaining = 72.0 + 160.0 - r;
        if !(remaining < w_long - 1.0 && remaining > 60.0) {
            continue;
        }
        let b = c.page.para(P::new("b", &st).at(72.0, a.below(1.0)).w(160.0).n(4).pitch(12.0).frags(2, 4).lead(wlong).last(0.55).no_cap());
        let d = c.page.para(P::new("c", &st).at(72.0, b.below(2.0)).w(160.0).n(3).pitch(12.0).frags(2, 4).last(0.6));
        c.expect = vec![cat(vec![a.all(), b.all()]), d.all()];
        return c;
    }
    panic!("DSL: no seed produced a short line that really cannot take the next word");
}

fn c_widow_lines() -> Case {
    let mut c = Case::must(
        "paragraph-ends-with-a-one-word-and-a-one-letter-last-line",
        "Two ragged 10 pt paragraphs whose last 'line' is a single tiny object: the first ends with the 9 pt wide word 'it.' \
         on a line of its own, the second with the single letter 'a'. A blank line separates each from what follows. A person \
         sees the stray word and letter as the end of their paragraphs: three blocks.",
    );
    let st = doc(10.0);
    let a = c.page.para(P::new("a", &st).at(72.0, 100.0).w(400.0).n(5).pitch(12.0).frags(3, 6).last(0.90).no_period());
    c.page.text("w1", &st, 72.0, a.below(1.0), "it.");
    let b = c.page.para(P::new("b", &st).at(72.0, a.below(1.0) + 24.0).w(400.0).n(4).pitch(12.0).frags(3, 6).last(0.90).no_period());
    c.page.text("w2", &st, 72.0, b.below(1.0), "I.");
    let d = c.page.para(P::new("c", &st).at(72.0, b.below(1.0) + 24.0).w(400.0).n(3).pitch(12.0).frags(3, 6).last(0.50));
    c.expect = vec![cat(vec![a.all(), lab("w1")]), cat(vec![b.all(), lab("w2")]), d.all()];
    c
}

fn c_single_letter_objects() -> Case {
    let mut c = Case::must(
        "paragraph-made-of-single-letter-objects",
        "A 5-line justified 8 pt paragraph in which every glyph is its own text object (a glyph-by-glyph export), next to a \
         normal column. A person sees a paragraph: two blocks; one-letter objects count like any others.",
    );
    let st = body(8.0);
    let a = c.page.para(P::new("a", &st).at(X1, 100.0).w(COL_W).n(5).pitch(9.6).just().frags(200, 200).last(0.6));
    let b = c.page.para(P::new("b", &st).at(X2, 100.0).w(COL_W).n(5).pitch(9.6).just().frags(3, 8).last(0.5));
    c.expect = vec![a.all(), b.all()];
    c
}

fn c_letter_spaced_heading() -> Case {
    let mut c = Case::must(
        "letter-spaced-heading-of-single-glyph-objects",
        "A semibold 9 pt heading 'HSI LIGHTING DATASHEET' tracked out (0.35 em between letters, 1 em between words), every \
         letter its own object, and a 10 pt paragraph 24 pt below. A person sees a heading and a paragraph: two blocks.",
    );
    let t = c.page.tracked("t", &semibold(9.0), 72.0, 100.0, "HSI LIGHTING DATASHEET", 0.35, 1.0);
    let b = c.page.para(P::new("b", &doc(10.0)).at(72.0, 124.0).w(400.0).n(4).pitch(12.0).frags(3, 6).last(0.6));
    c.expect = vec![t, b.all()];
    c
}

fn c_text_wrap_around_figure() -> Case {
    let mut c = Case::must(
        "text-wraps-around-a-figure",
        "A 100 x 60 pt figure at the left margin; the first five lines of a ragged 10 pt paragraph start 10 pt right of it \
         (x=182), the next five lines start at the margin again (x=72); the right margin is the same throughout. A blank \
         line, then another paragraph. A person sees one paragraph wrapped round a picture: two blocks.",
    );
    c.page.rect("fig", 72.0, 100.0, 172.0, 160.0);
    let a = c.page.para(P::new("a", &doc(10.0)).at(182.0, 108.0).w(290.0).n(5).pitch(12.0).frags(3, 5).no_period());
    let b = c.page.para(P::new("b", &doc(10.0)).at(72.0, 168.0).w(400.0).n(5).pitch(12.0).frags(3, 5).last(0.6).no_cap());
    let d = c.page.para(P::new("c", &doc(10.0)).at(72.0, b.below(2.0)).w(400.0).n(3).pitch(12.0).frags(3, 5).last(0.5));
    c.expect = vec![cat(vec![a.all(), b.all()]), d.all()];
    c
}

fn c_indented_block_quote() -> Case {
    let mut c = Case::must(
        "indented-block-quote-between-two-paragraphs",
        "Three paragraphs of the same 10 pt style at a constant 12 pt pitch and no blank lines: a body paragraph ending in a \
         short line, a quotation indented 36 pt on BOTH sides (every line, not only the first), and a body paragraph back at \
         the margin. A person sees the indentation as a quotation: three blocks.",
    );
    let st = doc(10.0);
    let a = c.page.para(P::new("a", &st).at(72.0, 100.0).w(400.0).n(4).pitch(12.0).frags(3, 6).last(0.45));
    let q = c.page.para(P::new("q", &st).at(108.0, a.below(1.0)).w(328.0).n(4).pitch(12.0).frags(3, 5).last(0.50));
    let b = c.page.para(P::new("b", &st).at(72.0, q.below(1.0)).w(400.0).n(4).pitch(12.0).frags(3, 6).last(0.60));
    c.expect = vec![a.all(), q.all(), b.all()];
    c
}

fn c_nested_form_shapes_ignored() -> Case {
    let mut c = Case::must(
        "shapes-inside-nested-forms-are-ignored",
        "An 8-line justified paragraph and two shapes that report form-XObject depth 1 and 2 (a rule running between lines 4 and 5 \
         and a box round lines 6-8 in their reported, form-local coordinates). The contract says only depth 0 is considered \
         (the form is really drawn elsewhere), so what a person sees is one paragraph: one block.",
    );
    let st = body(8.0);
    let p = c.page.para(P::new("p", &st).at(X1, 359.0).w(COL_W).n(8).pitch(9.6).just().frags(3, 8).last(0.5));
    let y = p.bases[3] + 4.8;
    c.page.add_shape_depth("deep.rule", X1, y - 0.25, X1 + COL_W, y + 0.25, 1);
    c.page.add_shape_depth("deep.box", X1 - 5.0, p.bases[5] - 8.0, X1 + COL_W + 5.0, p.bases[7] + 3.0, 2);
    c.expect = vec![p.all()];
    c
}

fn c_toc_leader_lines() -> Case {
    let mut c = Case::ambiguous(
        "table-of-contents-lines-with-dot-leaders",
        "Five table-of-contents lines at a constant 12 pt pitch: a title at the left, a run of dots (one text object) leading to a \
         right-aligned page number. Reading 'entries': each line (title, leaders and number) is one block. Reading \
         'whole-contents': the list is one block.",
    );
    let st = doc(10.0);
    let entries = [("Introduction", "3"), ("Photometric data", "7"), ("Thermal management", "12"), ("Electrical connections", "15"), ("Certification", "19")];
    let mut lines = vec![];
    for (k, (t, n)) in entries.iter().enumerate() {
        let base = 100.0 + 12.0 * k as f32;
        let mut line = c.page.cut_line(&format!("e{k}t"), &st, 72.0, base, t, 2);
        let right = 72.0 + ink_w(t, 10.0);
        let dots = (((468.0 - (right + 4.0)) / (adv('.') * 10.0)).floor() as usize).max(3);
        let dl = format!("e{k}d");
        c.page.text(&dl, &st, right + 4.0, base, &".".repeat(dots));
        line.push(dl);
        let nl = format!("e{k}n");
        c.page.text_right(&nl, &st, 480.0, base, n);
        line.push(nl);
        lines.push(line);
    }
    c.reading("entries", lines.clone());
    c.reading("whole-contents", vec![lines.concat()]);
    c
}

fn c_baseline_jitter() -> Case {
    let mut c = Case::must(
        "ocr-like-baseline-jitter-within-a-line",
        "Two 10 pt ragged paragraphs (8 and 4 lines, blank line between) whose objects each sit up to 0.10 em (1 pt) above or \
         below the true baseline, as in an OCR text layer. A person sees ordinary lines: two blocks.",
    );
    let st = doc(10.0);
    let a = c.page.para(P::new("a", &st).at(72.0, 100.0).w(400.0).n(8).pitch(12.0).frags(4, 7).last(0.5));
    let b = c.page.para(P::new("b", &st).at(72.0, a.below(2.0)).w(400.0).n(4).pitch(12.0).frags(4, 7).last(0.6));
    let all = cat(vec![a.all(), b.all()]);
    c.page.jitter(0.10, &all);
    c.expect = vec![a.all(), b.all()];
    c
}

fn c_skewed_scan() -> Case {
    let mut c = Case::must(
        "scan-like-skew-of-every-line",
        "Two 10 pt ragged paragraphs (8 and 4 lines, blank line between) rotated by 0.17 degrees as by a slightly crooked scan: \
         every line drops 1.2 pt from its left to its right end (objects of one line differ by up to 1.2 pt in baseline). A \
         person sees ordinary lines: two blocks.",
    );
    let st = doc(10.0);
    let a = c.page.para(P::new("a", &st).at(72.0, 100.0).w(400.0).n(8).pitch(12.0).frags(4, 7).last(0.5));
    let b = c.page.para(P::new("b", &st).at(72.0, a.below(2.0)).w(400.0).n(4).pitch(12.0).frags(4, 7).last(0.6));
    let all = cat(vec![a.all(), b.all()]);
    c.page.skew(0.003, 72.0, &all);
    c.expect = vec![a.all(), b.all()];
    c
}

// ---------------------------------------------------------------------------------------------------
// added by R2
// ---------------------------------------------------------------------------------------------------

fn c_one_object_per_line() -> Case {
    let mut c = Case::must(
        "one-text-object-per-line-paragraphs-6pt-spacing",
        "Word-processor export: every visual line of the page is exactly ONE text object (no cut at a word gap or inside a word), \
         11 pt text on a 13.2 pt pitch, four ragged paragraphs separated only by 6 pt of space after (19.2 pt between the \
         baselines instead of 13.2); the second and third paragraphs end in nearly full lines. There is no gap inside any line \
         to analyse: only the vertical rhythm and the line lengths say where a paragraph ends. A person sees four paragraphs: \
         four blocks.",
    );
    let st = doc(11.0);
    let spec = [(5usize, 0.55f32), (4, 0.95), (6, 0.93), (3, 0.40)];
    let mut base = 100.0;
    for (k, (n, last)) in spec.iter().enumerate() {
        let p = c.page.para(P::new(&format!("p{k}"), &st).at(72.0, base).w(420.0).n(*n).pitch(13.2).frags(1, 1).last(*last));
        base = p.below(1.0) + 6.0;
        c.expect.push(p.all());
    }
    c
}

fn c_word_per_object_columns() -> Case {
    let mut c = Case::must(
        "ocr-style-one-object-per-word-justified-columns",
        "An OCR-style text layer: every WORD is its own text object (the box hugs the word, so every gap between two objects is a \
         visible word gap of 0.3-1.2 em and none is a tiny kerning gap), two justified columns in datasheet geometry (118.8 pt \
         measure, 12 pt gutter, 8 pt on a 9.6 pt pitch). The left column is one paragraph, the right column two paragraphs \
         (the break is a short last line). A person sees three paragraphs: three blocks.",
    );
    let st = body(8.0);
    let a = c.page.para(P::new("a", &st).at(X1, 359.0).w(COL_W).n(12).pitch(9.6).just().gap(0.3, 1.2).word_objects().last(0.5));
    let b = c.page.para(P::new("b", &st).at(X2, 359.0).w(COL_W).n(7).pitch(9.6).just().gap(0.3, 1.2).word_objects().last(0.45));
    let d = c.page.para(P::new("c", &st).at(X2, b.below(1.0)).w(COL_W).n(6).pitch(9.6).just().gap(0.3, 1.2).word_objects().last(0.55));
    c.expect = vec![a.all(), b.all(), d.all()];
    c
}

fn c_full_width_heading_and_footer_paragraph() -> Case {
    let mut c = Case::must(
        "full-width-heading-and-paragraph-around-two-columns",
        "The classic brochure page: a bold 11 pt heading across the whole text width (both columns and the gutter), 20 pt below \
         it two justified 8 pt columns of 9 lines (118.8 pt measure, 12 pt gutter), and 14 pt under their last baseline a \
         full-width justified paragraph of 4 lines. The wide lines overlap both columns horizontally. A person sees the \
         heading, the two columns and the closing paragraph: four blocks (the wide objects must not glue the columns together).",
    );
    let (h, b) = (head(11.0), body(8.0));
    let hl = c.page.cut_line("h", &h, X1, 345.0, "Technical performance overview", 4);
    let l = c.page.para(P::new("l", &b).at(X1, 365.0).w(COL_W).n(9).pitch(9.6).just().frags(3, 8).last(0.50));
    let r = c.page.para(P::new("r", &b).at(X2, 365.0).w(COL_W).n(9).pitch(9.6).just().frags(3, 8).last(0.62));
    let w = 2.0 * COL_W + GUT;
    let f = c.page.para(P::new("f", &b).at(X1, l.last_base() + 14.0).w(w).n(4).pitch(9.6).just().frags(4, 8).last(0.40));
    c.expect = vec![hl, l.all(), r.all(), f.all()];
    c
}

fn c_right_aligned_paragraphs() -> Case {
    let mut c = Case::must(
        "right-aligned-ragged-left-paragraphs-rtl-layout",
        "Three right-aligned 10 pt paragraphs of 5, 4 and 6 lines, right edge flush at x=522, left edges ragged (the layout of Arabic, \
         Persian and Hebrew running text); the first gap is one blank line, the second 6 pt of space after, the last lines are \
         55, 90 and 40 % of the measure. Every left edge differs from the one above it, so no line looks like an indent of \
         its neighbour. A person sees three paragraphs: three blocks.",
    );
    let st = doc(10.0);
    let a = c.page.para(P::new("a", &st).at(222.0, 100.0).w(300.0).n(5).pitch(12.0).right().frags(1, 3).last(0.55));
    let b = c.page.para(P::new("b", &st).at(222.0, a.below(2.0)).w(300.0).n(4).pitch(12.0).right().frags(1, 3).last(0.90));
    let d = c.page.para(P::new("c", &st).at(222.0, b.below(1.0) + 6.0).w(300.0).n(6).pitch(12.0).right().frags(1, 3).last(0.40));
    c.expect = vec![a.all(), b.all(), d.all()];
    c
}

fn c_double_spaced_manuscript() -> Case {
    let mut c = Case::must(
        "double-spaced-manuscript-indent-is-the-only-cue",
        "A typewriter manuscript: 12 pt text double spaced (24 pt pitch = 2.0 em), three paragraphs of 5, 6 and 4 lines with a 36 pt \
         first-line indent and no other mark; the first two paragraphs end in nearly full lines (95 and 92 %). The line pitch is \
         far wider than in running text, so a fixed 'line distance' limit would see every line as a block of its own. A person \
         sees three paragraphs: three blocks.",
    );
    let st = doc(12.0);
    let a = c.page.para(P::new("a", &st).at(72.0, 100.0).w(450.0).n(5).pitch(24.0).frags(3, 6).indent(36.0).last(0.95));
    let b = c.page.para(P::new("b", &st).at(72.0, a.below(1.0)).w(450.0).n(6).pitch(24.0).frags(3, 6).indent(36.0).last(0.92));
    let d = c.page.para(P::new("c", &st).at(72.0, b.below(1.0)).w(450.0).n(4).pitch(24.0).frags(3, 6).indent(36.0).last(0.50));
    c.expect = vec![a.all(), b.all(), d.all()];
    c
}

fn c_tight_leading() -> Case {
    let mut c = Case::must(
        "tight-leading-paragraphs-overlapping-line-boxes",
        "Dense small print: 9 pt text on a 7.9 pt pitch (0.88 em, tighter than the type is tall, so a descender of one line and \
         an ascender of the next line overlap vertically), three paragraphs of 6, 4 and 5 lines separated by one blank line \
         (15.8 pt). A person sees three paragraphs of cramped text: three blocks.",
    );
    c.overlap_ok = true; // the line boxes of one paragraph overlap by design
    let st = doc(9.0);
    let a = c.page.para(P::new("a", &st).at(72.0, 100.0).w(380.0).n(6).pitch(7.9).frags(3, 6).last(0.50));
    let b = c.page.para(P::new("b", &st).at(72.0, a.below(2.0)).w(380.0).n(4).pitch(7.9).frags(3, 6).last(0.90));
    let d = c.page.para(P::new("c", &st).at(72.0, b.below(2.0)).w(380.0).n(5).pitch(7.9).frags(3, 6).last(0.40));
    c.expect = vec![a.all(), b.all(), d.all()];
    c
}

fn c_two_columns_tight_gutter() -> Case {
    let mut c = Case::must(
        "justified-two-columns-tight-gutter-barely-wider-than-word-gaps",
        "Two justified 8 pt columns (118.8 pt measure, 9.6 pt pitch) whose gutter is only 1.1 em (8.8 pt) while the justified word \
         gaps run up to 1.0 em (8 pt): the white band between the columns is just 0.8 pt wider than the widest word gap, but it \
         is straight and continuous down all 12 / 10 lines while word gaps wander. A person sees two columns: two blocks.",
    );
    let st = body(8.0);
    let a = c.page.para(P::new("a", &st).at(X1, 359.0).w(COL_W).n(12).pitch(9.6).just().gap(0.3, 1.0).frags(3, 8).last(0.55));
    let b = c.page.para(P::new("b", &st).at(X1 + COL_W + 8.8, 359.0).w(COL_W).n(10).pitch(9.6).just().gap(0.3, 1.0).frags(3, 8).last(0.45));
    c.expect = vec![a.all(), b.all()];
    c
}

fn c_inline_underlines() -> Case {
    let mut c = Case::must(
        "underlined-words-and-hyperlinks-inside-a-paragraph",
        "A 9-line ragged 10 pt paragraph in which four words carry a hairline underline (a thin path 1.4 pt under the baseline, \
         as for hyperlinks: the rule runs under one text object only), followed after a blank line by a 3-line paragraph. The \
         underlines lie between two text lines geometrically but belong to their own line. A person sees ordinary text with \
         underlined words: two blocks (an underline under a word is not a rule between paragraphs).",
    );
    let st = doc(10.0);
    let p = c.page.para(P::new("p", &st).at(72.0, 100.0).w(400.0).n(9).pitch(12.0).frags(4, 6).last(0.5));
    let q = c.page.para(P::new("q", &st).at(72.0, p.below(2.0)).w(400.0).n(3).pitch(12.0).frags(3, 6).last(0.6));
    for (k, (line, idx)) in [(1usize, 1usize), (3, 0), (5, 2), (6, 1)].iter().enumerate() {
        let (l, r, base) = {
            let f = c.page.get(&p.lines[*line][*idx]);
            (f.left, f.right, f.baseline)
        };
        c.page.rule_h(&format!("ul{k}"), l, r, base + 1.4);
    }
    c.expect = vec![p.all(), q.all()];
    c
}

fn c_highlight_boxes() -> Case {
    let mut c = Case::must(
        "highlight-boxes-behind-words-inside-a-paragraph",
        "An 8-line ragged 10 pt paragraph in which two phrases (two text objects each, in the middle of lines 3 and 6, text on \
         both sides) sit on a filled highlight rectangle exactly one text line high; a blank line, then a 3-line paragraph. A \
         person sees a highlighted phrase inside one paragraph: two blocks (a one-line box round part of a line is not a drawn \
         box round a block of text).",
    );
    let st = doc(10.0);
    let p = c.page.para(P::new("p", &st).at(72.0, 100.0).w(400.0).n(8).pitch(12.0).frags(5, 7).last(0.5));
    let q = c.page.para(P::new("q", &st).at(72.0, p.below(2.0)).w(400.0).n(3).pitch(12.0).frags(3, 6).last(0.6));
    for (k, line) in [2usize, 5].iter().enumerate() {
        let (l, r, base) = {
            let (a, b) = (c.page.get(&p.lines[*line][1]), c.page.get(&p.lines[*line][2]));
            (a.left, b.right, a.baseline)
        };
        c.page.rect(&format!("hl{k}"), l - 1.0, base - 8.6, r + 1.0, base + 2.6);
    }
    c.expect = vec![p.all(), q.all()];
    c
}

fn c_business_letter() -> Case {
    let mut c = Case::must(
        "business-letter-one-object-per-line-mixed-elements",
        "A business letter as a word processor exports it (every line of text is ONE text object): a bold 16 pt letterhead name with a \
         right-aligned 8.5 pt contact line 14 pt above its right end and a rule under both, then date, bold subject, salutation, a \
         5-line and a 4-line paragraph, a three-item bulleted list (bullet objects at x=90, text at x=108, 2 / 1 / 2 lines, no space \
         between the items), a 3-line paragraph, the closing line, the sender's bold name 52 pt lower, and a centred footer under a \
         rule. 11 pt text on a 13.2 pt pitch with one blank line between the elements. A person sees every element as a block of \
         its own: 14 blocks.",
    );
    let (b, t) = (doc_bold(11.0), doc(11.0));
    c.page.text("lh.name", &doc_bold(16.0), 72.0, 72.0, "HSI Lighting FZE");
    c.page.text_right("lh.contact", &doc(8.5), 523.0, 58.0, "www.hsilighting.com   |   +971 4 555 0123   |   info@hsilighting.com");
    c.page.rule_h("lh.rule", 72.0, 523.0, 82.0);
    c.page.text("date", &t, 72.0, 125.0, "3 October 2026");
    c.page.text("subj", &b, 72.0, 160.0, "Subject: Quotation DQ-1042 for recessed downlights");
    c.page.text("sal", &t, 72.0, 190.0, "Dear Mr. Rahman,");
    let p1 = c.page.para(P::new("p1", &t).at(72.0, 216.0).w(451.0).n(5).pitch(13.2).frags(1, 1).last(0.55));
    let p2 = c.page.para(P::new("p2", &t).at(72.0, p1.below(2.0)).w(451.0).n(4).pitch(13.2).frags(1, 1).last(0.90));
    c.expect = vec![lab("lh.name"), lab("lh.contact"), lab("date"), lab("subj"), lab("sal"), p1.all(), p2.all()];
    let mut base = p2.below(2.0);
    for (k, (n, last)) in [(2usize, 0.5f32), (1, 0.9), (2, 0.4)].iter().enumerate() {
        let bl = format!("li{k}.b");
        c.page.text(&bl, &t, 90.0, base, "\u{2022}");
        let it = c.page.para(P::new(&format!("li{k}"), &t).at(108.0, base).w(415.0).n(*n).pitch(13.2).frags(1, 1).last(*last));
        base = it.below(1.0);
        c.expect.push(cat(vec![lab(&bl), it.all()]));
    }
    let p3 = c.page.para(P::new("p3", &t).at(72.0, base + 13.2).w(451.0).n(3).pitch(13.2).frags(1, 1).last(0.60));
    c.page.text("close", &t, 72.0, p3.below(2.0), "Yours sincerely,");
    c.page.text("name", &b, 72.0, p3.below(2.0) + 52.0, "Ahmed Al Mansoori");
    c.page.rule_h("foot.rule", 72.0, 523.0, 790.0);
    c.page.text_centre("foot", &doc(8.0), 297.5, 806.0, "HSI Lighting FZE   |   Office 12, Business Bay, Dubai   |   Trade licence 123456");
    c.expect.push(p3.all());
    c.expect.push(lab("close"));
    c.expect.push(lab("name"));
    c.expect.push(lab("foot"));
    c
}

fn c_two_column_paper() -> Case {
    let mut c = Case::must(
        "two-column-paper-page-title-abstract-headings-caption-references",
        "A page of a scientific paper: a centred two-line 17 pt bold title across the page, a centred author line, an abstract inset \
         30 pt on both sides (8.5 pt, opening with a bold run-in 'Abstract.'), then two justified 9 pt columns (235 pt wide, 17 pt \
         gutter, 11 pt pitch). Left column: heading '1. Introduction', two paragraphs with 12 pt first-line indents and NO space \
         between them (indent and a short last line are the only cues), heading '2. Method', a paragraph, and a footnote under a \
         short rule (bold '*' marker, 7.5 pt, 2 lines). Right column: a paragraph, a figure frame of shapes with a two-line italic \
         caption, heading '3. Results', a paragraph, heading 'References' and three hanging-indent entries (7.5 pt, two lines each, \
         no space between them). A person sees title, authors, abstract, every heading, paragraph, caption, footnote and reference \
         entry as a block of its own: 17 blocks.",
    );
    const CW: f32 = 235.0;
    let (xl, xr) = (54.0f32, 54.0 + CW + 17.0);
    let body = doc(9.0);
    let title = c.page.para(P::new("title", &doc_bold(17.0)).at(54.0, 80.0).w(487.0).n(2).pitch(20.0).centre().frags(1, 3).last(0.6));
    let auth_base = title.last_base() + 26.0;
    c.page.text_centre("auth", &doc(10.0), 297.5, auth_base, "A. Rahman, B. Singh and C. Lee");
    let ab_base = auth_base + 30.0;
    c.page.text("abs.lead", &doc_bold(8.5), 84.0, ab_base, "Abstract.");
    let abs = c.page.para(
        P::new("abs", &doc(8.5)).at(84.0, ab_base).w(427.0).n(5).pitch(10.2).just().indent(ink_w("Abstract.", 8.5) + 3.5).frags(3, 6).last(0.5),
    );
    let y0 = abs.last_base() + 32.0;
    // left column
    let h1 = c.page.cut_line("h1", &doc_bold(10.0), xl, y0, "1. Introduction", 2);
    let l1 = c.page.para(P::new("l1", &body).at(xl, y0 + 14.0).w(CW).n(9).pitch(11.0).just().indent(12.0).frags(3, 6).last(0.45));
    let l2 = c.page.para(P::new("l2", &body).at(xl, l1.below(1.0)).w(CW).n(8).pitch(11.0).just().indent(12.0).frags(3, 6).last(0.60));
    let h2b = l2.last_base() + 22.0;
    let h2 = c.page.cut_line("h2", &doc_bold(10.0), xl, h2b, "2. Method", 2);
    let l3 = c.page.para(P::new("l3", &body).at(xl, h2b + 14.0).w(CW).n(7).pitch(11.0).just().frags(3, 6).last(0.35));
    let fy = l3.last_base() + 44.0;
    c.page.rule_h("fn.rule", xl, xl + 60.0, fy - 9.0);
    c.page.text("fn.star", &doc_bold(7.5), xl, fy, "*");
    let fnp = c.page.para(P::new("fn", &doc(7.5)).at(xl, fy).w(CW).n(2).pitch(9.0).just().indent(6.0).frags(2, 5).last(0.7));
    // right column
    let r1 = c.page.para(P::new("r1", &body).at(xr, y0 + 14.0).w(CW).n(6).pitch(11.0).just().frags(3, 6).last(0.40));
    let fig_top = r1.last_base() + 12.0;
    c.page.rect("fig.frame", xr, fig_top, xr + CW, fig_top + 90.0);
    c.page.rect("fig.curve", xr + 20.0, fig_top + 15.0, xr + CW - 20.0, fig_top + 75.0);
    let cap = c.page.para(P::new("cap", &doc_italic(8.0)).at(xr, fig_top + 102.0).w(CW).n(2).pitch(9.6).frags(2, 4).last(0.6));
    let h3b = cap.last_base() + 22.0;
    let h3 = c.page.cut_line("h3", &doc_bold(10.0), xr, h3b, "3. Results", 2);
    let r2 = c.page.para(P::new("r2", &body).at(xr, h3b + 14.0).w(CW).n(8).pitch(11.0).just().frags(3, 6).last(0.5));
    let h4b = r2.last_base() + 22.0;
    let h4 = c.page.cut_line("h4", &doc_bold(10.0), xr, h4b, "References", 1);
    let mut by = h4b + 12.0;
    let mut refs = vec![];
    for (k, (n, last)) in [(2usize, 0.6f32), (2, 0.45), (2, 0.8)].iter().enumerate() {
        let r = c.page.para(P::new(&format!("ref{k}"), &doc(7.5)).at(xr, by).w(CW).n(*n).pitch(9.0).hang(10.0).frags(2, 5).last(*last));
        by = r.below(1.0);
        refs.push(r.all());
    }
    c.expect = vec![
        title.all(),
        lab("auth"),
        cat(vec![lab("abs.lead"), abs.all()]),
        h1,
        l1.all(),
        l2.all(),
        h2,
        l3.all(),
        cat(vec![lab("fn.star"), fnp.all()]),
        r1.all(),
        cap.all(),
        h3,
        r2.all(),
        h4,
    ];
    c.expect.extend(refs);
    c
}

fn c_quotation_table() -> Case {
    let mut c = Case::must(
        "quotation-table-ruled-rows-with-right-aligned-numbers",
        "A quotation as an ERP prints it: a bold header row ('Description', 'Qty', 'Unit price (AED)', 'Total (AED)') over a hairline \
         rule, five item rows with a hairline rule under each, whose description is 1 or 2 lines of 9 pt text at x=72 and whose three \
         numbers are right-aligned at x=350, 430 and 523 on the row's first baseline, then three total rows (label right-aligned at 430, \
         value at 523, the last one bold, a short rule under each). The description and its numbers are 4-30 em apart, the numbers of \
         neighbouring columns 6-10 em. A person sees every ruled item row and every total row as one unit (as the user marked the \
         datasheet's label/value rows): 8 blocks. The four header objects are NOT asserted (free): a column title can be read as a \
         cell of its own or as one header row.",
    );
    let (hd, st) = (doc_bold(9.0), doc(9.0));
    let (x0, x1) = (72.0f32, 523.0f32);
    let mut top = 100.0f32;
    let hb = top + 12.0;
    c.page.text("h.desc", &hd, x0, hb, "Description");
    c.page.text_right("h.qty", &hd, 350.0, hb, "Qty");
    c.page.text_right("h.unit", &hd, 430.0, hb, "Unit price (AED)");
    c.page.text_right("h.tot", &hd, 523.0, hb, "Total (AED)");
    c.free = ["h.desc", "h.qty", "h.unit", "h.tot"].iter().map(|s| s.to_string()).collect();
    top += 18.0;
    c.page.rule_h("h.rule", x0, x1, top);
    let items: [(&str, Option<&str>, &str, &str, &str); 5] = [
        ("Recessed downlight VEGA 150, 4000K, 40W", None, "12", "185.00", "2,220.00"),
        ("Track spot light 30W, black housing,", Some("24 degree optic with anti-glare snoot"), "8", "240.00", "1,920.00"),
        ("Driver 1050mA dimmable 1-10V", None, "20", "62.50", "1,250.00"),
        ("Linear suspended profile 1200mm, aluminium,", Some("opal diffuser and two mounting cables"), "6", "410.00", "2,460.00"),
        ("Installation and commissioning", None, "1", "900.00", "900.00"),
    ];
    for (k, (l1, l2, q, u, t)) in items.iter().enumerate() {
        let base = top + 12.0;
        let mut row = c.page.cut_line(&format!("r{k}.a"), &st, x0, base, l1, 2);
        if let Some(l2) = l2 {
            row.extend(c.page.cut_line(&format!("r{k}.b"), &st, x0, base + 11.0, l2, 2));
        }
        row.push(c.page.text_right(&format!("r{k}.q"), &st, 350.0, base, q));
        row.push(c.page.text_right(&format!("r{k}.u"), &st, 430.0, base, u));
        row.push(c.page.text_right(&format!("r{k}.t"), &st, 523.0, base, t));
        top += if l2.is_some() { 29.0 } else { 18.0 };
        c.page.rule_h(&format!("r{k}.rule"), x0, x1, top);
        c.expect.push(row);
    }
    for (k, (label, value, bold)) in [("Subtotal", "8,750.00", false), ("VAT 5 %", "437.50", false), ("Total (AED)", "9,187.50", true)].iter().enumerate() {
        let s = if *bold { &hd } else { &st };
        let base = top + 12.0;
        let a = c.page.text_right(&format!("t{k}.l"), s, 430.0, base, label);
        let b = c.page.text_right(&format!("t{k}.v"), s, 523.0, base, value);
        top += 18.0;
        c.page.rule_h(&format!("t{k}.rule"), 340.0, x1, top);
        c.expect.push(vec![a, b]);
    }
    c
}

/// One ruled label/value row of the datasheet's left column (x 17-172, 7.5 pt): `two_line` puts the value on the line below the
/// label (rule 3.4 pt under it, row 21 pt high), otherwise label at x=20 and value at x=112 on one baseline (row 11 pt high).
fn ds_row(c: &mut Case, k: usize, top: &mut f32, label: &str, value: &str, two_line: bool) {
    let (ls, vs) = (semibold(7.5), body(7.5));
    let base = *top + 8.0;
    let mut row = c.page.cut_line(&format!("r{k}l"), &ls, 20.0, base, label, 2);
    if two_line {
        row.extend(c.page.cut_line(&format!("r{k}v"), &vs, 20.0, base + 9.6, value, 3));
        *top += 21.0;
    } else {
        row.extend(c.page.cut_line(&format!("r{k}v"), &vs, 112.0, base, value, 2));
        *top += 11.0;
    }
    c.page.rule_h(&format!("r{k}.rule"), 17.0, 172.0, *top);
    c.expect.push(row);
}

fn c_datasheet_left_column_twin() -> Case {
    let mut c = Case::must(
        "datasheet-left-column-twin-rows-subheading-twin-heading-marks-caption",
        "A synthetic twin of the datasheet's left column (x 17-172, 7.5 pt rows, labels semibold at x=20, values light at x=112, one face \
         name for every weight): a bold title row over a rule, two two-line rows (label line, value line, one rule under the value), six \
         one-line rows, the bold underlined sub-heading 'FIXTURE DATA:', a two-line row and four one-line rows, then 'Color Options' drawn \
         twice at the identical position above a grey swatch holding one letter, 'Compliance' with five bold marks each inside its own \
         rectangle (6 pt text, 14-20 pt apart), and a one-line caption. Every row between two rules is one block (the user's marking), \
         every heading, mark, swatch letter and the caption is a block of its own.",
    );
    c.overlap_ok = true; // the twin heading's objects overlap their originals by design
    let hd = head(8.0);
    let mut top = 190.0f32;
    let title = c.page.cut_line("title", &hd, 20.0, top + 8.0, "HS150LC-2006-A", 2);
    top += 11.0;
    c.page.rule_h("title.rule", 17.0, 172.0, top);
    c.expect.push(title);
    let mut k = 0;
    for (l, v, two) in [
        ("Power Input:", "40W", true),
        ("Current Input:", "1050mA respectively for power", true),
        ("System Lumen:", "4000lm", false),
        ("COB Efficacy:", "Max 180 lm/Watt", false),
        ("CCT:", "3500K,", false),
        ("CRI:", ">90", false),
        ("Driver Power Factor:", ">90", false),
        ("Sound Rating:", "Class A", false),
    ] {
        ds_row(&mut c, k, &mut top, l, v, two);
        k += 1;
    }
    let fx = c.page.cut_line("fx", &hd, 20.0, top + 8.0, "FIXTURE DATA:", 2);
    top += 11.0;
    c.page.rule_h("fx.rule", 17.0, 172.0, top);
    c.expect.push(fx);
    for (l, v, two) in [
        ("Recommended for indoor plaster Ceiling:", "Recessed", true),
        ("Material:", "Die-cast Aluminium", false),
        ("Color:", "Black/ White", false),
        ("Finishing:", "Matt powder coated", false),
        ("Beam Angle:", "Spot, Medium, Flood", false),
    ] {
        ds_row(&mut c, k, &mut top, l, v, two);
        k += 1;
    }
    // heading drawn twice at the identical position, then a swatch with one letter in it
    let co_base = top + 30.0;
    let co = c.page.cut_line("co", &hd, 20.0, co_base, "Color Options", 3);
    let mut co_all = co.clone();
    for l in &co {
        co_all.push(c.page.twin(l));
    }
    c.expect.push(co_all);
    c.page.rect("sw", 20.0, co_base + 8.0, 100.0, co_base + 30.0);
    c.page.text_centre("sw.c", &hd.clone().size(9.0), 60.0, co_base + 22.0, "C");
    c.expect.push(lab("sw.c"));
    // compliance marks: five labels, each inside its own rectangle
    let cp_base = co_base + 62.0;
    let cp = c.page.cut_line("cp", &hd, 20.0, cp_base, "Compliance", 2);
    c.expect.push(cp);
    let marks = ["CE", "CB", "ESMA", "ROHS", "KUKAS"];
    let mut x = 20.0f32;
    for (i, m) in marks.iter().enumerate() {
        let st = hd.clone().size(6.0);
        let w = ink_w(m, 6.0) + 10.0;
        let (by, label) = (cp_base + 24.0, format!("mk{i}"));
        c.page.rect(&format!("mk{i}.box"), x, by - 9.0, x + w, by + 3.0);
        c.page.text_centre(&label, &st, x + w / 2.0, by, m);
        c.expect.push(lab(&label));
        x += w + 8.0;
    }
    c.page.text("cap", &body(7.0), 20.0, cp_base + 56.0, "Subject to technical alternations.");
    c.expect.push(lab("cap"));
    c
}

// @@MORE-CASES@@

fn c_heavy_30000() -> Case {
    let mut c = Case::must(
        "thirty-thousand-fragments-ten-columns",
        "HEAVY (skippable with PAGIFY_BLOCKS_SKIP_HEAVY=1): one very large page of exactly 30,000 text objects: ten \
         justified columns, each holding twenty 25-line paragraphs separated by a blank line, six objects on every line. \
         A person sees 200 paragraphs. The expectation is the plain grouping, and detect must finish within the time bound \
         (60 s in a release build; PAGIFY_BLOCKS_HEAVY_SECS overrides) even though a quadratic pass over 30,000 objects \
         is already a few seconds.",
    );
    c.heavy = true;
    let st = body(8.0);
    let mut groups = vec![];
    for col in 0..10 {
        let x = 20.0 + col as f32 * (COL_W + GUT);
        let mut base = 30.0;
        for k in 0..20 {
            let p = c.page.para(P::new(&format!("c{col}p{k}"), &st).at(x, base).w(COL_W).n(25).pitch(9.6).just().frags(6, 6).last(0.5));
            base = p.below(2.0);
            groups.push(p.all());
        }
    }
    c.expect = groups;
    c
}

fn c_heavy_one_column() -> Case {
    let mut c = Case::must(
        "thirty-thousand-fragments-one-tall-column",
        "HEAVY (skippable with PAGIFY_BLOCKS_SKIP_HEAVY=1): the other worst case for complexity: ONE justified column (118.8 pt wide) \
         holding 200 paragraphs of 25 lines with six objects per line, 30,000 objects in a single chain of 5,000 rows about 52,000 pt \
         tall, so that any pass comparing every row with every other row (instead of its neighbours) takes seconds. A person sees 200 \
         paragraphs.",
    );
    c.heavy = true;
    let st = body(8.0);
    let mut base = 30.0;
    for k in 0..200 {
        let p = c.page.para(P::new(&format!("p{k}"), &st).at(X1, base).w(COL_W).n(25).pitch(9.6).just().frags(6, 6).last(0.5));
        base = p.below(2.0);
        c.expect.push(p.all());
    }
    c
}

fn c_heavy_one_row() -> Case {
    let mut c = Case::must(
        "thirty-thousand-fragments-in-one-row",
        "HEAVY (skippable with PAGIFY_BLOCKS_SKIP_HEAVY=1): one text line of 30,000 words, every word its own text object (about \
         750,000 pt wide: an extreme ticker / log line), with a 4-line paragraph 40 pt below it. Any pass that compares every object \
         of a row with every other (a gap test, a corridor test) is quadratic here. A person sees one line and one paragraph: two \
         blocks, the long line whole.",
    );
    c.heavy = true;
    let st = body(8.0);
    let words: Vec<String> = (0..30_000).map(|_| rand_word(&mut c.page.rng)).collect();
    let (long, _, _) = c.page.put_line("long", &st, 20.0, 100.0, &words, adv(' ') * 8.0, 0, None, true);
    let p = c.page.para(P::new("p", &st).at(20.0, 140.0).w(300.0).n(4).pitch(9.6).frags(2, 4).last(0.6));
    c.expect = vec![long, p.all()];
    c
}


/// Phase of [`oracle_self_check_standins_mutants_and_perfect`]: the perfect
/// detector over MUST cases on three other seeds. Returns the report line
/// and appends failures to `problems`.
fn self_check_seeded(must: &[&Case], problems: &mut Vec<String>) -> String {
    let mut report = String::new();
    let (mut seeded_runs, mut seeded_fail) = (0usize, vec![]);
    for k in 1..=3u64 {
        let cs = all_cases_seeded(false, k);
        for c in cs.iter().filter(|c| c.strength == Strength::Must) {
            let det = perfect(c, Mutation::None);
            for v in &FULL_VARIANTS {
                seeded_runs += 1;
                let ev = evaluate(c, &det, v, true);
                if !ev.problems.is_empty() {
                    seeded_fail.push(format!("{} [seed {k}, {}]: {}", c.name, v.name, ev.problems.join(" | ")));
                }
            }
        }
    }
    report.push_str(&format!(
        "SELF-CHECK perfect detector on 3 other seeds: {} of {seeded_runs} case x variant runs clean\n",
        seeded_runs - seeded_fail.len()
    ));
    problems.extend(seeded_fail);
    report
}

/// Phase of [`oracle_self_check_standins_mutants_and_perfect`]: split/merge/
/// peel mutants must be caught by the checker.
fn self_check_mutants(must: &[&Case], problems: &mut Vec<String>) -> String {
    let mut report = String::new();
    for (name, m) in [
        ("split-largest-block", Mutation::Split),
        ("merge-first-two-blocks", Mutation::Merge),
        ("peel-one-fragment-off-the-largest-block", Mutation::Peel),
    ] {
        let (mut applicable, mut caught) = (0, 0);
        let mut missed = vec![];
        for c in &must {
            let groups_ok = c.check_groups
                && match m {
                    Mutation::Split | Mutation::Peel => c.expect.iter().any(|g| g.len() >= 2),
                    Mutation::Merge => c.expect.len() >= 2,
                    Mutation::None => false,
                };
            if !groups_ok {
                continue;
            }
            applicable += 1;
            let det = perfect(c, m);
            let ev = evaluate(c, &det, &V_X1, true);
            if ev.problems.is_empty() {
                missed.push(c.name);
            } else {
                caught += 1;
            }
        }
        report.push_str(&format!(
            "SELF-CHECK mutant '{name}': caught by the checker in {caught}/{applicable} applicable MUST cases\n"
        ));
        if !missed.is_empty() {
            problems.push(format!("mutant '{name}' NOT caught in: {}", missed.join(", ")));
        }
    }
    report
}

/// Phase of [`oracle_self_check_standins_mutants_and_perfect`]: contract
/// breakers must be flagged by the invariant checks.
fn self_check_breakers(must: &[&Case], problems: &mut Vec<String>) -> String {
    let mut report = String::new();
    for (name, kind) in [
        ("drop-one-object (coverage)", Breaker::DropOne),
        ("report-a-block-twice (exactly-once)", Breaker::DuplicateBlock),
        ("alternate-block-order-between-calls (determinism)", Breaker::Flaky),
        ("block-box-that-does-not-enclose-its-members", Breaker::ShrinkBox),
    ] {
        let (mut applicable, mut flagged) = (0, 0);
        let mut missed = vec![];
        for c in &must {
            let ok = c.check_groups
                && !c.degenerate
                && !c.dup_ids
                && !c.heavy
                && match kind {
                    Breaker::DropOne => c.expect.iter().any(|g| g.len() >= 2),
                    Breaker::DuplicateBlock | Breaker::ShrinkBox => !c.expect.is_empty(),
                    Breaker::Flaky => c.expect.len() >= 2,
                };
            if !ok {
                continue;
            }
            applicable += 1;
            let base = perfect(c, Mutation::None);
            let flip = std::cell::Cell::new(false);
            let det = |f: &[Frag], s: &[Shape]| {
                let mut b = base(f, s);
                match kind {
                    Breaker::DropOne => {
                        if let Some(blk) = b.iter_mut().find(|x| x.objects().len() >= 2) {
                            if let Some(l) = blk.lines.iter_mut().rev().find(|l| !l.objects.is_empty()) {
                                l.objects.pop();
                            }
                        }
                    }
                    Breaker::DuplicateBlock => {
                        if let Some(first) = b.first().cloned() {
                            b.push(first);
                        }
                    }
                    Breaker::Flaky => {
                        flip.set(!flip.get());
                        if flip.get() {
                            b.reverse();
                        }
                    }
                    Breaker::ShrinkBox => {
                        for blk in b.iter_mut() {
                            blk.right = blk.left - 5.0;
                        }
                    }
                }
                b
            };
            let ev = evaluate(c, &det, &V_X1, false);
            if ev.problems.iter().any(|p| p.starts_with("INV")) {
                flagged += 1;
            } else {
                missed.push(c.name);
            }
        }
        report.push_str(&format!(
            "SELF-CHECK contract breaker '{name}': flagged by the invariant checks in {flagged}/{applicable} applicable MUST cases\n"
        ));
        if !missed.is_empty() {
            problems.push(format!("contract breaker '{name}' NOT flagged in: {}", missed.join(", ")));
        }
    }
    report
}
