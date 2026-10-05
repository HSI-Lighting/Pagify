//! Properties of the block detector that must hold on any page: scale invariance (no absolute point
//! threshold), independence of the input order, never panicking on hostile input, speed.

mod common;

use common::*;
use pagify_shell::blocks::*;
use std::time::Instant;

fn assert_same_partition(a: &[Block], b: &[Block], what: &str) {
    assert_eq!(canonical(a), canonical(b), "{what}: the blocks differ");
}

// ------------------------------------------------------------------------------------------------
// scale invariance
// ------------------------------------------------------------------------------------------------

#[test]
fn scaling_a_datasheet_page_changes_no_block() {
    for pg in 1..=3 {
        let page = load_page(pg);
        let base = detect(&real_frags(&page), &page.shapes);
        assert!(base.len() > 100);
        for k in [0.5f32, 2.0, 3.7] {
            let (f, s) = scaled(&real_frags(&page), &page.shapes, k);
            let got = detect(&f, &s);
            assert_same_partition(&base, &got, &format!("datasheet page {pg} x{k}"));
            // the reasons a block began must not move either
            let why = |b: &[Block]| {
                let mut v: Vec<(Vec<usize>, &str)> = b.iter().map(|x| (x.objects(), x.starts_because)).collect();
                v.sort();
                v
            };
            assert_eq!(why(&base), why(&got), "datasheet page {pg} x{k}: starts_because moved");
            // outlined members too
            let outl = |b: &[Block]| {
                let mut v: Vec<Vec<usize>> = b.iter().flat_map(|x| x.lines.iter().map(|l| l.outlined.clone())).filter(|o| !o.is_empty()).collect();
                v.sort();
                v
            };
            assert_eq!(outl(&base), outl(&got), "datasheet page {pg} x{k}: outlined members moved");
        }
    }
}

#[test]
fn scaling_a_synthetic_page_changes_no_block() {
    let l = sample_page();
    let base = detect(&l.frags, &l.shapes);
    assert!(base.len() >= 12, "the sample page should have many blocks, got {}", base.len());
    for k in [0.5f32, 2.0, 3.7, 0.1, 10.0] {
        let (f, s) = scaled(&l.frags, &l.shapes, k);
        assert_same_partition(&base, &detect(&f, &s), &format!("sample page x{k}"));
    }
}

// ------------------------------------------------------------------------------------------------
// determinism: the order of the input is irrelevant
// ------------------------------------------------------------------------------------------------

#[test]
fn shuffling_the_input_gives_identical_blocks() {
    let mut pages: Vec<(String, Vec<Frag>, Vec<Shape>)> = (1..=3)
        .map(|pg| {
            let p = load_page(pg);
            (format!("datasheet {pg}"), real_frags(&p), p.shapes)
        })
        .collect();
    let l = sample_page();
    pages.push(("sample".to_string(), l.frags, l.shapes));
    for (name, frags, shapes) in pages {
        let base = detect(&frags, &shapes);
        for seed in 1..=4u64 {
            let mut rng = Rng(0xABCD_EF01 + seed * 7919);
            let (mut f, mut s) = (frags.clone(), shapes.clone());
            rng.shuffle(&mut f);
            rng.shuffle(&mut s);
            let got = detect(&f, &s);
            assert_eq!(base, got, "{name}: shuffle {seed} changed the result (exact equality, order included)");
        }
    }
}

#[test]
fn the_output_is_ordered_top_to_bottom_then_left_to_right() {
    let page = load_page(1);
    let blocks = detect(&real_frags(&page), &page.shapes);
    for w in blocks.windows(2) {
        let (a, b) = (&w[0].lines[0], &w[1].lines[0]);
        assert!(a.top < b.top || (a.top == b.top && a.left <= b.left), "blocks out of order: {:?} then {:?}", (a.top, a.left), (b.top, b.left));
    }
    for b in &blocks {
        for w in b.lines.windows(2) {
            assert!(w[0].baseline < w[1].baseline, "lines of a block must run top to bottom");
        }
        for l in &b.lines {
            let lefts: Vec<f32> = l.objects.iter().map(|o| page.frags.iter().find(|f| f.object == *o).unwrap().left).collect();
            assert!(lefts.windows(2).all(|w| w[0] <= w[1]), "objects of a line must run left to right: {lefts:?}");
        }
    }
}

// ------------------------------------------------------------------------------------------------
// robustness
// ------------------------------------------------------------------------------------------------

fn one(object: usize, left: f32, base: f32, text: &str) -> Frag {
    Frag { object, left, top: base - 6.4, right: left + 20.0, bottom: base + 2.0, baseline: base, size: 8.0, font: 0, stem: None, face: String::new(), rgb: [0; 3], text: text.to_string(), rotated: false }
}

#[test]
fn empty_and_single_inputs() {
    assert!(detect(&[], &[]).is_empty());
    let f = one(7, 10.0, 50.0, "hello");
    let b = detect(&[f.clone()], &[]);
    assert_eq!(b.len(), 1);
    assert_eq!(b[0].objects(), vec![7]);
    assert_eq!(b[0].starts_because, "start");
    // shapes only: no text, no blocks
    let s = Shape { object: 1, left: 0.0, top: 0.0, right: 10.0, bottom: 10.0, depth: 0 };
    assert!(detect(&[], &[s]).is_empty());
}

#[test]
fn duplicate_fragments_never_double_count() {
    // identical rects, different objects: both stay, never lost
    let a = one(1, 10.0, 50.0, "same");
    let b = one(2, 10.0, 50.0, "same");
    let blocks = detect(&[a.clone(), b.clone()], &[]);
    assert_partition(&[a.clone(), b.clone()], &blocks);
    // the same object id twice: the second is dropped, deterministically, whatever the order
    let mut c = one(1, 10.0, 50.0, "same");
    c.text = "other".to_string();
    let x = detect(&[a.clone(), c.clone()], &[]);
    let y = detect(&[c.clone(), a.clone()], &[]);
    assert_eq!(x, y);
    assert_eq!(x.iter().map(|b| b.objects().len()).sum::<usize>(), 1);
}

#[test]
fn hostile_numbers_never_panic_and_every_fragment_is_somewhere() {
    let bad = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.0, -1.0, 1e30, -1e30, 1e-30, f32::MAX, f32::MIN, f32::MIN_POSITIVE];
    let mut frags: Vec<Frag> = (0..30).map(|i| one(1000 + i, 10.0 + 22.0 * (i % 5) as f32, 50.0 + 9.6 * (i / 5) as f32, "word")).collect();
    let mut id = 1;
    for &v in &bad {
        for field in 0..7 {
            let mut f = one(id, 30.0, 70.0, "x");
            id += 1;
            match field {
                0 => f.left = v,
                1 => f.top = v,
                2 => f.right = v,
                3 => f.bottom = v,
                4 => f.baseline = v,
                5 => f.size = v,
                _ => {
                    f.left = v;
                    f.right = v;
                    f.top = v;
                    f.bottom = v;
                    f.baseline = v;
                    f.size = v;
                }
            }
            frags.push(f);
        }
    }
    let shapes: Vec<Shape> = bad
        .iter()
        .flat_map(|&v| vec![Shape { object: 5000, left: v, top: v, right: v, bottom: v, depth: 0 }, Shape { object: 5001, left: 0.0, top: 0.0, right: v, bottom: v, depth: 0 }])
        .collect();
    let blocks = detect(&frags, &shapes);
    assert_partition(&frags, &blocks);
}

#[test]
fn unplaceable_fragments_take_no_part_in_the_layout() {
    // NaN, infinite or non-positive: such a fragment is a block of its own and changes nothing else
    let grid: Vec<Frag> = (0..30).map(|i| one(1000 + i, 10.0 + 22.0 * (i % 5) as f32, 50.0 + 9.6 * (i / 5) as f32, "word")).collect();
    let base = detect(&grid, &[]);
    let mut frags = grid.clone();
    let mut id = 1;
    for v in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        for field in 0..6 {
            let mut f = one(id, 30.0, 70.0, "x");
            id += 1;
            match field {
                0 => f.left = v,
                1 => f.top = v,
                2 => f.right = v,
                3 => f.bottom = v,
                4 => f.baseline = v,
                _ => f.size = v,
            }
            frags.push(f);
        }
    }
    for v in [0.0f32, -1.0, -1e30] {
        let mut f = one(id, 30.0, 70.0, "x");
        id += 1;
        f.size = v;
        frags.push(f);
    }
    let blocks = detect(&frags, &[]);
    assert_partition(&frags, &blocks);
    let unplaced = blocks.iter().filter(|b| b.starts_because == "unplaced").count();
    assert_eq!(unplaced, id - 1, "every unplaceable fragment is a block of its own");
    let rest: Vec<Vec<usize>> = { let mut v: Vec<Vec<usize>> = blocks.iter().filter(|b| b.starts_because != "unplaced").map(|b| b.objects()).collect(); v.sort(); v };
    assert_eq!(canonical(&base), rest);
}

#[test]
fn top_below_bottom_is_normalised() {
    let l = sample_page();
    let base = detect(&l.frags, &l.shapes);
    let flipped: Vec<Frag> = l.frags.iter().map(|f| Frag { top: f.bottom, bottom: f.top, left: f.right, right: f.left, ..f.clone() }).collect();
    let fshapes: Vec<Shape> = l.shapes.iter().map(|s| Shape { top: s.bottom, bottom: s.top, left: s.right, right: s.left, ..*s }).collect();
    assert_same_partition(&base, &detect(&flipped, &fshapes), "flipped rects");
    // normalised means the same numbers, not only the same groups: every rectangle of every block and line
    assert_eq!(base, detect(&flipped, &fshapes), "flipped rects give the same blocks, rectangles included");
}

#[test]
fn rotated_fragments_are_one_line_blocks_of_their_own_and_change_nothing_else() {
    let l = sample_page();
    let base = detect(&l.frags, &l.shapes);
    // rotated text right in the middle of column 1, overlapping real lines
    let mut frags = l.frags.clone();
    let mut ids = Vec::new();
    for i in 0..5 {
        let mut f = one(9000 + i, 60.0 + 3.0 * i as f32, 130.0 + 4.0 * i as f32, "166mm");
        f.rotated = true;
        ids.push(f.object);
        frags.push(f);
    }
    let blocks = detect(&frags, &l.shapes);
    assert_partition(&frags, &blocks);
    for id in &ids {
        let b = &blocks[block_index_of(&blocks, *id).unwrap()];
        assert_eq!(b.objects(), vec![*id], "a rotated fragment is alone in its block");
        assert_eq!(b.lines.len(), 1);
        assert_eq!(b.starts_because, "rotated");
    }
    let without: Vec<&Block> = blocks.iter().filter(|b| b.starts_because != "rotated").collect();
    let mut got: Vec<Vec<usize>> = without.iter().map(|b| b.objects()).collect();
    got.sort();
    assert_eq!(canonical(&base), got, "rotated text must not take part in the layout");
}

#[test]
fn blank_text_is_tolerated_and_takes_no_part_in_any_decision() {
    // the integration layer does not pass blank-text objects, but if it did: a twin of a word at the
    // same rectangle (the faux-bold second layer of a heading), a space object between two words, and
    // a blank object in the middle of nowhere are each in exactly one block, and the grouping of every
    // other object is the grouping without them
    let l = sample_page();
    let base = detect(&l.frags, &l.shapes);
    let mut frags = l.frags.clone();
    let mut blanks = Vec::new();
    for (i, f) in l.frags.iter().enumerate().step_by(7) {
        let mut twin = f.clone();
        twin.object = 50_000 + i;
        twin.text = String::new();
        blanks.push(twin.object);
        frags.push(twin);
        let mut space = f.clone();
        space.object = 60_000 + i;
        space.text = " ".to_string();
        // a space object inside the rectangle of a word (it adds no ink to the line)
        space.left = 0.5 * (f.left + f.right);
        space.right = space.left + 0.25 * f.size;
        blanks.push(space.object);
        frags.push(space);
    }
    let mut lone = l.frags[0].clone();
    lone.object = 70_000;
    lone.text = "  ".to_string();
    lone.left = 600.0;
    lone.right = 604.0;
    lone.top = 900.0;
    lone.bottom = 908.0;
    lone.baseline = 906.0;
    blanks.push(lone.object);
    frags.push(lone);
    let blocks = detect(&frags, &l.shapes);
    assert_partition(&frags, &blocks);
    let strip = |bs: &[Block]| {
        let mut v: Vec<Vec<usize>> = bs.iter().map(|b| b.objects().into_iter().filter(|o| !blanks.contains(o)).collect::<Vec<usize>>()).filter(|g| !g.is_empty()).collect();
        v.sort();
        v
    };
    assert_eq!(strip(&blocks), canonical(&base), "blank objects changed the grouping of the real ones");
}

#[test]
fn hostile_shapes_alone_are_harmless() {
    let l = sample_page();
    let base = detect(&l.frags, &l.shapes);
    let mut shapes = l.shapes.clone();
    for (i, v) in [f32::NAN, f32::INFINITY, -5.0, 1e30].iter().enumerate() {
        shapes.push(Shape { object: 7000 + i, left: *v, top: 100.0, right: 200.0, bottom: *v, depth: 0 });
        shapes.push(Shape { object: 7100 + i, left: 100.0, top: *v, right: *v, bottom: 120.0, depth: 3 });
    }
    // shapes below depth 0 are ignored altogether
    for i in 0..50 {
        shapes.push(Shape { object: 7200 + i, left: 40.0, top: 100.0 + i as f32, right: 250.0, bottom: 100.2 + i as f32, depth: 1 });
    }
    let got = detect(&l.frags, &shapes);
    assert_same_partition(&base, &got, "hostile and nested shapes");
}

// ------------------------------------------------------------------------------------------------
// speed
// ------------------------------------------------------------------------------------------------

/// `cols` columns of `lines` lines of 3 fragments each: a huge page, rows of 3*cols fragments.
fn big_page(cols: usize, lines: usize) -> Layout {
    let mut rng = Rng(77);
    let mut l = Layout::new();
    for c in 0..cols {
        let x0 = 20.0 + c as f32 * 112.0;
        for i in 0..lines {
            let w = fitted_words(&mut rng, 100.0, 8.0, 0.9);
            let ws: Vec<&str> = if w.len() < 3 { vec!["a", "b", "c"] } else { w };
            l.line(x0, x0 + 100.0, 50.0 + 9.6 * i as f32, &LIGHT, &ws, (ws.len() + 2) / 3, true);
        }
    }
    l
}

#[test]
fn thirty_thousand_fragments_in_two_hundred_columns_take_well_under_a_second() {
    let l = big_page(200, 58);
    assert!(l.frags.len() >= 29_000 && l.frags.len() <= 32_000, "got {} fragments", l.frags.len());
    let t = Instant::now();
    let blocks = detect(&l.frags, &l.shapes);
    let dt = t.elapsed();
    println!("{} fragments in 200 columns: {} blocks in {:.1} ms", l.frags.len(), blocks.len(), dt.as_secs_f64() * 1e3);
    assert_partition(&l.frags, &blocks);
    assert!(blocks.len() >= 150, "the columns must come out as blocks, got {}", blocks.len());
    assert!(dt.as_secs_f64() < 0.9, "30,000 fragments took {:?}", dt);
}

#[test]
fn a_datasheet_page_takes_a_few_milliseconds() {
    for pg in 1..=3 {
        let page = load_page(pg);
        let _warm = detect(&real_frags(&page), &page.shapes);
        let t = Instant::now();
        let n = 20;
        for _ in 0..n {
            let _ = detect(&real_frags(&page), &page.shapes);
        }
        let ms = t.elapsed().as_secs_f64() * 1e3 / n as f64;
        println!("datasheet page {pg}: {} fragments, {} shapes: {:.2} ms per detect", page.frags.len(), page.shapes.len(), ms);
        assert!(ms < 50.0, "page {pg} took {ms:.1} ms");
    }
}

// ------------------------------------------------------------------------------------------------
// fuzz: 200 random hostile pages
// ------------------------------------------------------------------------------------------------

/// Hostile input (NaN, infinities, zero and negative sizes, inverted and identical rectangles, empty
/// and odd text, rotated text, shapes of every size and depth) must neither panic nor lose an object.
#[test]
fn two_hundred_random_hostile_pages_every_object_lands_in_exactly_one_block() {
    let mut rng = Rng(99);
    let texts = ["a", "", "1.", "word.", "\u{2}", " ", "\u{2022}", "*", "(a)", "-", "fi"];
    for round in 0..200 {
        let n = 1 + rng.below(60);
        let mut weird = |x: f32, r: f32| -> f32 {
            if r < 0.03 {
                f32::NAN
            } else if r < 0.05 {
                f32::INFINITY
            } else if r < 0.07 {
                -x
            } else if r < 0.09 {
                0.0
            } else if r < 0.10 {
                f32::NEG_INFINITY
            } else {
                x
            }
        };
        let mut frags: Vec<Frag> = Vec::new();
        for k in 0..n {
            let (x, y, s) = (rng.unit() * 300.0, rng.unit() * 100.0, 4.0 + rng.unit() * 12.0);
            let mut r = [rng.unit(), rng.unit(), rng.unit(), rng.unit(), rng.unit(), rng.unit()];
            if rng.unit() < 0.1 {
                r = [0.5; 6]; // the same numbers in every field: identical rectangles
            }
            frags.push(Frag {
                object: k,
                left: weird(x, r[0]),
                top: weird(y - 6.0, r[1]),
                right: weird(x + 5.0 + rng.unit() * 40.0, r[2]),
                bottom: weird(y + 2.0, r[3]),
                baseline: weird(y, r[4]),
                size: weird(s, r[5]),
                font: rng.below(3) as u32,
                stem: if rng.unit() < 0.5 { None } else { Some(rng.below(200) as u16) },
                face: ["", "A", "B"][rng.below(3)].to_string(),
                rgb: [rng.below(256) as u8; 3],
                text: texts[rng.below(texts.len())].to_string(),
                rotated: rng.unit() < 0.1,
            });
        }
        let shapes: Vec<Shape> = (0..20)
            .map(|k| {
                let (x, y) = (rng.unit() * 300.0, rng.unit() * 100.0);
                Shape {
                    object: 1000 + k,
                    left: weird(x, rng.unit()),
                    top: weird(y, rng.unit()),
                    right: weird(x + rng.unit() * 60.0, rng.unit()),
                    bottom: weird(y + rng.unit() * 12.0, rng.unit()),
                    depth: rng.below(2) as u32,
                }
            })
            .collect();
        let blocks = detect(&frags, &shapes);
        let mut seen = vec![0u8; n];
        for b in &blocks {
            for o in b.objects() {
                seen[o] += 1;
            }
        }
        assert!(seen.iter().all(|&c| c == 1), "round {round}: an object was lost or counted twice: {seen:?}");
        // the same page in another order: the same blocks
        let mut f2 = frags.clone();
        rng.shuffle(&mut f2);
        assert_eq!(blocks, detect(&f2, &shapes), "round {round}: the order of the input changed the result");
    }
}

/// Every fragment cut into its characters (about 4,600 fragments per page, as after a split of the
/// text objects): the paragraphs must still be the paragraphs.
#[test]
fn a_page_split_into_single_characters_keeps_its_paragraphs() {
    let mut total = 0;
    let mut same = 0;
    for pg in 1..=3 {
        let page = load_page(pg);
        let real = real_frags(&page);
        let base = detect(&real, &page.shapes);
        let mut split: Vec<Frag> = Vec::new();
        let mut origin: std::collections::BTreeMap<usize, usize> = std::collections::BTreeMap::new();
        for f in &real {
            let chars: Vec<char> = f.text.chars().collect();
            let cw = (f.right - f.left) / chars.len().max(1) as f32;
            for (i, c) in chars.iter().enumerate() {
                let mut g = f.clone();
                g.object = f.object * 1000 + i;
                g.text = c.to_string();
                g.left = f.left + i as f32 * cw;
                g.right = f.left + (i + 1) as f32 * cw;
                origin.insert(g.object, f.object);
                split.push(g);
            }
        }
        let blocks = detect(&split, &page.shapes);
        let back = |bs: &[Block]| {
            let mut v: Vec<Vec<usize>> = bs
                .iter()
                .map(|b| {
                    let mut o: Vec<usize> = b.objects().iter().map(|x| origin.get(x).copied().unwrap_or(*x)).collect();
                    o.sort();
                    o.dedup();
                    o
                })
                .collect();
            v.sort();
            v
        };
        let (a, b) = (canonical(&base), back(&blocks));
        let a_set: std::collections::BTreeSet<&Vec<usize>> = a.iter().collect();
        total += b.len();
        same += b.iter().filter(|g| a_set.contains(g)).count();
        // every paragraph of the labelers (3 or more lines) is still one block
        for (_who, truth) in &page.labels {
            for t in truth.iter().filter(|t| t.kind == "paragraph") {
                let ids: Vec<usize> = t.objects.iter().copied().filter(|o| real.iter().any(|f| f.object == *o)).collect();
                let first = split.iter().find(|f| origin.get(&f.object) == ids.first()).map(|f| f.object).unwrap();
                let bi = block_index_of(&blocks, first).unwrap();
                let got: std::collections::BTreeSet<usize> = blocks[bi].objects().iter().map(|x| origin[x]).collect();
                let want: std::collections::BTreeSet<usize> = ids.iter().copied().collect();
                assert_eq!(got, want, "page {pg}: paragraph {} fell apart when split into characters", t.id);
            }
        }
    }
    println!("per-character split: {same} of {total} blocks identical to the unsplit page");
    assert!(same * 100 >= total * 90, "only {same} of {total} blocks survived the split");
}
