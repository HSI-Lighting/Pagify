//! Shared helpers of the block-detector tests: the datasheet fixtures, the two labelers' truth, the
//! metrics, and a small layout builder for synthetic pages.
#![allow(dead_code)]

use pagify_shell::blocks::{block_index_of, Block, Frag, Shape};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub fn fixture_path(name: &str) -> String {
    // PAGIFY_FIXTURE_DIR: another set of the same fixtures (the experiments with other stem measurements)
    match std::env::var("PAGIFY_FIXTURE_DIR") {
        Ok(dir) if !dir.is_empty() => format!("{dir}/{name}"),
        _ => format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR")),
    }
}

#[derive(Clone, Debug)]
pub struct Label {
    pub id: String,
    pub kind: String,
    pub objects: Vec<usize>,
    pub alt: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ProtoBlock {
    pub why: String,
    pub objects: Vec<usize>,
    pub opaque: Vec<usize>,
}

pub struct Page {
    pub page: u32,
    pub frags: Vec<Frag>,
    pub shapes: Vec<Shape>,
    /// labelers "A" and "B"
    pub labels: Vec<(&'static str, Vec<Label>)>,
    pub proto: Vec<ProtoBlock>,
}

fn f32_at(v: &Value, i: usize) -> f32 {
    v[i].as_f64().expect("number") as f32
}

pub fn load_page(n: u32) -> Page {
    let text = std::fs::read_to_string(fixture_path(&format!("datasheet_p{n}.json"))).expect("fixture");
    let doc: Value = serde_json::from_str(&text).expect("json");
    let frags = doc["frags"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| {
            let hex = a[10].as_str().unwrap();
            let c = u32::from_str_radix(hex, 16).unwrap();
            Frag {
                object: a[0].as_u64().unwrap() as usize,
                left: f32_at(a, 1),
                top: f32_at(a, 2),
                right: f32_at(a, 3),
                bottom: f32_at(a, 4),
                baseline: f32_at(a, 5),
                size: f32_at(a, 6),
                font: a[7].as_u64().unwrap() as u32,
                stem: a[8].as_u64().map(|s| s as u16),
                face: a[9].as_str().unwrap().to_string(),
                rgb: [(c >> 16) as u8, (c >> 8) as u8, c as u8],
                text: a[11].as_str().unwrap().to_string(),
                rotated: a[12].as_u64().unwrap() != 0,
            }
        })
        .collect();
    let shapes = doc["shapes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| Shape { object: a[0].as_u64().unwrap() as usize, left: f32_at(a, 1), top: f32_at(a, 2), right: f32_at(a, 3), bottom: f32_at(a, 4), depth: a[5].as_u64().unwrap() as u32 })
        .collect();
    let mut labels = Vec::new();
    for who in ["A", "B"] {
        let v: Vec<Label> = doc["labels"][who]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| Label {
                id: b["id"].as_str().unwrap().to_string(),
                kind: b["kind"].as_str().unwrap().to_string(),
                objects: b["objects"].as_array().unwrap().iter().map(|o| o.as_u64().unwrap() as usize).collect(),
                alt: b["alt"].as_str().map(|s| s.to_string()),
            })
            .collect();
        labels.push((who, v));
    }
    let proto = doc["proto"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| ProtoBlock {
            why: b["why"].as_str().unwrap().to_string(),
            objects: b["objects"].as_array().unwrap().iter().map(|o| o.as_u64().unwrap() as usize).collect(),
            opaque: b["opaque"].as_array().unwrap().iter().map(|o| o.as_u64().unwrap() as usize).collect(),
        })
        .collect();
    Page { page: n, frags, shapes, labels, proto }
}

/// Every text object of the blocks, as sets, restricted to `universe`.
pub fn block_sets(blocks: &[Block], universe: &BTreeSet<usize>) -> Vec<BTreeSet<usize>> {
    blocks.iter().map(|b| b.objects().into_iter().filter(|o| universe.contains(o)).collect()).collect()
}

#[derive(Default, Clone, Debug)]
pub struct Score {
    pub blocks: usize,
    pub exact: usize,
    pub by_kind: BTreeMap<String, (usize, usize)>, // kind -> (exact, total)
    pub seeds: usize,
    pub seeds_exact: usize,
    pub seeds_by_kind: BTreeMap<String, (usize, usize)>,
    pub tp: f64,
    pub fp: f64,
    pub fn_: f64,
    pub fails: Vec<(String, String, usize, usize, String)>, // label id, kind, objects, predicted blocks spanned, text
    pub seed_fails: Vec<(usize, String, String, usize, usize)>, // seed, kind, label id, truth size, predicted size
}

impl Score {
    pub fn f1(&self) -> f64 {
        let p = self.tp / (self.tp + self.fp).max(1.0);
        let r = self.tp / (self.tp + self.fn_).max(1.0);
        if p + r == 0.0 {
            0.0
        } else {
            2.0 * p * r / (p + r)
        }
    }
}

fn pairs(n: usize) -> f64 {
    (n * n.saturating_sub(1)) as f64 / 2.0
}

/// Score `blocks` against one labeler's truth, over `universe` (the objects both sides know).
/// Block level: a truth block is exact when all its objects sit in one predicted block and that block
/// holds exactly them. Seed level: for every object, the predicted block's object set equals the truth
/// block's. Pairwise precision/recall as in the research evaluation.
pub fn score(blocks: &[Block], truth: &[Label], universe: &BTreeSet<usize>, texts: &BTreeMap<usize, String>) -> Score {
    let mut s = Score::default();
    let pred_of: BTreeMap<usize, usize> = universe.iter().filter_map(|&o| block_index_of(blocks, o).map(|b| (o, b))).collect();
    let pred_sets = block_sets(blocks, universe);
    let mut truth_of: BTreeMap<usize, usize> = BTreeMap::new();
    let truth_sets: Vec<BTreeSet<usize>> = truth.iter().map(|b| b.objects.iter().copied().filter(|o| universe.contains(o)).collect()).collect();
    for (ti, set) in truth_sets.iter().enumerate() {
        for &o in set {
            truth_of.insert(o, ti);
        }
    }
    let mut joint: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    for &o in universe {
        if let (Some(&p), Some(&t)) = (pred_of.get(&o), truth_of.get(&o)) {
            *joint.entry((p, t)).or_default() += 1;
        }
    }
    s.tp = joint.values().map(|&n| pairs(n)).sum();
    // predicted pairs counted over the labelled universe only
    let mut pred_count: BTreeMap<usize, usize> = BTreeMap::new();
    for (&o, &p) in &pred_of {
        if truth_of.contains_key(&o) {
            *pred_count.entry(p).or_default() += 1;
        }
    }
    let pred_pairs: f64 = pred_count.values().map(|&n| pairs(n)).sum();
    let truth_pairs: f64 = truth_sets.iter().map(|t| pairs(t.len())).sum();
    s.fp = pred_pairs - s.tp;
    s.fn_ = truth_pairs - s.tp;
    for (ti, set) in truth_sets.iter().enumerate() {
        if set.is_empty() {
            continue;
        }
        s.blocks += 1;
        let preds: BTreeSet<usize> = set.iter().filter_map(|o| pred_of.get(o).copied()).collect();
        let ok = preds.len() == 1 && pred_sets[*preds.iter().next().unwrap()] == *set;
        let e = s.by_kind.entry(truth[ti].kind.clone()).or_default();
        e.1 += 1;
        if ok {
            e.0 += 1;
            s.exact += 1;
        } else {
            let first = set.iter().next().copied().unwrap();
            s.fails.push((truth[ti].id.clone(), truth[ti].kind.clone(), set.len(), preds.len(), texts.get(&first).cloned().unwrap_or_default()));
        }
        for &o in set {
            s.seeds += 1;
            let e = s.seeds_by_kind.entry(truth[ti].kind.clone()).or_default();
            e.1 += 1;
            let pset = pred_of.get(&o).map(|&b| &pred_sets[b]);
            if pset == Some(set) {
                e.0 += 1;
                s.seeds_exact += 1;
            } else {
                s.seed_fails.push((o, truth[ti].kind.clone(), truth[ti].id.clone(), set.len(), pset.map_or(0, |x| x.len())));
            }
        }
    }
    s
}

/// The objects a labeler's truth is compared on: every non-rotated text object with text. Blank-text
/// objects (the faux-bold twin layer of "Color Options") are dropped from both sides: the integration
/// layer does not pass them to the detector.
pub fn universe_of(page: &Page) -> BTreeSet<usize> {
    page.frags.iter().filter(|f| !f.rotated && !f.text.trim().is_empty()).map(|f| f.object).collect()
}

pub fn texts_of(page: &Page) -> BTreeMap<usize, String> {
    page.frags.iter().map(|f| (f.object, f.text.clone())).collect()
}

/// Scale every length of a page by `k` (coordinates, sizes, shape sizes).
pub fn scaled(frags: &[Frag], shapes: &[Shape], k: f32) -> (Vec<Frag>, Vec<Shape>) {
    let f = frags
        .iter()
        .map(|f| Frag { left: f.left * k, top: f.top * k, right: f.right * k, bottom: f.bottom * k, baseline: f.baseline * k, size: f.size * k, ..f.clone() })
        .collect();
    let s = shapes.iter().map(|s| Shape { left: s.left * k, top: s.top * k, right: s.right * k, bottom: s.bottom * k, ..*s }).collect();
    (f, s)
}

/// The partition of a page into blocks as a canonical, order-independent value: each block's object
/// list (in output order), the list of blocks sorted.
pub fn canonical(blocks: &[Block]) -> Vec<Vec<usize>> {
    let mut v: Vec<Vec<usize>> = blocks.iter().map(|b| b.objects()).collect();
    v.sort();
    v
}

// ------------------------------------------------------------------------------------------------
// A small deterministic layout builder for synthetic pages
// ------------------------------------------------------------------------------------------------

/// xorshift: deterministic pseudo-random numbers, so a "random" layout is the same on every run.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    /// uniform in [0, 1)
    pub fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    pub fn shuffle<T>(&mut self, v: &mut [T]) {
        for i in (1..v.len()).rev() {
            let j = self.below(i + 1);
            v.swap(i, j);
        }
    }
}

/// A font look: id, measured stem, name, size, colour.
#[derive(Clone, Debug)]
pub struct Look {
    pub font: u32,
    pub stem: Option<u16>,
    pub face: &'static str,
    pub size: f32,
    pub rgb: [u8; 3],
}

pub const LIGHT: Look = Look { font: 1, stem: Some(51), face: "Montserrat-Thin", size: 8.0, rgb: [0x6d, 0x6e, 0x71] };
pub const BOLD: Look = Look { font: 2, stem: Some(198), face: "Montserrat-Thin", size: 8.0, rgb: [0x6d, 0x6e, 0x71] };

#[derive(Default)]
pub struct Layout {
    pub frags: Vec<Frag>,
    pub shapes: Vec<Shape>,
    pub next: usize,
}

impl Layout {
    pub fn new() -> Layout {
        Layout { frags: Vec::new(), shapes: Vec::new(), next: 100 }
    }

    pub fn id(&mut self) -> usize {
        self.next += 1;
        self.next
    }

    /// One text object, `width` points wide, on baseline `base`.
    pub fn frag(&mut self, x: f32, base: f32, width: f32, look: &Look, text: &str) -> usize {
        let object = self.id();
        self.frags.push(Frag {
            object,
            left: x,
            top: base - 0.8 * look.size,
            right: x + width,
            bottom: base + 0.25 * look.size,
            baseline: base,
            size: look.size,
            font: look.font,
            stem: look.stem,
            face: look.face.to_string(),
            rgb: look.rgb,
            text: text.to_string(),
            rotated: false,
        });
        object
    }

    /// A path object.
    pub fn shape(&mut self, l: f32, t: f32, r: f32, b: f32) -> usize {
        let object = self.id();
        self.shapes.push(Shape { object, left: l, top: t, right: r, bottom: b, depth: 0 });
        object
    }

    /// A horizontal rule `x0..x1` at height `y`.
    pub fn hrule(&mut self, x0: f32, x1: f32, y: f32) -> usize {
        self.shape(x0, y - 0.25, x1, y + 0.25)
    }

    /// A vertical rule at `x` from `y0` to `y1`.
    pub fn vrule(&mut self, x: f32, y0: f32, y1: f32) -> usize {
        self.shape(x - 0.25, y0, x + 0.25, y1)
    }

    /// One visual line of `words`, cut into fragments of `per` words. The text fills the width
    /// `x0..x1` exactly when `justified` (the gaps between fragments absorb the slack, as in real
    /// justified text); otherwise it is set ragged with ordinary spaces. Returns the object ids.
    pub fn line(&mut self, x0: f32, x1: f32, base: f32, look: &Look, words: &[&str], per: usize, justified: bool) -> Vec<usize> {
        let em = look.size;
        let char_w = 0.5 * em;
        let groups: Vec<String> = words.chunks(per.max(1)).map(|c| c.join(" ")).collect();
        let widths: Vec<f32> = groups.iter().map(|g| g.chars().count() as f32 * char_w).collect();
        let total: f32 = widths.iter().sum();
        let gaps = groups.len().saturating_sub(1).max(1) as f32;
        let gap = if justified { ((x1 - x0) - total) / gaps } else { 0.3 * em };
        let mut x = x0;
        let mut ids = Vec::new();
        for (g, w) in groups.iter().zip(widths) {
            ids.push(self.frag(x, base, w, look, g));
            x += w + gap;
        }
        ids
    }

    /// A paragraph: `lines` of words, each justified except the last (set ragged, ending at its text),
    /// `pitch` apart, the first baseline at `base`. Returns the object ids of every line.
    pub fn paragraph(&mut self, x0: f32, x1: f32, base: f32, pitch: f32, look: &Look, lines: &[Vec<&str>], per: usize) -> Vec<Vec<usize>> {
        let n = lines.len();
        lines.iter().enumerate().map(|(i, w)| self.line(x0, x1, base + pitch * i as f32, look, w, per, i + 1 < n)).collect()
    }
}

/// Words for synthetic paragraphs: `n` words of a fixed pseudo-random length mix.
pub fn words(rng: &mut Rng, n: usize) -> Vec<&'static str> {
    const W: [&str; 12] = ["light", "of", "the", "downlight", "is", "available", "in", "different", "sizes", "optics", "and", "colour"];
    (0..n).map(|_| W[rng.below(W.len())]).collect()
}

/// Words for a justified line of width `w` at size `em`: random words up to `fill` of the width, then the
/// longest word that still fits closes the line, so the justified gaps stay near an ordinary stretched word
/// space (0.4 to 1.0 em) the way real justified text does. Letters are 0.5 em wide, a natural space 0.25 em.
pub fn fitted_words(rng: &mut Rng, w: f32, em: f32, fill: f32) -> Vec<&'static str> {
    const BY_LENGTH: [&str; 12] = ["downlight", "available", "different", "colour", "optics", "sizes", "light", "and", "the", "of", "in", "is"];
    let wid = |s: &str| (s.chars().count() as f32 * 0.5 + 0.25) * em;
    let mut out: Vec<&'static str> = Vec::new();
    let mut used = 0.0;
    loop {
        let next = words(rng, 1)[0];
        if used + wid(next) > fill * w {
            break;
        }
        used += wid(next);
        out.push(next);
    }
    if let Some(closer) = BY_LENGTH.iter().find(|c| used + wid(c) <= fill * w) {
        out.push(closer);
    }
    out
}

mod synth;
pub use synth::*;

/// What the integration layer hands to `detect`: the page's text objects without the blank-text ones.
pub fn real_frags(page: &Page) -> Vec<Frag> {
    page.frags.iter().filter(|f| !f.text.trim().is_empty()).cloned().collect()
}
