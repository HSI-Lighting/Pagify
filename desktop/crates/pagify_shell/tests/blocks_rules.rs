//! One small synthetic layout per rule of the block detector, each stated as a person would judge it
//! (what is one paragraph, what is a heading, what is a row), then the same layout with the rule
//! switched off through `Params` to show that the rule, and nothing else, makes the difference.

mod common;

use common::*;
use pagify_shell::blocks::*;

/// The page as sorted groups of sorted object ids.
fn groups(blocks: &[Block]) -> Vec<Vec<usize>> {
    let mut v: Vec<Vec<usize>> = blocks
        .iter()
        .map(|b| {
            let mut o = b.objects();
            o.sort();
            o
        })
        .collect();
    v.sort();
    v
}

fn sorted(mut v: Vec<usize>) -> Vec<usize> {
    v.sort();
    v
}

fn flat(lines: &[Vec<usize>]) -> Vec<usize> {
    sorted(lines.concat())
}

fn sorted_groups(mut g: Vec<Vec<usize>>) -> Vec<Vec<usize>> {
    g.sort();
    g
}

fn block_of(blocks: &[Block], object: usize) -> &Block {
    &blocks[block_index_of(blocks, object).expect("object must be in a block")]
}

macro_rules! assert_groups {
    ($l:expr, $blocks:expr, $want:expr) => { assert_groups!($l, $blocks, $want, "grouping") };
    ($l:expr, $blocks:expr, $want:expr, $($fmt:tt)+) => {{
        let want: Vec<Vec<usize>> = $want;
        let got = groups(&$blocks);
        if got != want {
            panic!("{}\nwant {:?}\ngot  {:?}\n{}", format!($($fmt)+), want, got, dump(&$l.frags, &$l.shapes, &$blocks));
        }
    }};
}

fn off(f: impl Fn(&mut Params)) -> Params {
    let mut p = Params::default();
    f(&mut p);
    p
}

/// A column of `n` lines, all but the last justified to `x0..x1` with words fitted to `fill` of the width
/// (`loose` lines are set at 0.78 so their word gaps are wide); the last line is short and ragged.
fn column(l: &mut Layout, rng: &mut Rng, x0: f32, x1: f32, base: f32, n: usize, fill: f32, loose: &[usize]) -> Vec<Vec<usize>> {
    let mut lines: Vec<Vec<&str>> = Vec::new();
    for i in 0..n {
        if i + 1 == n {
            lines.push(words(rng, 3));
        } else {
            let f = if loose.contains(&i) { 0.78 } else { fill + 0.05 * rng.unit() };
            lines.push(fitted_words(rng, x1 - x0, 8.0, f));
        }
    }
    l.paragraph(x0, x1, base, 9.6, &LIGHT, &lines, 1)
}

/// `n` lines that all run to the margin (so no short line ends the paragraph), set justified.
fn full_lines(l: &mut Layout, rng: &mut Rng, x0: f32, x1: f32, base: f32, n: usize, look: &Look) -> Vec<usize> {
    let mut ids = Vec::new();
    for i in 0..n {
        let w = fitted_words(rng, x1 - x0, look.size, 0.93);
        ids.extend(l.line(x0, x1, base + 9.6 * i as f32, look, &w, 1, true));
    }
    ids
}

// ------------------------------------------------------------------------------------------------
// columns: the corridor
// ------------------------------------------------------------------------------------------------

/// Justified word gaps reach 1.1 to 1.4 em, the gutter is 1.46 em: no gap threshold separates the two.
fn two_columns() -> (Layout, Vec<usize>, Vec<usize>) {
    let mut rng = Rng(5);
    let mut l = Layout::new();
    let a = column(&mut l, &mut rng, 50.0, 250.0, 100.0, 9, 0.90, &[2, 5]);
    let b = column(&mut l, &mut rng, 261.7, 461.7, 100.0, 9, 0.90, &[3, 6]);
    (l, flat(&a), flat(&b))
}

#[test]
fn two_justified_columns_with_a_gutter_are_two_paragraphs() {
    let (l, a, b) = two_columns();
    let blocks = detect(&l.frags, &l.shapes);
    assert_groups!(l, blocks, sorted_groups(vec![a.clone(), b.clone()]), "one paragraph per column");
    assert_eq!(block_of(&blocks, a[0]).lines.len(), 9);
    assert_eq!(block_of(&blocks, b[0]).lines.len(), 9);
}

#[test]
fn without_the_corridor_no_threshold_gets_the_columns_right() {
    let (l, a, b) = two_columns();
    let want = sorted_groups(vec![a, b]);
    // the naive rule: a gap wider than T em cuts. Try every T from 0.9 to 4.0 in steps of 0.05.
    let mut right = Vec::new();
    let mut t = 0.9f32;
    while t < 4.0 {
        let p = off(|p| {
            p.corridor = false;
            p.gap_only = t;
        });
        if groups(&detect_with(&l.frags, &l.shapes, &p)) == want {
            right.push(t);
        }
        t += 0.05;
    }
    let window = right.iter().cloned().fold(f32::MIN, f32::max) - right.iter().cloned().fold(f32::MAX, f32::min);
    assert!(right.is_empty() || window <= 0.2, "a pure gap threshold separates the columns over a window of {window} em ({right:?}): the layout does not prove the corridor");
}

#[test]
fn a_river_of_aligned_word_gaps_over_four_lines_is_no_gutter() {
    // four lines of a justified paragraph whose word gaps (1.0 em, as wide as justification makes them)
    // happen to line up at x=150, over two lines without the gap: three proving rows are not enough for a
    // gap under 1.2 em, a real gutter of such a width needs four
    let mut rng = Rng(69);
    let mut l = Layout::new();
    let mut ids = Vec::new();
    for i in 0..4 {
        let base = 100.0 + 9.6 * i as f32;
        ids.push(l.frag(50.0, base, 96.0, &LIGHT, "the light of the"));
        ids.push(l.frag(154.0, base, 96.0, &LIGHT, "downlight is here"));
    }
    ids.extend(full_lines(&mut l, &mut rng, 50.0, 250.0, 100.0 + 9.6 * 4.0, 2, &LIGHT));
    let blocks = detect(&l.frags, &l.shapes);
    assert_groups!(l, blocks, vec![sorted(ids)], "the river is not a gutter");
    let eager = off(|p| p.pos3 = 3);
    assert!(detect_with(&l.frags, &l.shapes, &eager).len() > 1, "three proving rows cut the river");
}

#[test]
fn justified_lines_stay_whole_across_their_word_gaps() {
    let (l, a, _b) = two_columns();
    let blocks = detect(&l.frags, &l.shapes);
    assert!(block_of(&blocks, a[0]).lines.iter().all(|ln| ln.objects.len() >= 3), "justified lines must not be cut at their word gaps");
}

#[test]
fn a_line_across_two_columns_belongs_to_neither() {
    // one full-width line of the same look over, and another under, two justified columns: without the
    // rule the first would be the top line of the left column and the second its bottom line
    let mut rng = Rng(37);
    let mut l = Layout::new();
    let w = fitted_words(&mut rng, 411.7, 8.0, 0.93);
    let over = l.line(50.0, 461.7, 100.0, &LIGHT, &w, 1, true);
    let left = full_lines(&mut l, &mut rng, 50.0, 250.0, 109.6, 4, &LIGHT);
    let right = full_lines(&mut l, &mut rng, 261.7, 461.7, 109.6, 4, &LIGHT);
    let w = fitted_words(&mut rng, 411.7, 8.0, 0.93);
    let under = l.line(50.0, 461.7, 109.6 + 9.6 * 4.0, &LIGHT, &w, 1, true);
    let blocks = detect(&l.frags, &l.shapes);
    assert_groups!(l, blocks, sorted_groups(vec![sorted(over), sorted(left), sorted(right), sorted(under)]));
    let linked = off(|p| p.spanning_lines = false);
    assert_ne!(groups(&detect_with(&l.frags, &l.shapes, &linked)), groups(&blocks), "the rule is what keeps them apart");
}

// ------------------------------------------------------------------------------------------------
// heading versus body
// ------------------------------------------------------------------------------------------------

fn heading_over_body(heading: &Look) -> (Layout, Vec<usize>, Vec<usize>) {
    let mut rng = Rng(9);
    let mut l = Layout::new();
    let h = l.line(50.0, 250.0, 100.0, heading, &["Light", "Quality"], 1, false);
    let body = full_lines(&mut l, &mut rng, 50.0, 250.0, 109.6, 6, &LIGHT);
    (l, h, sorted(body))
}

#[test]
fn a_heading_that_differs_by_weight_only_is_its_own_block() {
    // same size, same face name (the datasheet's page 1 calls five weights "Montserrat-Thin"), same
    // colour, the same line pitch to the body below it: only the measured stem tells them apart
    let (l, h, body) = heading_over_body(&BOLD);
    let blocks = detect(&l.frags, &l.shapes);
    assert_groups!(l, blocks, sorted_groups(vec![sorted(h.clone()), body.clone()]));
    assert_eq!(block_of(&blocks, body[0]).starts_because, "style");
    // (the heading is a short line over justified text: the short-line rule would cut it too)
    let merged = detect_with(&l.frags, &l.shapes, &off(|p| {
        p.style_rule = false;
        p.short_lines = false;
    }));
    assert_eq!(merged.len(), 1, "without the style rule heading and body are one block");
}

#[test]
fn every_step_of_the_weight_ladder_makes_a_heading() {
    for stem in [20u16, 51, 74, 100, 124, 198] {
        for other in [20u16, 51, 74, 100, 124, 198] {
            let heading = Look { stem: Some(stem), ..BOLD.clone() };
            let body_look = Look { stem: Some(other), ..LIGHT.clone() };
            let mut rng = Rng(9);
            let mut l = Layout::new();
            let h = l.line(50.0, 250.0, 100.0, &heading, &["Light", "Quality"], 1, false);
            let body = full_lines(&mut l, &mut rng, 50.0, 250.0, 109.6, 5, &body_look);
            let blocks = detect_with(&l.frags, &l.shapes, &off(|p| p.short_lines = false));
            let split = block_index_of(&blocks, h[0]) != block_index_of(&blocks, body[0]);
            assert_eq!(split, stem != other, "stems {stem} over {other}");
        }
    }
}

#[test]
fn unknown_stems_and_equal_names_are_no_evidence_so_nothing_splits() {
    let unknown = Look { stem: None, ..BOLD.clone() };
    let body_unknown = Look { stem: None, ..LIGHT.clone() };
    let mut rng = Rng(9);
    let mut l = Layout::new();
    l.line(50.0, 250.0, 100.0, &unknown, &["Light", "Quality"], 1, false);
    full_lines(&mut l, &mut rng, 50.0, 250.0, 109.6, 5, &body_unknown);
    let no_short = off(|p| p.short_lines = false);
    assert_eq!(detect_with(&l.frags, &l.shapes, &no_short).len(), 1);
    // the other policy (the prior-art author's): an unknown stem merges nothing, so two font
    // resources with unknown stems split even when their names agree
    let strict = off(|p| {
        p.short_lines = false;
        p.unknown_stem_same_face = false;
    });
    assert_eq!(detect_with(&l.frags, &l.shapes, &strict).len(), 2);
    // a different face name is evidence even without stems
    let named = Look { face: "Montserrat-ExtraBold", ..unknown.clone() };
    let body_named = Look { face: "Montserrat-Light", ..body_unknown.clone() };
    let mut l = Layout::new();
    let h = l.line(50.0, 250.0, 100.0, &named, &["Light", "Quality"], 1, false);
    let mut rng = Rng(9);
    let body = full_lines(&mut l, &mut rng, 50.0, 250.0, 109.6, 5, &body_named);
    let blocks = detect_with(&l.frags, &l.shapes, &no_short);
    assert_ne!(block_index_of(&blocks, h[0]), block_index_of(&blocks, body[0]));
}

#[test]
fn one_font_embedded_as_two_resources_is_one_style() {
    // the same weight and name under two font ids: a paragraph whose lines alternate between the two
    // stays one block
    let twin = Look { font: 7, ..LIGHT.clone() };
    let mut rng = Rng(11);
    let mut l = Layout::new();
    for i in 0..6 {
        let look = if i % 2 == 0 { &LIGHT } else { &twin };
        let w = fitted_words(&mut rng, 200.0, 8.0, 0.93);
        l.line(50.0, 250.0, 100.0 + 9.6 * i as f32, look, &w, 1, true);
    }
    assert_eq!(detect(&l.frags, &l.shapes).len(), 1);
}

#[test]
fn a_minority_fragment_of_another_weight_neither_splits_the_line_nor_the_block() {
    // "HSI " in Medium inside a Light line, 14 % of the line's characters: the dominant font speaks
    let medium = Look { font: 3, stem: Some(100), ..LIGHT.clone() };
    let mut l = Layout::new();
    let mut ids = Vec::new();
    for i in 0..5 {
        let base = 100.0 + 9.6 * i as f32;
        if i == 2 {
            ids.push(l.frag(50.0, base, 80.0, &LIGHT, "the new downlight of"));
            ids.push(l.frag(133.0, base, 12.0, &medium, "HSI "));
            ids.push(l.frag(148.0, base, 98.0, &LIGHT, "lighting is available"));
        } else {
            ids.push(l.frag(50.0, base, 90.0, &LIGHT, "a long line of body text"));
            ids.push(l.frag(143.0, base, 107.0, &LIGHT, "that fills the column fully"));
        }
    }
    let blocks = detect(&l.frags, &l.shapes);
    assert_groups!(l, blocks, vec![sorted(ids)], "one block");
    assert_eq!(blocks[0].lines.len(), 5);
    assert_eq!(blocks[0].lines[2].objects.len(), 3, "the line stays whole");
}

#[test]
fn a_short_line_with_a_large_minority_is_not_a_style_change_either() {
    // a last line of 17 characters, 3 of them in Medium (18 %): not pure, so it says nothing about style
    let medium = Look { font: 3, stem: Some(100), ..LIGHT.clone() };
    let mut l = Layout::new();
    let mut rng = Rng(3);
    let mut all = full_lines(&mut l, &mut rng, 50.0, 250.0, 100.0, 4, &LIGHT);
    let base = 100.0 + 9.6 * 4.0;
    all.push(l.frag(50.0, base, 52.0, &LIGHT, "ready for"));
    all.push(l.frag(105.0, base, 12.0, &medium, "HSI"));
    all.push(l.frag(120.0, base, 30.0, &LIGHT, "lights."));
    let blocks = detect(&l.frags, &l.shapes);
    assert_groups!(l, blocks, vec![sorted(all)]);
}

#[test]
fn a_title_in_another_size_says_nothing_about_where_the_text_under_it_ends() {
    // a 12 pt bold title whose right edge (300) lies 50 pt beyond the margin of the justified 8 pt text
    // under it: line 1 of the text closes a sentence on the margin, and the title must not make that look
    // like a line with room left
    let big = Look { size: 12.0, ..BOLD.clone() };
    let mut rng = Rng(73);
    let mut l = Layout::new();
    let w = fitted_words(&mut rng, 250.0, 12.0, 0.93);
    let title = l.line(50.0, 300.0, 100.0, &big, &w, 1, true);
    let mut body = Vec::new();
    for i in 0..5 {
        let w = fitted_words(&mut rng, 200.0, 8.0, 0.93);
        let ids = l.line(50.0, 250.0, 114.0 + 9.6 * i as f32, &LIGHT, &w, 1, true);
        if i == 1 {
            let last = l.frags.iter().position(|f| f.object == *ids.last().unwrap()).unwrap();
            l.frags[last].text.push('.');
        }
        body.extend(ids);
    }
    let blocks = detect(&l.frags, &l.shapes);
    assert_groups!(l, blocks, sorted_groups(vec![sorted(title), sorted(body)]));
}

#[test]
fn a_wider_line_of_another_look_says_nothing_about_the_margin_of_a_justified_column() {
    // a bold heading line 350 pt wide over a justified paragraph 200 pt wide whose line 3 ends short
    // without closing a sentence (a forced break): the heading's width must not make the paragraph
    // "ragged", so line 3 still ends the first stretch
    let mut rng = Rng(63);
    let mut l = Layout::new();
    let w = fitted_words(&mut rng, 350.0, 8.0, 0.93);
    let h = l.line(50.0, 400.0, 100.0, &BOLD, &w, 1, true);
    let mut first = full_lines(&mut l, &mut rng, 50.0, 250.0, 109.6, 3, &LIGHT);
    let w = fitted_words(&mut rng, 140.0, 8.0, 0.93);
    first.extend(l.line(50.0, 190.0, 109.6 + 9.6 * 3.0, &LIGHT, &w, 1, true));
    let second = full_lines(&mut l, &mut rng, 50.0, 250.0, 109.6 + 9.6 * 4.0, 2, &LIGHT);
    let blocks = detect(&l.frags, &l.shapes);
    assert_groups!(l, blocks, sorted_groups(vec![sorted(h), sorted(first), sorted(second)]));
}

#[test]
fn a_line_set_mostly_in_another_weight_is_no_heading_and_does_not_split_the_paragraph() {
    // a run-in lead phrase: 25 of the line's 38 characters (66 %) are Bold, so no font holds 85 % of
    // the line and the line says nothing about style
    let mut l = Layout::new();
    let mut rng = Rng(33);
    let mut all = full_lines(&mut l, &mut rng, 50.0, 250.0, 100.0, 2, &LIGHT);
    all.push(l.frag(50.0, 100.0 + 9.6 * 2.0, 120.0, &BOLD, "Run-in lead phrase of the line"));
    all.push(l.frag(171.0, 100.0 + 9.6 * 2.0, 79.0, &LIGHT, "then plain text"));
    all.extend(full_lines(&mut l, &mut rng, 50.0, 250.0, 100.0 + 9.6 * 3.0, 2, &LIGHT));
    let blocks = detect(&l.frags, &l.shapes);
    assert_groups!(l, blocks, vec![sorted(all.clone())], "one paragraph");
    let speaks = off(|p| p.pure_share = 0.5);
    assert!(detect_with(&l.frags, &l.shapes, &speaks).len() >= 2, "were the dominant font allowed to speak at 66 %, the bold line would be a heading");
}

#[test]
fn a_slightly_larger_line_is_a_style_change_but_a_rounding_difference_is_not() {
    // 8.64 pt over 8 pt is +8 %: too little for "size", enough for "style"; +3 % is rounding
    for (size, split) in [(8.64f32, true), (8.24, false)] {
        let look = Look { size, ..LIGHT.clone() };
        let mut rng = Rng(31);
        let mut l = Layout::new();
        let body = full_lines(&mut l, &mut rng, 50.0, 250.0, 100.0, 4, &LIGHT);
        let last = full_lines(&mut l, &mut rng, 50.0, 250.0, 100.0 + 9.6 * 4.0, 1, &look);
        let blocks = detect(&l.frags, &l.shapes);
        assert_eq!(block_index_of(&blocks, body[0]) != block_index_of(&blocks, last[0]), split, "size {size}");
        if split {
            assert_eq!(block_of(&blocks, last[0]).starts_because, "style");
            let no_term = off(|p| p.size_rel = 1.0);
            assert_eq!(detect_with(&l.frags, &l.shapes, &no_term).len(), 1, "without the size term of the style rule they are one block");
        }
    }
}

#[test]
fn a_last_line_mostly_in_a_larger_size_is_no_size_change() {
    // the last line is 20 characters, 14 of them (70 %) in 10 pt inside an 8 pt paragraph: no size holds
    // 85 % of the line, so the line says nothing about size (it is not a heading)
    let big = Look { size: 10.0, ..LIGHT.clone() };
    let mut rng = Rng(45);
    let mut l = Layout::new();
    let mut all = full_lines(&mut l, &mut rng, 50.0, 250.0, 100.0, 4, &LIGHT);
    let base = 100.0 + 9.6 * 4.0;
    all.push(l.frag(50.0, base, 40.0, &LIGHT, "the end"));
    all.push(l.frag(91.0, base, 70.0, &big, "of the paragraph"));
    let blocks = detect(&l.frags, &l.shapes);
    assert_groups!(l, blocks, vec![sorted(all)], "one paragraph");
    // were the dominant size allowed to speak at 70 %, the line would be a heading
    let speaks = off(|p| p.pure_share = 0.5);
    assert!(detect_with(&l.frags, &l.shapes, &speaks).len() >= 2);
}

#[test]
fn one_letter_fragments_cut_inside_words_form_one_heading_line() {
    // the datasheet's heading "Description:" is five objects D|es|cr|ipti|on:
    let mut l = Layout::new();
    let mut ids = Vec::new();
    let mut x = 50.0;
    for (part, w) in [("D", 6.5), ("es", 9.9), ("cr", 8.6), ("ipti", 14.2), ("on:", 13.5)] {
        ids.push(l.frag(x, 100.0, w, &BOLD, part));
        x += w + 0.1;
    }
    let mut rng = Rng(21);
    let body = full_lines(&mut l, &mut rng, 50.0, 250.0, 109.6, 4, &LIGHT);
    let blocks = detect(&l.frags, &l.shapes);
    assert_groups!(l, blocks, sorted_groups(vec![sorted(ids.clone()), sorted(body)]));
    assert_eq!(block_of(&blocks, ids[0]).lines.len(), 1);
    assert_eq!(block_of(&blocks, ids[0]).lines[0].objects, ids);
}

#[test]
fn a_change_of_size_or_colour_makes_a_heading_too() {
    let big = Look { size: 12.0, ..LIGHT.clone() };
    let red = Look { rgb: [200, 30, 30], ..LIGHT.clone() };
    for (look, why) in [(big, "size"), (red, "style")] {
        let mut rng = Rng(9);
        let mut l = Layout::new();
        let h = l.line(50.0, 250.0, 98.0, &look, &["Light", "Quality"], 1, false);
        let body = full_lines(&mut l, &mut rng, 50.0, 250.0, 109.6, 5, &LIGHT);
        let blocks = detect(&l.frags, &l.shapes);
        assert_ne!(block_index_of(&blocks, h[0]), block_index_of(&blocks, body[0]), "{why}");
        assert_eq!(block_of(&blocks, body[0]).starts_because, why);
    }
}

// ------------------------------------------------------------------------------------------------
// paragraph ends
// ------------------------------------------------------------------------------------------------

#[test]
fn a_short_last_line_ends_a_paragraph_that_has_no_blank_line_after_it() {
    let mut rng = Rng(13);
    let mut l = Layout::new();
    let a = column(&mut l, &mut rng, 50.0, 250.0, 100.0, 5, 0.9, &[]);
    let b = column(&mut l, &mut rng, 50.0, 250.0, 100.0 + 9.6 * 5.0, 5, 0.9, &[]);
    let blocks = detect(&l.frags, &l.shapes);
    assert_groups!(l, blocks, sorted_groups(vec![flat(&a), flat(&b)]));
    assert_eq!(block_of(&blocks, b[0][0]).starts_because, "short-line");
    let merged = detect_with(&l.frags, &l.shapes, &off(|p| p.short_lines = false));
    assert_eq!(merged.len(), 1, "without the short-line rule the two paragraphs run together");
}

#[test]
fn two_ragged_lines_that_end_together_are_no_proof_of_a_justified_paragraph() {
    // right edges 232, 250, 249.6 and a short last line: two lines within a bin of the longest by chance
    // (about one time in twenty for ragged text) must not make line 0 a "short line" of a justified column
    let mut rng = Rng(41);
    let mut l = Layout::new();
    let mut ids = Vec::new();
    for (i, x1) in [232.0f32, 250.0, 249.6].iter().enumerate() {
        let w = fitted_words(&mut rng, x1 - 50.0, 8.0, 0.93);
        ids.extend(l.line(50.0, *x1, 100.0 + 9.6 * i as f32, &LIGHT, &w, 1, true));
    }
    let last = words(&mut rng, 3);
    ids.extend(l.line(50.0, 200.0, 100.0 + 9.6 * 3.0, &LIGHT, &last, 1, false));
    let blocks = detect(&l.frags, &l.shapes);
    assert_groups!(l, blocks, vec![sorted(ids.clone())], "one ragged paragraph");
    // two lines on the margin were enough before: line 0 is then cut off as short
    let eager = off(|p| {
        p.edge_lines = 2;
        p.justify_evidence = 2;
    });
    assert!(detect_with(&l.frags, &l.shapes, &eager).len() > 1, "two lines proving a margin cut this paragraph");
}

#[test]
fn a_sentence_end_ends_a_ragged_paragraph_only_where_the_next_word_would_have_fitted() {
    // ragged lines ending at 236 and 220; line 1 closes a sentence (16 pt of room left); line 2 starts
    // with a long word (60 pt: it did not fit, so the break is an ordinary wrap) or with a short one (9 pt:
    // it would have fitted, so the line was ended on purpose)
    for (first_word, width, whole) in [("extraordinarily", 60.0f32, true), ("so", 9.0, false)] {
        let mut l = Layout::new();
        let mut rng = Rng(53);
        let mut ids = Vec::new();
        for (i, x1) in [236.0f32, 220.0].iter().enumerate() {
            let w = fitted_words(&mut rng, x1 - 50.0, 8.0, 0.93);
            ids.extend(l.line(50.0, *x1, 100.0 + 9.6 * i as f32, &LIGHT, &w, 1, true));
        }
        // line 1 ends with a full stop
        let last = l.frags.iter().position(|f| f.object == *ids.last().unwrap()).unwrap();
        l.frags[last].text.push('.');
        ids.push(l.frag(50.0, 100.0 + 9.6 * 2.0, width, &LIGHT, first_word));
        ids.push(l.frag(50.0 + width + 3.0, 100.0 + 9.6 * 2.0, 120.0, &LIGHT, "the paragraph goes on here"));
        let blocks = detect(&l.frags, &l.shapes);
        assert_eq!(blocks.len() == 1, whole, "first word {first_word:?}\n{}", dump(&l.frags, &l.shapes, &blocks));
    }
}

#[test]
fn a_hyphenated_line_end_is_not_a_short_line() {
    for hyphen in ["\u{2}", "-", "\u{ad}", "\u{2010}"] {
        let mut rng = Rng(17);
        let mut l = Layout::new();
        let mut all = Vec::new();
        for i in 0..6 {
            let base = 100.0 + 9.6 * i as f32;
            if i == 2 {
                // a line that falls short of the margin because its last word is carried over
                all.push(l.frag(50.0, base, 120.0, &LIGHT, "a line that ends in a word contin"));
                all.push(l.frag(171.0, base, 6.0, &LIGHT, hyphen));
            } else {
                let w = fitted_words(&mut rng, 200.0, 8.0, 0.93);
                all.extend(l.line(50.0, 250.0, base, &LIGHT, &w, 1, true));
            }
        }
        let blocks = detect(&l.frags, &l.shapes);
        assert_groups!(l, blocks, vec![sorted(all.clone())], "line end {hyphen:?}");
        // the same short line without the hyphen ends the paragraph
        let plain = l.frags.iter().position(|f| f.text == hyphen).unwrap();
        l.frags[plain].text = ";".to_string();
        assert!(detect(&l.frags, &l.shapes).len() >= 2, "without the hyphen the short line ends the paragraph");
    }
}

#[test]
fn a_paragraph_gap_wider_than_the_pitch_starts_a_block() {
    let mut rng = Rng(19);
    let mut l = Layout::new();
    let mut a = full_lines(&mut l, &mut rng, 50.0, 250.0, 100.0, 6, &LIGHT);
    // 14 pt between the baselines over a 9.6 pt pitch: the datasheet's paragraph-to-heading gap
    let b = full_lines(&mut l, &mut rng, 50.0, 250.0, 100.0 + 9.6 * 5.0 + 14.0, 6, &LIGHT);
    a.sort();
    let blocks = detect(&l.frags, &l.shapes);
    assert_groups!(l, blocks, sorted_groups(vec![sorted(a), sorted(b)]));
    assert_eq!(blocks.iter().filter(|b| b.starts_because == "pitch").count(), 1);
    let merged = detect_with(&l.frags, &l.shapes, &off(|p| p.pitch_rule = false));
    assert_eq!(merged.len(), 1, "without the pitch rule they merge");
}

#[test]
fn a_double_spaced_paragraph_is_one_block_and_a_bigger_gap_still_ends_it() {
    // 16 pt between the baselines of 8 pt text (2 em): a gap nobody calls a paragraph gap when every line
    // has it; the gap between the two paragraphs is 28.8 pt
    fn spaced(l: &mut Layout, rng: &mut Rng, base: f32, n: usize) -> Vec<usize> {
        let mut ids = Vec::new();
        for i in 0..n {
            let w = fitted_words(rng, 200.0, 8.0, 0.93);
            ids.extend(l.line(50.0, 250.0, base + 16.0 * i as f32, &LIGHT, &w, 1, true));
        }
        ids
    }
    let mut rng = Rng(35);
    let mut l = Layout::new();
    let a = spaced(&mut l, &mut rng, 100.0, 6);
    let b = spaced(&mut l, &mut rng, 100.0 + 16.0 * 5.0 + 28.8, 4);
    let blocks = detect(&l.frags, &l.shapes);
    assert_groups!(l, blocks, sorted_groups(vec![sorted(a), sorted(b)]));
    // the leading is trusted because the run has at least three equal pitches; trusted only up to the
    // ordinary leading, every one of its lines would start a block
    let ordinary = off(|p| p.wide_leading = p.leading);
    assert!(detect_with(&l.frags, &l.shapes, &ordinary).len() > 2);
}

#[test]
fn the_pitch_of_a_heading_run_is_not_the_pitch_of_the_body_below_it() {
    // a two-line 14 pt heading (16.8 pt pitch) over 10 pt text (12 pt pitch): the typical pitch is
    // taken inside each run of one look, not over the column
    let big = Look { size: 14.0, ..BOLD.clone() };
    let body = Look { size: 10.0, ..LIGHT.clone() };
    let mut rng = Rng(23);
    let mut l = Layout::new();
    let mut h = l.line(72.0, 372.0, 100.0, &big, &["Heating", "and", "cooling"], 1, false);
    h.extend(l.line(72.0, 372.0, 116.8, &big, &["of", "rooms"], 1, false));
    let mut ids = Vec::new();
    for i in 0..5 {
        let w = fitted_words(&mut rng, 400.0, 10.0, 0.93);
        ids.extend(l.line(72.0, 472.0, 142.8 + 12.0 * i as f32, &body, &w, 1, true));
    }
    let blocks = detect(&l.frags, &l.shapes);
    assert_groups!(l, blocks, sorted_groups(vec![sorted(h), sorted(ids)]));
}

// ------------------------------------------------------------------------------------------------
// lists, indents, margins
// ------------------------------------------------------------------------------------------------

/// `n` items of two lines that all run to the margin, a marker of its own 18 pt left of the text.
fn marked_items(l: &mut Layout, rng: &mut Rng, markers: &[&str]) -> Vec<Vec<usize>> {
    let mut items = Vec::new();
    let mut base = 100.0;
    for m in markers {
        let mut ids = vec![l.frag(50.0, base, 5.0 * m.chars().count() as f32, &LIGHT, m)];
        ids.extend(full_lines(l, rng, 68.0, 250.0, base, 2, &LIGHT));
        base += 19.2;
        items.push(sorted(ids));
    }
    items
}

#[test]
fn list_items_with_the_marker_as_an_object_of_its_own_are_one_block_each() {
    for marker in ["\u{2022}", "-", "*"] {
        let mut rng = Rng(29);
        let mut l = Layout::new();
        let items = marked_items(&mut l, &mut rng, &[marker; 4]);
        let blocks = detect(&l.frags, &l.shapes);
        assert_groups!(l, blocks, sorted_groups(items.clone()), "marker {marker:?}");
        assert_eq!(block_of(&blocks, items[1][0]).starts_because, "item");
        // (the corridor walk does not see symbol glyphs either, so both switches go off together)
        let apart = detect_with(&l.frags, &l.shapes, &off(|p| {
            p.marker_merge = false;
            p.stray_marks = false;
        }));
        for item in &items {
            let (m, text) = (item[0], item[1]);
            assert_ne!(block_index_of(&apart, m), block_index_of(&apart, text), "marker {marker:?}: without the merge the marker stands alone");
        }
        let merged = detect_with(&l.frags, &l.shapes, &off(|p| p.list_items = false));
        assert!(merged.len() < items.len(), "marker {marker:?}: without the item rule the items run together");
    }
}

#[test]
fn a_blank_object_between_a_marker_and_its_text_changes_nothing() {
    // the "-" marker, an empty-text object (a space the producer emitted) and then the text: the blank
    // object is no word, so the marker still stands alone in its piece and is joined to its text
    let mut rng = Rng(57);
    let mut l = Layout::new();
    let mut items = marked_items(&mut l, &mut rng, &["-"; 6]); // (six: the corridor between marker and text needs four proving rows)
    for (i, item) in items.iter_mut().enumerate() {
        item.push(l.frag(56.0, 100.0 + 19.2 * i as f32, 4.0, &LIGHT, " "));
        item.sort();
    }
    let blocks = detect(&l.frags, &l.shapes);
    assert_groups!(l, blocks, sorted_groups(items), "the blank object is no word between marker and text");
}

#[test]
fn numbered_items_need_a_second_number_of_their_kind() {
    let mut rng = Rng(31);
    let mut l = Layout::new();
    let items = marked_items(&mut l, &mut rng, &["1.", "2.", "3."]);
    let blocks = detect(&l.frags, &l.shapes);
    assert_groups!(l, blocks, sorted_groups(items.clone()));
    // a wrapped line of an ordinary paragraph that happens to start with "a." is no list
    let mut l = Layout::new();
    let mut all = full_lines(&mut l, &mut rng, 50.0, 250.0, 100.0, 4, &LIGHT);
    let mut line5: Vec<&str> = vec!["a."];
    line5.extend(fitted_words(&mut rng, 200.0, 8.0, 0.9));
    all.extend(l.line(50.0, 250.0, 100.0 + 9.6 * 4.0, &LIGHT, &line5, 2, true));
    all.extend(full_lines(&mut l, &mut rng, 50.0, 250.0, 100.0 + 9.6 * 5.0, 2, &LIGHT));
    assert_groups!(l, detect(&l.frags, &l.shapes), vec![sorted(all)]);
}

#[test]
fn an_acronym_in_brackets_is_not_a_list_marker() {
    let mut l = Layout::new();
    let mut rng = Rng(23);
    let mut all = full_lines(&mut l, &mut rng, 50.0, 250.0, 100.0, 4, &LIGHT);
    all.push(l.frag(50.0, 100.0 + 9.6 * 4.0, 200.0, &LIGHT, "(CCT) is the colour temperature"));
    assert_groups!(l, detect(&l.frags, &l.shapes), vec![sorted(all)]);
}

#[test]
fn a_first_line_indent_starts_a_paragraph() {
    // ragged paragraphs with a first-line indent and no blank line between them; every line runs to the
    // margin, so nothing but the indent says a new paragraph begins
    let mut l = Layout::new();
    let mut ids: Vec<Vec<usize>> = Vec::new();
    let mut base = 100.0;
    for _ in 0..3 {
        let mut ps = Vec::new();
        for i in 0..4 {
            let x = if i == 0 { 62.0 } else { 50.0 };
            ps.push(l.frag(x, base, 250.0 - x, &LIGHT, "a full line of text"));
            base += 9.6;
        }
        ids.push(sorted(ps));
    }
    let blocks = detect(&l.frags, &l.shapes);
    assert_groups!(l, blocks, sorted_groups(ids.clone()));
    assert_eq!(block_of(&blocks, ids[1][0]).starts_because, "indent");
    let merged = detect_with(&l.frags, &l.shapes, &off(|p| p.indent_rule = false));
    assert_eq!(merged.len(), 1);
}

#[test]
fn entries_of_a_reference_list_start_where_a_flush_line_follows_lines_that_hang_in() {
    // three entries of three lines: the first line flush at x=50, the other two hang 18 pt in; every line
    // runs to the margin, so no short line says where an entry ends: only the hanging pattern does
    let mut rng = Rng(67);
    let mut l = Layout::new();
    let mut entries = Vec::new();
    for e in 0..3 {
        let base = 100.0 + 9.6 * 3.0 * e as f32;
        let mut ids = Vec::new();
        for k in 0..3 {
            let x0 = if k == 0 { 50.0 } else { 68.0 };
            let w = fitted_words(&mut rng, 250.0 - x0, 8.0, 0.93);
            ids.extend(l.line(x0, 250.0, base + 9.6 * k as f32, &LIGHT, &w, 1, true));
        }
        entries.push(sorted(ids));
    }
    let blocks = detect(&l.frags, &l.shapes);
    assert_groups!(l, blocks, sorted_groups(entries.clone()), "one block per entry");
    assert_eq!(blocks.iter().filter(|b| b.starts_because == "item").count(), 2);
    let flat = off(|p| p.hanging_items = false);
    assert!(detect_with(&l.frags, &l.shapes, &flat).len() < entries.len(), "without the rule the entries run together");
}

#[test]
fn one_flush_line_among_inset_lines_is_no_entry_and_neither_is_ragged_left_text() {
    // (a) eight lines inset by 18 pt, line 3 flush: a flush line between two inset lines, but only one
    // line sits on that edge (a reference list has an outdent edge its entries share)
    let mut rng = Rng(71);
    let mut l = Layout::new();
    let mut ids = Vec::new();
    for i in 0..8 {
        let x0 = if i == 3 { 50.0 } else { 68.0 };
        let w = fitted_words(&mut rng, 250.0 - x0, 8.0, 0.93);
        ids.extend(l.line(x0, 250.0, 100.0 + 9.6 * i as f32, &LIGHT, &w, 1, true));
    }
    let blocks = detect(&l.frags, &l.shapes);
    assert_groups!(l, blocks, vec![sorted(ids)], "one flush line is no entry");
    // (b) nine ragged-left lines whose left edges are 68, 50, 68, 50, 68, 100, 50, 90, 140: the pattern
    // inset - flush - inset occurs, with three lines on each edge, but a third of the lines sit on neither
    let mut l = Layout::new();
    let mut ids = Vec::new();
    for (i, x0) in [68.0f32, 50.0, 68.0, 50.0, 68.0, 100.0, 50.0, 90.0, 140.0].iter().enumerate() {
        let w = fitted_words(&mut rng, 250.0 - x0, 8.0, 0.93);
        ids.extend(l.line(*x0, 250.0, 100.0 + 9.6 * i as f32, &LIGHT, &w, 1, true));
    }
    let blocks = detect(&l.frags, &l.shapes);
    assert_groups!(l, blocks, vec![sorted(ids)], "text with many left edges has no entries");
}

#[test]
fn lines_inset_by_a_figure_are_not_first_line_indents() {
    // five lines start 110 pt in (a figure sits beside them), the next five start at the margin; the
    // right margin is the same throughout: one paragraph wrapped round a picture
    let mut rng = Rng(37);
    let mut l = Layout::new();
    let mut all = full_lines(&mut l, &mut rng, 180.0, 450.0, 100.0, 5, &LIGHT);
    all.extend(full_lines(&mut l, &mut rng, 70.0, 450.0, 100.0 + 9.6 * 5.0, 5, &LIGHT));
    assert_groups!(l, detect(&l.frags, &l.shapes), vec![sorted(all)]);
}

#[test]
fn right_aligned_lines_have_no_first_line_indent() {
    // four right-aligned lines: every left edge differs, the indent rule must keep out
    let mut l = Layout::new();
    let mut all = Vec::new();
    for (i, w) in [90.0f32, 70.0, 100.0, 60.0, 80.0].iter().enumerate() {
        all.push(l.frag(400.0 - w, 100.0 + 9.6 * i as f32, *w, &LIGHT, "an address line"));
    }
    let _ = &mut all;
    assert_eq!(detect(&l.frags, &l.shapes).len(), 1);
}

#[test]
fn two_left_edges_that_coincide_by_chance_are_no_common_edge_for_the_indent_rule() {
    // right-aligned lines starting at 70, 95, 70.4 and 120: lines 0 and 2 share a left edge by chance, so
    // line 1 sits "indented" between two flush lines, but two lines are no margin
    let mut rng = Rng(47);
    let mut l = Layout::new();
    for (i, x0) in [70.0f32, 95.0, 70.4, 120.0].iter().enumerate() {
        let w = fitted_words(&mut rng, 250.0 - x0, 8.0, 0.93);
        l.line(*x0, 250.0, 100.0 + 9.6 * i as f32, &LIGHT, &w, 1, true);
    }
    let blocks = detect(&l.frags, &l.shapes);
    assert_eq!(blocks.len(), 1, "{}", dump(&l.frags, &l.shapes, &blocks));
    let eager = off(|p| p.edge_lines = 2);
    assert!(detect_with(&l.frags, &l.shapes, &eager).len() > 1, "with two lines as an edge, line 1 is an indent");
}

#[test]
fn ragged_text_inset_on_the_left_whose_right_edge_wanders_a_little_is_no_quotation() {
    // eight ragged lines: four from x=50, four from x=80 (a figure on the left). The longest of the first
    // four ends at 250, the longest of the last four at 240: the right edge moved 10 pt (1.25 em) while
    // the left edge moved 30 pt, so the right edge did not move "with" the left one: not a quotation
    let mut rng = Rng(61);
    let mut l = Layout::new();
    let mut ids = Vec::new();
    let lines = [(50.0f32, 250.0f32), (50.0, 236.0), (50.0, 244.0), (50.0, 230.0), (80.0, 240.0), (80.0, 226.0), (80.0, 233.0), (80.0, 220.0)];
    for (i, (x0, x1)) in lines.iter().enumerate() {
        let w = fitted_words(&mut rng, x1 - x0, 8.0, 0.93);
        ids.extend(l.line(*x0, *x1, 100.0 + 9.6 * i as f32, &LIGHT, &w, 1, true));
    }
    let blocks = detect(&l.frags, &l.shapes);
    assert_groups!(l, blocks, vec![sorted(ids)], "one paragraph");
}

#[test]
fn a_block_quote_inset_on_both_sides_is_its_own_block() {
    // three paragraphs of the same style at one pitch and no blank lines, every line running to its
    // margin: the quotation is inset by 36 pt on BOTH sides; nothing else says where it starts
    let mut rng = Rng(41);
    let mut l = Layout::new();
    let a = full_lines(&mut l, &mut rng, 72.0, 472.0, 100.0, 4, &LIGHT);
    let q = full_lines(&mut l, &mut rng, 108.0, 436.0, 100.0 + 9.6 * 4.0, 4, &LIGHT);
    let b = full_lines(&mut l, &mut rng, 72.0, 472.0, 100.0 + 9.6 * 8.0, 4, &LIGHT);
    let blocks = detect(&l.frags, &l.shapes);
    assert_groups!(l, blocks, sorted_groups(vec![sorted(a), sorted(q.clone()), sorted(b)]));
    assert_eq!(block_of(&blocks, q[0]).starts_because, "margin");
    // without the margin rule the quote is only cut where its short lines (inset from the margin) say so
    assert_ne!(detect_with(&l.frags, &l.shapes, &off(|p| p.margin_rule = false)).len(), 3);
}

#[test]
fn a_footnote_sign_starts_a_footnote_and_stays_with_its_text() {
    // a paragraph whose last line runs to the margin, and one line under it a footnote that opens with a
    // dagger object of its own, at the same size and pitch: only the sign says a footnote starts
    let mut l = Layout::new();
    let mut rng = Rng(29);
    let para = full_lines(&mut l, &mut rng, 50.0, 250.0, 100.0, 3, &LIGHT);
    let base = 100.0 + 9.6 * 3.0;
    let mark = l.frag(50.0, base, 4.0, &LIGHT, "\u{2020}");
    let text = l.frag(62.0, base, 120.0, &LIGHT, "max wattage allowed vs temperature");
    let blocks = detect(&l.frags, &l.shapes);
    assert_groups!(l, blocks, sorted_groups(vec![sorted(para.clone()), vec![mark, text]]));
    assert_eq!(block_of(&blocks, mark).starts_because, "item");
    assert_eq!(detect_with(&l.frags, &l.shapes, &off(|p| p.list_items = false)).len(), 1);
}

#[test]
fn a_raised_numeral_at_the_start_of_a_line_starts_a_footnote() {
    // the line under a paragraph opens with a small raised "2" glued to its text, at the same pitch and
    // size: only the mark says a footnote starts (the dagger of the test above is a marker, this is a
    // script)
    let small = Look { size: 5.0, ..LIGHT.clone() };
    let mut l = Layout::new();
    let mut rng = Rng(51);
    let para = full_lines(&mut l, &mut rng, 50.0, 250.0, 100.0, 3, &LIGHT);
    let base = 100.0 + 9.6 * 3.0;
    let mark = l.frag(50.0, base - 3.0, 2.5, &small, "2");
    let text = l.frag(53.0, base, 140.0, &LIGHT, "max wattage allowed vs temperature");
    let blocks = detect(&l.frags, &l.shapes);
    assert_groups!(l, blocks, sorted_groups(vec![sorted(para), vec![mark, text]]));
    assert_eq!(block_of(&blocks, mark).starts_because, "item");
    assert_eq!(block_of(&blocks, mark).lines.len(), 1, "the mark is on the line of its text");
}

#[test]
fn super_and_subscript_marks_join_the_paragraph_they_belong_to() {
    let small = Look { size: 5.0, ..LIGHT.clone() };
    let mut rng = Rng(43);
    let mut l = Layout::new();
    let mut ids = full_lines(&mut l, &mut rng, 50.0, 250.0, 100.0, 5, &LIGHT);
    // a raised "1" glued to the end of line 2, a lowered "2" glued to the end of line 4
    let r2 = l.frags.iter().filter(|f| (f.baseline - 109.6).abs() < 0.1).map(|f| f.right).fold(0.0, f32::max);
    let r4 = l.frags.iter().filter(|f| (f.baseline - 128.8).abs() < 0.1).map(|f| f.right).fold(0.0, f32::max);
    let sup = l.frag(r2 + 0.4, 109.6 - 3.0, 2.5, &small, "1");
    let sub = l.frag(r4 + 0.4, 128.8 + 2.0, 2.5, &small, "2");
    ids.push(sup);
    ids.push(sub);
    let blocks = detect(&l.frags, &l.shapes);
    assert_groups!(l, blocks, vec![sorted(ids)]);
    let apart = detect_with(&l.frags, &l.shapes, &off(|p| p.script_marks = false));
    assert!(apart.len() >= 3, "without the rule each mark is a block of its own, got {}", apart.len());
}

// ------------------------------------------------------------------------------------------------
// drawn lines and boxes
// ------------------------------------------------------------------------------------------------

#[test]
fn ruled_table_rows_are_one_block_each() {
    // label at x=20, value at x=112, a rule under every row (11 pt apart), no vertical rules: the gap
    // between label and value (6 to 12 em) is no evidence that they belong apart
    let mut l = Layout::new();
    let mut rows: Vec<Vec<usize>> = Vec::new();
    l.hrule(17.0, 172.0, 92.0);
    for i in 0..6 {
        let base = 100.0 + 11.0 * i as f32;
        let ids = vec![l.frag(20.0, base, 50.0, &LIGHT, "Power Input"), l.frag(112.0, base, 40.0, &LIGHT, "40W max")];
        l.hrule(17.0, 172.0, base + 3.0);
        rows.push(sorted(ids));
    }
    let blocks = detect(&l.frags, &l.shapes);
    assert_groups!(l, blocks, sorted_groups(rows.clone()));
    let apart = detect_with(&l.frags, &l.shapes, &off(|p| p.ruled_bands = false));
    assert_eq!(apart.len(), 2 * rows.len(), "without the band rule label and value are separated by the gap");
}

#[test]
fn ruled_rows_whose_description_wraps_keep_their_numbers() {
    // a quotation: descriptions of one or two lines at x=72, three numbers 13 to 20 em away on the first
    // baseline, a hairline under every row. The rule under the first line of a two-line row is a line and
    // a half away from it
    let mut rng = Rng(65);
    let mut l = Layout::new();
    let mut rows = Vec::new();
    let mut base = 100.0f32;
    l.hrule(60.0, 540.0, base - 10.0);
    for lines in [1usize, 2, 1, 2] {
        let mut ids = Vec::new();
        for k in 0..lines {
            let w = words(&mut rng, 3);
            ids.extend(l.line(72.0, 222.0, base + 9.6 * k as f32, &LIGHT, &w, 1, false));
        }
        ids.push(l.frag(330.0, base, 20.0, &LIGHT, "12"));
        ids.push(l.frag(400.0, base, 30.0, &LIGHT, "30.00"));
        ids.push(l.frag(480.0, base, 40.0, &LIGHT, "360.00"));
        let last = base + 9.6 * (lines as f32 - 1.0);
        l.hrule(60.0, 540.0, last + 3.5);
        rows.push(sorted(ids));
        base = last + 3.5 + 11.0;
    }
    let blocks = detect(&l.frags, &l.shapes);
    assert_groups!(l, blocks, sorted_groups(rows.clone()), "one block per ruled row");
    // a cell of one line only: the numbers of the two-line rows are cut off their description
    let one_line = off(|p| p.cell_lines = 1);
    assert!(detect_with(&l.frags, &l.shapes, &one_line).len() > rows.len());
}

#[test]
fn a_vertical_rule_still_separates_the_cells_of_a_ruled_row() {
    let mut l = Layout::new();
    let mut left = Vec::new();
    let mut right = Vec::new();
    l.hrule(17.0, 172.0, 92.0);
    for i in 0..4 {
        let base = 100.0 + 11.0 * i as f32;
        left.push(l.frag(20.0, base, 50.0, &LIGHT, "Power Input"));
        right.push(l.frag(112.0, base, 40.0, &LIGHT, "40W max"));
        l.hrule(17.0, 172.0, base + 3.0);
    }
    l.vrule(100.0, 92.0, 139.0);
    // a grid: every cell is a block (the rules under the rows cut the columns into cells)
    let blocks = detect(&l.frags, &l.shapes);
    let cells: Vec<Vec<usize>> = left.iter().chain(right.iter()).map(|&o| vec![o]).collect();
    assert_groups!(l, blocks, sorted_groups(cells));
}

#[test]
fn two_lines_between_rules_are_one_cell_whatever_their_weights() {
    // a label line in bold and its value line in regular, a rule under the value, 21 pt from row to row
    let mut l = Layout::new();
    let mut cells: Vec<Vec<usize>> = Vec::new();
    for i in 0..4 {
        let y = 100.0 + 21.0 * i as f32;
        let mut ids = l.line(20.0, 160.0, y, &BOLD, &["Power", "Input:"], 1, false);
        ids.extend(l.line(20.0, 160.0, y + 9.6, &LIGHT, &["40W", "max"], 1, false));
        l.hrule(17.0, 172.0, y + 13.0);
        cells.push(sorted(ids));
    }
    let blocks = detect(&l.frags, &l.shapes);
    assert_groups!(l, blocks, sorted_groups(cells.clone()));
    let apart = detect_with(&l.frags, &l.shapes, &off(|p| p.ruled_bands = false));
    assert_eq!(apart.len(), 2 * cells.len(), "without the cell rule the weight splits label and value");
}

#[test]
fn a_chart_axis_under_a_label_is_not_a_ruled_cell() {
    // a small label above a row of tick labels with an axis line under them: different sizes, so the
    // lines stay two blocks although a rule lies right under the second
    let small = Look { size: 6.0, ..LIGHT.clone() };
    let mut l = Layout::new();
    let a = l.frag(30.0, 100.0, 4.0, &small, "7");
    let ticks = l.line(30.0, 160.0, 114.7, &LIGHT, &["30 35 40 45"], 1, false);
    l.hrule(25.0, 165.0, 117.0);
    let blocks = detect(&l.frags, &l.shapes);
    assert_groups!(l, blocks, sorted_groups(vec![vec![a], sorted(ticks)]));
}

#[test]
fn a_hyperlink_underline_is_no_rule_but_a_table_rule_and_a_heading_underline_are() {
    // a hairline under line 1 of a five-line paragraph: it covers `share` of the line and sits `drop` pt
    // under its baseline (1.2 pt = 0.15 em hugs the glyphs, 2.6 pt = 0.33 em is a table rule's distance)
    let cases = [(0.65f32, 1.2f32, false, "a hyperlink underlines a phrase"), (1.0, 1.2, true, "a heading underline spans the line"), (0.65, 2.6, true, "a table rule at its usual distance")];
    for (share, drop, cuts, what) in cases {
        let mut rng = Rng(43);
        let mut l = Layout::new();
        full_lines(&mut l, &mut rng, 50.0, 250.0, 100.0, 5, &LIGHT);
        l.hrule(50.0, 50.0 + 200.0 * share, 109.6 + drop);
        let blocks = detect(&l.frags, &l.shapes);
        assert_eq!(blocks.len() > 1, cuts, "{what}\n{}", dump(&l.frags, &l.shapes, &blocks));
        if cuts {
            assert!(blocks.iter().any(|b| b.starts_because == "rule"), "{what}");
        }
        if !cuts {
            // without the distinction the long underline is a rule like any other
            let plain = off(|p| p.underline_zone = 0.0);
            assert!(detect_with(&l.frags, &l.shapes, &plain).len() > 1, "{what}: the underline zone is what keeps the paragraph whole");
        }
    }
}

#[test]
fn a_short_rule_between_two_lines_is_no_separator() {
    // a hairline 2.6 pt under line 1 that covers a fifth of it: a word's underline set low, a dash
    let mut rng = Rng(59);
    let mut l = Layout::new();
    full_lines(&mut l, &mut rng, 50.0, 250.0, 100.0, 5, &LIGHT);
    l.hrule(50.0, 90.0, 109.6 + 2.6);
    assert_eq!(detect(&l.frags, &l.shapes).len(), 1);
    let any = off(|p| p.rule_cover = 0.1);
    assert!(detect_with(&l.frags, &l.shapes, &any).len() > 1, "a rule over a fifth of the lines is a rule where any touch counts");
}

#[test]
fn a_drawn_box_around_one_side_of_a_gap_cuts_the_line() {
    let mut l = Layout::new();
    let mut left = Vec::new();
    let mut right = Vec::new();
    for i in 0..2 {
        let base = 100.0 + 9.6 * i as f32;
        left.push(l.frag(50.0, base, 80.0, &LIGHT, "plain text"));
        right.push(l.frag(137.2, base, 80.0, &LIGHT, "boxed text"));
    }
    // a bordered cell around the right-hand text of every row, with room around it
    l.shape(135.0, 92.0, 222.0, 112.0);
    let blocks = detect(&l.frags, &l.shapes);
    assert_groups!(l, blocks, sorted_groups(vec![sorted(left), sorted(right)]));
    let joined = detect_with(&l.frags, &l.shapes, &off(|p| p.boxes = false));
    assert_eq!(joined.len(), 1);
}

#[test]
fn a_vertical_rule_between_two_cells_cuts_a_line_that_a_gap_alone_would_not() {
    // two cells side by side with a 0.9 em gap: a column border drawn in it
    let mut l = Layout::new();
    let mut left = Vec::new();
    let mut right = Vec::new();
    for i in 0..2 {
        let base = 100.0 + 9.6 * i as f32;
        left.push(l.frag(50.0, base, 80.0, &LIGHT, "left cell text"));
        right.push(l.frag(137.2, base, 80.0, &LIGHT, "right cell text"));
    }
    l.vrule(133.6, 92.0, 112.0);
    let blocks = detect(&l.frags, &l.shapes);
    assert_groups!(l, blocks, sorted_groups(vec![sorted(left), sorted(right)]));
    let joined = detect_with(&l.frags, &l.shapes, &off(|p| p.rules = false));
    assert_eq!(joined.len(), 1, "without the rule the gap is a word gap");
}

#[test]
fn a_lone_symbol_in_a_gutter_does_not_join_the_two_columns() {
    // two justified columns and an asterisk object in the middle of the 12 pt gutter on one line, 4.5 pt
    // from each column: on that row the three look like one line with tight gaps
    let mut rng = Rng(47);
    let mut l = Layout::new();
    let a = column(&mut l, &mut rng, 50.0, 250.0, 100.0, 12, 0.93, &[]);
    let b = column(&mut l, &mut rng, 262.0, 462.0, 100.0, 12, 0.93, &[]);
    let star = l.frag(253.0, 100.0 + 9.6 * 5.0, 5.0, &LIGHT, "*");
    let blocks = detect(&l.frags, &l.shapes);
    assert_eq!(block_of(&blocks, a[0][0]).objects().len(), flat(&a).len());
    assert_eq!(block_of(&blocks, b[0][0]).objects().len(), flat(&b).len());
    assert_eq!(block_of(&blocks, star).objects(), vec![star], "the mark stands alone");
    let joined = detect_with(&l.frags, &l.shapes, &off(|p| p.stray_marks = false));
    assert!(block_of(&joined, a[0][0]).objects().len() != flat(&a).len() || block_of(&joined, b[0][0]).objects().len() != flat(&b).len(), "without the rule the columns are mixed up");
}

// ------------------------------------------------------------------------------------------------
// outlined words
// ------------------------------------------------------------------------------------------------

/// A 6-line paragraph whose third line has a 5 em hole (a word drawn as outlines).
fn paragraph_with_a_hole() -> (Layout, Vec<usize>, usize) {
    let mut rng = Rng(31);
    let mut l = Layout::new();
    let mut ids = Vec::new();
    let mut shape = 0;
    for i in 0..6 {
        let base = 100.0 + 9.6 * i as f32;
        if i == 2 {
            // text up to x = 120, a 40 pt hole (5 em) holding the outlined word, text again from 168
            ids.push(l.frag(50.0, base, 70.0, &LIGHT, "text of the line"));
            // taller than the text boxes, so the union has to grow to hold it
            shape = l.shape(125.0, base - 7.0, 163.0, base + 2.4);
            ids.push(l.frag(168.0, base, 82.0, &LIGHT, "goes on to the margin"));
        } else {
            let w = fitted_words(&mut rng, 200.0, 8.0, 0.93);
            ids.extend(l.line(50.0, 250.0, base, &LIGHT, &w, 1, true));
        }
    }
    (l, ids, shape)
}

#[test]
fn an_outlined_word_bridges_the_hole_it_leaves_in_its_line() {
    let (l, ids, shape) = paragraph_with_a_hole();
    let blocks = detect(&l.frags, &l.shapes);
    assert_groups!(l, blocks, vec![sorted(ids)], "one paragraph");
    let b = &blocks[0];
    assert_eq!(b.lines.len(), 6);
    assert_eq!(b.lines[2].outlined, vec![shape]);
    assert!(b.lines.iter().enumerate().all(|(i, ln)| i == 2 || ln.outlined.is_empty()));
    assert!(!b.objects().contains(&shape), "an outlined word is not a text object");
    // the line's rectangle is the union of text and outlined word
    let ln = &b.lines[2];
    let sh = l.shapes.iter().find(|s| s.object == shape).unwrap();
    assert!(ln.top <= sh.top + 1e-3 && ln.bottom >= sh.bottom - 1e-3 && ln.left <= sh.left && ln.right >= sh.right);
    assert!(b.top <= sh.top + 1e-3);
    // without the bridge the hole cuts the line and the paragraph falls apart
    let apart = detect_with(&l.frags, &l.shapes, &off(|p| p.outlined = false));
    assert!(apart.len() > 1, "without the bridge the hole splits the paragraph, got {} block", apart.len());
}

#[test]
fn a_word_sized_path_with_no_text_near_it_is_not_an_outlined_word() {
    let (mut l, ids, shape) = paragraph_with_a_hole();
    let stray = l.shape(300.0, 400.0, 330.0, 407.6); // in empty space
    let blocks = detect(&l.frags, &l.shapes);
    assert_groups!(l, blocks, vec![sorted(ids)]);
    assert!(blocks.iter().all(|b| b.lines.iter().all(|ln| !ln.outlined.contains(&stray))));
    assert!(blocks[0].lines[2].outlined == vec![shape]);
}

#[test]
fn a_word_sized_path_lying_on_text_is_a_cell_not_an_outlined_word() {
    let (mut l, ids, shape) = paragraph_with_a_hole();
    // a text-sized box drawn over a word of line 4 (underlay of a highlighted word)
    let word = l.frags.iter().find(|f| (f.baseline - (100.0 + 9.6 * 4.0)).abs() < 0.1).unwrap().clone();
    let cell = l.shape(word.left, word.top + 1.0, word.right, word.bottom - 0.5);
    let blocks = detect(&l.frags, &l.shapes);
    assert_groups!(l, blocks, vec![sorted(ids.clone())]);
    assert!(blocks.iter().all(|b| b.lines.iter().all(|ln| !ln.outlined.contains(&cell))), "text lies on it: no outlined word");
    assert_eq!(blocks[0].lines[2].outlined, vec![shape]);
    // with both cell tests off (the share of the path under text, a text object inside the path) it would be
    // bridged into the line
    let bridged = detect_with(&l.frags, &l.shapes, &off(|p| {
        p.cover_max = 10.0;
        p.cell_inside = 10.0;
    }));
    assert!(bridged.iter().any(|b| b.lines.iter().any(|ln| ln.outlined.contains(&cell))));
}

#[test]
fn a_whole_outlined_line_inside_a_paragraph_is_a_line_without_text_objects() {
    let mut rng = Rng(37);
    let mut l = Layout::new();
    let mut ids = Vec::new();
    let mut shapes = Vec::new();
    for i in 0..6 {
        let base = 100.0 + 9.6 * i as f32;
        if i == 3 {
            // five words of the line drawn as outlines, justified over the full width
            let mut x = 50.0;
            for w in [34.0, 22.0, 41.0, 30.0, 37.0] {
                shapes.push(l.shape(x, base - 6.0, x + w, base + 1.6));
                x += w + 9.0;
            }
        } else {
            let w = fitted_words(&mut rng, 200.0, 8.0, 0.93);
            ids.extend(l.line(50.0, 250.0, base, &LIGHT, &w, 1, i < 5));
        }
    }
    let blocks = detect(&l.frags, &l.shapes);
    assert_groups!(l, blocks, vec![sorted(ids)]);
    let b = &blocks[0];
    assert_eq!(b.lines.len(), 6);
    assert!(b.lines[3].objects.is_empty(), "no text object on that line");
    assert_eq!(b.lines[3].outlined, shapes, "the five outlined words, left to right");
}

// ------------------------------------------------------------------------------------------------
// the contract's small print
// ------------------------------------------------------------------------------------------------

#[test]
fn block_rects_are_unions_and_every_block_has_a_reason() {
    let l = sample_page();
    let blocks = detect(&l.frags, &l.shapes);
    let valid = ["start", "column", "size", "style", "margin", "rule", "pitch", "item", "indent", "short-line", "rotated", "unplaced"];
    for b in &blocks {
        assert!(valid.contains(&b.starts_because), "unknown reason {}", b.starts_because);
        let (mut l0, mut t0, mut r0, mut b0) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for ln in &b.lines {
            l0 = l0.min(ln.left);
            t0 = t0.min(ln.top);
            r0 = r0.max(ln.right);
            b0 = b0.max(ln.bottom);
            for o in &ln.objects {
                let f = l.frags.iter().find(|f| f.object == *o).unwrap();
                assert!(ln.left <= f.left && ln.right >= f.right && ln.top <= f.top && ln.bottom >= f.bottom, "line must enclose its members");
            }
        }
        assert_eq!((b.left, b.top, b.right, b.bottom), (l0, t0, r0, b0));
    }
}

#[test]
fn contains_looks_at_text_objects_only() {
    let (l, ids, shape) = paragraph_with_a_hole();
    let blocks = detect(&l.frags, &l.shapes);
    assert!(blocks[0].contains(ids[0]));
    assert!(!blocks[0].contains(shape));
    assert_eq!(block_index_of(&blocks, shape), None);
    assert_eq!(blocks[0].objects().len(), ids.len());
}

#[test]
fn the_font_style_rule_in_one_table() {
    // (stem a, name a, stem b, name b, same style?)
    let cases: &[(Option<u16>, &str, Option<u16>, &str, bool)] = &[
        (Some(51), "Montserrat-Thin", Some(74), "Montserrat-Thin", false),
        (Some(74), "Montserrat-Thin", Some(100), "Montserrat-Thin", false),
        (Some(100), "Montserrat-Thin", Some(124), "Montserrat-Thin", false),
        (Some(124), "Montserrat-Thin", Some(198), "Montserrat-Thin", false),
        (Some(20), "Montserrat-Thin", Some(51), "Montserrat-Thin", false),
        (Some(198), "Montserrat-Thin", Some(190), "Montserrat-Thin", true), // the I against the l of one font
        (Some(124), "Montserrat-Thin", Some(125), "Montserrat-Thin", true),
        (Some(74), "ABCDEF+Arial", Some(74), "Arial", true), // one font, two subset tags
        (Some(74), "Arial", Some(74), "Calibri", false),     // same weight, another typeface
        (Some(84), "Calibri", Some(74), "Montserrat-Regular", false),
        (None, "Arial", Some(74), "Arial", true),
        (None, "Arial", Some(74), "Calibri", false),
        (None, "", Some(74), "Calibri", true), // no evidence either way
        (None, "", None, "", true),
        (Some(51), "A", Some(198), "A", false), // the weight beats equal names
    ];
    for &(sa, fa, sb, fb, same) in cases {
        assert_eq!(same_font_style(sa, fa, sb, fb, 12), same, "{sa:?} {fa} vs {sb:?} {fb}");
        assert_eq!(same_font_style(sb, fb, sa, fa, 12), same, "symmetric: {sb:?} {fb} vs {sa:?} {fa}");
    }
}

#[test]
fn every_switch_is_on_by_default_and_the_default_is_what_detect_uses() {
    let p = Params::default();
    for flag in [
        p.corridor, p.outlined, p.rules, p.boxes, p.size_rule, p.style_rule, p.pitch_rule, p.list_items, p.indent_rule, p.margin_rule, p.short_lines,
        p.script_marks, p.marker_merge, p.stray_marks, p.ruled_bands, p.unknown_stem_same_face, p.mixed_looks,
    ] {
        assert!(flag);
    }
    let l = sample_page();
    assert_eq!(detect(&l.frags, &l.shapes), detect_with(&l.frags, &l.shapes, &Params::default()));
}
