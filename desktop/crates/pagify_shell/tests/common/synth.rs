//! Synthetic pages: partition check and a page with a bit of everything.
#![allow(dead_code)]

use super::*;
use std::collections::{BTreeMap, BTreeSet};

/// Every object of `frags` (first of each id) is in exactly one block, and no block holds an unknown object.
pub fn assert_partition(frags: &[Frag], blocks: &[Block]) {
    let mut seen: BTreeMap<usize, usize> = BTreeMap::new();
    for (bi, b) in blocks.iter().enumerate() {
        assert!(!b.lines.is_empty(), "block {bi} has no lines");
        for o in b.objects() {
            assert!(seen.insert(o, bi).is_none(), "object {o} is in two blocks (or twice in one)");
        }
    }
    let ids: BTreeSet<usize> = frags.iter().map(|f| f.object).collect();
    let got: BTreeSet<usize> = seen.keys().copied().collect();
    assert_eq!(ids, got, "the blocks must hold exactly the input objects");
}

/// A page that has a bit of everything: a heading, two justified columns with a gutter, a ruled table, a
/// list, a footnote mark, an outlined word. Used by the property tests (scale, shuffle, perf).
pub fn sample_page() -> Layout {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut l = Layout::new();
    let cols = [(40.0f32, 250.0f32), (261.7, 471.7)];
    for (c, &(x0, x1)) in cols.iter().enumerate() {
        l.frag(x0, 90.4 + c as f32, 60.0, &BOLD, "Heading words");
        let mut base = 100.0 + c as f32;
        for para in 0..3 {
            let n = 4 + rng.below(5);
            let mut lines: Vec<Vec<&str>> = Vec::new();
            for i in 0..n {
                if i + 1 < n {
                    let fill = 0.9 + 0.08 * rng.unit();
                    lines.push(fitted_words(&mut rng, x1 - x0, 8.0, fill));
                } else {
                    let k = 3 + rng.below(3);
                    lines.push(words(&mut rng, k));
                }
            }
            l.paragraph(x0, x1, base, 9.6, &LIGHT, &lines, 2);
            base += 9.6 * n as f32 + if para == 1 { 14.0 - 9.6 } else { 0.0 };
        }
    }
    // a ruled table under column 1
    for row in 0..4 {
        let y = 300.0 + row as f32 * 20.0;
        l.hrule(40.0, 250.0, y);
        l.frag(41.0, y + 9.0, 50.0, &LIGHT, "Power Input:");
        l.frag(60.0, y + 17.0, 20.0, &LIGHT, "40W");
    }
    l.hrule(40.0, 250.0, 380.0);
    // a list under column 2
    for item in 0..3 {
        let base = 310.0 + item as f32 * 20.0;
        l.frag(261.7, base, 4.0, &LIGHT, "\u{2022}");
        l.frag(270.0, base, 150.0, &LIGHT, "an item with some words in it");
        l.frag(270.0, base + 9.6, 100.0, &LIGHT, "and a second line.");
    }
    // a footnote mark in a smaller size at the start of a line
    let small = Look { size: 5.0, ..LIGHT.clone() };
    l.frag(40.0, 420.0, 3.0, &small, "*");
    l.frag(44.0, 420.0, 120.0, &LIGHT, "Subject to technical alternations.");
    // an outlined word inside a line of column 1
    let base = 450.0;
    l.frag(40.0, base - 9.6, 200.0, &LIGHT, "text above the outlined word in this line.");
    l.frag(40.0, base, 90.0, &LIGHT, "before the hole");
    l.shape(135.0, base - 6.0, 175.0, base + 1.6);
    l.frag(180.0, base, 60.0, &LIGHT, "after it");
    l.frag(40.0, base + 9.6, 200.0, &LIGHT, "text below the outlined word in this line.");
    l
}

/// A readable dump of the blocks of a synthetic layout, for failing assertions and debugging.
pub fn dump(frags: &[Frag], shapes: &[Shape], blocks: &[Block]) -> String {
    let mut s = String::new();
    for (i, b) in blocks.iter().enumerate() {
        s.push_str(&format!("block {i} [{}] {} lines, rect ({:.1},{:.1},{:.1},{:.1})\n", b.starts_because, b.lines.len(), b.left, b.top, b.right, b.bottom));
        for l in &b.lines {
            let text: Vec<String> = l.objects.iter().map(|o| frags.iter().find(|f| f.object == *o).map(|f| f.text.clone()).unwrap_or_default()).collect();
            let outl: Vec<String> = l.outlined.iter().map(|o| format!("<{}>", shapes.iter().find(|x| x.object == *o).map(|_| *o).unwrap_or(0))).collect();
            s.push_str(&format!("    base {:.1} x {:.1}..{:.1}: {} {}\n", l.baseline, l.left, l.right, text.join("|"), outl.join(" ")));
        }
    }
    s
}
