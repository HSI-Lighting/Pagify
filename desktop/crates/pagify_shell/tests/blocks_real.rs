//! Acceptance of the block detector on the user's REAL documents (quotations, purchase orders, invoices,
//! datasheets, a lux report, CAD sheets): 55 pages whose text objects were labelled by hand by two
//! independent labelers and reconciled into one truth (6,386 objects in 4,482 blocks; 86 % of the blocks
//! are one object, 302 are multi-line table cells).
//!
//! The fixtures are the schema of the datasheet fixtures plus extra keys (`labels.final`, `universe`,
//! `ambiguous`, ...). They are written by `real\regen_fixtures.js` in the session scratch (they derive from the
//! owner's private quotations and purchase orders, so they are NOT in the repo); the directory is
//! `PAGIFY_REAL_FIXTURE_DIR` or the scratch default below. When it is absent the tests say so and pass.
//!
//! What the detector is given is exactly what the app gives it: `detect(&adapted.frags, &adapted.shapes)`,
//! the fixture's `frags` and `shapes` (the output of `block_input::adapt`). The metric is the datasheet's:
//! a seed (one text object) is right when the block the detector returns for it holds exactly the objects of
//! its truth block, over the objects the truth knows (`universe`). Objects the adapter admits but nobody
//! labelled (thin glyphs, 0.2-0.5 pt CAD text) are given to the detector and never scored.
//!
//! `cargo test -p pagify_shell --release --test blocks_real -- --nocapture` prints the tables;
//! `PAGIFY_REAL_DUMP=<file>` also writes one JSON line per wrong truth block (page, kind, truth objects, the
//! predicted blocks that touch it) for analysis.
//!
//! MEASURED (2026-10-05, the detector as left by P2a, wave 1; see the numbers in `THRESHOLDS`): see the
//! comments at the assertions.

mod common;

use common::Label;
use pagify_shell::blocks::*;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

const SCRATCH_DEFAULT: &str = r"C:\Users\hsili\AppData\Local\Temp\claude\C--Users-hsili\2f819b04-e4a0-440c-abae-e04c9029a62b\scratchpad\paragraphs\real\fixtures";

fn fixture_dir() -> PathBuf {
    match std::env::var("PAGIFY_REAL_FIXTURE_DIR") {
        Ok(d) if !d.is_empty() => PathBuf::from(d),
        _ => PathBuf::from(SCRATCH_DEFAULT),
    }
}

struct Truth {
    label: Label,
    low: bool,
}

struct RealPage {
    id: String,
    provisional: bool,
    frags: Vec<Frag>,
    shapes: Vec<Shape>,
    universe: BTreeSet<usize>,
    truth: Vec<Truth>,
    /// seed -> the object sets that count as right in the lenient reading: the final groups of the labelers'
    /// recorded ambiguities and each of their alternative partitions
    accept: BTreeMap<usize, BTreeSet<Vec<usize>>>,
}

fn f32_at(v: &Value, i: usize) -> f32 {
    v[i].as_f64().expect("number") as f32
}

fn ids_of(v: &Value) -> Vec<usize> {
    v.as_array().unwrap().iter().map(|o| o.as_u64().unwrap() as usize).collect()
}

fn load(path: &std::path::Path) -> RealPage {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let doc: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).expect("json");
    let frags = doc["frags"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| {
            let c = u32::from_str_radix(a[10].as_str().unwrap(), 16).unwrap();
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
    let universe: BTreeSet<usize> = ids_of(&doc["universe"]).into_iter().collect();
    let truth: Vec<Truth> = doc["labels"]["final"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| Truth {
            label: Label {
                id: b["id"].as_str().unwrap().to_string(),
                kind: b["kind"].as_str().unwrap().to_string(),
                objects: ids_of(&b["objects"]),
                alt: b["alt"].as_str().map(|s| s.to_string()),
            },
            low: b["conf"].as_str() == Some("low"),
        })
        .collect();
    let mut accept: BTreeMap<usize, BTreeSet<Vec<usize>>> = BTreeMap::new();
    let mut note = |group: Vec<usize>| {
        let mut g: Vec<usize> = group.into_iter().filter(|o| universe.contains(o)).collect();
        g.sort();
        for &s in &g {
            accept.entry(s).or_default().insert(g.clone());
        }
    };
    for c in doc["ambiguous"].as_array().map(|v| v.as_slice()).unwrap_or(&[]) {
        for g in c["final_groups"].as_array().unwrap() {
            note(ids_of(g));
        }
        for alt in c["alternatives"].as_array().unwrap() {
            for g in alt["groups"].as_array().unwrap() {
                note(ids_of(g));
            }
        }
    }
    RealPage {
        id: doc["id"].as_str().unwrap().to_string(),
        provisional: doc["labelers"].as_str().unwrap_or("").contains("provisional"),
        frags,
        shapes,
        universe,
        truth,
        accept,
    }
}

/// Every fixture of the directory, sorted by page id; None when the directory is absent (the tests then say so).
fn load_all() -> Option<Vec<RealPage>> {
    let dir = fixture_dir();
    let Ok(rd) = std::fs::read_dir(&dir) else {
        println!("blocks_real: SKIPPED, no real-document fixtures at {} (set PAGIFY_REAL_FIXTURE_DIR; they derive from private documents and are not in the repo)", dir.display());
        return None;
    };
    let mut files: Vec<PathBuf> = rd.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().map_or(false, |x| x == "json") && !p.file_name().unwrap().to_string_lossy().starts_with('_')).collect();
    files.sort();
    if files.is_empty() {
        println!("blocks_real: SKIPPED, no *.json fixtures in {}", dir.display());
        return None;
    }
    Some(files.iter().map(|f| load(f)).collect())
}

// ------------------------------------------------------------------------------------------------
// scoring
// ------------------------------------------------------------------------------------------------

/// What the detector did to one truth block.
struct Outcome {
    page: usize,
    truth: usize,
    /// the distinct predicted blocks (their objects restricted to the universe) that hold an object of the truth block
    preds: Vec<BTreeSet<usize>>,
    exact_seeds: usize,
    lenient_seeds: usize,
}

struct PageRun {
    blocks: Vec<Block>,
    /// object -> index of its predicted block
    pred_of: BTreeMap<usize, usize>,
    /// predicted blocks restricted to the universe
    pred_sets: Vec<BTreeSet<usize>>,
    /// object -> index of its truth block
    truth_of: BTreeMap<usize, usize>,
    truth_sets: Vec<BTreeSet<usize>>,
}

fn run_page(page: &RealPage) -> PageRun {
    let blocks = detect(&page.frags, &page.shapes);
    let pred_sets: Vec<BTreeSet<usize>> = blocks.iter().map(|b| b.objects().into_iter().filter(|o| page.universe.contains(o)).collect()).collect();
    let mut pred_of = BTreeMap::new();
    for (i, s) in pred_sets.iter().enumerate() {
        for &o in s {
            pred_of.insert(o, i);
        }
    }
    let truth_sets: Vec<BTreeSet<usize>> = page.truth.iter().map(|t| t.label.objects.iter().copied().filter(|o| page.universe.contains(o)).collect()).collect();
    let mut truth_of = BTreeMap::new();
    for (i, s) in truth_sets.iter().enumerate() {
        for &o in s {
            truth_of.insert(o, i);
        }
    }
    PageRun { blocks, pred_of, pred_sets, truth_of, truth_sets }
}

fn outcomes(pi: usize, page: &RealPage, run: &PageRun) -> Vec<Outcome> {
    let mut out = Vec::new();
    for (ti, set) in run.truth_sets.iter().enumerate() {
        if set.is_empty() {
            continue;
        }
        let mut preds: Vec<usize> = set.iter().filter_map(|o| run.pred_of.get(o).copied()).collect();
        preds.sort();
        preds.dedup();
        let (mut exact, mut lenient) = (0, 0);
        for &o in set {
            let Some(&p) = run.pred_of.get(&o) else { continue };
            if run.pred_sets[p] == *set {
                exact += 1;
                lenient += 1;
            } else {
                let key: Vec<usize> = run.pred_sets[p].iter().copied().collect();
                if page.accept.get(&o).map_or(false, |a| a.contains(&key)) {
                    lenient += 1;
                }
            }
        }
        out.push(Outcome { page: pi, truth: ti, preds: preds.iter().map(|&p| run.pred_sets[p].clone()).collect(), exact_seeds: exact, lenient_seeds: lenient });
    }
    out
}

/// A predicted block that holds objects of two or more truth blocks, and is not an alternative the labelers recorded.
struct Merge {
    page: usize,
    objects: BTreeSet<usize>,
    kinds: Vec<String>,
    cells: usize,
}

fn merges(pi: usize, page: &RealPage, run: &PageRun) -> Vec<Merge> {
    let mut out = Vec::new();
    for set in &run.pred_sets {
        let touched: BTreeSet<usize> = set.iter().filter_map(|o| run.truth_of.get(o).copied()).collect();
        if touched.len() < 2 {
            continue;
        }
        let key: Vec<usize> = set.iter().copied().collect();
        if set.iter().all(|o| page.accept.get(o).map_or(false, |a| a.contains(&key))) {
            continue; // a reading the labelers themselves recorded as an alternative
        }
        let kinds: Vec<String> = touched.iter().map(|&t| page.truth[t].label.kind.clone()).collect();
        let cells = kinds.iter().filter(|k| *k == "cell").count();
        // blocks of low confidence (tiny artwork marks, hidden text) are not judged
        if touched.iter().all(|&t| page.truth[t].low) {
            continue;
        }
        out.push(Merge { page: pi, objects: set.clone(), kinds, cells });
    }
    out
}

fn text_of(page: &RealPage, objs: &BTreeSet<usize>, n: usize) -> String {
    let mut v: Vec<&Frag> = page.frags.iter().filter(|f| objs.contains(&f.object)).collect();
    v.sort_by(|a, b| a.baseline.total_cmp(&b.baseline).then(a.left.total_cmp(&b.left)));
    let s: Vec<String> = v.iter().map(|f| f.text.trim().replace('\u{2}', "-")).collect();
    s.join(" | ").chars().take(n).collect()
}

#[derive(Default, Clone, Copy)]
struct Tally {
    seeds: usize,
    exact: usize,
    lenient: usize,
    blocks: usize,
    blocks_exact: usize,
}

impl Tally {
    fn add(&mut self, o: &Outcome, n: usize) {
        self.seeds += n;
        self.exact += o.exact_seeds;
        self.lenient += o.lenient_seeds;
        self.blocks += 1;
        self.blocks_exact += (o.exact_seeds == n) as usize;
    }
    fn pct(&self) -> f64 {
        100.0 * self.exact as f64 / self.seeds.max(1) as f64
    }
}

fn is_quotation(id: &str) -> bool {
    id.starts_with("mumuso") || id.starts_with("dq8")
}

/// The family of a page id, for the tables.
fn family(id: &str) -> &'static str {
    if id.starts_with("mumuso") || id.starts_with("dq8") {
        "quotation (MUMUSO, DQ-8)"
    } else if id.starts_with("qt") {
        "quotation (other)"
    } else if id.starts_with("lpo") || id.starts_with("po_") || id.starts_with("pending_lpos") {
        "purchase order"
    } else if id.starts_with("klds") {
        "invoice / packing list"
    } else if id.starts_with("orange") || id.starts_with("kasim") || id.starts_with("linea") || id.starts_with("mean_well") {
        "drawing / artwork"
    } else {
        "other documents"
    }
}

struct Results {
    pages: Vec<RealPage>,
    runs: Vec<PageRun>,
    outcomes: Vec<Outcome>,
}

fn measure() -> Option<Results> {
    let pages = load_all()?;
    let runs: Vec<PageRun> = pages.iter().map(run_page).collect();
    // PAGIFY_REAL_BLOCKS=<dir>: write what the detector returned for every page (analysis scripts read it)
    if let Ok(dir) = std::env::var("PAGIFY_REAL_BLOCKS") {
        if !dir.is_empty() {
            std::fs::create_dir_all(&dir).expect("create PAGIFY_REAL_BLOCKS");
            for (p, run) in pages.iter().zip(&runs) {
                let blocks: Vec<String> = run
                    .blocks
                    .iter()
                    .map(|b| {
                        let lines: Vec<String> = b.lines.iter().map(|l| format!("{{\"objects\":{:?},\"outlined\":{:?},\"l\":{},\"t\":{},\"r\":{},\"b\":{},\"base\":{}}}", l.objects, l.outlined, l.left, l.top, l.right, l.bottom, l.baseline)).collect();
                        format!("{{\"why\":\"{}\",\"lines\":[{}]}}", b.starts_because, lines.join(","))
                    })
                    .collect();
                std::fs::write(std::path::Path::new(&dir).join(format!("{}.json", p.id)), format!("[{}]", blocks.join(","))).expect("write blocks");
            }
        }
    }
    let outcomes: Vec<Outcome> = pages
        .iter()
        .enumerate()
        .flat_map(|(pi, p)| outcomes(pi, p, &runs[pi]))
        .collect();
    Some(Results { pages, runs, outcomes })
}

fn size_of(r: &Results, o: &Outcome) -> usize {
    r.runs[o.page].truth_sets[o.truth].len()
}

fn percent(a: usize, b: usize) -> f64 {
    100.0 * a as f64 / b.max(1) as f64
}

// ------------------------------------------------------------------------------------------------
// tests
// ------------------------------------------------------------------------------------------------

/// The headline numbers: per page, per kind, per family, and the thresholds.
///
/// Measured on 2026-10-05 (release check, detector unchanged since P2a's pass; adapter as of that evening),
/// strict exact per seed, fixtures regenerated from the real PDFs by `regen_adapted.cmd` + `regen_fixtures.js`:
///   quotation pages (MUMUSO x4, DQ-8):  436/496 = 87.90 % (lenient 93.95 %), blocks 228/256
///   all 55 pages:                       5197/6386 = 81.38 % (lenient 83.32 %), blocks 3734/4482
///   two-labeler pages only (37 pages): 4323/5074 = 85.20 %
///   by family: quotation (other) 90.8 %, purchase order 89.2 %, invoice / packing list 89.3 %,
///   drawing / artwork 78.7 %, other documents 66.5 % (the lux report's number grids are the weak spot)
/// What these numbers do NOT say: whether the wrong blocks are harmless. The detector joins 159 blocks that hold
/// objects of two or more truth blocks without being a recorded alternative; 27 of them hold two or more truth
/// CELLS (printed by `no_table_cell_is_merged_with_another_cell`), 131 if a one-object block counts as a cell.
/// Stacked ones (each line holds one truth block) are edited line by line and lose nothing; the damaging ones put
/// cells side by side in one line, and on the quotation, purchase-order and invoice pages the app's guard C17
/// (`block_input.rs`) refuses every one of them. `PAGIFY_REAL_BLOCKS=<dir>` dumps the blocks for that analysis.
#[test]
fn real_pages_exact_per_seed_by_page_kind_and_family() {
    let Some(r) = measure() else { return };
    // the same metric as the datasheet's `common::score`: cross-check it on every page
    for (pi, p) in r.pages.iter().enumerate() {
        let truth: Vec<Label> = p.truth.iter().map(|t| t.label.clone()).collect();
        let texts: BTreeMap<usize, String> = p.frags.iter().map(|f| (f.object, f.text.clone())).collect();
        let s = common::score(&r.runs[pi].blocks, &truth, &p.universe, &texts);
        let mine: usize = r.outcomes.iter().filter(|o| o.page == pi).map(|o| o.exact_seeds).sum();
        assert_eq!(s.seeds_exact, mine, "{}: this file's scoring must equal common::score", p.id);
    }
    let mut by_page: BTreeMap<usize, Tally> = BTreeMap::new();
    let mut by_kind: BTreeMap<String, Tally> = BTreeMap::new();
    let mut by_family: BTreeMap<&'static str, Tally> = BTreeMap::new();
    let (mut all, mut all_final, mut quotation) = (Tally::default(), Tally::default(), Tally::default());
    for o in &r.outcomes {
        let n = size_of(&r, o);
        let p = &r.pages[o.page];
        by_page.entry(o.page).or_default().add(o, n);
        by_kind.entry(p.truth[o.truth].label.kind.clone()).or_default().add(o, n);
        by_family.entry(family(&p.id)).or_default().add(o, n);
        all.add(o, n);
        if !p.provisional {
            all_final.add(o, n);
        }
        if is_quotation(&p.id) {
            quotation.add(o, n);
        }
    }
    println!("\nper page (strict seeds exact / seeds, lenient, blocks exact / blocks):");
    for (pi, t) in &by_page {
        let p = &r.pages[*pi];
        println!("  {:34} {:5}/{:5} {:5.1}%  lenient {:5.1}%  blocks {:4}/{:4}{}", p.id, t.exact, t.seeds, t.pct(), percent(t.lenient, t.seeds), t.blocks_exact, t.blocks, if p.provisional { "  (one labeler)" } else { "" });
    }
    println!("\nper kind:");
    for (k, t) in &by_kind {
        println!("  {k:10} {:5}/{:5} {:5.1}%  lenient {:5.1}%  blocks {:4}/{:4}", t.exact, t.seeds, t.pct(), percent(t.lenient, t.seeds), t.blocks_exact, t.blocks);
    }
    println!("\nper family:");
    for (k, t) in &by_family {
        println!("  {k:26} {:5}/{:5} {:5.1}%  lenient {:5.1}%  blocks {:4}/{:4}", t.exact, t.seeds, t.pct(), percent(t.lenient, t.seeds), t.blocks_exact, t.blocks);
    }
    println!(
        "\nTOTAL all pages {}/{} ({:.2}%) lenient {:.2}%, blocks {}/{}; two-labeler pages only {}/{} ({:.2}%); quotation pages (MUMUSO, DQ-8) {}/{} ({:.2}%) lenient {:.2}%",
        all.exact,
        all.seeds,
        all.pct(),
        percent(all.lenient, all.seeds),
        all.blocks_exact,
        all.blocks,
        all_final.exact,
        all_final.seeds,
        all_final.pct(),
        quotation.exact,
        quotation.seeds,
        quotation.pct(),
        percent(quotation.lenient, quotation.seeds)
    );
    assert!(quotation.seeds > 0, "the quotation pages must be among the fixtures");
}

/// How the wrong seeds went wrong, and where: merged beside (same baseline) or stacked, or split.
#[test]
fn real_pages_failures_by_type() {
    let Some(r) = measure() else { return };
    // type -> (truth blocks, seeds)
    let mut kinds: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut rows: Vec<(usize, String, String, String, String)> = Vec::new();
    let mut dump = String::new();
    for o in &r.outcomes {
        let n = size_of(&r, o);
        if o.exact_seeds == n {
            continue;
        }
        let p = &r.pages[o.page];
        let t = &r.runs[o.page].truth_sets[o.truth];
        let ty = failure_type(p, t, &o.preds);
        let e = kinds.entry(format!("{ty} / {}", family(&p.id))).or_default();
        e.0 += 1;
        e.1 += n - o.exact_seeds;
        rows.push((n - o.exact_seeds, p.id.clone(), p.truth[o.truth].label.kind.clone(), ty.to_string(), text_of(p, t, 50)));
        dump.push_str(&format!(
            "{{\"page\":\"{}\",\"id\":\"{}\",\"kind\":\"{}\",\"alt\":{},\"low\":{},\"type\":\"{}\",\"truth\":{:?},\"preds\":{:?}}}\n",
            p.id,
            p.truth[o.truth].label.id,
            p.truth[o.truth].label.kind,
            p.truth[o.truth].label.alt.is_some(),
            p.truth[o.truth].low,
            ty,
            t.iter().collect::<Vec<_>>(),
            o.preds.iter().map(|s| s.iter().collect::<Vec<_>>()).collect::<Vec<_>>()
        ));
    }
    if let Ok(path) = std::env::var("PAGIFY_REAL_DUMP") {
        if !path.is_empty() {
            std::fs::write(&path, dump).expect("write PAGIFY_REAL_DUMP");
        }
    }
    println!("\nwrong truth blocks by failure type and family (blocks, wrong seeds):");
    for (k, (b, s)) in &kinds {
        println!("  {k:60} {b:4} blocks {s:5} seeds");
    }
    rows.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    println!("\nthe 40 wrong truth blocks with the most wrong seeds (wrong seeds, page, kind, type, text):");
    for w in rows.iter().take(40) {
        println!("  {:4} {:30} {:8} {:16} '{}'", w.0, w.1, w.2, w.3, w.4);
    }
}

/// merged beside (the extra objects share a baseline with the truth block), merged stacked, merged both ways, split, or mixed.
fn failure_type(page: &RealPage, truth: &BTreeSet<usize>, preds: &[BTreeSet<usize>]) -> &'static str {
    let frag = |o: &usize| page.frags.iter().find(|f| f.object == *o);
    if preds.len() > 1 && preds.iter().all(|p| p.is_subset(truth)) {
        return "split";
    }
    if preds.len() == 1 && truth.is_subset(&preds[0]) {
        let extra: Vec<&Frag> = preds[0].difference(truth).filter_map(frag).collect();
        let tf: Vec<&Frag> = truth.iter().filter_map(frag).collect();
        let beside = |e: &Frag| tf.iter().any(|t| (t.baseline - e.baseline).abs() <= 0.3 * t.size.min(e.size));
        let (b, s) = (extra.iter().filter(|e| beside(e)).count(), extra.iter().filter(|e| !beside(e)).count());
        return match (b > 0, s > 0) {
            (true, false) => "merged beside",
            (false, true) => "merged stacked",
            _ => "merged both",
        };
    }
    "mixed"
}

/// (i) No cell of the truth is ever merged with another cell: the damage the owner cannot accept (applying an
/// edit to a merged block collapses the neighbouring cells into the first one). Every violation is listed with
/// the page id and the object ids.
#[test]
fn no_table_cell_is_merged_with_another_cell() {
    let Some(r) = measure() else { return };
    let mut all: Vec<Merge> = Vec::new();
    for (pi, p) in r.pages.iter().enumerate() {
        all.extend(merges(pi, p, &r.runs[pi]));
    }
    let cell_cell: Vec<&Merge> = all.iter().filter(|m| m.cells >= 2).collect();
    let cell_other: Vec<&Merge> = all.iter().filter(|m| m.cells == 1).collect();
    println!("\nmerged blocks that are no recorded alternative: {} in all; {} hold two or more truth CELLS; {} hold one cell and other blocks", all.len(), cell_cell.len(), cell_other.len());
    let mut per_page: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for m in &all {
        let e = per_page.entry(r.pages[m.page].id.as_str()).or_default();
        if m.cells >= 2 {
            e.0 += 1;
        } else if m.cells == 1 {
            e.1 += 1;
        }
    }
    for (id, (a, b)) in &per_page {
        if a + b > 0 {
            println!("  {id:34} cell+cell {a:3}   cell+other {b:3}");
        }
    }
    for m in cell_cell.iter() {
        let p = &r.pages[m.page];
        println!("  VIOLATION {} objects {:?} kinds {:?}: '{}'", p.id, m.objects.iter().collect::<Vec<_>>(), m.kinds, text_of(p, &m.objects, 90));
    }
    println!("  (cell + other merges, first 60):");
    for m in cell_other.iter().take(60) {
        let p = &r.pages[m.page];
        println!("  merge {} objects {:?} kinds {:?}: '{}'", p.id, m.objects.iter().collect::<Vec<_>>(), m.kinds, text_of(p, &m.objects, 90));
    }
}
