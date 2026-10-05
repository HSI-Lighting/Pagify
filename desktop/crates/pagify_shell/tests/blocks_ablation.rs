//! Which rule earns its keep, and how much the numbers matter: the datasheet metrics with every switch
//! turned off in turn (the mutation table of the detector: an acceptance number that does not move when a
//! rule is switched off proves nothing about that rule), and with every tunable swept over a range.
//!
//! `cargo test -p pagify_shell --release --test blocks_ablation -- --nocapture` prints both tables.

mod common;

use common::*;
use pagify_shell::blocks::*;

#[derive(Clone, Copy, Debug, PartialEq)]
struct M {
    paragraphs: usize, // of 108 (18 per page, two labelers)
    headings: usize,   // of 102
    blocks: usize,     // of 656
    seeds: usize,      // of 6320
    f1: f64,
}

fn measure(p: &Params) -> M {
    let mut m = M { paragraphs: 0, headings: 0, blocks: 0, seeds: 0, f1: 0.0 };
    let mut n = 0.0;
    for pg in 1..=3 {
        let page = load_page(pg);
        let blocks = detect_with(&real_frags(&page), &page.shapes, p);
        let universe = universe_of(&page);
        let texts = texts_of(&page);
        for (_who, truth) in &page.labels {
            let s = score(&blocks, truth, &universe, &texts);
            m.paragraphs += s.by_kind.get("paragraph").map_or(0, |k| k.0);
            m.headings += s.by_kind.get("heading").map_or(0, |k| k.0);
            m.blocks += s.exact;
            m.seeds += s.seeds_exact;
            m.f1 += s.f1();
            n += 1.0;
        }
    }
    m.f1 /= n;
    m
}

fn row(name: &str, m: &M) -> String {
    format!("{name:34} paragraphs {:3}/108  headings {:3}/102  blocks {:3}/656  seeds {:4}/6320  F1 {:.4}", m.paragraphs, m.headings, m.blocks, m.seeds, m.f1)
}

type Switch = (&'static str, fn(&mut Params));

const SWITCHES: &[Switch] = &[
    ("corridor (pure threshold 1.25 em)", |p| p.corridor = false),
    ("outlined words", |p| p.outlined = false),
    ("rules (vertical cuts, ruled links)", |p| p.rules = false),
    ("boxes", |p| p.boxes = false),
    ("size rule", |p| p.size_rule = false),
    ("style rule", |p| p.style_rule = false),
    ("pitch rule", |p| p.pitch_rule = false),
    ("list items", |p| p.list_items = false),
    ("indent rule", |p| p.indent_rule = false),
    ("hanging entries", |p| p.hanging_items = false),
    ("margin rule (block quotes)", |p| p.margin_rule = false),
    ("short-line rule", |p| p.short_lines = false),
    ("script marks", |p| p.script_marks = false),
    ("marker merge", |p| p.marker_merge = false),
    ("stray marks", |p| p.stray_marks = false),
    ("ruled bands and cells", |p| p.ruled_bands = false),
    ("unknown stem + same face = same", |p| p.unknown_stem_same_face = false),
    ("outlined coverage test", |p| p.cover_max = 10.0),
    ("outlined: text inside = cell", |p| p.cell_inside = 10.0),
    ("outlined: touch 12 em (the old reach)", |p| p.outlined_touch = 12.0),
    ("outlined: overlapping paths = art", |p| p.art_overlap = 10.0),
    ("sparse gaps (2 em, no flush row)", |p| p.sparse_gap = 1000.0),
    ("one-row cells (header bars)", |p| p.lone_cell_gap = 1000.0),
    ("mixed looks (no look in common)", |p| p.mixed_looks = false),
    ("tiny lines (a label over a long line)", |p| p.tiny_line = 0),
    ("stem tolerance 0 (stems must be equal)", |p| p.stem_tol = 0),
    ("stem tolerance 30 (Light = Regular)", |p| p.stem_tol = 30),
];

#[test]
fn the_default_numbers() {
    let m = measure(&Params::default());
    println!("{}", row("default", &m));
    // 599 before the chart rows (every tick label and legend title of the wattage chart a block of its own: the
    // sparse-gap and tiny-line rules) and the boxes without a size limit (the plot frame holds "35°" but not
    // "30°"): 611 now
    assert_eq!((m.paragraphs, m.headings, m.blocks), (108, 96, 611));
    assert!(m.seeds >= 6062 && m.f1 >= 0.994);
}

#[test]
fn every_switch_that_matters_on_the_datasheet_makes_it_worse() {
    let base = measure(&Params::default());
    println!("{}", row("default", &base));
    let mut moved = Vec::new();
    for (name, f) in SWITCHES {
        let mut p = Params::default();
        f(&mut p);
        let m = measure(&p);
        let verdict = if m == base { "no effect on the datasheet" } else if m.blocks > base.blocks || m.seeds > base.seeds { "BETTER here, see the notes" } else { "worse" };
        println!("{}  -> {verdict}", row(name, &m));
        if m != base {
            moved.push(*name);
        }
    }
    // the rules the datasheet exercises: each must change the numbers when it is off ("ruled bands and
    // cells" does not here: with the stems of the TT resources only, the value cells of page 2 are one
    // style with their labels anyway; with every font handle's measured stem, where the substitute font
    // of those cells measures 20 against 51, the rule is worth four blocks)
    for must in [
        "corridor (pure threshold 1.25 em)",
        "outlined words",
        "rules (vertical cuts, ruled links)",
        "style rule",
        "pitch rule",
        "short-line rule",
        "boxes",
    ] {
        assert!(moved.contains(&must), "switching off '{must}' left every datasheet number unchanged");
    }
    // and the headline ones must cost paragraphs, not only boxes
    for (must, name) in [(true, "corridor (pure threshold 1.25 em)"), (true, "outlined words"), (true, "short-line rule"), (true, "style rule")] {
        let (_, f) = SWITCHES.iter().find(|s| s.0 == name).unwrap();
        let mut p = Params::default();
        f(&mut p);
        let m = measure(&p);
        assert!(must && m.paragraphs < base.paragraphs, "switching off '{name}' must cost paragraphs, got {}", m.paragraphs);
    }
}

/// One tunable at a time over a range, the others at their defaults: the numbers the shipped values sit in.
#[test]
fn sensitivity_of_every_tunable() {
    type Set = fn(&mut Params, f32);
    let sweeps: Vec<(&str, Vec<f32>, Set)> = vec![
        ("row_tol", vec![0.15, 0.22, 0.28, 0.35, 0.45], |p, v| p.row_tol = v),
        ("word_gap", vec![0.5, 0.65, 0.8, 1.0, 1.2], |p, v| p.word_gap = v),
        ("max_hole", vec![2.5, 3.0, 4.0, 6.0, 10.0], |p, v| p.max_hole = v),
        ("corridor_width", vec![0.5, 0.65, 0.8, 1.0, 1.4], |p, v| p.corridor_width = v),
        ("pos3", vec![2.0, 3.0, 4.0, 5.0], |p, v| p.pos3 = v as usize),
        ("pos2_gap", vec![0.9, 1.2, 1.5], |p, v| p.pos2_gap = v),
        ("pos1_gap", vec![1.5, 2.5, 3.5], |p, v| p.pos1_gap = v),
        ("iso_gap", vec![1.5, 2.5, 4.0], |p, v| p.iso_gap = v),
        ("near", vec![1.5, 2.0, 3.0, 4.0, 6.0], |p, v| p.near = v),
        ("scan_window", vec![4.0, 8.0, 16.0], |p, v| p.scan_window = v),
        ("link_overlap", vec![0.2, 0.4, 0.6, 0.8], |p, v| p.link_overlap = v),
        ("pitch_extra", vec![0.1, 0.2, 0.3, 0.45, 0.7, 1.0], |p, v| p.pitch_extra = v),
        ("indent", vec![0.5, 0.9, 1.5, 2.5], |p, v| p.indent = v),
        ("short_margin", vec![0.2, 0.7, 1.5, 2.5], |p, v| p.short_margin = v),
        ("short_slack", vec![0.25, 0.5, 0.75, 1.0], |p, v| p.short_slack = v),
        ("size_break", vec![0.05, 0.1, 0.15, 0.3], |p, v| p.size_break = v),
        ("size_rel", vec![0.02, 0.05, 0.1, 0.2], |p, v| p.size_rel = v),
        ("pure_share", vec![0.6, 0.75, 0.85, 0.95], |p, v| p.pure_share = v),
        ("colour_delta", vec![10.0, 25.0, 40.0, 80.0], |p, v| p.colour_delta = v as u8),
        ("stem_tol", vec![0.0, 5.0, 12.0, 20.0, 22.0, 30.0], |p, v| p.stem_tol = v as u16),
        ("edge_tol", vec![0.06, 0.125, 0.25, 0.5], |p, v| p.edge_tol = v),
        ("edge_dominance", vec![0.3, 0.5, 0.7], |p, v| p.edge_dominance = v),
        ("edge_lines", vec![2.0, 3.0, 4.0, 5.0], |p, v| p.edge_lines = v as usize),
        ("justify_evidence", vec![3.0, 4.0, 6.0, 8.0, 12.0, 16.0, 24.0], |p, v| p.justify_evidence = v as usize),
        ("underline_zone", vec![0.0, 0.1, 0.25, 0.35, 0.6], |p, v| p.underline_zone = v),
        ("underline_cover", vec![0.6, 0.75, 0.9, 1.0], |p, v| p.underline_cover = v),
        ("rule_cover", vec![0.3, 0.45, 0.6, 0.8, 0.95], |p, v| p.rule_cover = v),
        ("margin_shift", vec![0.5, 0.9, 2.0], |p, v| p.margin_shift = v),
        ("script_gap", vec![0.15, 0.35, 0.6], |p, v| p.script_gap = v),
        ("script_shift", vec![0.4, 0.65, 0.9], |p, v| p.script_shift = v),
        ("marker_gap", vec![3.0, 6.0, 12.0], |p, v| p.marker_gap = v),
        ("band_below", vec![0.8, 1.2, 2.0], |p, v| p.band_below = v),
        ("band_above", vec![1.6, 2.4, 3.5], |p, v| p.band_above = v),
        ("cover_max", vec![0.1, 0.3, 0.6, 0.9], |p, v| p.cover_max = v),
        ("cell_inside", vec![0.25, 0.5, 0.75, 1.0], |p, v| p.cell_inside = v),
        ("outlined_touch", vec![0.8, 1.2, 1.5, 2.5, 4.0, 12.0], |p, v| p.outlined_touch = v),
        ("art_overlap", vec![0.1, 0.25, 0.5, 0.75, 1.0], |p, v| p.art_overlap = v),
        ("sparse_gap", vec![1.5, 1.8, 2.0, 2.5, 3.0, 4.0], |p, v| p.sparse_gap = v),
        ("flush_tol", vec![0.25, 0.5, 1.0, 2.0, 4.0], |p, v| p.flush_tol = v),
        ("lone_cell_gap", vec![0.8, 1.2, 1.5, 2.0, 3.0], |p, v| p.lone_cell_gap = v),
        ("lone_cell_h", vec![1.8, 2.2, 2.6, 3.2, 4.0], |p, v| p.lone_cell_h = v),
        ("tiny_line", vec![0.0, 2.0, 4.0, 6.0, 12.0], |p, v| p.tiny_line = v as usize),
        ("word_h_min", vec![0.5, 0.625, 0.7, 0.75, 0.8], |p, v| p.word_h.0 = v),
    ];
    let base = measure(&Params::default());
    let mut cliffs = Vec::new();
    for (name, values, set) in &sweeps {
        for &v in values {
            let mut p = Params::default();
            set(&mut p, v);
            let m = measure(&p);
            let flag = if m.paragraphs < 108 { "  <- paragraphs lost" } else { "" };
            println!("{}{flag}", row(&format!("{name} = {v}"), &m));
            if m.paragraphs < base.paragraphs {
                cliffs.push(format!("{name}={v}"));
            }
        }
    }
    println!("\nsettings that lose paragraphs: {cliffs:?}");
}
