//! The block-input bridge on the real datasheet: P's fixtures (the Frag form of the three pages) are
//! turned back into what PDFium reports (runs, styles, font names, drawn paths) and sent through
//! `build_page_blocks_from`, the way a click does. Whatever the detector does today, the editor must
//! never be offered a block that breaks an invariant, so these assert invariants and structure.
//!
//! `cargo test -p pagify_shell --release --test block_input_datasheet -- --nocapture` prints the census.
//!
//! Self-contained on purpose: it reads the fixture files itself and shares nothing with `tests/common`,
//! which the detector's own tests are free to change.

use pagify_shell::block_input::*;
use pdf_core::document::{Color, DrawnKind, DrawnObject, Point, Rect, RunStyle, TextRun};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap, HashSet};

/// One text object as the fixture stores it: [object, left, top, right, bottom, baseline, size, font,
/// stem, face, rgb hex, text, rotated].
struct Obj {
    object: usize,
    rect: Rect,
    baseline: f32,
    size: f32,
    font: u32,
    stem: Option<u16>,
    face: String,
    rgb: [u8; 3],
    text: String,
    rotated: bool,
}

/// One path object: [object, left, top, right, bottom, depth].
struct Path {
    object: usize,
    rect: Rect,
    depth: usize,
}

struct Page {
    number: u32,
    objects: Vec<Obj>,
    paths: Vec<Path>,
}

fn load_page(n: u32) -> Page {
    let file = format!("{}/tests/fixtures/datasheet_p{n}.json", env!("CARGO_MANIFEST_DIR"));
    let doc: Value = serde_json::from_str(&std::fs::read_to_string(&file).expect("fixture")).expect("json");
    let f = |a: &Value, i: usize| a[i].as_f64().expect("number") as f32;
    let objects = doc["frags"]
        .as_array()
        .expect("frags")
        .iter()
        .map(|a| {
            let c = u32::from_str_radix(a[10].as_str().expect("colour"), 16).expect("hex");
            Obj {
                object: a[0].as_u64().expect("id") as usize,
                rect: Rect { left: f(a, 1), top: f(a, 2), right: f(a, 3), bottom: f(a, 4) },
                baseline: f(a, 5),
                size: f(a, 6),
                font: a[7].as_u64().expect("font") as u32,
                stem: a[8].as_u64().map(|s| s as u16),
                face: a[9].as_str().expect("face").to_string(),
                rgb: [(c >> 16) as u8, (c >> 8) as u8, c as u8],
                text: a[11].as_str().expect("text").to_string(),
                rotated: a[12].as_u64().expect("rotated") != 0,
            }
        })
        .collect();
    let paths = doc["shapes"]
        .as_array()
        .expect("shapes")
        .iter()
        .map(|a| Path { object: a[0].as_u64().expect("id") as usize, rect: Rect { left: f(a, 1), top: f(a, 2), right: f(a, 3), bottom: f(a, 4) }, depth: a[5].as_u64().expect("depth") as usize })
        .collect();
    Page { number: n, objects, paths }
}

/// The fixtures keep the box and the baseline of every object but not where its text starts. The
/// stage-0 dump of the same pages (`origin_x` next to `left`) shows the text starting a little left of
/// its ink (0.0 to 1.0 pt, 0.08 em on average) and, for an object that begins with a space, exactly at
/// the box's left edge. Without that, narrow neighbours such as ": " (box 0.67 pt wide) and " Ma" would
/// share an origin that they do not share in the file (the real two are 1.28 pt apart).
fn origin_x(left: f32, size: f32, text: &str) -> f32 {
    if text.starts_with(char::is_whitespace) {
        left
    } else {
        left - 0.08 * size
    }
}

type Inputs = (Vec<TextRun>, HashMap<usize, RunStyle>, HashMap<usize, String>, Vec<DrawnObject>);

fn inputs(page: &Page) -> Inputs {
    let mut runs = Vec::new();
    let mut styles = HashMap::new();
    let mut faces = HashMap::new();
    for o in &page.objects {
        runs.push(TextRun {
            object: o.object,
            text: o.text.clone(),
            rect: o.rect,
            origin: Point { x: origin_x(o.rect.left, o.size, &o.text), y: o.baseline },
            size: o.size,
            color: Color { r: o.rgb[0], g: o.rgb[1], b: o.rgb[2], a: 255 },
        });
        styles.insert(o.object, RunStyle { font: o.font, stem_milli_em: o.stem, axis: if o.rotated { (0.0, 1.0) } else { (1.0, 0.0) } });
        if !o.face.is_empty() {
            // the way the file names a subset font
            faces.insert(o.object, format!("ABCDEF+{}", o.face));
        }
    }
    let shapes = page
        .paths
        .iter()
        .map(|p| DrawnObject { object: p.object, kind: DrawnKind::Shape, rect: p.rect, label: String::new(), depth: p.depth, opacity: 1.0, movable: p.depth == 0 })
        .collect();
    (runs, styles, faces, shapes)
}

fn build(page: &Page) -> PageBlocks {
    let (runs, styles, faces, shapes) = inputs(page);
    build_page_blocks_from(page.number as usize - 1, 1, 1, runs, styles, faces, shapes)
}

fn has_area(r: &Rect) -> bool {
    (r.right - r.left).abs() > 0.0 && (r.bottom - r.top).abs() > 0.0
}

/// xorshift: the same shuffle on every run.
fn shuffle<T>(v: &mut [T], mut seed: u64) {
    for i in (1..v.len()).rev() {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        v.swap(i, (seed % (i as u64 + 1)) as usize);
    }
}

#[test]
fn block_input_every_object_is_placed_exactly_once_and_every_block_passes_the_invariants() {
    for n in 1..=3 {
        let page = load_page(n);
        let pb = build(&page);

        // every object of the page is a frag, or excluded for one reason, never both and never twice
        let mut place: HashMap<usize, &str> = HashMap::new();
        let mut put = |id: usize, why: &'static str| {
            assert!(place.insert(id, why).is_none(), "page {n}: object {id} is placed twice ({why} and {:?})", place[&id]);
        };
        for f in &pb.frags {
            put(f.object, "frag");
        }
        for (why, ids) in [("blank", &pb.excluded.blank), ("invisible", &pb.excluded.invisible), ("degenerate", &pb.excluded.degenerate), ("shadowed", &pb.excluded.shadowed), ("rotated", &pb.excluded.rotated)] {
            for &id in ids {
                put(id, why);
            }
        }
        let all: BTreeSet<usize> = page.objects.iter().map(|o| o.object).collect();
        let placed: BTreeSet<usize> = place.keys().copied().collect();
        assert_eq!(placed, all, "page {n}: every object of the page is a frag or excluded");

        // runs: every object with area, the excluded ones too, and none without
        let with_area: BTreeSet<usize> = page.objects.iter().filter(|o| has_area(&o.rect)).map(|o| o.object).collect();
        let kept: BTreeSet<usize> = pb.runs.keys().copied().collect();
        assert_eq!(kept, with_area, "page {n}: runs holds exactly the objects with area");

        // by_object holds the admitted objects, each once, and every block's lines say the same
        assert_eq!(pb.by_object.len(), pb.frags.len(), "page {n}");
        let in_blocks: usize = pb.blocks.iter().map(|b| b.objects().len()).sum();
        assert_eq!(in_blocks, pb.frags.len(), "page {n}: each admitted object is in exactly one line of one block");
        for f in &pb.frags {
            let (bi, li) = pb.by_object[&f.object];
            assert!(pb.blocks[bi].lines[li].objects.contains(&f.object), "page {n}: by_object points at the wrong line for {}", f.object);
            assert!(!f.rotated && f.font != u32::MAX && !f.face.contains('+'), "page {n}: frag {} was adapted wrongly: {f:?}", f.object);
        }

        // the guard accepts every block the detector makes, but the vector art it bridged as text: a block
        // with more lines drawn as shapes than lines with text (the QR code under 'ROHS' on page 2, four
        // phantom lines over one line of text) is C14's. Stated as an equivalence, so it holds whatever
        // the detector does with that code: a block is refused for C14 exactly when it is that.
        let mut refused: Vec<(usize, String)> = Vec::new();
        let mut art = 0;
        let (mut frozen_blocks, mut hole_blocks, mut whole_blocks, mut multi, mut lines_total) = (0, 0, 0, 0, 0);
        for bi in 0..pb.blocks.len() {
            let with_text = pb.blocks[bi].lines.iter().filter(|l| !l.objects.is_empty()).count();
            let drawn = pb.blocks[bi].lines.len() - with_text;
            let is_art = with_text > 0 && drawn > with_text;
            match check_editor_invariants(&pb, bi) {
                Ok(()) => assert!(!is_art, "page {n} block {bi}: {drawn} drawn lines against {with_text} with text is vector art and must be refused"),
                Err(e) if is_art && e.starts_with("C14:") => art += 1,
                Err(e) => refused.push((bi, e)),
            }
            let specs = editor_lines(&pb, bi);
            assert_eq!(specs.len(), pb.blocks[bi].lines.len(), "page {n} block {bi}: no line is dropped");
            lines_total += specs.len();
            let frozen = specs.iter().filter(|s| s.frozen).count();
            frozen_blocks += usize::from(frozen > 0);
            hole_blocks += usize::from(specs.iter().any(|s| s.frozen && !s.objects.is_empty()));
            whole_blocks += usize::from(specs.iter().any(|s| s.frozen && s.objects.is_empty()));
            multi += usize::from(pb.blocks[bi].objects().len() > 1);
        }
        println!(
            "page {n}: {} text objects, {} admitted, excluded {{blank {}, invisible {}, degenerate {}, shadowed {}, rotated {}}}, {} blocks ({multi} of 2+ objects, {lines_total} lines), {frozen_blocks} blocks with frozen lines ({hole_blocks} with a hole in a text line, {whole_blocks} with a whole outlined line), {art} refused as vector art, build {:.2} ms (detect {:.2} ms)",
            page.objects.len(),
            pb.frags.len(),
            pb.excluded.blank.len(),
            pb.excluded.invisible.len(),
            pb.excluded.degenerate.len(),
            pb.excluded.shadowed.len(),
            pb.excluded.rotated.len(),
            pb.blocks.len(),
            pb.build_ms,
            pb.detect_ms,
        );
        assert!(refused.is_empty(), "page {n}: {} of {} blocks refused by the editor guard (apart from {art} vector art):\n{refused:#?}", refused.len(), pb.blocks.len());
        assert!(art <= 1, "page {n}: {art} blocks of vector art; the datasheet has one QR code, on page 2");
        assert!(pb.detect_ms > 0.0 && pb.build_ms >= pb.detect_ms, "page {n}: the build times itself ({} ms, detect {} ms)", pb.build_ms, pb.detect_ms);
        assert_eq!((pb.page, pb.epoch, pb.generation), (n as usize - 1, 1, 1));
    }
}

#[test]
fn block_input_what_the_adapter_leaves_out_of_the_real_pages() {
    let (p1, p2, p3) = (build(&load_page(1)), build(&load_page(2)), build(&load_page(3)));
    // the five faux-bold twins of the "Color Options" heading on pages 1 and 2 are blank and drawn over the real ones
    assert_eq!(p1.excluded.blank, vec![826, 827, 828, 829, 830]);
    assert_eq!(p2.excluded.blank, vec![2659, 2660, 2661, 2662, 2663]);
    assert!(p3.excluded.blank.is_empty());
    // the rotated dimension labels have size 0 (the matrix's d) and are rotated, not degenerate
    assert_eq!(p2.excluded.rotated, vec![2800, 2801, 2816, 2817]);
    assert_eq!(p3.excluded.rotated, vec![811, 812, 813]);
    for pb in [&p1, &p2, &p3] {
        assert!(pb.excluded.degenerate.is_empty() && pb.excluded.invisible.is_empty(), "nothing else is wrong with these pages: {:?}", pb.excluded);
        assert!(pb.excluded.shadowed.is_empty(), "no real word is mistaken for a twin: {:?}", pb.excluded.shadowed);
    }
}

#[test]
fn block_input_a_click_on_a_blank_twin_opens_the_readable_object_beneath() {
    for (n, hair) in [(1, 0.0f32), (2, 0.0), (1, 0.05), (2, 0.05)] {
        let page = load_page(n);
        let (mut runs, styles, faces, shapes) = inputs(&page);
        // a twin whose box is a hair smaller than the real one wins the smallest-rect rule; one with an
        // identical box only loses the tie. Both must end on the real object.
        for r in runs.iter_mut().filter(|r| r.text.trim().is_empty()) {
            r.rect = Rect { left: r.rect.left + hair, top: r.rect.top + hair, right: r.rect.right - hair, bottom: r.rect.bottom - hair };
        }
        let pb = build_page_blocks_from(n as usize - 1, 1, 1, runs, styles, faces, shapes);
        assert_eq!(pb.excluded.blank.len(), 5, "page {n}");
        let mut twins = 0;
        for &t in &pb.excluded.blank {
            let r = pb.runs[&t].rect;
            let (x, y) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
            let (seed, rule) = pick_seed(&pb, x, y, 3.0).expect("a click on a twin finds something");
            assert!(pb.by_object.contains_key(&seed), "page {n}: a click on twin {t} opened {seed} ({rule:?}), which is not a block member");
            assert_ne!(seed, t);
            if rule == SeedRule::Twin {
                twins += 1;
            }
        }
        if hair > 0.0 {
            assert_eq!(twins, 5, "page {n}: with smaller twins every click is a twin swap");
        }
    }
}

#[test]
fn block_input_the_users_paragraph_on_page_1_is_one_block_with_two_frozen_lines() {
    let page = load_page(1);
    let pb = build(&page);
    let &(bi, _) = pb.by_object.get(&985).expect("object 985 is admitted");
    let block = &pb.blocks[bi];

    // the paragraph is objects 985 to 1043; 1026 and 1035 are outlined words (paths, not text)
    let members: BTreeSet<usize> = block.objects().into_iter().collect();
    let want: BTreeSet<usize> = (985..=1043).filter(|o| *o != 1026 && *o != 1035).collect();
    assert_eq!(members, want, "the paragraph is exactly one block (starts because {:?})", block.starts_because);
    for path in [1026usize, 1035] {
        assert!(page.paths.iter().any(|p| p.object == path && p.depth == 0), "{path} is a top-level path of page 1");
        assert!(!pb.runs.contains_key(&path));
    }

    // thirteen lines, and the frozen ones are exactly the two that hold an outlined word
    let specs = editor_lines(&pb, bi);
    assert_eq!(specs.len(), 13);
    let frozen: Vec<usize> = specs.iter().enumerate().filter(|(_, s)| s.frozen).map(|(i, _)| i).collect();
    let holding: Vec<(usize, Vec<usize>)> = block.lines.iter().enumerate().filter(|(_, l)| !l.outlined.is_empty()).map(|(i, l)| (i, l.outlined.clone())).collect();
    assert_eq!(holding.len(), 2, "two lines hold an outlined word: {holding:?}");
    assert_eq!(frozen, holding.iter().map(|(i, _)| *i).collect::<Vec<_>>());
    assert_eq!(holding.iter().flat_map(|(_, o)| o.iter().copied()).collect::<Vec<_>>(), vec![1026, 1035]);
    assert_eq!(frozen, vec![8, 10], "the 9th and the 11th line");
    for &i in &frozen {
        assert!(!specs[i].objects.is_empty() && specs[i].placeholder.is_none(), "a hole in a text line keeps its text objects");
    }
    // each outlined word's centre lies inside the rect of the line it freezes
    for ((i, outlined), path) in holding.iter().zip([1026usize, 1035]) {
        let s = page.paths.iter().find(|p| p.object == path).unwrap().rect;
        let (cx, cy) = ((s.left + s.right) / 2.0, (s.top + s.bottom) / 2.0);
        let r = specs[*i].rect;
        assert!(cx >= r.left && cx <= r.right && cy >= r.top && cy <= r.bottom, "outlined {outlined:?} sits in line {i}");
    }

    // the block around it is its own: the bold lines above and below start other blocks
    for other in [980usize, 1044] {
        assert_ne!(pb.by_object[&other].0, bi, "object {other} is not part of the paragraph");
    }

    // the text: the plain concatenation, no space invented between "lig" and "ht"
    let texts = line_texts(&pb, &specs);
    assert_eq!(texts[0], "The COB in HSI products sup\u{2}");
    assert_eq!(texts[3], "lighting industry conforming");
    assert_eq!(texts[8], "COB could be even", "the outlined word leaves a hole in the text of its line");
    assert_eq!(texts.join("\n").split('\n').count(), 13, "one buffer line per editor line");
    assert!(texts.iter().all(|t| !t.contains('\n') && !t.contains('\r')));
    assert_eq!(check_editor_invariants(&pb, bi), Ok(()));

    // a click on any word of it seeds this block
    for o in [985usize, 1000, 1029, 1043] {
        let r = pb.runs[&o].rect;
        let (seed, _) = pick_seed(&pb, (r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0, 3.0).unwrap();
        assert_eq!(pb.by_object[&seed].0, bi, "a click on {o} opens the paragraph (seed {seed})");
    }

    // and the one log line a click on it makes
    let mut t = PickTrace::new(0, (212.4, 501.2));
    t.page_facts(&pb);
    t.block_facts(&pb, bi);
    t.seed = Some(1029);
    t.rule = Some(SeedRule::Exact);
    t.path = PickPath::Block;
    let line = format_pick_line(&t);
    println!("{line}");
    assert!(line.contains(" lines=13 objects=57 outlined=2 twins=0 frozen=[8,10] "), "{line}");
}

#[test]
fn block_input_the_same_blocks_whatever_the_order_the_runs_arrive_in() {
    for n in 1..=3 {
        let page = load_page(n);
        let (runs, styles, faces, shapes) = inputs(&page);
        let reference = build_page_blocks_from(0, 0, 0, runs.clone(), styles.clone(), faces.clone(), shapes.clone());
        let mut shuffled = runs;
        shuffle(&mut shuffled, 0x1234_5678_9ABC_DEF1 ^ n as u64);
        let again = build_page_blocks_from(0, 0, 0, shuffled, styles, faces, shapes);
        assert_eq!(again.frags, reference.frags, "page {n}: the same frags");
        assert_eq!(again.blocks, reference.blocks, "page {n}: the same blocks");
        assert_eq!(again.excluded, reference.excluded, "page {n}: the same exclusions");
        assert_eq!(again.by_object, reference.by_object, "page {n}");
    }
}

#[test]
fn block_input_a_click_on_every_admitted_object_finds_a_member_or_says_why_not() {
    // the smallest rect under a point is usually the word clicked; where it is not, the page has
    // something else drawn there (an excluded object), and the census says which
    for n in 1..=3 {
        let page = load_page(n);
        let pb = build(&page);
        let mut off: Vec<(usize, usize, &'static str)> = Vec::new();
        for f in pb.frags.iter().filter(|f| has_area(&Rect { left: f.left, top: f.top, right: f.right, bottom: f.bottom })) {
            let (x, y) = ((f.left + f.right) / 2.0, (f.top + f.bottom) / 2.0);
            let (seed, _) = pick_seed(&pb, x, y, 3.0).expect("the centre of a word is on text");
            if !pb.by_object.contains_key(&seed) {
                off.push((f.object, seed, pb.excluded.reason(seed).unwrap_or("none")));
            }
        }
        println!("page {n}: {} of {} word centres land on an object that is not a block member: {off:?}", off.len(), pb.frags.len());
        assert!(off.is_empty(), "page {n}: clicks that did not reach a member: {off:?}");
    }
}

#[test]
fn block_input_an_unreadable_page_and_a_hostile_one_do_not_panic() {
    let page = load_page(1);
    let (mut runs, styles, faces, shapes) = inputs(&page);
    // poison a spread of objects in every way the adapter has a word for
    for (i, r) in runs.iter_mut().enumerate() {
        match i % 11 {
            0 => r.size = f32::NAN,
            1 => r.rect.left = f32::INFINITY,
            2 => r.color.a = 0,
            3 => r.text = "a\nb\r\nc".into(),
            4 => r.rect = Rect { left: r.rect.right, top: r.rect.bottom, right: r.rect.left, bottom: r.rect.top },
            5 => r.origin = Point { x: f32::NAN, y: 0.0 },
            _ => {}
        }
    }
    let pb = build_page_blocks_from(0, 0, 0, runs, styles, faces, shapes);
    for bi in 0..pb.blocks.len() {
        // (C17 is a judgement about what a short block is, not about the page being malformed: poisoning the
        // objects between two words leaves a hole in their line, which is a row of cells to it)
        match check_editor_invariants(&pb, bi) {
            Ok(()) => {}
            Err(e) => assert!(e.starts_with("C17:"), "block {bi}: whatever the adapter let through is safe: {e}"),
        }
    }
    let hit: HashSet<usize> = pb.by_object.keys().copied().collect();
    assert_eq!(hit.len(), pb.frags.len());
    let _ = pick_seed(&pb, 200.0, 450.0, 3.0);
}
