//! Acceptance of the block detector on the real datasheet (3 pages, justified text in three columns,
//! every line cut into 3 to 8 text objects, words drawn as outlines, two independent hand labelings).
//!
//! `cargo test -p pagify_shell --release --test blocks_datasheet -- --nocapture` prints the tables.
//!
//! What the detector is given is what the integration layer gives it: every text object of the page with
//! text (the blank-text twin of the heading "Color Options" is not passed) and every path object. The
//! labelers' truth is compared on those objects only.

mod common;

use common::*;
use pagify_shell::blocks::*;
use std::collections::{BTreeMap, BTreeSet};

fn run(pg: u32) -> (Page, Vec<Block>) {
    let page = load_page(pg);
    let blocks = detect(&real_frags(&page), &page.shapes);
    (page, blocks)
}

fn text_ids(page: &Page, range: std::ops::RangeInclusive<usize>) -> Vec<usize> {
    let mut v: Vec<usize> = real_frags(page).iter().map(|f| f.object).filter(|o| range.contains(o)).collect();
    v.sort();
    v
}

fn sorted(mut v: Vec<usize>) -> Vec<usize> {
    v.sort();
    v
}

// ------------------------------------------------------------------------------------------------
// (i) the user's paragraph
// ------------------------------------------------------------------------------------------------

/// The paragraph the user marked red on page 1: text objects 985..=1043 (57 of them; 1026 and 1035 are the
/// path objects of the two outlined words "efficacy"), 13 lines at a constant 9.6 pt pitch from baseline
/// 421.0 to 536.2. Its heading (980..=984) is above it, the heading "Light Quality" (1044..=1047) below
/// it, the paragraph 951..=979 above the heading: four blocks, not one.
#[test]
fn the_users_paragraph_is_one_block_with_its_two_outlined_words() {
    let (page, blocks) = run(1);
    let paragraph = text_ids(&page, 985..=1043);
    assert_eq!(paragraph.len(), 57, "the fixture must hold the 57 text objects of the paragraph");
    let bi = block_index_of(&blocks, 985).expect("object 985 is in a block");
    let b = &blocks[bi];
    assert_eq!(sorted(b.objects()), paragraph, "the block holds exactly the 57 objects");
    assert_eq!(b.lines.len(), 13);
    // clicking-equivalent lookup: the same block for every one of the 57 objects
    for o in &paragraph {
        assert_eq!(block_index_of(&blocks, *o), Some(bi), "object {o}");
    }
    assert_ne!(block_index_of(&blocks, 984), Some(bi), "the heading above is its own block");
    assert_ne!(block_index_of(&blocks, 1044), Some(bi), "the heading below is its own block");
    // the two outlined words, each on its own line
    let outlined: Vec<(usize, usize)> = b.lines.iter().enumerate().flat_map(|(i, l)| l.outlined.iter().map(move |&s| (i, s))).collect();
    assert_eq!(outlined.iter().map(|x| x.1).collect::<Vec<_>>(), vec![1026, 1035]);
    for (i, s) in &outlined {
        let sh = page.shapes.iter().find(|x| x.object == *s).unwrap();
        let l = &b.lines[*i];
        assert!(l.left <= sh.left && l.right >= sh.right && l.top <= sh.top + 0.01 && l.bottom >= sh.bottom - 0.01, "line {i} must hold its outlined word {s}");
    }
    // the lines: baselines 421.0 .. 536.2 at 9.6 pt
    for (i, l) in b.lines.iter().enumerate() {
        assert!((l.baseline - (421.0 + 9.6 * i as f32)).abs() < 0.5, "line {i} baseline {}", l.baseline);
    }
    assert_eq!(b.starts_because, "style", "it starts where the bold heading ends");
    // its neighbours
    for (name, range, lines) in [("heading above", 980..=984usize, 1usize), ("heading below", 1044..=1047, 1), ("paragraph above", 951..=979, 5)] {
        let ids = text_ids(&page, range);
        let blk = &blocks[block_index_of(&blocks, ids[0]).unwrap()];
        assert_eq!(sorted(blk.objects()), ids, "{name}");
        assert_eq!(blk.lines.len(), lines, "{name}");
    }
}

// ------------------------------------------------------------------------------------------------
// (ii) the 18 paragraphs of each page, against both labelers
// ------------------------------------------------------------------------------------------------

#[test]
fn every_paragraph_of_every_page_matches_both_labelers_exactly() {
    let mut total = 0;
    let mut exact = 0;
    for pg in 1..=3 {
        let (page, blocks) = run(pg);
        let universe = universe_of(&page);
        let sets = block_sets(&blocks, &universe);
        for (who, truth) in &page.labels {
            let paragraphs: Vec<&Label> = truth.iter().filter(|t| t.kind == "paragraph").collect();
            assert_eq!(paragraphs.len(), 18, "labeler {who} page {pg}: 18 paragraphs");
            for t in paragraphs {
                total += 1;
                let want: BTreeSet<usize> = t.objects.iter().copied().filter(|o| universe.contains(o)).collect();
                let first = *want.iter().next().unwrap();
                let got = &sets[block_index_of(&blocks, first).unwrap()];
                if *got == want {
                    exact += 1;
                } else {
                    println!("labeler {who} page {pg}: paragraph {} differs: truth {} objects, detector {}", t.id, want.len(), got.len());
                }
            }
        }
    }
    println!("paragraphs exact: {exact}/{total}");
    assert_eq!((exact, total), (108, 108));
}

/// The labelers' own documented ambiguities (labeler B lists an ALT reading for nine blocks per page):
/// which side the detector took. Only the labelled primary reading is asserted for paragraphs (the
/// detector reads a justified line that stops short as the end of a paragraph, "Getting different uniform
/// shades" and "Dimmable function is recommended ..." are paragraphs of their own); for the other kinds
/// the table is printed.
#[test]
fn the_labelers_ambiguities_and_the_side_the_detector_took() {
    for pg in 1..=3 {
        let (page, blocks) = run(pg);
        let universe = universe_of(&page);
        let sets = block_sets(&blocks, &universe);
        for (who, truth) in &page.labels {
            for t in truth.iter().filter(|t| t.alt.is_some()) {
                let want: BTreeSet<usize> = t.objects.iter().copied().filter(|o| universe.contains(o)).collect();
                let Some(&first) = want.iter().next() else { continue };
                let got = &sets[block_index_of(&blocks, first).unwrap()];
                let side = if *got == want { "primary reading" } else { "NOT the primary reading" };
                println!("labeler {who} page {pg} {} [{}]: {side}; ALT: {}", t.id, t.kind, t.alt.as_deref().unwrap_or(""));
                if t.kind == "paragraph" {
                    assert_eq!(*got, want, "labeler {who} page {pg}: the ambiguous paragraph {} must come out as labelled", t.id);
                }
            }
        }
    }
}

// ------------------------------------------------------------------------------------------------
// (iii) per-seed exact match over every clickable object
// ------------------------------------------------------------------------------------------------

/// For every clickable seed object: is the block the detector returns exactly the labeler's block?
/// Measured on 2026-10-03 with the detector's default parameters (every number below is the measured
/// value, kept as a floor so that a change of the detector cannot lose ground unnoticed):
///   paragraphs 100 % of seeds on every page and for both labelers;
///   all kinds together 6062 of 6320 seeds (95.9 %); blocks 611 of 656 (93.1 %); mean pairwise F1 0.9943
///   (before the chart rows, the unlimited boxes and the stricter outlined words: 6050 seeds, 599 blocks).
#[test]
fn per_seed_exact_match_by_labeler_page_and_kind() {
    let mut by_kind: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut by_who: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let (mut seeds, mut seeds_exact, mut blocks_total, mut blocks_exact) = (0usize, 0usize, 0usize, 0usize);
    let mut f1 = 0.0;
    let mut n = 0.0;
    let mut worst: Vec<(usize, String, String, usize, usize, String)> = Vec::new(); // wrong seeds, kind, id, truth size, predicted blocks spanned, text
    for pg in 1..=3 {
        let (page, blocks) = run(pg);
        let universe = universe_of(&page);
        let texts = texts_of(&page);
        for (who, truth) in &page.labels {
            let s = score(&blocks, truth, &universe, &texts);
            let row: Vec<String> = s.seeds_by_kind.iter().map(|(k, (a, b))| format!("{k} {a}/{b}")).collect();
            println!("labeler {who} page {pg}: seeds {}/{} ({:.1}%), blocks {}/{}, F1 {:.4}; per kind: {}", s.seeds_exact, s.seeds, 100.0 * s.seeds_exact as f64 / s.seeds as f64, s.exact, s.blocks, s.f1(), row.join(", "));
            seeds += s.seeds;
            seeds_exact += s.seeds_exact;
            blocks_total += s.blocks;
            blocks_exact += s.exact;
            f1 += s.f1();
            n += 1.0;
            for (k, (a, b)) in &s.seeds_by_kind {
                let e = by_kind.entry(k.clone()).or_default();
                e.0 += a;
                e.1 += b;
            }
            let e = by_who.entry(format!("{who} page {pg}")).or_default();
            e.0 += s.seeds_exact;
            e.1 += s.seeds;
            // the truth blocks with wrong seeds, worst first
            let mut wrong: BTreeMap<String, (usize, String, usize, BTreeSet<usize>)> = BTreeMap::new();
            for (seed, kind, id, tsize, _psize) in &s.seed_fails {
                let e = wrong.entry(id.clone()).or_insert((0, kind.clone(), *tsize, BTreeSet::new()));
                e.0 += 1;
                e.3.insert(block_index_of(&blocks, *seed).unwrap_or(usize::MAX));
                let _ = seed;
            }
            for (id, (cnt, kind, tsize, preds)) in wrong {
                let first = truth.iter().find(|t| t.id == id).and_then(|t| t.objects.first().copied()).unwrap_or(0);
                let text: String = texts.get(&first).cloned().unwrap_or_default().chars().take(40).collect();
                worst.push((cnt, format!("{who} p{pg}"), format!("{id} [{kind}]"), tsize, preds.len(), text));
            }
        }
    }
    println!("\nper kind (seeds exact / seeds):");
    for (k, (a, b)) in &by_kind {
        println!("  {k:10} {a:5}/{b:5}  {:.1}%", 100.0 * *a as f64 / *b as f64);
    }
    worst.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    println!("\nthe 20 truth blocks with the most wrong seeds (wrong seeds, labeler page, block, truth size, predicted blocks spanned, first text):");
    for w in worst.iter().take(20) {
        println!("  {:3} {:6} {:22} truth {:3} objects, {} predicted blocks, '{}'", w.0, w.1, w.2, w.3, w.4, w.5);
    }
    let rate = seeds_exact as f64 / seeds as f64;
    println!("\nTOTAL seeds {seeds_exact}/{seeds} ({:.1}%), blocks {blocks_exact}/{blocks_total} ({:.1}%), mean pairwise F1 {:.4}", 100.0 * rate, 100.0 * blocks_exact as f64 / blocks_total as f64, f1 / n);
    assert_eq!(by_kind["paragraph"].0, by_kind["paragraph"].1, "every seed of every paragraph must return exactly its paragraph");
    assert!(seeds_exact >= 6062, "seeds exact {seeds_exact} (measured 6062)");
    assert!(blocks_exact >= 611, "blocks exact {blocks_exact} (measured 611)");
    assert!(f1 / n >= 0.994, "mean pairwise F1 {} (measured 0.9943)", f1 / n);
}

// ------------------------------------------------------------------------------------------------
// (iv) parity with the research prototype
// ------------------------------------------------------------------------------------------------

/// The blocks of the research prototype (para_proto, advance-width font classes, absolute point
/// thresholds, the same fragments and shapes) against this module. Every difference is listed here with
/// its classification; a new difference fails the test.
///
/// Every page: the footer "HSI Lighting | HUE . SATURATION . INTENSITY" (a 7 pt brand, a grey 9 pt bar glyph,
/// a 5.5 pt tagline on one baseline, 0.6 em between the bar and the tagline) is two blocks in the
/// prototype, which cut a row where the type size jumps by 1.3 or more, and one block here: that cut was
/// dropped (neither labeler nor the independent synthetic corpus needed it, and the prior-art author's
/// final detector dropped it too). INTENDED; labeler B has the footer as one block, labeler A as three.
///
/// Page 1 otherwise: identical (117 of 119 blocks, the footer pair aside).
///
/// Page 2, six prototype blocks against three here, all INTENDED (the font evidence): the value cells
/// "35W" and " 900mA" are drawn in a second, non-embedded resource of the same face "Montserrat-Light";
/// the prototype measured the average advance per character, 4 % apart, and called them another font and
/// cut them off their labels; here an unknown stem with the same face name is no evidence of another
/// style, and the two lines between two rules are a ruled cell in any case. Both labelers have
/// "Power Input: / 35W" and "Current Input: ... / 900mA" as one row each: this module agrees with them.
/// "Light output ratio 85%" and "Luminaire Efficacy 100 lm/w" (same face, same weight, same size, same
/// pitch) are now one block; labeler B lists exactly that grouping as the alternative reading, labeler A
/// has them apart like the prototype (the prototype's split was an accident of its advance proxy).
///
/// Page 1, the wattage chart, INTENDED: the y label "16" stands 1.6 em over the row of x tick labels (one text
/// object of 27 characters, on the same left edge); the prototype read the pair as a two-line paragraph, here a label
/// of a few characters over a line twelve times as wide is not the first line of anything (`tiny_line`: the next word
/// would have fitted on it twice over). Labeler B has two label blocks, labeler A part of the whole diagram.
///
/// Page 2, the wattage chart, INTENDED: the six x tick labels "30°" .. "55° 60°" (one baseline, 2.3 to 2.55 em
/// apart at 5.97 pt, the axis title 1.3 em under them in another weight) are one block in the prototype and one
/// block each here, and so are the polar diagram's "30°" and "15°" (3.4 em apart under a caption line): a gap
/// that wide is no word space in a row with no justified neighbour (`sparse_gap`; the widest gap inside the
/// datasheet's justified paragraphs is 1.66 em). Labeler B draws every one of them as a label of its own,
/// labeler A as part of the whole diagram.
///
/// Page 3, one prototype block against two here, INTENDED: the y-axis tick label "5" and the row of
/// x-axis tick labels under it are 10.16 pt apart; the prototype rounded the dominant size to 6.0 pt and
/// 1.7 x 6.0 = 10.2 > 10.16 let it see a paragraph pitch; here the size is exact (5.97) and the same pitch
/// is outside the window. Labeler B has them as two blocks, labeler A as part of the whole diagram.
#[test]
fn parity_with_the_prototype_every_difference_is_documented() {
    type Set = BTreeSet<usize>;
    let sets = |v: &[&[usize]]| -> BTreeSet<Set> { v.iter().map(|s| s.iter().copied().collect()).collect() };
    let expected: [(BTreeSet<Set>, BTreeSet<Set>); 3] = [
        // (page 1: the chart's y label "16" over the row of x tick labels, one block there, two here)
        (sets(&[&[811, 813]]), sets(&[&[811], &[813]])),
        (
            // only the prototype has
            sets(&[&[1065], &[2838, 2839, 2840], &[2844], &[2845], &[2846, 2848], &[2850], &[1080, 1081], &[2627, 2628, 2629, 2630, 2631, 2632]]),
            // only this module has
            sets(&[&[1065, 2838, 2839, 2840], &[2844, 2845], &[2846, 2848, 2850], &[1080], &[1081], &[2627], &[2628], &[2629], &[2630], &[2631], &[2632]]),
        ),
        (sets(&[&[1869, 1870]]), sets(&[&[1869], &[1870]])),
    ];
    for pg in 1..=3u32 {
        let (page, blocks) = run(pg);
        let universe: Set = page.proto.iter().flat_map(|b| b.objects.iter().copied()).collect();
        let mine: BTreeSet<Set> = block_sets(&blocks, &universe).into_iter().filter(|s| !s.is_empty()).collect();
        let theirs: BTreeSet<Set> = page.proto.iter().map(|b| b.objects.iter().copied().collect::<Set>()).filter(|s: &Set| !s.is_empty()).collect();
        let only_theirs: BTreeSet<Set> = theirs.difference(&mine).cloned().collect();
        let only_mine: BTreeSet<Set> = mine.difference(&theirs).cloned().collect();
        println!("page {pg}: {} blocks here, {} in the prototype, {} identical, {} only here, {} only there", mine.len(), theirs.len(), mine.intersection(&theirs).count(), only_mine.len(), only_theirs.len());
        let (e_theirs, e_mine) = &expected[(pg - 1) as usize];
        // the footer: two blocks there (brand and bar | tagline), one here (their union)
        let rest_theirs: BTreeSet<Set> = only_theirs.difference(e_theirs).cloned().collect();
        let rest_mine: BTreeSet<Set> = only_mine.difference(e_mine).cloned().collect();
        assert_eq!(rest_theirs.len(), 2, "page {pg}: the prototype's footer blocks, got {rest_theirs:?}");
        assert_eq!(rest_mine.len(), 1, "page {pg}: this module's footer block, got {rest_mine:?}");
        let union: Set = rest_theirs.iter().flatten().copied().collect();
        assert_eq!(rest_mine.iter().next().unwrap(), &union, "page {pg}: the footer is the union of the prototype's two blocks");
        assert!(texts_of(&page)[union.iter().next().unwrap()].starts_with("HSI"), "page {pg}: it is the footer");
        assert!(e_theirs.is_subset(&only_theirs) && e_mine.is_subset(&only_mine), "page {pg}: the documented differences");
    }
}

/// Blank-text objects are not passed by the integration layer; if they are, the grouping of every other
/// object does not change (the twin layer of "Color Options" joins the heading).
#[test]
fn the_blank_twin_of_color_options_joins_its_heading_and_changes_nothing_else() {
    let page = load_page(1);
    let with = detect(&page.frags, &page.shapes);
    let without = detect(&real_frags(&page), &page.shapes);
    let blank: BTreeSet<usize> = page.frags.iter().filter(|f| f.text.trim().is_empty()).map(|f| f.object).collect();
    assert_eq!(blank.len(), 5, "objects 826..=830");
    let strip = |bs: &[Block]| {
        let mut v: Vec<Vec<usize>> = bs.iter().map(|b| b.objects().into_iter().filter(|o| !blank.contains(o)).collect::<Vec<usize>>()).filter(|g| !g.is_empty()).collect();
        v.sort();
        v
    };
    assert_eq!(strip(&with), strip(&without));
    let heading = &with[block_index_of(&with, 817).unwrap()];
    for o in 826..=830 {
        assert!(heading.contains(o), "the empty twin {o} belongs to the heading's block");
    }
}
