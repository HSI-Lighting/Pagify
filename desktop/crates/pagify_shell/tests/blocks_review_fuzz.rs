//! V4 REVIEW FUZZ (not part of the product's suite; every test here is #[ignore]d).
//!
//! An independent page generator (datasheet-like PDFium geometry: justified and ragged paragraphs cut into 3-8 objects,
//! headings, lists, rule-less tables, outlined words, twins, rotated/size-0/alpha-0/empty runs), an independent restatement of the
//! editor contract C1..C13 written from the scout's text, and these properties of pagify_shell::blocks / block_input:
//!   fuzz     realistic pages: every object once, the guard agrees with the independent contract, determinism, shuffle,
//!            translation and scale invariance, cross-column interference, ragged-rule false cuts, rule-less tables
//!   hostile  NaN/inf/zero/negative/huge numbers, duplicate ids, extreme Params: no panic, every object once, ordering and
//!            enclosure, the guard never accepts a violation, guard completeness by block mutation
//!   text     unicode, line breaks, RTL, sentence ends, typography, faux bold
//!   repro    minimal reproductions of the defects found (they assert the CORRECT behaviour and fail today)
//!   perf     growth from 10^4 to 10^5 fragments, realistic and adversarial
//!
//!   cargo test -p pagify_shell --release --test blocks_review_fuzz -- --ignored --nocapture [name]
//!   V4_NOASSERT=1 turns every violation into a printed fact; V4_SEEDS / V4_HOSTILE / V4_MUTSEEDS scale the seed counts.
//!   Debug and release both work; `--profile` with overflow checks is the strongest for the hostile fuzz.
#![allow(dead_code, unused_imports, unused_variables, unused_mut)]

mod v4gen {
#![allow(dead_code)]

use pagify_shell::block_input::*;
use pagify_shell::blocks::{self, Block, Frag, Shape};
use pdf_core::document::{Color, DrawnKind, DrawnObject, Point, Rect, RunStyle, TextRun};
use std::collections::{BTreeSet, HashMap, HashSet};

pub mod common {
#![allow(dead_code)]

use super::*;
use std::panic::{catch_unwind, AssertUnwindSafe};

pub fn seeds_env(name: &str, default: u64) -> u64 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

pub fn noassert() -> bool {
    std::env::var("V4_NOASSERT").map(|v| v == "1").unwrap_or(false)
}

/// Run `f` for seeds base..base+n, catching panics. Returns (failures, first examples).
pub fn run_seeds(name: &str, base: u64, n: u64, f: impl Fn(u64) -> Result<(), String>) -> (u64, Vec<String>) {
    let mut failed = 0u64;
    let mut ex: Vec<String> = Vec::new();
    for s in base..base + n {
        let res = catch_unwind(AssertUnwindSafe(|| f(s)));
        let msg = match res {
            Ok(Ok(())) => continue,
            Ok(Err(m)) => m,
            Err(p) => format!("PANIC: {}", p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default()),
        };
        failed += 1;
        if ex.len() < 6 {
            ex.push(format!("seed {s}: {msg}"));
        }
    }
    eprintln!("[{name}] seeds {base}..{} : {failed} failed of {n}", base + n);
    for e in &ex {
        eprintln!("    {e}");
    }
    (failed, ex)
}

pub fn finish(name: &str, failed: u64) {
    if failed > 0 && !noassert() {
        panic!("{name}: {failed} failing seeds (see stderr)");
    }
}

pub fn opts_for(seed: u64) -> PageOpts {
    let mut r = Rng::new(seed ^ 0xABCD);
    PageOpts { columns: r.between(1, 3), body: [4.0, 6.0, 8.0, 9.0, 10.0, 12.0][r.int(6)], weird: r.chance(0.7), outlined: r.chance(0.6), tables: true, lists: true, height: 792.0, gap_extra: None, rtl: false, force_justify: None }
}

}
pub mod hostile {
#![allow(dead_code)]

use super::*;

pub fn hostile_f32(r: &mut Rng, lo: f32, hi: f32) -> f32 {
    const SPECIAL: [f32; 22] = [
        0.0, -0.0, 1.0, -1.0, 0.5, -0.5, 9.0, 612.0, 792.0, 1e-30, 1e30, f32::MAX, f32::MIN, f32::MIN_POSITIVE, f32::EPSILON, f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1e-45, 3.4e38, -3.4e38, 1e19,
    ];
    if r.chance(0.18) {
        SPECIAL[r.int(SPECIAL.len())]
    } else {
        r.range(lo as f64, hi as f64) as f32
    }
}

pub const TEXTS: [&str; 16] = [
    "", " ", "  ", "word", "word ", " word", "two words", "a\nb", "a\r\nb", "x\u{2}", "caf\u{e9}", "\u{1F600}\u{1F600}", "e\u{301}e\u{301}", "\u{0627}\u{0644}\u{0639}\u{0631}\u{0628}\u{064A}\u{0629}", "\u{4F60}\u{597D}\u{4E16}\u{754C}", "\u{200B}\u{FEFF}",
];

pub fn hostile_frag(r: &mut Rng, id: usize, mode: u32) -> Frag {
    let x = hostile_f32(r, 0.0, 600.0);
    let y = hostile_f32(r, 0.0, 800.0);
    let (w, h) = if mode == 0 { (r.range(1.0, 80.0) as f32, r.range(4.0, 14.0) as f32) } else { (hostile_f32(r, -50.0, 100.0), hostile_f32(r, -20.0, 30.0)) };
    let size = if mode == 0 { r.range(3.0, 20.0) as f32 } else { hostile_f32(r, -5.0, 40.0) };
    Frag {
        object: id,
        left: x,
        top: y,
        right: x + w,
        bottom: y + h,
        baseline: if mode == 0 { y + h * 0.8 } else { hostile_f32(r, 0.0, 800.0) },
        size,
        font: if r.chance(0.2) { u32::MAX } else { r.int(6) as u32 },
        stem: if r.chance(0.3) { None } else { Some(r.int(260) as u16) },
        face: ["", "Arial", "ABCDEF+Arial", "x+y", "ABCDEF+"][r.int(5)].to_string(),
        rgb: [r.int(256) as u8, r.int(256) as u8, r.int(256) as u8],
        text: TEXTS[r.int(TEXTS.len())].to_string(),
        rotated: r.chance(0.05),
    }
}

pub fn hostile_shape(r: &mut Rng, id: usize) -> Shape {
    let x = hostile_f32(r, 0.0, 600.0);
    let y = hostile_f32(r, 0.0, 800.0);
    Shape { object: id, left: x, top: y, right: x + hostile_f32(r, 0.0, 300.0), bottom: y + hostile_f32(r, 0.0, 30.0), depth: if r.chance(0.8) { 0 } else { r.int(4) as u32 } }
}

pub fn hostile_run(r: &mut Rng, id: usize, mode: u32) -> TextRun {
    let x = hostile_f32(r, 0.0, 600.0);
    let y = hostile_f32(r, 0.0, 800.0);
    let (w, h) = if mode == 0 { (r.range(1.0, 80.0) as f32, r.range(4.0, 14.0) as f32) } else { (hostile_f32(r, -50.0, 100.0), hostile_f32(r, -20.0, 30.0)) };
    TextRun {
        object: id,
        text: TEXTS[r.int(TEXTS.len())].to_string(),
        rect: Rect { left: x, top: y, right: x + w, bottom: y + h },
        origin: Point { x: if mode == 0 { x } else { hostile_f32(r, 0.0, 600.0) }, y: if mode == 0 { y + h * 0.8 } else { hostile_f32(r, 0.0, 800.0) } },
        size: if mode == 0 { r.range(3.0, 20.0) as f32 } else { hostile_f32(r, -5.0, 40.0) },
        color: Color { r: r.int(256) as u8, g: r.int(256) as u8, b: r.int(256) as u8, a: if r.chance(0.1) { 0 } else { 255 } },
    }
}

pub fn hostile_style(r: &mut Rng) -> RunStyle {
    RunStyle {
        font: r.int(6) as u32,
        stem_milli_em: if r.chance(0.3) { None } else { Some(r.int(260) as u16) },
        axis: if r.chance(0.9) { (1.0, 0.0) } else { (hostile_f32(r, -1.0, 1.0), hostile_f32(r, -1.0, 1.0)) },
    }
}

}
pub use common::*;
pub use hostile::*;

// ------------------------------------------------------------------------------------------------
// rng
// ------------------------------------------------------------------------------------------------

#[derive(Clone)]
pub struct Rng(pub u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        let mut r = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03);
        r.next();
        r.next();
        r
    }
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    pub fn f(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    pub fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.f()
    }
    pub fn int(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
    pub fn between(&mut self, lo: usize, hi: usize) -> usize {
        lo + self.int(hi - lo + 1)
    }
    pub fn chance(&mut self, p: f64) -> bool {
        self.f() < p
    }
    pub fn shuffle<T>(&mut self, v: &mut [T]) {
        for i in (1..v.len()).rev() {
            let j = self.int(i + 1);
            v.swap(i, j);
        }
    }
}

// ------------------------------------------------------------------------------------------------
// fonts and glyph widths
// ------------------------------------------------------------------------------------------------

pub const FONTS: [(u32, Option<u16>, &str); 6] = [
    (0, Some(51), "ABCDEF+Montserrat-Light"),
    (1, Some(74), "GHIJKL+Montserrat-Regular"),
    (2, Some(100), "MNOPQR+Montserrat-Medium"),
    (3, Some(124), "STUVWX+Montserrat-SemiBold"),
    (4, Some(198), "YZABCD+Montserrat-ExtraBold"),
    (5, None, "Arial"),
];

pub fn adv(c: char) -> f64 {
    match c {
        ' ' => 0.26,
        'i' | 'l' | 'j' | '.' | ',' | ';' | ':' | '\'' | '!' | '|' | '\u{2}' => 0.26,
        't' | 'f' | 'r' | 'I' | '-' | '(' | ')' => 0.34,
        'm' | 'w' | 'M' | 'W' => 0.84,
        'A'..='Z' => 0.66,
        '0'..='9' => 0.56,
        _ => 0.54,
    }
}

pub fn text_w(t: &str, size: f64) -> f64 {
    t.chars().map(adv).sum::<f64>() * size
}

const SYL: [&str; 40] = [
    "lu", "min", "ous", "flux", "bea", "m", "an", "gle", "ther", "mal", "ef", "fi", "ca", "cy", "dri", "ver", "light", "co", "lour", "tem", "per", "a", "ture", "out", "put", "in", "put", "pow", "er", "con", "trol", "lens", "op", "ti", "cal", "re", "flec", "tor", "ho", "us",
];

pub fn word(r: &mut Rng) -> String {
    let n = r.between(1, 3);
    let mut w = String::new();
    for _ in 0..n {
        w.push_str(SYL[r.int(SYL.len())]);
    }
    if w.chars().count() > 11 {
        w = w.chars().take(11).collect();
    }
    if r.chance(0.04) {
        w = w.to_uppercase();
    } else if r.chance(0.05) {
        let mut c = w.chars();
        let first = c.next().unwrap().to_uppercase().to_string();
        w = first + c.as_str();
    }
    if r.chance(0.04) {
        w.push_str(&r.between(1, 99).to_string());
    }
    if r.chance(0.10) {
        w.push(if r.chance(0.5) { ',' } else { '.' });
    }
    w
}

// ------------------------------------------------------------------------------------------------
// page model
// ------------------------------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Truth {
    pub kind: &'static str,
    pub objs: Vec<usize>,
    pub just: bool,
    pub nlines: usize,
}

#[derive(Clone, Default)]
pub struct GenPage {
    pub runs: Vec<TextRun>,
    pub styles: HashMap<usize, RunStyle>,
    pub faces: HashMap<usize, String>,
    pub shapes: Vec<DrawnObject>,
    pub truth: Vec<Truth>,
    pub next: usize,
    pub no_split: bool,
}

#[derive(Clone, Copy)]
pub struct Look {
    pub font: u32,
    pub size: f64,
    pub rgb: [u8; 3],
}

impl GenPage {
    pub fn new() -> GenPage {
        GenPage::default()
    }

    pub fn id(&mut self) -> usize {
        let i = self.next;
        self.next += 1;
        i
    }

    /// One text object whose ink starts at `x` on `baseline`.
    pub fn text(&mut self, text: &str, x: f64, baseline: f64, look: Look) -> usize {
        let id = self.id();
        let size = look.size;
        let w = text_w(text.trim_end(), size);
        let lead = if text.starts_with(' ') { 0.0 } else { 0.08 * size };
        let rect = Rect { left: x as f32, top: (baseline - 0.80 * size) as f32, right: (x + w) as f32, bottom: (baseline + 0.22 * size) as f32 };
        self.runs.push(TextRun {
            object: id,
            text: text.to_string(),
            rect,
            origin: Point { x: (x - lead) as f32, y: baseline as f32 },
            size: size as f32,
            color: Color { r: look.rgb[0], g: look.rgb[1], b: look.rgb[2], a: 255 },
        });
        let (font, stem, face) = FONTS[look.font as usize % FONTS.len()];
        self.styles.insert(id, RunStyle { font, stem_milli_em: stem, axis: (1.0, 0.0) });
        self.faces.insert(id, face.to_string());
        id
    }

    pub fn shape(&mut self, l: f64, t: f64, r: f64, b: f64) -> usize {
        let id = self.id();
        self.shapes.push(DrawnObject { object: id, kind: DrawnKind::Shape, rect: Rect { left: l as f32, top: t as f32, right: r as f32, bottom: b as f32 }, label: String::new(), depth: 0, opacity: 1.0, movable: true });
        id
    }

    pub fn last_run_mut(&mut self) -> &mut TextRun {
        self.runs.last_mut().unwrap()
    }
}

/// A line of `words` laid out from `x`: returns the object ids. Words are cut into objects at random word
/// boundaries (3..8 per line as on the datasheet) and, rarely, inside a word (a kerning gap of under 1 pt).
/// `slack` is spread over the word gaps (justified).
pub fn layout_line(pg: &mut GenPage, r: &mut Rng, words: &[String], x: f64, baseline: f64, look: Look, extra_gap: f64, p_cut: f64) -> Vec<usize> {
    layout_line_to(pg, r, words, x, baseline, look, extra_gap, p_cut, None)
}

/// Like [layout_line]; with `target_right` the word gaps are widened so the line ends exactly there (justified,
/// kerning gaps inside split words included).
pub fn layout_line_to(pg: &mut GenPage, r: &mut Rng, words: &[String], x: f64, baseline: f64, look: Look, extra_gap: f64, p_cut: f64, target_right: Option<f64>) -> Vec<usize> {
    let size = look.size;
    let space = 0.26 * size;
    // atoms: (text, relative x with no extra gap, is_continuation_of_previous_word, word index)
    let mut atoms: Vec<(String, f64, bool, usize)> = Vec::new();
    let mut cx = 0.0;
    for (wi, w) in words.iter().enumerate() {
        if wi > 0 {
            cx += space;
        }
        let chars: Vec<char> = w.chars().collect();
        if chars.len() >= 4 && !pg.no_split && r.chance(0.06) {
            let at = r.between(2, chars.len() - 2);
            let a: String = chars[..at].iter().collect();
            let b: String = chars[at..].iter().collect();
            atoms.push((a.clone(), cx, false, wi));
            cx += text_w(&a, size) + r.range(0.2, 1.0);
            atoms.push((b.clone(), cx, true, wi));
            cx += text_w(&b, size);
        } else {
            atoms.push((w.clone(), cx, false, wi));
            cx += text_w(w, size);
        }
    }
    let natural = cx;
    let gaps = words.len().saturating_sub(1);
    let extra = match target_right {
        Some(tr) if gaps > 0 => ((tr - x - natural) / gaps as f64).max(0.0),
        _ => extra_gap,
    };
    // group atoms into objects
    let mut ids = Vec::new();
    let mut i = 0;
    while i < atoms.len() {
        let mut j = i + 1;
        while j < atoms.len() && !(r.chance(p_cut) || (atoms[j].2 && r.chance(0.6))) {
            j += 1;
        }
        let mut text = String::new();
        for k in i..j {
            if k > i && !atoms[k].2 {
                text.push(' ');
            }
            text.push_str(&atoms[k].0);
        }
        if j < atoms.len() && !atoms[j].2 {
            text.push(' '); // PDFium's synthetic trailing space before a gap
        }
        let ax = x + atoms[i].1 + extra * atoms[i].3 as f64;
        // inside an object the extra gaps are part of the object's own width: widen its rect to the last atom
        let last = &atoms[j - 1];
        let right = x + last.1 + extra * last.3 as f64 + text_w(&last.0, size);
        let id = pg.text(&text, ax, baseline, look);
        pg.runs.last_mut().unwrap().rect.right = right as f32;
        ids.push(id);
        i = j;
    }
    ids
}

/// Greedy line breaking of `words` into lines no wider than `width`.
pub fn break_lines(words: &[String], width: f64, size: f64) -> Vec<Vec<String>> {
    let space = 0.26 * size;
    let mut lines: Vec<Vec<String>> = vec![Vec::new()];
    let mut cur = 0.0;
    for w in words {
        let ww = text_w(w, size);
        let need = if lines.last().unwrap().is_empty() { ww } else { cur + space + ww };
        if need > width && !lines.last().unwrap().is_empty() {
            lines.push(vec![w.clone()]);
            cur = ww;
        } else {
            lines.last_mut().unwrap().push(w.clone());
            cur = need;
        }
    }
    lines
}

pub fn line_width(words: &[String], size: f64) -> f64 {
    words.iter().map(|w| text_w(w, size)).sum::<f64>() + 0.26 * size * (words.len().saturating_sub(1)) as f64
}

pub struct ParaSpec {
    pub x: f64,
    pub width: f64,
    pub top: f64, // top of the first line's em box
    pub look: Look,
    pub nwords: usize,
    pub justified: bool,
    pub pitch_em: f64,
    pub p_cut: f64,
    pub outlined_p: f64,
    pub indent_first: f64,
    /// Right-to-left: every line is mirrored inside the column after layout (first word rightmost, the last line short on the LEFT).
    pub rtl: bool,
}

/// Lay out one paragraph; returns (object ids, outlined shape ids, bottom y of the last line's baseline).
pub fn paragraph(pg: &mut GenPage, r: &mut Rng, sp: &ParaSpec) -> (Vec<usize>, Vec<usize>, f64) {
    let words: Vec<String> = (0..sp.nwords).map(|_| word(r)).collect();
    let size = sp.look.size;
    let lines = break_lines(&words, sp.width - sp.indent_first, size);
    let mut objs = Vec::new();
    let mut outl = Vec::new();
    let mut base = sp.top + 0.8 * size;
    for (li, line) in lines.iter().enumerate() {
        let last = li + 1 == lines.len();
        let lw = line_width(line, size);
        let avail = sp.width - if li == 0 { sp.indent_first } else { 0.0 };
        let _ = (lw, avail);
        let extra = 0.0;
        let target = if sp.justified && !last && line.len() > 1 { Some(sp.x + sp.width) } else { None };
        let x0 = sp.x + if li == 0 { sp.indent_first } else { 0.0 };
        // outlined words: a ligature word drawn as a path instead of text
        let mut keep: Vec<String> = Vec::new();
        let mut pending_outline: Vec<(usize, String)> = Vec::new();
        for (wi, w) in line.iter().enumerate() {
            if sp.outlined_p > 0.0 && line.len() > 3 && wi > 0 && wi + 1 < line.len() && w.len() > 3 && r.chance(sp.outlined_p) {
                pending_outline.push((wi, w.clone()));
            }
            keep.push(w.clone());
        }
        if pending_outline.is_empty() {
            objs.extend(layout_line_to(pg, r, &keep, x0, base, sp.look, extra, sp.p_cut, target));
        } else {
            // lay the whole line out as text to know the x of each word, then turn the chosen words into paths
            // (text objects covering them are removed from the line by laying out the segments around them)
            let space = 0.26 * size;
            let mut xs = Vec::new();
            let mut cx = x0;
            let extra = match target {
                Some(tr) => ((tr - x0 - line_width(line, size)) / (line.len() - 1) as f64).max(0.0),
                None => 0.0,
            };
            let was = pg.no_split;
            pg.no_split = true;
            for (wi, w) in line.iter().enumerate() {
                if wi > 0 {
                    cx += space + extra;
                }
                xs.push(cx);
                cx += text_w(w, size);
            }
            let mut seg_start = 0;
            let mut cuts: Vec<usize> = pending_outline.iter().map(|p| p.0).collect();
            cuts.push(line.len());
            for c in cuts {
                if c > seg_start {
                    let seg: Vec<String> = line[seg_start..c].to_vec();
                    objs.extend(layout_line(pg, r, &seg, xs[seg_start], base, sp.look, extra, sp.p_cut));
                }
                if c < line.len() {
                    let wd = text_w(&line[c], size);
                    let id = pg.shape(xs[c], base - 0.8 * size, xs[c] + wd, base + 0.22 * size);
                    outl.push(id);
                    seg_start = c + 1;
                }
            }
            pg.no_split = was;
        }
        if !last {
            base += sp.pitch_em * size;
        }
    }
    (objs, outl, base)
}

// ------------------------------------------------------------------------------------------------
// whole realistic pages
// ------------------------------------------------------------------------------------------------

pub struct PageOpts {
    pub columns: usize,
    pub body: f64,
    pub weird: bool,   // twins, blank layers, rotated labels, zero-size, alpha 0, scripts
    pub outlined: bool,
    pub tables: bool,
    pub lists: bool,
    pub height: f64,
    /// Paragraph spacing: extra baseline pitch (em) of the first line of a paragraph over the line pitch. None = random 0.37..0.97.
    pub gap_extra: Option<f64>,
    /// Mirror every paragraph inside its column (right-to-left script).
    pub rtl: bool,
    /// Force every paragraph justified (Some(true)) or ragged (Some(false)); None = per column at random.
    pub force_justify: Option<bool>,
}

impl Default for PageOpts {
    fn default() -> PageOpts {
        PageOpts { columns: 2, body: 9.0, weird: true, outlined: true, tables: true, lists: true, height: 792.0, gap_extra: None, rtl: false, force_justify: None }
    }
}

pub fn body_look(font: u32, size: f64) -> Look {
    Look { font, size, rgb: [40, 40, 40] }
}

pub fn gen_page(seed: u64, o: &PageOpts) -> GenPage {
    let mut r = Rng::new(seed);
    let mut pg = GenPage::new();
    let (page_w, margin) = (612.0, 54.0);
    let gutter = r.range(14.0, 26.0).max(1.3 * o.body);
    let mut cols = o.columns.max(1);
    while cols > 1 && (page_w - 2.0 * margin - gutter * (cols - 1) as f64) / (cols as f64) < 16.0 * o.body {
        cols -= 1;
    }
    let colw = (page_w - 2.0 * margin - gutter * (cols - 1) as f64) / cols as f64;
    let body_font = if r.chance(0.5) { 0 } else { 1 };
    for c in 0..cols {
        let x = margin + c as f64 * (colw + gutter);
        let mut y = margin;
        let justified = match o.force_justify { Some(j) => j, None => r.chance(0.5) };
        while y < o.height - margin - 60.0 {
            let pick = r.f();
            if pick < 0.14 {
                // heading: bigger and bolder, one or two lines
                let size = o.body * r.range(1.3, 2.0);
                let look = Look { font: 4, size, rgb: if r.chance(0.3) { [200, 30, 30] } else { [20, 20, 20] } };
                let n = r.between(1, 5);
                let words: Vec<String> = (0..n).map(|_| word(&mut r)).collect();
                let base = y + 0.8 * size;
                let ids = layout_line(&mut pg, &mut r, &words, x, base, look, 0.0, 0.3);
                pg.truth.push(Truth { kind: "heading", objs: ids, just: false, nlines: 1 });
                y = base + 0.22 * size + o.body * r.range(0.8, 1.6);
            } else if pick < 0.24 && o.lists {
                let n = r.between(2, 5);
                let look = body_look(body_font, o.body);
                let mut ids_all = Vec::new();
                for _ in 0..n {
                    let bullet = pg.text("\u{2022}", x, y + 0.8 * o.body, look);
                    let nwords = r.between(4, 22);
                    let (mut ids, _, bottom) = paragraph(&mut pg, &mut r, &ParaSpec { x: x + 1.6 * o.body, width: colw - 1.6 * o.body, top: y, look, nwords, justified: false, pitch_em: 1.25, p_cut: 0.35, outlined_p: 0.0, indent_first: 0.0, rtl: o.rtl });
                    ids.insert(0, bullet);
                    pg.truth.push(Truth { kind: "item", objs: ids.clone(), just: false, nlines: 1 });
                    ids_all.extend(ids);
                    y = bottom + 0.22 * o.body + 0.5 * o.body;
                }
                y += 0.5 * o.body;
            } else if pick < 0.30 && o.tables {
                // a table without rules: rows x cols, one object per cell
                let nr = r.between(2, 6);
                let nc = r.between(2, 4).min(((colw / (7.0 * o.body)) as usize).max(2));
                let look = body_look(1, o.body * 0.93);
                let cw = colw / nc as f64;
                for _ in 0..nr {
                    let base = y + 0.8 * look.size;
                    let mut row_ids = Vec::new();
                    for k in 0..nc {
                        let t = if r.chance(0.5) { word(&mut r) } else { format!("{}.{}", r.between(1, 999), r.between(10, 99)) };
                        row_ids.push(pg.text(&t, x + k as f64 * cw, base, look));
                    }
                    pg.truth.push(Truth { kind: "row", objs: row_ids, just: false, nlines: 1 });
                    y = base + 0.22 * look.size + 0.45 * look.size;
                }
                y += o.body;
            } else {
                // a paragraph
                let look = body_look(body_font, o.body);
                let nw = r.between(8, 120);
                let (ids, outl, bottom) = paragraph(
                    &mut pg,
                    &mut r,
                    &ParaSpec {
                        x,
                        width: colw,
                        top: y,
                        look,
                        nwords: nw,
                        justified,
                        pitch_em: 1.25,
                        p_cut: 0.35,
                        outlined_p: if o.outlined { 0.03 } else { 0.0 },
                        indent_first: 0.0,
                        rtl: o.rtl,
                    },
                );
                let _ = outl;
                let nl = {
                    let mut bs: Vec<i64> = ids.iter().map(|o| (pg.runs.iter().find(|r| r.object == *o).unwrap().origin.y * 10.0) as i64).collect();
                    bs.sort_unstable();
                    bs.dedup();
                    bs.len()
                };
                pg.truth.push(Truth { kind: "paragraph", objs: ids, just: justified, nlines: nl });
                y = bottom + 0.22 * o.body + o.body * match o.gap_extra { Some(e) => e + 0.23, None => r.range(0.6, 1.2) };
            }
            if o.weird && r.chance(0.04) {
                // a blank twin of the last text object (faux bold), same box and origin
                if let Some(last) = pg.runs.last().cloned() {
                    let id = pg.id();
                    let mut t = last.clone();
                    t.object = id;
                    t.text = String::new();
                    pg.runs.push(t);
                    if let Some(st) = pg.styles.get(&last.object).copied() {
                        pg.styles.insert(id, st);
                    }
                }
            }
        }
    }
    if o.weird {
        weird_objects(&mut pg, &mut r, o);
    }
    pg
}

/// Things real files contain that are not text lines: rotated labels, size-0 rotated runs, alpha-0 layers, a
/// readable duplicate, a superscript mark, an empty-text run, a run with no area.
pub fn weird_objects(pg: &mut GenPage, r: &mut Rng, o: &PageOpts) {
    let size = o.body;
    if r.chance(0.5) {
        let id = pg.id();
        pg.runs.push(TextRun { object: id, text: "54mm".into(), rect: Rect { left: 20.0, top: 300.0, right: 26.0, bottom: 330.0 }, origin: Point { x: 26.0, y: 330.0 }, size: 0.0, color: Color { r: 0, g: 0, b: 0, a: 255 } });
        pg.styles.insert(id, RunStyle { font: 1, stem_milli_em: Some(74), axis: (0.0, -1.0) });
    }
    if r.chance(0.4) {
        let id = pg.id();
        pg.runs.push(TextRun { object: id, text: "invisible ocr layer".into(), rect: Rect { left: 100.0, top: 400.0, right: 180.0, bottom: 410.0 }, origin: Point { x: 100.0, y: 408.0 }, size: size as f32, color: Color { r: 0, g: 0, b: 0, a: 0 } });
    }
    if r.chance(0.4) && !pg.runs.is_empty() {
        let pick = r.int(pg.runs.len());
        let mut t = pg.runs[pick].clone();
        let id = pg.id();
        t.object = id;
        t.origin.x += 0.4;
        pg.runs.push(t);
        if let Some(st) = pg.styles.get(&pg.runs[pick].object).copied() {
            pg.styles.insert(id, st);
        }
    }
    if r.chance(0.3) {
        let id = pg.id();
        pg.runs.push(TextRun { object: id, text: " ".into(), rect: Rect { left: 200.0, top: 500.0, right: 200.0, bottom: 508.0 }, origin: Point { x: 200.0, y: 507.0 }, size: size as f32, color: Color { r: 0, g: 0, b: 0, a: 255 } });
    }
}

// ------------------------------------------------------------------------------------------------
// transforms
// ------------------------------------------------------------------------------------------------

pub fn scale_run(t: &TextRun, s: f32, dx: f32, dy: f32) -> TextRun {
    let mut t = t.clone();
    t.rect = Rect { left: t.rect.left * s + dx, top: t.rect.top * s + dy, right: t.rect.right * s + dx, bottom: t.rect.bottom * s + dy };
    t.origin = Point { x: t.origin.x * s + dx, y: t.origin.y * s + dy };
    t.size *= s;
    t
}

pub fn scale_drawn(d: &DrawnObject, s: f32, dx: f32, dy: f32) -> DrawnObject {
    let mut d = d.clone();
    d.rect = Rect { left: d.rect.left * s + dx, top: d.rect.top * s + dy, right: d.rect.right * s + dx, bottom: d.rect.bottom * s + dy };
    d
}

pub fn transformed(pg: &GenPage, s: f32, dx: f32, dy: f32) -> GenPage {
    let mut o = pg.clone();
    o.runs = pg.runs.iter().map(|t| scale_run(t, s, dx, dy)).collect();
    o.shapes = pg.shapes.iter().map(|d| scale_drawn(d, s, dx, dy)).collect();
    o
}

pub fn scale_frag(f: &Frag, s: f32, dx: f32, dy: f32) -> Frag {
    let mut f = f.clone();
    f.left = f.left * s + dx;
    f.right = f.right * s + dx;
    f.top = f.top * s + dy;
    f.bottom = f.bottom * s + dy;
    f.baseline = f.baseline * s + dy;
    f.size *= s;
    f
}

pub fn scale_shape(h: &Shape, s: f32, dx: f32, dy: f32) -> Shape {
    let mut h = *h;
    h.left = h.left * s + dx;
    h.right = h.right * s + dx;
    h.top = h.top * s + dy;
    h.bottom = h.bottom * s + dy;
    h
}

pub fn build(pg: &GenPage) -> PageBlocks {
    build_page_blocks_from(0, 0, 0, pg.runs.clone(), pg.styles.clone(), pg.faces.clone(), pg.shapes.clone())
}

// ------------------------------------------------------------------------------------------------
// canonical forms
// ------------------------------------------------------------------------------------------------

/// The partition of text objects into blocks and lines, as a comparable value (blocks by their set of
/// (line index, objects), ignoring `starts_because` and float rounding of rects).
pub fn partition(blocks: &[Block]) -> BTreeSet<Vec<Vec<usize>>> {
    blocks.iter().map(|b| b.lines.iter().map(|l| l.objects.clone()).collect::<Vec<_>>()).collect()
}

/// The coarser partition: block -> sorted set of objects (line structure ignored).
pub fn block_sets(blocks: &[Block]) -> BTreeSet<Vec<usize>> {
    blocks
        .iter()
        .map(|b| {
            let mut v: Vec<usize> = b.lines.iter().flat_map(|l| l.objects.iter().chain(l.outlined.iter()).copied()).collect();
            v.sort_unstable();
            v
        })
        .collect()
}

// ------------------------------------------------------------------------------------------------
// an independent statement of the editor contract (from the scout's C1..C13 text, not from the guard's code)
// ------------------------------------------------------------------------------------------------

fn dist(a: (f32, f32), b: (f32, f32)) -> f32 {
    (((a.0 - b.0) as f64).powi(2) + ((a.1 - b.1) as f64).powi(2)).sqrt() as f32
}

/// Everything apply needs to hold for block `bi` of `pb`, as a list of violated invariants (empty = fine).
/// Pairwise and quadratic on purpose: no sweep, no shared code with the guard.
pub fn independent_violations(pb: &PageBlocks, bi: usize) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    let Some(b) = pb.blocks.get(bi) else { return vec!["C1 no such block".into()] };
    if b.lines.is_empty() {
        v.push("C1 no lines".into());
    }
    let specs = editor_lines(pb, bi);
    let texts = line_texts(pb, &specs);
    if specs.len() != b.lines.len() {
        v.push("C13 line dropped".into());
    }
    // members, in order
    let mut members: Vec<(usize, usize)> = Vec::new(); // (line, object)
    for (li, l) in b.lines.iter().enumerate() {
        if l.objects.is_empty() && l.outlined.is_empty() {
            v.push(format!("C1 line {li} empty"));
        }
        for &o in &l.objects {
            members.push((li, o));
        }
    }
    // C3 across the whole page: every object once
    let mut seen: HashMap<usize, usize> = HashMap::new();
    for (bj, bb) in pb.blocks.iter().enumerate() {
        for l in &bb.lines {
            for &o in &l.objects {
                if let Some(first) = seen.insert(o, bj) {
                    if bj == bi || first == bi {
                        v.push(format!("C3 object {o} in two blocks or twice"));
                    }
                }
            }
        }
    }
    for &(li, o) in &members {
        let Some(run) = pb.runs.get(&o) else {
            v.push(format!("C2 object {o} not a run"));
            continue;
        };
        let (w, h) = ((run.rect.right - run.rect.left).abs(), (run.rect.bottom - run.rect.top).abs());
        if !(w > 0.5 && h > 0.5) || !run.rect.left.is_finite() || !run.rect.top.is_finite() || !run.rect.right.is_finite() || !run.rect.bottom.is_finite() {
            v.push(format!("C2 object {o} no area"));
        }
        if run.text.trim().is_empty() {
            v.push(format!("C5 object {o} blank"));
        }
        if run.color.a == 0 {
            v.push(format!("C6 object {o} alpha 0"));
        }
        if !(run.size.is_finite() && run.size > 0.0 && run.origin.x.is_finite() && run.origin.y.is_finite()) {
            v.push(format!("C6 object {o} size/origin"));
        }
        if let Some(st) = pb.styles.get(&o) {
            if (st.axis.0 - 1.0).abs() > 0.02 || st.axis.1.abs() > 0.02 {
                v.push(format!("C6 object {o} axis"));
            }
        }
        if run.text.trim().chars().count() >= 4 && (run.rect.bottom - run.rect.top).abs() > (run.rect.right - run.rect.left).abs() * 1.2 {
            v.push(format!("C6 object {o} looks rotated"));
        }
        if pb.excluded.reason(o).is_some() {
            v.push(format!("C4 object {o} is excluded ({:?})", pb.excluded.reason(o)));
        }
        let _ = li;
    }
    // C4 pairwise
    for i in 0..members.len() {
        for j in i + 1..members.len() {
            let (a, c) = (members[i].1, members[j].1);
            if let (Some(ra), Some(rc)) = (pb.runs.get(&a), pb.runs.get(&c)) {
                if dist((ra.origin.x, ra.origin.y), (rc.origin.x, rc.origin.y)) <= 1.0 {
                    v.push(format!("C4 objects {a} and {c} share an origin"));
                }
            }
        }
    }
    // per line: C7, C8 baseline spread + gaps
    let mut baselines: Vec<Option<f32>> = Vec::new();
    for (li, l) in b.lines.iter().enumerate() {
        let runs: Vec<&TextRun> = l.objects.iter().filter_map(|o| pb.runs.get(o)).collect();
        for w in runs.windows(2) {
            if w[1].rect.left.min(w[1].rect.right) < w[0].rect.left.min(w[0].rect.right) {
                v.push(format!("C7 line {li} not left to right"));
            }
        }
        if let Some(first) = runs.first() {
            let em = runs.iter().map(|r| r.size).fold(0.0f32, f32::max);
            for r in &runs {
                if (r.origin.y - first.origin.y).abs() > 0.75 * em {
                    v.push(format!("C8 line {li} baselines differ"));
                }
            }
            if l.outlined.is_empty() {
                let mut reach = f32::MIN;
                for w in runs.windows(2) {
                    reach = reach.max(w[0].rect.left.max(w[0].rect.right));
                    let gap = w[1].rect.left.min(w[1].rect.right) - reach;
                    if gap > 6.0 * w[0].size.max(w[1].size) {
                        v.push(format!("C8 line {li} has a {gap:.1} pt gap"));
                    }
                }
            }
            baselines.push(Some(first.origin.y));
        } else {
            baselines.push(None);
        }
    }
    for li in 1..b.lines.len() {
        if let (Some(a), Some(c)) = (baselines[li - 1], baselines[li]) {
            let em = [li - 1, li]
                .iter()
                .map(|&k| b.lines[k].objects.iter().filter_map(|o| pb.runs.get(o)).map(|r| r.size).fold(0.0f32, f32::max))
                .fold(0.0f32, f32::max);
            if c - a <= 0.4 * em {
                v.push(format!("C8 line {li} not below line {}", li - 1));
            }
        }
    }
    // C9 text
    for (li, l) in b.lines.iter().enumerate() {
        let want: String = if l.objects.is_empty() { "[drawn text]".to_string() } else { l.objects.iter().filter_map(|o| pb.runs.get(o)).map(|r| r.text.replace(['\n', '\r'], " ")).collect() };
        if texts.get(li) != Some(&want) {
            v.push(format!("C9 line {li} text differs"));
        }
        if texts.get(li).map_or(false, |t| t.contains('\n') || t.contains('\r')) {
            v.push(format!("C8 line {li} has a break"));
        }
        // C10
        if !l.outlined.is_empty() && !specs.get(li).map_or(false, |s| s.frozen) {
            v.push(format!("C10 line {li} not frozen"));
        }
        // C11
        if let Some(s) = specs.get(li) {
            let r = s.rect;
            if !(r.left.is_finite() && r.top.is_finite() && r.right.is_finite() && r.bottom.is_finite()) || r.left > r.right || r.top > r.bottom {
                v.push(format!("C11 line {li} rect"));
            }
        }
    }
    let joined = texts.join("\n");
    if joined.split('\n').count() != b.lines.len() {
        v.push("C8 buffer lines != lines".into());
    }
    v
}

pub fn objects_of(blocks: &[Block]) -> Vec<usize> {
    let mut v: Vec<usize> = blocks.iter().flat_map(|b| b.objects()).collect();
    v.sort_unstable();
    v
}

pub fn ids_set(v: &[usize]) -> HashSet<usize> {
    v.iter().copied().collect()
}

}

mod fuzz {
    use super::v4gen::*;

use pagify_shell::block_input::*;
use pagify_shell::blocks::{self, Block, Frag, Shape};
use pdf_core::document::{DrawnKind, DrawnObject, TextRun};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::time::Instant;

// ------------------------------------------------------------------------------------------------
// 0. the generator itself: is it realistic enough to judge anything? (does the detector agree with its truth)
// ------------------------------------------------------------------------------------------------

#[test]
#[ignore = "V4 review: run with --ignored (see the file header)"]
fn v4_generator_sanity_the_detector_agrees_with_the_generators_own_truth_on_most_blocks() {
    let n = seeds_env("V4_SEEDS", 200);
    let mut total_truth = 0usize;
    let mut exact = 0usize;
    let mut para_total = 0usize;
    let mut para_exact = 0usize;
    let mut by_kind: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    let mut nobj = 0usize;
    for s in 0..n {
        let pg = gen_page(s, &opts_for(s));
        let pb = build(&pg);
        nobj += pg.runs.len();
        let sets = block_sets(&pb.blocks);
        for t in &pg.truth {
            let mut want: Vec<usize> = t.objs.clone();
            want.sort_unstable();
            // the generator's truth lists text objects only; outlined words are bridged in by the detector
            let got_exact = pb.blocks.iter().any(|b| {
                let mut o = b.objects();
                o.sort_unstable();
                o == want
            });
            total_truth += 1;
            let e = by_kind.entry(t.kind).or_default();
            e.0 += 1;
            if got_exact {
                exact += 1;
                e.1 += 1;
            }
            if t.kind == "paragraph" {
                para_total += 1;
                if got_exact {
                    para_exact += 1;
                }
            }
        }
        let _ = sets;
    }
    eprintln!("[generator sanity] {n} pages, {nobj} runs, truth blocks {total_truth}, detector exact {exact} ({:.1}%), paragraphs {para_exact}/{para_total} ({:.1}%)", 100.0 * exact as f64 / total_truth as f64, 100.0 * para_exact as f64 / para_total as f64);
    for (k, (t, e)) in &by_kind {
        eprintln!("    {k:10} {e}/{t} ({:.1}%)", 100.0 * *e as f64 / *t as f64);
    }
    // the generator is only useful if the detector mostly agrees with it: that shows the pages are paragraphs
    // and headings and not noise (the fuzz below then asks the contract questions on realistic input).
    assert!(para_exact as f64 / para_total as f64 > 0.6, "generator pages are not paragraph-like enough");
}

// ------------------------------------------------------------------------------------------------
// 1. realistic pages through adapt + detect + guard
// ------------------------------------------------------------------------------------------------

#[test]
#[ignore = "V4 review: run with --ignored (see the file header)"]
fn v4_realistic_pages_contract_guard_and_independent_contract_agree() {
    let n = seeds_env("V4_SEEDS", 300);
    let refusals: std::cell::RefCell<BTreeMap<String, usize>> = Default::default();
    let blocks_total = std::cell::Cell::new(0usize);
    let multi_total = std::cell::Cell::new(0usize);
    let refused_total = std::cell::Cell::new(0usize);
    let (failed, _) = run_seeds("realistic: contract + guard + independent", 0, n, |s| {
        let pg = gen_page(s, &opts_for(s));
        let pb = build(&pg);
        // every admitted object once, every excluded object in one list, none in both
        let admitted: HashSet<usize> = pb.frags.iter().map(|f| f.object).collect();
        let in_blocks = objects_of(&pb.blocks);
        if in_blocks.len() != admitted.len() || ids_set(&in_blocks) != admitted {
            return Err(format!("blocks hold {} objects, admitted {}", in_blocks.len(), admitted.len()));
        }
        if pb.by_object.len() != admitted.len() {
            return Err("by_object size".into());
        }
        let ex = &pb.excluded;
        let mut all: Vec<usize> = Vec::new();
        for l in [&ex.blank, &ex.invisible, &ex.degenerate, &ex.shadowed, &ex.rotated] {
            all.extend(l.iter().copied());
        }
        let set: HashSet<usize> = all.iter().copied().collect();
        if set.len() != all.len() {
            return Err("an id is in two excluded lists".into());
        }
        if set.iter().any(|o| admitted.contains(o)) {
            return Err("an excluded id is also admitted".into());
        }
        if admitted.len() + set.len() != pg.runs.len() {
            return Err(format!("admitted {} + excluded {} != runs {}", admitted.len(), set.len(), pg.runs.len()));
        }
        for bi in 0..pb.blocks.len() {
            blocks_total.set(blocks_total.get() + 1);
            let b = &pb.blocks[bi];
            if b.objects().len() > 1 {
                multi_total.set(multi_total.get() + 1);
            }
            let guard = check_editor_invariants(&pb, bi);
            let ind = independent_violations(&pb, bi);
            match (&guard, ind.is_empty()) {
                (Ok(()), true) => {}
                (Ok(()), false) => return Err(format!("GUARD ACCEPTED block {bi} but the independent contract finds {:?}", ind)),
                (Err(m), _) => {
                    refused_total.set(refused_total.get() + 1);
                    let code: String = m.chars().take_while(|c| *c != ':').collect();
                    *refusals.borrow_mut().entry(code).or_default() += 1;
                    if ind.is_empty() {
                        // the guard is stricter than the independent statement: record, not an error
                        *refusals.borrow_mut().entry("(guard stricter than independent)".into()).or_default() += 1;
                    }
                }
            }
        }
        Ok(())
    });
    eprintln!("[realistic] blocks {} (multi-object {}), guard refusals {} : {:?}", blocks_total.get(), multi_total.get(), refused_total.get(), refusals.borrow());
    finish("realistic", failed);
}

// ------------------------------------------------------------------------------------------------
// 2. determinism, shuffle invariance, translation invariance
// ------------------------------------------------------------------------------------------------

fn canon_pb(pb: &PageBlocks) -> (BTreeSet<Vec<Vec<usize>>>, Vec<usize>, Vec<usize>, Vec<usize>, Vec<usize>, Vec<usize>) {
    let mut e = pb.excluded.clone();
    for l in [&mut e.blank, &mut e.invisible, &mut e.degenerate, &mut e.shadowed, &mut e.rotated] {
        l.sort_unstable();
    }
    (partition(&pb.blocks), e.blank, e.invisible, e.degenerate, e.shadowed, e.rotated)
}

#[test]
#[ignore = "V4 review: run with --ignored (see the file header)"]
fn v4_determinism_and_shuffle_invariance_of_the_whole_pipeline() {
    let n = seeds_env("V4_SEEDS", 300);
    let (failed, _) = run_seeds("determinism + shuffle", 0, n, |s| {
        let pg = gen_page(s, &opts_for(s));
        let a = build(&pg);
        let b = build(&pg);
        if a.blocks != b.blocks || canon_pb(&a) != canon_pb(&b) {
            return Err("same input twice gave different blocks".into());
        }
        // exact equality of the Block structs (including float rects and starts_because) under shuffles
        let mut r = Rng::new(s ^ 77);
        for k in 0..3 {
            let mut p2 = pg.clone();
            r.shuffle(&mut p2.runs);
            r.shuffle(&mut p2.shapes);
            let c = build(&p2);
            if a.blocks != c.blocks {
                return Err(format!("shuffle {k}: blocks differ"));
            }
            if canon_pb(&a) != canon_pb(&c) {
                return Err(format!("shuffle {k}: excluded lists differ"));
            }
            // the same click gives the same seed whatever the order the runs came in
            for _ in 0..8 {
                let (x, y) = (r.range(0.0, 612.0) as f32, r.range(0.0, 792.0) as f32);
                if pick_seed(&a, x, y, 3.0) != pick_seed(&c, x, y, 3.0) {
                    return Err(format!("pick_seed differs at ({x},{y}) after a shuffle"));
                }
            }
        }
        Ok(())
    });
    finish("determinism+shuffle", failed);
}

#[test]
#[ignore = "V4 review: run with --ignored (see the file header)"]
fn v4_translation_invariance_exact_grid_and_arbitrary() {
    let n = seeds_env("V4_SEEDS", 300);
    // (a) grid-exact: coordinates snapped to 1/16 pt, translation by a multiple of 16: every f32 sum is exact, so
    //     the detector has no excuse
    let (f1, _) = run_seeds("translation (exact grid)", 0, n, |s| {
        let pg = snap(&gen_page(s, &opts_for(s)), 16.0);
        let a = build(&pg);
        let mut r = Rng::new(s ^ 5);
        let (dx, dy) = ((r.between(0, 40) as f32 - 20.0) * 16.0, (r.between(0, 40) as f32 - 20.0) * 16.0);
        let b = build(&transformed(&pg, 1.0, dx, dy));
        if block_sets(&a.blocks) != block_sets(&b.blocks) || partition(&a.blocks) != partition(&b.blocks) {
            return Err(format!("grouping changed under an exact translation of ({dx},{dy})"));
        }
        if canon_pb(&a).1 != canon_pb(&b).1 || canon_pb(&a).4 != canon_pb(&b).4 {
            return Err(format!("excluded lists changed under an exact translation of ({dx},{dy})"));
        }
        Ok(())
    });
    finish("translation exact", f1);
    // (b) arbitrary small translation (rounding in f32 may legitimately move a borderline decision): count
    let mut changed = 0u64;
    let mut ex = Vec::new();
    for s in 0..n {
        let pg = gen_page(s, &opts_for(s));
        let a = build(&pg);
        let mut r = Rng::new(s ^ 6);
        let (dx, dy) = (r.range(-300.0, 300.0) as f32, r.range(-300.0, 300.0) as f32);
        let b = build(&transformed(&pg, 1.0, dx, dy));
        if partition(&a.blocks) != partition(&b.blocks) {
            changed += 1;
            if ex.len() < 4 {
                ex.push((s, dx, dy));
            }
        }
    }
    eprintln!("[translation (arbitrary f32 offsets)] grouping changed on {changed} of {n} pages {:?}", ex);
}

/// Snap every coordinate and size of a page to a 1/q grid.
fn snap(pg: &GenPage, q: f32) -> GenPage {
    let sn = |v: f32| (v * q).round() / q;
    let mut o = pg.clone();
    for t in &mut o.runs {
        t.rect = pdf_core::document::Rect { left: sn(t.rect.left), top: sn(t.rect.top), right: sn(t.rect.right), bottom: sn(t.rect.bottom) };
        t.origin = pdf_core::document::Point { x: sn(t.origin.x), y: sn(t.origin.y) };
        t.size = sn(t.size);
    }
    for d in &mut o.shapes {
        d.rect = pdf_core::document::Rect { left: sn(d.rect.left), top: sn(d.rect.top), right: sn(d.rect.right), bottom: sn(d.rect.bottom) };
    }
    o
}

// ------------------------------------------------------------------------------------------------
// 3. scale invariance: the detector alone, and the whole pipeline
// ------------------------------------------------------------------------------------------------

#[test]
#[ignore = "V4 review: run with --ignored (see the file header)"]
fn v4_scale_invariance_detector_alone_and_whole_pipeline() {
    let n = seeds_env("V4_SEEDS", 200);
    let scales: [f32; 9] = [0.1, 0.25, 0.4, 0.5, 2.0, 2.5, 4.0, 10.0, 8.0];
    for &sc in &scales {
        let mut det_changed = 0u64;
        let mut full_changed = 0u64;
        let mut lost_objects = 0usize;
        let mut base_objects = 0usize;
        let mut shadowed_more = 0usize;
        let mut c4_refusals = 0usize;
        let mut c4_base = 0usize;
        let mut example_full: Option<u64> = None;
        for s in 0..n {
            let mut o = opts_for(s);
            o.weird = false; // identical object sets at every scale: the weird objects have absolute sizes of their own
            let pg = gen_page(s, &o);
            let base = build(&pg);
            // (a) detector alone: scale the admitted frags and shapes of the base page
            let frags: Vec<Frag> = base.frags.iter().map(|f| scale_frag(f, sc, 0.0, 0.0)).collect();
            let shapes: Vec<Shape> = pg
                .shapes
                .iter()
                .map(|d| Shape { object: d.object, left: d.rect.left, top: d.rect.top, right: d.rect.right, bottom: d.rect.bottom, depth: 0 })
                .map(|h| scale_shape(&h, sc, 0.0, 0.0))
                .collect();
            let unscaled_shapes: Vec<Shape> = pg.shapes.iter().map(|d| Shape { object: d.object, left: d.rect.left, top: d.rect.top, right: d.rect.right, bottom: d.rect.bottom, depth: 0 }).collect();
            let b0 = blocks::detect(&base.frags, &unscaled_shapes);
            let b1 = blocks::detect(&frags, &shapes);
            if partition(&b0) != partition(&b1) {
                det_changed += 1;
            }
            // (b) whole pipeline on the scaled runs
            let scaled = build(&transformed(&pg, sc, 0.0, 0.0));
            if partition(&base.blocks) != partition(&scaled.blocks) {
                full_changed += 1;
                example_full.get_or_insert(s);
            }
            base_objects += base.frags.len();
            lost_objects += base.frags.len().saturating_sub(scaled.frags.len());
            shadowed_more += scaled.excluded.shadowed.len().saturating_sub(base.excluded.shadowed.len());
            for bi in 0..base.blocks.len() {
                if base.blocks[bi].objects().len() > 1 && check_editor_invariants(&base, bi).is_err() {
                    c4_base += 1;
                }
            }
            for bi in 0..scaled.blocks.len() {
                if scaled.blocks[bi].objects().len() > 1 && check_editor_invariants(&scaled, bi).is_err() {
                    c4_refusals += 1;
                }
            }
        }
        eprintln!(
            "[scale x{sc}] {n} pages: detector grouping changed {det_changed}; whole pipeline changed {full_changed} (first {:?}); objects dropped by the adapter {lost_objects}/{base_objects}; extra shadowed {shadowed_more}; multi-object blocks refused by the guard: {c4_refusals} (x1: {c4_base})",
            example_full
        );
    }
}

// ------------------------------------------------------------------------------------------------
// diagnostics (not assertions): what do the generator's paragraph misses look like?
// ------------------------------------------------------------------------------------------------

#[test]
#[ignore = "V4 review: run with --ignored (see the file header)"]
fn v4_diag_paragraph_misses() {
    let n = seeds_env("V4_SEEDS", 400);
    let mut kinds: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut shown = 0;
    for s in 0..n {
        let o = opts_for(s);
        let pg = gen_page(s, &o);
        let pb = build(&pg);
        for t in pg.truth.iter().filter(|t| t.kind == "paragraph") {
            let mut want = t.objs.clone();
            want.sort_unstable();
            let hit = pb.blocks.iter().any(|b| {
                let mut x = b.objects();
                x.sort_unstable();
                x == want
            });
            let key = format!("{} lines>={}", if t.just { "justified" } else { "ragged   " }, if t.nlines >= 4 { 4 } else { t.nlines });
            let e = kinds.entry(key).or_default();
            e.0 += 1;
            if hit {
                e.1 += 1;
                continue;
            }
            let mut owners: Vec<usize> = t.objs.iter().filter_map(|o| pb.by_object.get(o).map(|x| x.0)).collect();
            owners.sort_unstable();
            owners.dedup();
            let why: Vec<&str> = owners.iter().map(|&b| pb.blocks[b].starts_because).collect();
            let sizes: Vec<usize> = owners.iter().map(|&b| pb.blocks[b].objects().len()).collect();
            if shown < 14 {
                shown += 1;
                let first = pg.runs.iter().find(|r| r.object == t.objs[0]).unwrap();
                eprintln!("seed {s} body {} cols {} {} {}-line para of {} objs (first '{}'): owners {:?} starts {:?} sizes {:?}", o.body, o.columns, if t.just {"JUST"} else {"RAG"}, t.nlines, t.objs.len(), first.text.trim(), owners, why, sizes);
            }
        }
    }
    for (k, (t, e)) in &kinds {
        eprintln!("    {k}: exact {e}/{t} ({:.1}%)", 100.0 * *e as f64 / *t as f64);
    }
}

#[test]
#[ignore = "V4 review: run with --ignored (see the file header)"]
fn v4_diag_one_seed() {
    let s: u64 = std::env::var("V4_SEED").ok().and_then(|v| v.parse().ok()).unwrap_or(26);
    let o = opts_for(s);
    let pg = gen_page(s, &o);
    let pb = build(&pg);
    eprintln!("seed {s}: body {} cols {} runs {} blocks {}", o.body, o.columns, pg.runs.len(), pb.blocks.len());
    let by_id: HashMap<usize, &TextRun> = pg.runs.iter().map(|r| (r.object, r)).collect();
    for (bi, b) in pb.blocks.iter().enumerate() {
        let truths: Vec<String> = pg
            .truth
            .iter()
            .filter(|t| t.objs.iter().any(|o| b.contains(*o)))
            .map(|t| format!("{}[{}]", t.kind, t.objs.len()))
            .collect();
        let first = b.lines[0].objects.first().and_then(|o| by_id.get(o)).map(|r| r.text.trim().to_string()).unwrap_or_default();
        eprintln!("  block {bi:3} {:8} lines {:3} objs {:3} rect [{:.0},{:.0},{:.0},{:.0}] first '{}' truth {:?}", b.starts_because, b.lines.len(), b.objects().len(), b.left, b.top, b.right, b.bottom, first, truths);
    }
}

#[test]
#[ignore = "V4 review: run with --ignored (see the file header)"]
fn v4_diag_block_lines() {
    let s: u64 = std::env::var("V4_SEED").ok().and_then(|v| v.parse().ok()).unwrap_or(26);
    let bi: usize = std::env::var("V4_BLOCK").ok().and_then(|v| v.parse().ok()).unwrap_or(20);
    let o = opts_for(s);
    let pg = gen_page(s, &o);
    let pb = build(&pg);
    let by_id: HashMap<usize, &TextRun> = pg.runs.iter().map(|r| (r.object, r)).collect();
    let b = &pb.blocks[bi];
    let mut prev = None;
    for (li, l) in b.lines.iter().enumerate() {
        let first = by_id[&l.objects[0]];
        let lastr = by_id[l.objects.last().unwrap()];
        let truth = pg.truth.iter().find(|t| t.objs.contains(&l.objects[0])).map(|t| (t.kind, t.objs[0])).unwrap();
        eprintln!("  line {li}: baseline {:.2} pitch {} em, left {:.1} right {:.1} objs {} truth {:?} text '{}'...", l.baseline, prev.map_or("-".to_string(), |p: f32| format!("{:.2}", (l.baseline - p) / first.size)), l.left, l.right, l.objects.len(), truth, first.text.trim());
        let _ = lastr;
        prev = Some(l.baseline);
    }
}

#[test]
#[ignore = "V4 review: run with --ignored (see the file header)"]
fn v4_diag_line_objects() {
    let s: u64 = std::env::var("V4_SEED").ok().and_then(|v| v.parse().ok()).unwrap_or(26);
    let bi: usize = std::env::var("V4_BLOCK").ok().and_then(|v| v.parse().ok()).unwrap_or(20);
    let o = opts_for(s);
    let pg = gen_page(s, &o);
    let pb = build(&pg);
    let by_id: HashMap<usize, &TextRun> = pg.runs.iter().map(|r| (r.object, r)).collect();
    let b = &pb.blocks[bi];
    for (li, l) in b.lines.iter().enumerate() {
        let ys: Vec<String> = l.objects.iter().map(|o| format!("{}:{:.2}", o, by_id[o].origin.y)).collect();
        eprintln!("  line {li} (baseline {:.2}): {}", l.baseline, ys.join(" "));
    }
}

/// Does the presence of ANOTHER column change how a paragraph is grouped? Detect the page as is, and again with
/// only the objects of one paragraph's own column (x range) and compare the verdict on that paragraph.
#[test]
#[ignore = "V4 review: run with --ignored (see the file header)"]
fn v4_cross_column_interference_rows_are_page_wide() {
    let n = seeds_env("V4_SEEDS", 400);
    let (mut total, mut hit_together, mut hit_alone, mut only_alone) = (0, 0, 0, 0);
    let mut examples = Vec::new();
    for s in 0..n {
        let mut o = opts_for(s);
        o.columns = 2;
        o.weird = false;
        o.outlined = false;
        let pg = gen_page(s, &o);
        let pb = build(&pg);
        let by_id: HashMap<usize, &TextRun> = pg.runs.iter().map(|r| (r.object, r)).collect();
        for t in pg.truth.iter().filter(|t| t.kind == "paragraph") {
            let mut want = t.objs.clone();
            want.sort_unstable();
            let hit1 = pb.blocks.iter().any(|b| {
                let mut x = b.objects();
                x.sort_unstable();
                x == want
            });
            // alone: frags whose left edge lies in the paragraph's column
            let (xl, xr) = t.objs.iter().fold((f32::MAX, f32::MIN), |a, o| (a.0.min(by_id[o].rect.left), a.1.max(by_id[o].rect.right)));
            let frags: Vec<Frag> = pb.frags.iter().filter(|f| f.left >= xl - 1.0 && f.right <= xr + 1.0).cloned().collect();
            let shapes: Vec<Shape> = Vec::new();
            let b2 = blocks::detect(&frags, &shapes);
            let hit2 = b2.iter().any(|b| {
                let mut x = b.objects();
                x.sort_unstable();
                x == want
            });
            total += 1;
            hit_together += hit1 as usize;
            hit_alone += hit2 as usize;
            if hit2 && !hit1 {
                only_alone += 1;
                if examples.len() < 5 {
                    examples.push((s, t.objs[0], t.objs.len()));
                }
            }
        }
    }
    eprintln!("[cross-column] paragraphs {total}: exact with both columns present {hit_together} ({:.1}%), exact with the column alone {hit_alone} ({:.1}%); right only when alone: {only_alone} {:?}", 100.0 * hit_together as f64 / total as f64, 100.0 * hit_alone as f64 / total as f64, examples);
}

#[test]
#[ignore = "V4 review: run with --ignored (see the file header)"]
fn v4_diag_scale_flip() {
    let s: u64 = std::env::var("V4_SEED").ok().and_then(|v| v.parse().ok()).unwrap_or(43);
    let sc: f32 = std::env::var("V4_SCALE").ok().and_then(|v| v.parse().ok()).unwrap_or(2.5);
    let mut o = opts_for(s);
    o.weird = false;
    let pg = gen_page(s, &o);
    let base = build(&pg);
    let scaled = build(&transformed(&pg, sc, 0.0, 0.0));
    let a = block_sets(&base.blocks);
    let b = block_sets(&scaled.blocks);
    let by_id: HashMap<usize, &TextRun> = pg.runs.iter().map(|r| (r.object, r)).collect();
    eprintln!("seed {s} scale {sc}: only at x1:");
    for x in a.difference(&b) {
        let blk = base.blocks.iter().find(|bb| { let mut v: Vec<usize> = bb.lines.iter().flat_map(|l| l.objects.iter().chain(l.outlined.iter()).copied()).collect(); v.sort_unstable(); &v == x }).unwrap();
        eprintln!("   {} objs, {} lines, starts {}, first '{}' rect [{:.1},{:.1},{:.1},{:.1}]", x.len(), blk.lines.len(), blk.starts_because, by_id[&x[0]].text.trim(), blk.left, blk.top, blk.right, blk.bottom);
        for l in &blk.lines { eprintln!("        base {:.3} left {:.3} right {:.3} n {}", l.baseline, l.left, l.right, l.objects.len()); }
    }
    eprintln!("only at scaled:");
    for x in b.difference(&a) {
        let blk = scaled.blocks.iter().find(|bb| { let mut v: Vec<usize> = bb.lines.iter().flat_map(|l| l.objects.iter().chain(l.outlined.iter()).copied()).collect(); v.sort_unstable(); &v == x }).unwrap();
        eprintln!("   {} objs, {} lines, starts {}, first '{}' rect [{:.1},{:.1},{:.1},{:.1}]", x.len(), blk.lines.len(), blk.starts_because, by_id[&x[0]].text.trim(), blk.left / sc, blk.top / sc, blk.right / sc, blk.bottom / sc);
        for l in &blk.lines { eprintln!("        base {:.3} left {:.3} right {:.3} n {}", l.baseline / sc, l.left / sc, l.right / sc, l.objects.len()); }
    }
}

/// Paragraph gap sweep with columns out of phase: how often is a paragraph right with the other column present, and
/// with the column alone, as a function of the paragraph spacing (extra baseline pitch in em over the line pitch).
#[test]
#[ignore = "V4 review: run with --ignored (see the file header)"]
fn v4_cross_column_gap_sweep() {
    let n = seeds_env("V4_SEEDS", 300);
    eprintln!("[gap sweep] 2 columns, body 9 pt, justified or ragged at random, no lists/tables/outlines");
    for gap in [0.25f64, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 1.0, 1.2] {
        let (mut total, mut together, mut alone) = (0, 0, 0);
        for s in 0..n {
            let o = PageOpts { columns: 2, body: 9.0, weird: false, outlined: false, tables: false, lists: false, height: 792.0, gap_extra: Some(gap), rtl: false, force_justify: None };
            let pg = gen_page(s, &o);
            let pb = build(&pg);
            let by_id: HashMap<usize, &TextRun> = pg.runs.iter().map(|r| (r.object, r)).collect();
            for t in pg.truth.iter().filter(|t| t.kind == "paragraph") {
                let mut want = t.objs.clone();
                want.sort_unstable();
                let hit1 = pb.blocks.iter().any(|b| { let mut x = b.objects(); x.sort_unstable(); x == want });
                let (xl, xr) = t.objs.iter().fold((f32::MAX, f32::MIN), |a, o| (a.0.min(by_id[o].rect.left), a.1.max(by_id[o].rect.right)));
                let frags: Vec<Frag> = pb.frags.iter().filter(|f| f.left >= xl - 1.0 && f.right <= xr + 1.0).cloned().collect();
                let b2 = blocks::detect(&frags, &[]);
                let hit2 = b2.iter().any(|b| { let mut x = b.objects(); x.sort_unstable(); x == want });
                total += 1;
                together += hit1 as usize;
                alone += hit2 as usize;
            }
        }
        eprintln!("    paragraph spacing +{gap:.2} em: {total} paragraphs; exact with both columns {:.1}%, with the column alone {:.1}%", 100.0 * together as f64 / total as f64, 100.0 * alone as f64 / total as f64);
    }
}

/// How often does the ragged-text rule (a short line that ends a sentence) cut INSIDE a paragraph, on single-column
/// pages whose paragraphs are separated by a clear gap (so nothing else can cut them)?
#[test]
#[ignore = "V4 review: run with --ignored (see the file header)"]
fn v4_ragged_sentence_end_rule_false_cut_rate() {
    let n = seeds_env("V4_SEEDS", 600);
    for body in [8.0f64, 10.0, 12.0] {
        let (mut total, mut exact, mut split_short) = (0usize, 0usize, 0usize);
        let mut other = 0usize;
        for s in 0..n {
            let o = PageOpts { columns: 1, body, weird: false, outlined: false, tables: false, lists: false, height: 792.0, gap_extra: Some(0.8), rtl: false, force_justify: Some(false) };
            let pg = gen_page(s, &o);
            let pb = build(&pg);
            for t in pg.truth.iter().filter(|t| t.kind == "paragraph" && t.nlines >= 3) {
                let mut want = t.objs.clone();
                want.sort_unstable();
                total += 1;
                if pb.blocks.iter().any(|b| { let mut x = b.objects(); x.sort_unstable(); x == want }) {
                    exact += 1;
                    continue;
                }
                let mut owners: Vec<usize> = t.objs.iter().filter_map(|o| pb.by_object.get(o).map(|x| x.0)).collect();
                owners.sort_unstable();
                owners.dedup();
                if owners.iter().skip(1).any(|&b| pb.blocks[b].starts_because == "short-line") {
                    split_short += 1;
                } else {
                    other += 1;
                }
            }
        }
        eprintln!("[ragged rule] body {body} pt, single column, ragged, paragraph gap 0.8 em: {total} paragraphs of 3+ lines; exact {exact} ({:.2}%); cut by the short-line rule {split_short} ({:.2}%); other misses {other}", 100.0 * exact as f64 / total as f64, 100.0 * split_short as f64 / total as f64);
    }
}

/// Rule-less tables from the generator (cells of one object, 2-4 columns): how are their cells grouped?
#[test]
#[ignore = "V4 review: run with --ignored (see the file header)"]
fn v4_ruleless_table_grouping() {
    let n = seeds_env("V4_SEEDS", 400);
    let (mut rows, mut horiz_merged_rows, mut cells, mut cells_in_multi_line_blocks, mut cells_alone) = (0usize, 0usize, 0usize, 0usize, 0usize);
    for s in 0..n {
        let o = PageOpts { columns: 1, body: 10.0, weird: false, outlined: false, tables: true, lists: false, height: 792.0, gap_extra: Some(0.8), rtl: false, force_justify: Some(false) };
        let pg = gen_page(s, &o);
        let pb = build(&pg);
        for t in pg.truth.iter().filter(|t| t.kind == "row") {
            rows += 1;
            cells += t.objs.len();
            let mut merged = false;
            for &c in &t.objs {
                let Some(&(bi, li)) = pb.by_object.get(&c) else { continue };
                let line = &pb.blocks[bi].lines[li];
                if line.objects.iter().filter(|x| t.objs.contains(x)).count() >= 2 {
                    merged = true;
                }
                if pb.blocks[bi].lines.len() > 1 {
                    cells_in_multi_line_blocks += 1;
                } else if line.objects.len() == 1 {
                    cells_alone += 1;
                }
            }
            if merged {
                horiz_merged_rows += 1;
            }
        }
    }
    eprintln!("[rule-less tables] {rows} rows, {cells} cells: rows with two or more cells merged into ONE editor line {horiz_merged_rows} ({:.1}%); cells that open a multi-line block (a whole column) {cells_in_multi_line_blocks} ({:.1}%); cells alone in a one-line block {cells_alone} ({:.1}%)", 100.0 * horiz_merged_rows as f64 / rows as f64, 100.0 * cells_in_multi_line_blocks as f64 / cells as f64, 100.0 * cells_alone as f64 / cells as f64);
}

}

mod hostile {
    use super::v4gen::*;

use pagify_shell::block_input::*;
use pagify_shell::blocks::{self, Frag, Shape};
use pdf_core::document::{DrawnKind, DrawnObject, RunStyle, TextRun};
use std::collections::{HashMap, HashSet};

fn check_detect_contract(frags: &[Frag], shapes: &[Shape], p: &blocks::Params) -> Result<(), String> {
    let bl = blocks::detect_with(frags, shapes, p);
    let ids: HashSet<usize> = frags.iter().map(|f| f.object).collect();
    let mut seen: HashMap<usize, usize> = HashMap::new();
    for (bi, b) in bl.iter().enumerate() {
        if b.lines.is_empty() {
            return Err(format!("block {bi} has no lines"));
        }
        for l in &b.lines {
            if l.objects.is_empty() && l.outlined.is_empty() {
                return Err(format!("block {bi}: a line with nothing in it"));
            }
            for &o in &l.objects {
                if seen.insert(o, bi).is_some() {
                    return Err(format!("object {o} appears in two places"));
                }
            }
            if !(l.left.is_finite() && l.right.is_finite() && l.top.is_finite() && l.bottom.is_finite() && l.baseline.is_finite()) {
                return Err(format!("block {bi}: non-finite line geometry {:?}", (l.left, l.top, l.right, l.bottom, l.baseline)));
            }
            if l.left > l.right || l.top > l.bottom {
                return Err(format!("block {bi}: line rect not normalised"));
            }
        }
        if !(b.left.is_finite() && b.right.is_finite() && b.top.is_finite() && b.bottom.is_finite()) {
            return Err(format!("block {bi}: non-finite block geometry"));
        }
    }
    if seen.len() != ids.len() || seen.keys().any(|o| !ids.contains(o)) {
        return Err(format!("blocks hold {} objects, input has {} distinct ids", seen.len(), ids.len()));
    }
    // ordering and enclosure (the documented output contract): blocks by (first line top, left, first id); lines top to
    // bottom with strictly increasing baselines; objects left to right (ties by id); a line holds the rect of each member
    let mut by_obj: HashMap<usize, &Frag> = HashMap::new();
    for f in frags {
        by_obj.entry(f.object).and_modify(|e| { if (f.left, f.top, f.right, f.bottom).partial_cmp(&(e.left, e.top, e.right, e.bottom)) == Some(std::cmp::Ordering::Less) && false { *e = f; } }).or_insert(f);
    }
    for w in bl.windows(2) {
        let (a, b) = (&w[0].lines[0], &w[1].lines[0]);
        let ka = (a.top, a.left);
        let kb = (b.top, b.left);
        if ka.0.total_cmp(&kb.0).then(ka.1.total_cmp(&kb.1)) == std::cmp::Ordering::Greater {
            return Err(format!("blocks out of order: {ka:?} then {kb:?}"));
        }
    }
    for (bi, b) in bl.iter().enumerate() {
        for w in b.lines.windows(2) {
            if p.row_tol > 0.0 && !(w[0].baseline < w[1].baseline) {
                return Err(format!("block {bi}: lines not top to bottom ({} then {})", w[0].baseline, w[1].baseline));
            }
        }
        for l in &b.lines {
            let lefts: Vec<(f32, usize)> = l.objects.iter().filter_map(|o| by_obj.get(o).map(|f| (f.left.min(f.right), *o))).collect();
            // (the detector keeps the lowest-canonical duplicate of an id; with duplicate ids the left may differ: only check unique ids)
            if lefts.len() == l.objects.len() && frags.iter().filter(|f| l.objects.contains(&f.object)).count() == l.objects.len() {
                if lefts.windows(2).any(|w| w[0].0.total_cmp(&w[1].0).then(w[0].1.cmp(&w[1].1)) == std::cmp::Ordering::Greater) {
                    return Err(format!("block {bi}: objects of a line not left to right: {lefts:?}"));
                }
                for (o, _) in l.objects.iter().zip(0..) {
                    let f = by_obj[o];
                    let ok = (f.left as f64).is_finite() && (f.right as f64).is_finite() && (f.top as f64).is_finite() && (f.bottom as f64).is_finite();
                    if ok && (f.size > 0.0 && f.size.is_finite() && f.baseline.is_finite()) && !f.rotated {
                        let (fl, fr, ft, fb) = (f.left.min(f.right), f.left.max(f.right), f.top.min(f.bottom), f.top.max(f.bottom));
                        if !(l.left <= fl && l.right >= fr && l.top <= ft && l.bottom >= fb) {
                            return Err(format!("block {bi}: the line rect does not enclose member {o}"));
                        }
                    }
                }
            }
        }
    }
    let mut outl: HashSet<usize> = HashSet::new();
    for b in &bl {
        for l in &b.lines {
            for &o in &l.outlined {
                if !outl.insert(o) {
                    return Err(format!("outlined shape {o} appears twice"));
                }
            }
        }
    }
    let again = blocks::detect_with(frags, shapes, p);
    if again != bl {
        return Err("two runs on the same input differ".into());
    }
    Ok(())
}

#[test]
#[ignore = "V4 review: run with --ignored (see the file header)"]
fn v4_detect_hostile_input_never_panics_and_keeps_every_object_once() {
    let n = seeds_env("V4_HOSTILE", 20000);
    let (failed, _) = run_seeds("detect hostile", 0, n, |s| {
        let mut r = Rng::new(s);
        let mode = (s % 2) as u32; // 0 mostly sane, 1 every field hostile
        let nf = r.between(0, if s % 50 == 0 { 400 } else { 60 });
        let mut frags: Vec<Frag> = (0..nf)
            .map(|i| {
                let id = if r.chance(0.1) && i > 0 { r.int(i) } else { i * 3 };
                hostile_frag(&mut r, id, mode)
            })
            .collect();
        if r.chance(0.02) && !frags.is_empty() {
            let k = r.int(frags.len());
            frags[k].object = usize::MAX;
        }
        let ns = r.between(0, 30);
        let shapes: Vec<Shape> = (0..ns).map(|i| hostile_shape(&mut r, i * 3 + 1)).collect();
        check_detect_contract(&frags, &shapes, &blocks::Params::default())
    });
    finish("detect hostile", failed);
}

#[test]
#[ignore = "V4 review: run with --ignored (see the file header)"]
fn v4_detect_with_every_switch_and_extreme_numbers_never_panics() {
    // not the shipped configuration (Params::default is), but the ablation harness and tests use it
    let n = seeds_env("V4_HOSTILE", 3000);
    let (failed, _) = run_seeds("detect params fuzz", 0, n, |s| {
        let mut r = Rng::new(s ^ 0x5555);
        let mut p = blocks::Params::default();
        macro_rules! sw {
            ($($f:ident),*) => { $( if r.chance(0.15) { p.$f = !p.$f; } )* }
        }
        sw!(corridor, outlined, rules, boxes, size_rule, style_rule, pitch_rule, list_items, indent_rule, hanging_items, margin_rule, short_lines, script_marks, marker_merge, stray_marks, ruled_bands, spanning_lines, unknown_stem_same_face);
        macro_rules! num {
            ($($f:ident),*) => { $( if r.chance(0.2) { p.$f = [0.0f32, 1e-6, 0.01, 0.5, 1.0, 5.0, 100.0, 1e9, -1.0][r.int(9)]; } )* }
        }
        num!(row_tol, word_gap, max_hole, corridor_width, pos2_gap, pos1_gap, iso_gap, iso_style_gap, near, scan_window, gap_only, script_ratio, script_shift, script_gap, marker_gap, band_below, band_above, link_overlap, pitch_extra, leading, wide_leading, indent, margin_shift, edge_dominance, short_margin, short_slack, size_break, size_rel, pure_share, edge_tol, rule_thick, rule_len, rule_cover, underline_zone, underline_cover, cover_max);
        if r.chance(0.2) {
            p.pos3 = r.int(8);
        }
        if r.chance(0.2) {
            p.cell_lines = r.int(6);
        }
        if r.chance(0.2) {
            p.edge_lines = r.int(10);
        }
        if r.chance(0.2) {
            p.justify_evidence = r.int(30);
        }
        if r.chance(0.2) {
            p.stem_tol = r.int(300) as u16;
        }
        if r.chance(0.2) {
            p.colour_delta = r.int(256) as u8;
        }
        let mode = (s % 2) as u32;
        let (frags, shapes): (Vec<Frag>, Vec<Shape>) = if s % 3 != 0 {
            let pg = gen_page(s, &opts_for(s));
            let pb = build(&pg);
            let sh: Vec<Shape> = pg.shapes.iter().map(|d| Shape { object: d.object, left: d.rect.left, top: d.rect.top, right: d.rect.right, bottom: d.rect.bottom, depth: 0 }).collect();
            (pb.frags, sh)
        } else {
            let nf = r.between(0, 80);
            let ns = r.between(0, 20);
            let fr: Vec<Frag> = (0..nf).map(|i| hostile_frag(&mut r, i, mode)).collect();
            let sh: Vec<Shape> = (0..ns).map(|i| hostile_shape(&mut r, 1000 + i)).collect();
            (fr, sh)
        };
        check_detect_contract(&frags, &shapes, &p)
    });
    finish("detect params fuzz", failed);
}

#[test]
#[ignore = "V4 review: run with --ignored (see the file header)"]
fn v4_whole_pipeline_hostile_runs_never_panic_and_the_guard_never_accepts_a_violation() {
    let n = seeds_env("V4_HOSTILE", 10000);
    let accepted = std::cell::Cell::new(0u64);
    let refused = std::cell::Cell::new(0u64);
    let (failed, _) = run_seeds("pipeline hostile", 0, n, |s| {
        let mut r = Rng::new(s ^ 0xFEED);
        let mode = (s % 2) as u32;
        let nr = r.between(0, if s % 50 == 0 { 300 } else { 50 });
        let mut runs: Vec<TextRun> = (0..nr)
            .map(|i| {
                let id = if r.chance(0.08) && i > 0 { r.int(i) } else { i * 2 };
                hostile_run(&mut r, id, mode)
            })
            .collect();
        let mut styles: HashMap<usize, RunStyle> = HashMap::new();
        let mut faces: HashMap<usize, String> = HashMap::new();
        let mut shapes: Vec<DrawnObject> = Vec::new();
        // overlay a realistic page now and then so that real blocks form next to the hostile objects
        if s % 4 == 0 {
            let pg = gen_page(s, &opts_for(s));
            let off = 100_000;
            for t in &pg.runs {
                let mut t = t.clone();
                t.object += off;
                runs.push(t);
            }
            for (k, v) in &pg.styles {
                styles.insert(k + off, *v);
            }
            for (k, v) in &pg.faces {
                faces.insert(k + off, v.clone());
            }
            for d in &pg.shapes {
                let mut d = d.clone();
                d.object += off;
                shapes.push(d);
            }
            // hostile mutation of a few realistic runs
            for _ in 0..r.between(0, 6) {
                let i = r.int(runs.len());
                let t = &mut runs[i];
                match r.int(8) {
                    0 => t.rect.left = f32::NAN,
                    1 => t.origin.y = f32::INFINITY,
                    2 => t.size = 0.0,
                    3 => t.text = "\n".into(),
                    4 => t.color.a = 0,
                    5 => t.rect.right = t.rect.left,
                    6 => t.origin.x += 0.3,
                    _ => {}
                }
            }
        }
        for t in &runs {
            if r.chance(0.8) {
                styles.entry(t.object).or_insert_with(|| hostile_style(&mut r));
            }
            if r.chance(0.5) {
                faces.entry(t.object).or_insert_with(|| ["Arial", "ABCDEF+Times", ""][r.int(3)].to_string());
            }
        }
        for i in 0..r.between(0, 20) {
            shapes.push(DrawnObject {
                object: 50_000 + i,
                kind: if r.chance(0.8) { DrawnKind::Shape } else { DrawnKind::Group },
                rect: pdf_core::document::Rect { left: hostile_f32(&mut r, 0.0, 600.0), top: hostile_f32(&mut r, 0.0, 800.0), right: hostile_f32(&mut r, 0.0, 600.0), bottom: hostile_f32(&mut r, 0.0, 800.0) },
                label: String::new(),
                depth: r.int(3),
                opacity: 1.0,
                movable: true,
            });
        }
        let pb = build_page_blocks_from(1, 2, 3, runs, styles, faces, shapes);
        let admitted: HashSet<usize> = pb.frags.iter().map(|f| f.object).collect();
        let in_blocks = objects_of(&pb.blocks);
        if in_blocks.len() != admitted.len() || ids_set(&in_blocks) != admitted {
            return Err("blocks do not hold exactly the admitted objects".into());
        }
        for _ in 0..12 {
            let (x, y) = (hostile_f32(&mut r, 0.0, 612.0), hostile_f32(&mut r, 0.0, 792.0));
            let tol = hostile_f32(&mut r, 0.0, 6.0);
            let _ = pick_seed(&pb, x, y, tol);
        }
        for bi in 0..pb.blocks.len() + 1 {
            let guard = check_editor_invariants(&pb, bi);
            let lines = editor_lines(&pb, bi);
            let _ = line_texts(&pb, &lines);
            if bi < pb.blocks.len() {
                let mut t = PickTrace::new(1, (1.0, 2.0));
                t.page_facts(&pb);
                t.block_facts(&pb, bi);
                let _ = format_pick_line(&t);
                match guard {
                    Ok(()) => {
                        accepted.set(accepted.get() + 1);
                        let ind = independent_violations(&pb, bi);
                        if !ind.is_empty() {
                            return Err(format!("GUARD ACCEPTED block {bi} with violations {:?}", ind));
                        }
                    }
                    Err(m) => {
                        refused.set(refused.get() + 1);
                        if !m.starts_with('C') {
                            return Err(format!("refusal without a C-number: {m}"));
                        }
                    }
                }
            } else if guard.is_ok() {
                return Err("guard accepted a block index that does not exist".into());
            }
        }
        Ok(())
    });
    eprintln!("[pipeline hostile] blocks accepted by the guard {}, refused {}", accepted.get(), refused.get());
    finish("pipeline hostile", failed);
}

// ------------------------------------------------------------------------------------------------
// guard completeness: mutate valid blocks one way at a time; the guard must refuse every mutant that the independent
// statement of the contract flags (and this proves the independent statement itself can fail, per invariant)
// ------------------------------------------------------------------------------------------------

#[test]
#[ignore = "V4 review: run with --ignored (see the file header)"]
fn v4_guard_refuses_every_mutated_block_that_breaks_the_independent_contract() {
    let n = seeds_env("V4_MUTSEEDS", 3000);
    let stats: std::cell::RefCell<std::collections::BTreeMap<&'static str, (u64, u64, u64)>> = Default::default(); // mutation -> (applied, independent flags, guard refuses)
    let missed: std::cell::RefCell<Vec<String>> = Default::default();
    let (failed, _) = run_seeds("guard mutation", 0, n, |s| {
        let mut r = Rng::new(s ^ 0xBADD);
        let pg = gen_page(s, &PageOpts { weird: false, ..opts_for(s) });
        let base = build(&pg);
        // candidates: blocks with 3+ lines
        let cands: Vec<usize> = (0..base.blocks.len()).filter(|&b| base.blocks[b].lines.len() >= 3 && base.blocks[b].lines.iter().all(|l| l.objects.len() >= 2)).collect();
        if cands.is_empty() {
            return Ok(());
        }
        for round in 0..6 {
            let bi = cands[r.int(cands.len())];
            let mut pb = base.clone();
            let nl = pb.blocks[bi].lines.len();
            let (l1, l2) = (r.int(nl), (r.int(nl - 1) + 1 + 0) % nl);
            let name: &'static str = match r.int(14) {
                0 => {
                    let l = &mut pb.blocks[bi].lines[l1];
                    let k = r.int(l.objects.len() - 1);
                    l.objects.swap(k, k + 1);
                    "swap two objects of a line (C7)"
                }
                1 => {
                    let o = pb.blocks[bi].lines[l1].objects.pop().unwrap();
                    let t = if l1 == l2 { (l1 + 1) % nl } else { l2 };
                    pb.blocks[bi].lines[t].objects.push(o);
                    "move an object to another line (C7/C8)"
                }
                2 => {
                    let o = pb.blocks[bi].lines[l1].objects[0];
                    pb.blocks[bi].lines[l1].objects.push(o);
                    "duplicate an object in its line (C3)"
                }
                3 => {
                    let o = pb.blocks[bi].lines[l1].objects[0];
                    let t = if l1 == l2 { (l1 + 1) % nl } else { l2 };
                    pb.blocks[bi].lines[t].objects.push(o);
                    "duplicate an object into another line (C3)"
                }
                4 => {
                    pb.blocks[bi].lines[l1].objects[0] = usize::MAX;
                    "usize::MAX as an object (C2)"
                }
                5 => {
                    pb.blocks[bi].lines[l1].objects[0] = 9_999_999;
                    "an object id that is not on the page (C2)"
                }
                6 => {
                    pb.blocks[bi].lines[l1].objects.clear();
                    "empty a line (C1)"
                }
                7 => {
                    let t = if l1 == l2 { (l1 + 1) % nl } else { l2 };
                    pb.blocks[bi].lines.swap(l1, t);
                    "swap two lines (C8)"
                }
                8 => {
                    let o = pb.blocks[bi].lines[l1].objects[0];
                    pb.runs.get_mut(&o).unwrap().color.a = 0;
                    "alpha 0 member (C6)"
                }
                9 => {
                    let o = pb.blocks[bi].lines[l1].objects[0];
                    pb.runs.get_mut(&o).unwrap().text = "  ".into();
                    "blank member (C5)"
                }
                10 => {
                    let a = pb.blocks[bi].lines[l1].objects[0];
                    let t = if l1 == l2 { (l1 + 1) % nl } else { l2 };
                    let b = pb.blocks[bi].lines[t].objects[0];
                    let o = pb.runs[&a].origin;
                    pb.runs.get_mut(&b).unwrap().origin = o;
                    "two members at one origin (C4)"
                }
                11 => {
                    let o = pb.blocks[bi].lines[l1].objects[0];
                    pb.runs.get_mut(&o).unwrap().size = 0.0;
                    "size 0 member (C6)"
                }
                12 => {
                    let o = pb.blocks[bi].lines[l1].objects[0];
                    pb.runs.get_mut(&o).unwrap().origin.y = f32::NAN;
                    "NaN origin (C6)"
                }
                _ => {
                    // a far-away object appended to a line: a 'second column' piece in one line (C8 gap)
                    let o = pb.blocks[bi].lines[l1].objects[0];
                    let mut far = pb.runs[&o].clone();
                    far.object = 8_000_000 + round;
                    far.rect.left += 900.0;
                    far.rect.right += 900.0;
                    far.origin.x += 900.0;
                    pb.runs.insert(far.object, far.clone());
                    pb.blocks[bi].lines[l1].objects.push(far.object);
                    "a piece far to the right in the same line (C8)"
                }
            };
            let guard = check_editor_invariants(&pb, bi);
            let ind = independent_violations(&pb, bi);
            let mut st = stats.borrow_mut();
            let e = st.entry(name).or_default();
            e.0 += 1;
            e.1 += (!ind.is_empty()) as u64;
            e.2 += guard.is_err() as u64;
            if guard.is_ok() && !ind.is_empty() {
                if missed.borrow().len() < 8 {
                    missed.borrow_mut().push(format!("seed {s} block {bi}: {name}: independent {:?}", &ind[..ind.len().min(2)]));
                }
                return Err(format!("GUARD ACCEPTED a mutant ({name}): {:?}", &ind[..ind.len().min(2)]));
            }
        }
        Ok(())
    });
    eprintln!("[guard mutation] per mutation: (applied, independent contract flags it, guard refuses it)");
    for (k, v) in stats.borrow().iter() {
        eprintln!("    {k:55} {:?}", v);
    }
    for m in missed.borrow().iter() {
        eprintln!("    MISSED {m}");
    }
    finish("guard mutation", failed);
}

}

mod text {
    use super::v4gen::*;

use pagify_shell::block_input::*;
use pagify_shell::blocks::{self, Frag};
use pdf_core::document::{Color, Point, Rect, RunStyle, TextRun};
use std::collections::HashMap;

fn run(object: usize, text: &str, left: f32, base: f32) -> TextRun {
    let w = 4.5 * text.chars().count().max(1) as f32;
    TextRun { object, text: text.to_string(), rect: Rect { left, top: base - 6.0, right: left + w, bottom: base + 1.5 }, origin: Point { x: left, y: base }, size: 8.0, color: Color { r: 0, g: 0, b: 0, a: 255 } }
}

/// Objects of one line laid end to end (gap `gap` pt), ids from `first`.
fn line_of(first: usize, texts: &[&str], left: f32, base: f32, gap: f32) -> Vec<TextRun> {
    let mut x = left;
    texts
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let r = run(first + i, t, x, base);
            x = r.rect.right + gap;
            r
        })
        .collect()
}

fn pb_of(runs: Vec<TextRun>) -> PageBlocks {
    let st: HashMap<usize, RunStyle> = runs.iter().map(|r| (r.object, RunStyle { font: 1, stem_milli_em: Some(74), axis: (1.0, 0.0) })).collect();
    build_page_blocks_from(0, 0, 0, runs, st, HashMap::new(), vec![])
}

fn line_text_of(pb: &PageBlocks, object: usize) -> String {
    let (bi, li) = pb.by_object[&object];
    let specs = editor_lines(pb, bi);
    line_texts(pb, &specs)[li].clone()
}

#[test]
#[ignore = "V4 review: run with --ignored (see the file header)"]
fn v4_text_plain_concatenation_survives_astral_combining_zwj_and_controls() {
    // one line of 5 objects: an emoji (astral, a surrogate pair in UTF-16), a combining sequence, a ZWJ family emoji,
    // a control character and an ordinary word. No space may be invented between them whatever the gaps.
    let texts = ["\u{1F600}", "e\u{301}", "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}", "x\u{2}", "word "];
    let runs = line_of(1, &texts, 20.0, 100.0, 1.2);
    let pb = pb_of(runs);
    assert_eq!(pb.by_object.len(), 5, "every one of them is a member");
    let t = line_text_of(&pb, 1);
    assert_eq!(t, texts.concat(), "plain concatenation, no space from geometry, no loss");
    assert_eq!(check_editor_invariants(&pb, pb.by_object[&1].0), Ok(()));
}

#[test]
#[ignore = "V4 review: run with --ignored (see the file header)"]
fn v4_text_line_break_characters_other_than_lf_and_cr_are_left_alone() {
    // sanitise() replaces only \n and \r. U+2028, U+2029, U+0085, VT and FF pass through. They do not split a
    // buffer on '\n', so the buffer's line count is safe; this pins the fact.
    for (name, text) in [("LS", "a\u{2028}b"), ("PS", "a\u{2029}b"), ("NEL", "a\u{85}b"), ("VT", "a\u{b}b"), ("FF", "a\u{c}b")] {
        let pb = pb_of(line_of(1, &[text, "tail"], 20.0, 100.0, 1.2));
        let t = line_text_of(&pb, 1);
        assert_eq!(t, format!("{text}tail"), "{name} is kept");
        assert!(!t.contains('\n') && !t.contains('\r'));
        assert_eq!(t.split('\n').count(), 1);
    }
    // CRLF becomes two spaces (the pinned behaviour of line_breaks_in_the_text_become_spaces): the line is longer than the engine's text
    let pb = pb_of(vec![run(1, "a\r\nb", 20.0, 100.0)]);
    assert_eq!(line_text_of(&pb, 1), "a  b");
}

#[test]
#[ignore = "V4 review: run with --ignored (see the file header)"]
fn v4_text_a_lone_u0002_hyphen_marker_object_is_a_member_with_one_character() {
    // PDFium's hyphenation marker as an object of its own: not whitespace, so it is not blank and not excluded
    let pb = pb_of(vec![run(1, "supe", 20.0, 100.0), run(2, "\u{2}", 38.0, 100.0), run(3, "next line here", 20.0, 112.0)]);
    assert!(pb.excluded.blank.is_empty());
    assert_eq!(pb.by_object.len(), 3);
    assert_eq!(line_text_of(&pb, 1), "supe\u{2}");
}

#[test]
#[ignore = "V4 review: run with --ignored (see the file header)"]
fn v4_text_zero_width_and_bom_only_objects_count_as_readable_text() {
    // U+200B and U+FEFF are not White_Space: trim() keeps them, so such an object is admitted as a member with text
    for t in ["\u{200B}", "\u{FEFF}", "\u{200B}\u{200B}"] {
        let pb = pb_of(vec![run(1, t, 20.0, 100.0)]);
        eprintln!("object with text {:?}: blank={} admitted={}", t, !pb.excluded.blank.is_empty(), pb.by_object.contains_key(&1));
    }
    let pb = pb_of(vec![run(1, "\u{200B}", 20.0, 100.0)]);
    assert!(pb.by_object.contains_key(&1), "a zero-width space is admitted (not blank): the editor would show nothing and apply would still rewrite it");
    // NBSP is White_Space and is blank
    let pb = pb_of(vec![run(1, "\u{a0}", 20.0, 100.0)]);
    assert!(!pb.by_object.contains_key(&1));
}

#[test]
#[ignore = "V4 review: run with --ignored (see the file header)"]
fn v4_text_rtl_arabic_line_reads_in_visual_order_and_apply_target_is_the_last_word() {
    // an RTL line of four words as four objects, first word (reading order) at the RIGHT. Object ids ascend with x
    // (the Skia/Chrome order). The logical text of each object is in reading order of its own letters.
    let words = ["\u{0627}\u{0644}\u{0636}\u{0648}\u{0621} ", "\u{0645}\u{0646} ", "\u{0623}\u{0647}\u{0645} ", "\u{0639}\u{0646}\u{0627}\u{0635}\u{0631}"];
    // reading order: words[0] (rightmost) ... words[3] (leftmost)
    // visual left-to-right order is words[3], words[2], words[1], words[0]; ids ascend with x
    let visual_order: Vec<&str> = words.iter().rev().cloned().collect();
    let runs = line_of(10, &visual_order, 100.0, 100.0, 1.2);
    let pb = pb_of(runs);
    let (bi, li) = pb.by_object[&10];
    let specs = editor_lines(&pb, bi);
    assert_eq!(specs[li].objects, vec![10, 11, 12, 13], "objects run left to right: the LAST word first");
    let got = line_texts(&pb, &specs)[li].clone();
    let reading = words.concat();
    let visual: String = words.iter().rev().cloned().collect();
    assert_eq!(got, visual, "the editor line is the words in VISUAL order (reverse reading order)");
    assert_ne!(got, reading);
    // apply writes the typed line into objects[0]: the leftmost = the last word of the sentence
    assert_eq!(specs[li].objects[0], 10);
    assert_eq!(pb.runs[&10].text, words[3]);
    assert_eq!(check_editor_invariants(&pb, bi), Ok(()), "the guard accepts it: nothing in the contract mentions direction");
}

#[test]
#[ignore = "V4 review: run with --ignored (see the file header)"]
fn v4_text_sentence_end_marks_are_ascii_only() {
    // a ragged 3-line paragraph followed by a 2-line one with NO extra spacing: the only cue is the short line that
    // ends a sentence. Same geometry, only the full stop differs.
    fn page(stop: &str) -> Vec<usize> {
        let mut frags = Vec::new();
        let mut id = 0;
        let mut base = 100.0f32;
        for (p, n) in [3usize, 2].iter().enumerate() {
            for l in 0..*n {
                let last = l + 1 == *n;
                let right = if last { 160.0 } else { 400.0 };
                let text = if last { format!("short end{stop}") } else { "a full line of words in the paragraph".to_string() };
                frags.push(Frag { object: id, left: 100.0, top: base - 8.0, right, bottom: base + 2.5, baseline: base, size: 10.0, font: 0, stem: Some(74), face: "Regular".into(), rgb: [0; 3], text, rotated: false });
                id += 1;
                base += 12.5;
            }
            let _ = p;
        }
        let b = blocks::detect(&frags, &[]);
        let mut sizes: Vec<usize> = b.iter().map(|b| b.lines.len()).collect();
        sizes.sort_unstable();
        sizes
    }
    for stop in [".", "!", "?", "\u{2026}"] {
        assert_eq!(page(stop), vec![2, 3], "stop {stop:?} ends the paragraph");
    }
    for (name, stop) in [("ideographic full stop U+3002", "\u{3002}"), ("fullwidth ! U+FF01", "\u{FF01}"), ("fullwidth ? U+FF1F", "\u{FF1F}"), ("Arabic ? U+061F", "\u{061F}"), ("Urdu full stop U+06D4", "\u{06D4}")] {
        eprintln!("{name}: blocks of lines {:?} (two paragraphs would be [2, 3])", page(stop));
    }
}

// ------------------------------------------------------------------------------------------------
// the guard on ordinary typographic oddities: superscript, subscript, drop cap, inline larger word
// ------------------------------------------------------------------------------------------------

fn sized(object: usize, text: &str, left: f32, base: f32, size: f32) -> TextRun {
    let w = 0.5 * size * text.chars().count().max(1) as f32;
    TextRun { object, text: text.to_string(), rect: Rect { left, top: base - 0.8 * size, right: left + w, bottom: base + 0.22 * size }, origin: Point { x: left - 0.05 * size, y: base }, size, color: Color { r: 0, g: 0, b: 0, a: 255 } }
}

#[test]
#[ignore = "V4 review: run with --ignored (see the file header)"]
fn v4_guard_accepts_superscripts_subscripts_drop_caps_and_larger_inline_words() {
    let mut runs: Vec<TextRun> = Vec::new();
    let mut id = 0usize;
    let mut push = |runs: &mut Vec<TextRun>, t: &str, l: f32, b: f32, s: f32| {
        runs.push(sized(id, t, l, b, s));
        id += 1;
        id - 1
    };
    // four lines of one paragraph at 10 pt, pitch 12.5; line 2 carries a superscript (rise 0.4 em), line 3 a subscript
    // (drop 0.25 em) and a larger inline word (13 pt), line 4 ends the paragraph
    let words = ["The measured efficacy of the luminaire", "is given at the rated current", "and at the stated colour temperature", "see the notes below"];
    let (x0, mut base) = (60.0f32, 100.0f32);
    push(&mut runs, words[0], x0, base, 10.0);
    base += 12.5;
    let host = push(&mut runs, words[1], x0, base, 10.0);
    let host_right = runs[host].rect.right;
    let sup = push(&mut runs, "1", host_right + 0.3, base - 4.0, 6.5);
    base += 12.5;
    push(&mut runs, "and the formula H", x0, base, 10.0);
    let sub = push(&mut runs, "2", x0 + 0.5 * 10.0 * 17.0 + 0.2, base + 2.5, 6.5);
    push(&mut runs, "O", x0 + 0.5 * 10.0 * 17.0 + 4.0, base, 10.0);
    let big = push(&mut runs, "VALUE", x0 + 130.0, base, 13.0);
    base += 12.5;
    push(&mut runs, words[3], x0, base, 10.0);
    let pb = pb_of(runs);
    let (bi, _) = pb.by_object[&host];
    let verdicts: Vec<(&str, usize, Result<(), String>)> = [("superscript", sup), ("subscript", sub), ("larger inline word", big)].iter().map(|(n, o)| (*n, *o, pb.by_object.get(o).map(|x| check_editor_invariants(&pb, x.0)).unwrap_or(Err("not a member".into())))).collect();
    for (n, o, v) in &verdicts {
        let line = pb.by_object.get(o).map(|&(b, l)| line_texts(&pb, &editor_lines(&pb, b))[l].clone()).unwrap_or_default();
        eprintln!("{n}: object {o} in block {:?}, guard {v:?}, its line reads {line:?}", pb.by_object.get(o).map(|x| x.0));
    }
    assert_eq!(pb.blocks.len(), 1, "one paragraph: {} blocks", pb.blocks.len());
    assert_eq!(check_editor_invariants(&pb, bi), Ok(()));
}

#[test]
#[ignore = "V4 review: run with --ignored (see the file header)"]
fn v4_guard_drop_cap_paragraph() {
    // a 30 pt drop cap "T" whose baseline is the second line's, lines 1 and 2 indented to clear it
    let mut runs = Vec::new();
    let base1 = 100.0f32;
    runs.push(sized(0, "T", 60.0, base1 + 12.5, 30.0));
    runs.push(sized(1, "he first line wraps round the cap", 82.0, base1, 10.0));
    runs.push(sized(2, "and the second line too and then", 82.0, base1 + 12.5, 10.0));
    runs.push(sized(3, "the third line starts at the margin and runs on", 60.0, base1 + 25.0, 10.0));
    runs.push(sized(4, "the last line is short", 60.0, base1 + 37.5, 10.0));
    let pb = pb_of(runs);
    for (bi, b) in pb.blocks.iter().enumerate() {
        eprintln!("drop cap: block {bi} starts {:?}: lines {:?}, guard {:?}", b.starts_because, b.lines.iter().map(|l| l.objects.clone()).collect::<Vec<_>>(), check_editor_invariants(&pb, bi));
    }
    let (bi, _) = pb.by_object[&1];
    assert_eq!(check_editor_invariants(&pb, bi), Ok(()), "whatever the grouping, the guard must not refuse an ordinary drop cap block with a message about baselines");
}

// ------------------------------------------------------------------------------------------------
// faux bold: an overstrike copy is a twin only within 1 pt, whatever the type size (em-relative claim vs the adapter)
// ------------------------------------------------------------------------------------------------

#[test]
#[ignore = "V4 review: run with --ignored (see the file header)"]
fn v4_faux_bold_overstrike_offset_grows_with_the_type_size_but_the_twin_rule_does_not() {
    eprintln!("size  offset(0.04 em)  twin shadowed?  editor line");
    for size in [8.0f32, 12.0, 20.0, 24.0, 30.0, 40.0, 72.0] {
        let off = 0.04 * size; // a typical overstrike offset is a few hundredths of an em
        let a = sized(1, "HEADING", 100.0, 200.0, size);
        let mut b = sized(2, "HEADING", 100.0 + off, 200.0, size);
        b.origin.x = a.origin.x + off;
        let pb = pb_of(vec![a, b]);
        let (bi, li) = pb.by_object[&1];
        let text = line_texts(&pb, &editor_lines(&pb, bi))[li].clone();
        eprintln!("{size:>4}  {off:>5.2} pt        {:?}     {:?}   guard {:?}", pb.excluded.shadowed.contains(&2), text, check_editor_invariants(&pb, bi));
    }
    let size = 40.0f32;
    let a = sized(1, "HEADING", 100.0, 200.0, size);
    let mut b = sized(2, "HEADING", 100.0 + 0.04 * size, 200.0, size);
    b.origin.x = a.origin.x + 0.04 * size;
    let pb = pb_of(vec![a, b]);
    let (bi, li) = pb.by_object[&1];
    let text = line_texts(&pb, &editor_lines(&pb, bi))[li].clone();
    if std::env::var("V4_REPRO_EXPECT_FAIL").map(|v| v == "1").unwrap_or(false) {
        return;
    }
    assert_eq!(text, "HEADING", "an overstrike copy 1.6 pt away at 40 pt (0.04 em, as at 8 pt it would be 0.3 pt and shadowed) must not double the text");
}

}

mod repro {
    use super::v4gen::*;
use pagify_shell::blocks::{self, Frag};

fn f(object: usize, left: f32, right: f32, base: f32, size: f32, text: &str) -> Frag {
    Frag { object, left, top: base - 0.8 * size, right, bottom: base + 0.25 * size, baseline: base, size, font: 0, stem: Some(74), face: "Regular".into(), rgb: [0; 3], text: text.into(), rotated: false }
}

fn groups(b: &[blocks::Block]) -> Vec<Vec<usize>> {
    let mut v: Vec<Vec<usize>> = b
        .iter()
        .map(|b| {
            let mut o = b.objects();
            o.sort_unstable();
            o
        })
        .collect();
    v.sort();
    v
}

/// Two columns of full-width (flush) lines at 10 pt, line pitch 12.5 pt. The right column holds two paragraphs
/// separated only by extra paragraph spacing of `extra` em. The left column's own baseline grid is out of phase:
/// one of its baselines sits `dy` pt above the first line of the right column's second paragraph.
fn page(extra_em: f32, dy: f32, with_left: bool) -> (Vec<Frag>, Vec<usize>, Vec<usize>) {
    let size = 10.0;
    let pitch = 12.5;
    let mut frags = Vec::new();
    let (mut a, mut b) = (Vec::new(), Vec::new());
    let mut id = 0;
    // right column: paragraph A (3 lines), then B (3 lines) after the extra spacing
    let mut base = 100.0;
    for i in 0..3 {
        frags.push(f(id, 320.0, 560.0, base + pitch * i as f32, size, "a full line of right column text"));
        a.push(id);
        id += 1;
    }
    let b0 = base + pitch * 2.0 + pitch + extra_em * size;
    base = b0;
    for i in 0..3 {
        frags.push(f(id, 320.0, 560.0, base + pitch * i as f32, size, "a full line of right column text"));
        b.push(id);
        id += 1;
    }
    if with_left {
        // left column: lines on a grid that puts one baseline `dy` above b0
        let first = b0 - dy - pitch * 5.0;
        for i in 0..10 {
            frags.push(f(id, 50.0, 290.0, first + pitch * i as f32, size, "a full line of left column text"));
            id += 1;
        }
    }
    (frags, a, b)
}

fn run(extra: f32, dy: f32) -> (bool, bool) {
    let (fr, a, b) = page(extra, dy, true);
    let with = groups(&blocks::detect(&fr, &[]));
    let (fr0, _, _) = page(extra, dy, false);
    let alone = groups(&blocks::detect(&fr0, &[]));
    let want = {
        let mut v = vec![a.clone(), b.clone()];
        v.sort();
        v
    };
    (alone == want, with.contains(&a) && with.contains(&b))
}

#[test]
#[ignore = "V4 review: run with --ignored (see the file header)"]
fn v4_repro_a_neighbouring_column_hides_a_paragraph_break() {
    // paragraph spacing of 0.45 em extra (4.5 pt at 10 pt), the other column's baseline 2.5 pt above the line
    let (alone_ok, with_ok) = run(0.45, 2.5);
    eprintln!("extra spacing 0.45 em, neighbour baseline 2.5 pt above: right column alone -> two paragraphs: {alone_ok}; with the other column present -> two paragraphs: {with_ok}");
    assert!(alone_ok, "the layout itself must be detectable without the other column");
    if std::env::var("V4_REPRO_EXPECT_FAIL").map(|v| v == "1").unwrap_or(false) {
        return;
    }
    assert!(with_ok, "a paragraph break of 0.45 em extra spacing was lost because the neighbouring column's baseline pulled the row up");
}

#[test]
#[ignore = "V4 review: run with --ignored (see the file header)"]
fn v4_repro_sweep_how_far_above_and_how_much_spacing() {
    eprintln!("rows = extra paragraph spacing (em), columns = neighbour baseline offset above the paragraph's first line (pt): '.' both right, 'X' lost");
    eprint!("          ");
    let offs = [0.0f32, 0.5, 1.0, 1.5, 2.0, 2.5, 2.8, 3.2, 4.0];
    for o in offs {
        eprint!("{o:>5.1}");
    }
    eprintln!();
    for extra in [0.3f32, 0.4, 0.45, 0.5, 0.55, 0.6, 0.7, 0.8, 1.0] {
        eprint!("{extra:>8.2}  ");
        for o in offs {
            let (alone, with) = run(extra, o);
            eprint!("{:>5}", if !alone { "?" } else if with { "." } else { "X" });
        }
        eprintln!();
    }
}

// ------------------------------------------------------------------------------------------------
// right-to-left: the short-last-line and indent logic look at the wrong edge
// ------------------------------------------------------------------------------------------------

/// `n` paragraphs of `lines` justified lines each in a column x 100..400 at 10 pt. Every line but the last of a
/// paragraph is flush on both edges; the last line is short. LTR: it is short on the right. RTL (mirrored): short on
/// the LEFT, the first word at the right edge. `gap_em` is the extra spacing between paragraphs.
fn rtl_page(paras: usize, lines: usize, gap_em: f32, rtl: bool) -> (Vec<Frag>, Vec<Vec<usize>>) {
    let size = 10.0f32;
    let pitch = 12.5f32;
    let (x0, x1) = (100.0f32, 400.0f32);
    let mut frags = Vec::new();
    let mut truth = Vec::new();
    let mut id = 0usize;
    let mut base = 100.0f32;
    for p in 0..paras {
        let mut ids = Vec::new();
        for l in 0..lines {
            let last = l + 1 == lines;
            let w = if last { 120.0 } else { x1 - x0 };
            // three pieces per line, as the datasheet has
            for k in 0..3 {
                let (a, b) = (w * k as f32 / 3.0, w * (k as f32 + 1.0) / 3.0 - 6.0);
                let (left, right) = if rtl { (x1 - b, x1 - a) } else { (x0 + a, x0 + b) };
                frags.push(f(id, left, right, base, size, "a few words "));
                ids.push(id);
                id += 1;
            }
            base += pitch;
        }
        base += gap_em * size;
        truth.push(ids);
    }
    (frags, truth)
}

fn exact_paragraphs(frags: &[Frag], truth: &[Vec<usize>]) -> usize {
    let g = groups(&blocks::detect(frags, &[]));
    truth.iter().filter(|t| g.contains(t)).count()
}

#[test]
#[ignore = "V4 review: run with --ignored (see the file header)"]
fn v4_repro_rtl_paragraphs_split_or_merge_where_ltr_does_not() {
    eprintln!("5 paragraphs of 5 justified lines, three pieces per line; exact paragraphs of 5:");
    for gap in [0.0f32, 0.1, 0.2, 0.7, 1.0] {
        let (fl, tl) = rtl_page(5, 5, gap, false);
        let (fr, tr) = rtl_page(5, 5, gap, true);
        eprintln!("   extra spacing {gap:.1} em: LTR {}/5, RTL (mirror image) {}/5, blocks LTR {} RTL {}", exact_paragraphs(&fl, &tl), exact_paragraphs(&fr, &tr), blocks::detect(&fl, &[]).len(), blocks::detect(&fr, &[]).len());
    }
    // the two failure shapes, one assertion each
    let (fl, tl) = rtl_page(5, 5, 0.7, false);
    let (fr, tr) = rtl_page(5, 5, 0.7, true);
    assert_eq!(exact_paragraphs(&fl, &tl), 5, "the LTR layout is right");
    if std::env::var("V4_REPRO_EXPECT_FAIL").map(|v| v == "1").unwrap_or(false) {
        return;
    }
    assert_eq!(exact_paragraphs(&fr, &tr), 5, "the mirror image of a layout the detector gets right: the short last line of each RTL paragraph is cut off as an 'indent'");
    let (fl0, tl0) = rtl_page(5, 5, 0.0, false);
    let (fr0, tr0) = rtl_page(5, 5, 0.0, true);
    assert_eq!(exact_paragraphs(&fl0, &tl0), 5);
    assert_eq!(exact_paragraphs(&fr0, &tr0), 5, "with no extra spacing, RTL paragraphs end on a short LEFT edge and are merged");
}

// ------------------------------------------------------------------------------------------------
// a rule-less quotation table (geometry measured from a real Chrome-made PDF): cells of one row merge
// ------------------------------------------------------------------------------------------------

fn quotation_table() -> (Vec<Frag>, usize, usize, usize, usize) {
    let mut v: Vec<Frag> = Vec::new();
    let mut id = 0usize;
    let mut add = |v: &mut Vec<Frag>, l: f32, r: f32, base: f32, text: &str| -> usize {
        v.push(f(id, l, r, base, 10.0, text));
        id += 1;
        id - 1
    };
    // header
    add(&mut v, 63.0, 80.0, 191.0, "No. ");
    add(&mut v, 90.0, 144.0, 191.0, "Description ");
    add(&mut v, 406.0, 422.0, 191.0, "Qty ");
    add(&mut v, 454.0, 476.0, 191.0, "Unit");
    add(&mut v, 494.0, 533.0, 191.0, "Amount");
    add(&mut v, 454.0, 478.0, 203.0, "price");
    // row 1: the description wraps to two lines and runs close to the Qty column
    add(&mut v, 64.0, 66.0, 225.0, "1 ");
    let desc = add(&mut v, 90.5, 377.6, 225.0, "Recessed LED downlight 18 W, 3000 K, CRI 90, IP44, white trim,");
    let qty = add(&mut v, 406.1, 422.0, 225.0, "240 ");
    let price = add(&mut v, 452.4, 477.0, 225.0, "85.00 ");
    let amount = add(&mut v, 494.0, 532.0, 225.0, "20,400.00");
    add(&mut v, 90.5, 158.9, 238.5, "cut-out 150 mm");
    // rows 2..5: one-line descriptions, numbers right aligned in the same columns
    for (i, (d_right, q, p, a)) in [(311.0f32, "240 ", "32.50 ", "7,800.00"), (243.0, "36 ", "64.00 ", "2,304.00"), (234.0, "5 ", "450.00 ", "2,250.00"), (198.0, "1 ", "600.00 ", "600.00")].iter().enumerate() {
        let base = 258.5 + 20.5 * i as f32;
        add(&mut v, 63.0, 68.0, base, &format!("{} ", i + 2));
        add(&mut v, 91.0, *d_right, base, "a one line description of the item ");
        add(&mut v, 406.0 + (16.0 - 5.0 * q.trim().len() as f32).max(0.0), 422.0, base, q);
        add(&mut v, 477.0 - 5.0 * p.trim().len() as f32, 477.0, base, p);
        add(&mut v, 532.0 - 5.0 * a.trim().len() as f32, 532.0, base, a);
    }
    (v, desc, qty, price, amount)
}

#[test]
#[ignore = "V4 review: run with --ignored (see the file header)"]
fn v4_repro_table_cells_with_a_1_to_4_em_gap_merge_into_one_editor_line() {
    let (frags, desc, qty, price, amount) = quotation_table();
    let b = blocks::detect(&frags, &[]);
    let line_of = |o: usize| -> Vec<usize> {
        let bl = b.iter().find(|bl| bl.contains(o)).unwrap();
        bl.lines.iter().find(|l| l.objects.contains(&o)).unwrap().objects.clone()
    };
    eprintln!("description (gap to Qty {:.1} em) shares its line with: {:?}", (406.1 - 377.6) / 10.0, line_of(desc));
    eprintln!("unit price (gap to Amount {:.1} em) shares its line with: {:?}", (494.0 - 477.0) / 10.0, line_of(price));
    eprintln!("qty line: {:?}, amount line: {:?}", line_of(qty), line_of(amount));
    if std::env::var("V4_REPRO_EXPECT_FAIL").map(|v| v == "1").unwrap_or(false) {
        return;
    }
    assert_eq!(line_of(desc), vec![desc], "the description cell must not share an editor line with the Qty cell");
    assert_eq!(line_of(price), vec![price], "the unit price cell must not share an editor line with the Amount cell");
}

}

mod perf {
    use super::v4gen::*;

use pagify_shell::block_input::*;
use pagify_shell::blocks::{self, Frag, Shape};
use pdf_core::document::{Color, DrawnObject, Point, Rect, RunStyle, TextRun};
use std::collections::HashMap;
use std::time::Instant;

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

/// Merge `k` generated pages in a grid `across` pages wide (ids and positions offset).
fn tiled(k: usize, across: usize, seed: u64) -> GenPage {
    let mut out = GenPage::new();
    for i in 0..k {
        let o = PageOpts { weird: false, ..opts_for(seed + i as u64) };
        let pg = gen_page(seed + i as u64, &o);
        let (tx, ty) = ((i % across) as f32 * 612.0, (i / across) as f32 * 792.0);
        let off = out.next;
        for t in &pg.runs {
            let mut t = scale_run(t, 1.0, tx, ty);
            t.object += off;
            out.runs.push(t);
        }
        for (k2, v) in &pg.styles {
            out.styles.insert(k2 + off, *v);
        }
        for (k2, v) in &pg.faces {
            out.faces.insert(k2 + off, v.clone());
        }
        for d in &pg.shapes {
            let mut d = scale_drawn(d, 1.0, tx, ty);
            d.object += off;
            out.shapes.push(d);
        }
        out.next += pg.next;
    }
    out
}

struct Row {
    name: String,
    n: usize,
    adapt: f64,
    detect: f64,
    guard: f64,
    blocks: usize,
    refused: usize,
}

fn measure(name: &str, runs: Vec<TextRun>, styles: HashMap<usize, RunStyle>, faces: HashMap<usize, String>, shapes: Vec<DrawnObject>, guard: bool) -> Row {
    let n = runs.len();
    let t = Instant::now();
    let ad = adapt(&runs, &styles, &faces, &shapes);
    let adapt_ms = ms(t);
    let t = Instant::now();
    let bl = blocks::detect(&ad.frags, &ad.shapes);
    let detect_ms = ms(t);
    let (mut guard_ms, mut refused) = (0.0, 0);
    if guard {
        let pb = build_page_blocks_from(0, 0, 0, runs, styles, faces, shapes);
        let t = Instant::now();
        for bi in 0..pb.blocks.len() {
            if check_editor_invariants(&pb, bi).is_err() {
                refused += 1;
            }
        }
        guard_ms = ms(t);
    }
    Row { name: name.to_string(), n, adapt: adapt_ms, detect: detect_ms, guard: guard_ms, blocks: bl.len(), refused }
}

fn print(rows: &[Row]) {
    eprintln!("{:<44} {:>8} {:>10} {:>10} {:>10} {:>8} {:>8}", "scenario", "frags", "adapt ms", "detect ms", "guard ms", "blocks", "refused");
    for r in rows {
        eprintln!("{:<44} {:>8} {:>10.1} {:>10.1} {:>10.1} {:>8} {:>8}", r.name, r.n, r.adapt, r.detect, r.guard, r.blocks, r.refused);
    }
}

fn pg_measure(name: &str, pg: GenPage, guard: bool) -> Row {
    measure(name, pg.runs, pg.styles, pg.faces, pg.shapes, guard)
}

fn plain_run(id: usize, text: &str, x: f32, base: f32, size: f32) -> TextRun {
    TextRun { object: id, text: text.to_string(), rect: Rect { left: x, top: base - 0.8 * size, right: x + 0.55 * size * text.chars().count() as f32, bottom: base + 0.22 * size }, origin: Point { x: x - 0.05 * size, y: base }, size, color: Color { r: 0, g: 0, b: 0, a: 255 } }
}

fn style_for(runs: &[TextRun]) -> HashMap<usize, RunStyle> {
    runs.iter().map(|r| (r.object, RunStyle { font: 0, stem_milli_em: Some(74), axis: (1.0, 0.0) })).collect()
}

#[test]
#[ignore = "V4 review: run with --ignored (see the file header)"]
fn v4_perf_growth_realistic_and_adversarial() {
    let mut rows: Vec<Row> = Vec::new();
    // A. realistic pages tiled in a grid: rows run through many tiles (page-wide rows)
    for (k, across) in [(2usize, 2usize), (18, 6), (180, 14)] {
        let pg = tiled(k, across, 1000);
        let n = pg.runs.len();
        rows.push(pg_measure(&format!("A tiled realistic pages ({k} tiles, {across} across)"), pg, n <= 20_000));
    }
    // B. one tall column of text (a long document as one page)
    for k in [10usize, 100, 1000] {
        let o = PageOpts { columns: 1, weird: false, height: 792.0 * k as f32 as f64, ..PageOpts::default() };
        let pg = gen_page(7, &o);
        let n = pg.runs.len();
        rows.push(pg_measure(&format!("B one tall column (height {k} pages)"), pg, n <= 20_000));
    }
    // C. one single row of N tiny fragments
    for n in [10_000usize, 100_000] {
        let runs: Vec<TextRun> = (0..n).map(|i| plain_run(i, "ab", i as f32 * 12.0, 100.0, 9.0)).collect();
        let st = style_for(&runs);
        rows.push(measure(&format!("C single row of {n}"), runs, st, HashMap::new(), vec![], n <= 20_000));
    }
    // D. a grid table without rules, R x 20
    for rn in [500usize, 5000] {
        let mut runs = Vec::new();
        for r in 0..rn {
            for c in 0..20 {
                runs.push(plain_run(r * 20 + c, "123.4", 20.0 + c as f32 * 40.0, 20.0 + r as f32 * 12.0, 9.0));
            }
        }
        let n = runs.len();
        let st = style_for(&runs);
        rows.push(measure(&format!("D rule-less table {rn} rows x 20"), runs, st, HashMap::new(), vec![], n <= 20_000));
    }
    // E. dense small text on a few rows plus ONE giant heading (attach_scripts window = 0.65 x min(max size, 4 body))
    for n in [10_000usize, 100_000] {
        let mut runs: Vec<TextRun> = (0..n).map(|i| plain_run(i + 1, "ab", (i % 5000) as f32 * 4.0, 100.0 + (i / 5000) as f32 * 11.0, 9.0)).collect();
        runs.push(plain_run(0, "TITLE", 10.0, 60.0, 72.0));
        let st = style_for(&runs);
        rows.push(measure(&format!("E dense rows + one 72pt title, {n}"), runs, st, HashMap::new(), vec![], false));
    }
    // F. N text objects and N word-sized paths (outlined-word candidates)
    for n in [10_000usize, 50_000] {
        let runs: Vec<TextRun> = (0..n).map(|i| plain_run(i, "word", 20.0 + (i % 40) as f32 * 14.0, 20.0 + (i / 40) as f32 * 12.0, 9.0)).collect();
        let st = style_for(&runs);
        let shapes: Vec<DrawnObject> = (0..n).map(|i| DrawnObject { object: n + i, kind: pdf_core::document::DrawnKind::Shape, rect: Rect { left: 20.0 + (i % 40) as f32 * 14.0 + 3.0, top: 20.0 + (i / 40) as f32 * 12.0 - 6.0, right: 20.0 + (i % 40) as f32 * 14.0 + 20.0, bottom: 20.0 + (i / 40) as f32 * 12.0 + 2.0 }, label: String::new(), depth: 0, opacity: 1.0, movable: true }).collect();
        rows.push(measure(&format!("F {n} text + {n} word-sized paths"), runs, st, HashMap::new(), shapes, false));
    }
    // G. one tall column, a staircase of margins: pairs of lines alternately indented 0 / 20 pt, right edge moving too
    for n in [10_000usize, 100_000] {
        let mut runs = Vec::new();
        for i in 0..n {
            let step = (i / 2) % 2;
            let x = 54.0 + step as f32 * 20.0;
            let mut r = plain_run(i, "some words of a line that is long enough ", x, 20.0 + i as f32 * 11.0, 9.0);
            r.rect.right = x + 300.0 + step as f32 * 20.0;
            runs.push(r);
        }
        let st = style_for(&runs);
        rows.push(measure(&format!("G margin staircase, {n} lines"), runs, st, HashMap::new(), vec![], false));
    }
    // H. adapter: every run at ONE origin with a different box (no twin: boxes differ)
    for n in [5_000usize, 20_000] {
        let runs: Vec<TextRun> = (0..n)
            .map(|i| {
                let mut r = plain_run(i, "x", 100.0, 100.0, 9.0);
                r.rect = Rect { left: 100.0 + i as f32 * 3.0, top: 92.0, right: 104.0 + i as f32 * 3.0, bottom: 102.0 };
                r.origin = Point { x: 100.0, y: 100.0 };
                r
            })
            .collect();
        let st = style_for(&runs);
        rows.push(measure(&format!("H adapter: {n} runs, one origin, different boxes"), runs, st, HashMap::new(), vec![], false));
    }
    // I. one block of N lines all starting at the same x: C4's sweep inside the guard
    for n in [2_000usize, 20_000] {
        let runs: Vec<TextRun> = (0..n).map(|i| plain_run(i, "a justified line of body text that is long enough to fill ", 54.0, 20.0 + i as f32 * 11.0, 9.0)).collect();
        let st = style_for(&runs);
        rows.push(measure(&format!("I one block of {n} lines, guard on"), runs, st, HashMap::new(), vec![], true));
    }
    print(&rows);
}

}
