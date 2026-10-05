//! The block-input bridge on the real thing: the datasheet read through PDFium by
//! `Session::page_text_snapshot` and built into blocks by `build_page_blocks`, exactly as a click will,
//! so the origins, colours, sizes, styles, font names, area-less objects and drawn paths are what the
//! engine really reports (P's fixtures keep the boxes and baselines only).
//!
//! Skips, with a line saying so, when the datasheet is not on this machine. PDFium is found the way the
//! app finds it (`PAGIFY_PDFIUM_LIB`, beside the executable, or the vendored slice).
//!
//! `cargo test -p pagify_shell --release --test block_input_real_pdf -- --nocapture`

use pagify_shell::block_input::*;
use pagify_shell::Session;
use std::collections::BTreeSet;
use std::sync::OnceLock;

const DATASHEET: &str = r"C:\Users\hsili\Desktop\Datasheets - Editors market - Marina mall.pdf";

struct Real {
    pb: PageBlocks,
    /// Every text object the snapshot held, with or without area.
    objects: usize,
}

/// The first three pages, read once for all the tests (a page read is a fraction of a second).
fn pages() -> Option<&'static Vec<Real>> {
    static PAGES: OnceLock<Option<Vec<Real>>> = OnceLock::new();
    PAGES
        .get_or_init(|| {
            if !std::path::Path::new(DATASHEET).is_file() {
                eprintln!("skipped: the datasheet is not at {DATASHEET}");
                return None;
            }
            let session = Session::open(DATASHEET).expect("open the datasheet");
            let count = session.page_count().expect("page count").min(3);
            Some(
                (0..count)
                    .map(|page| {
                        let snapshot = session.page_text_snapshot(page).expect("snapshot");
                        let objects = snapshot.runs.len();
                        Real { pb: build_page_blocks(page, 1, 1, snapshot), objects }
                    })
                    .collect(),
            )
        })
        .as_ref()
}

#[test]
fn block_input_real_pdf_every_block_passes_the_invariants() {
    let Some(pages) = pages() else { return };
    assert_eq!(pages.len(), 3, "the datasheet has three pages");
    for (page, Real { pb, objects }) in pages.iter().enumerate() {
        // the closest two members of any block, by origin: the margin C4 works with
        let mut closest = f32::MAX;
        // (the vector art the detector bridged into a label's lines is refused by C14, and has its own test below)
        let mut refused: Vec<(usize, String)> = Vec::new();
        for bi in 0..pb.blocks.len() {
            if let Err(e) = check_editor_invariants(pb, bi) {
                if !e.starts_with("C14:") {
                    refused.push((bi, e));
                }
            }
        }
        let mut origins: Vec<(f32, f32)> = pb.by_object.keys().map(|o| (pb.runs[o].origin.x, pb.runs[o].origin.y)).collect();
        origins.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (i, a) in origins.iter().enumerate() {
            for b in origins[i + 1..].iter().take_while(|b| b.0 - a.0 < 3.0) {
                closest = closest.min((b.0 - a.0).hypot(b.1 - a.1));
            }
        }
        let frozen = (0..pb.blocks.len()).filter(|&bi| editor_lines(pb, bi).iter().any(|l| l.frozen)).count();
        println!(
            "real page {}: {objects} text objects ({} with area), {} admitted, excluded {{blank {}, invisible {}, degenerate {}, shadowed {}, rotated {}}}, {} blocks, {frozen} with frozen lines, closest two member origins {closest:.2} pt, build {:.2} ms (detect {:.2} ms)",
            page + 1,
            pb.runs.len(),
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
        assert!(refused.is_empty(), "page {}: {} of {} blocks refused by the editor guard:\n{refused:#?}", page + 1, refused.len(), pb.blocks.len());
        assert!(pb.excluded.shadowed.is_empty(), "page {}: a real word was taken for a twin: {:?}", page + 1, pb.excluded.shadowed);
        assert!(closest > SAME_ORIGIN_PT, "page {}: two members share an origin ({closest} pt)", page + 1);
        // every text object of the page is admitted or excluded for exactly one reason
        assert_eq!(pb.frags.len() + pb.excluded.count(), *objects, "page {}", page + 1);
        assert_eq!(pb.by_object.len(), pb.frags.len());
        assert_eq!(pb.blocks.iter().map(|b| b.objects().len()).sum::<usize>(), pb.frags.len());
    }
}

#[test]
fn block_input_real_pdf_the_twins_and_the_rotated_labels_are_where_they_were_measured() {
    let Some(pages) = pages() else { return };
    let [p1, p2, p3] = [&pages[0].pb, &pages[1].pb, &pages[2].pb];
    // five faux-bold twins of "Color Options" on pages 1 and 2, blank and drawn over the real objects
    assert_eq!(p1.excluded.blank, vec![826, 827, 828, 829, 830]);
    assert_eq!(p2.excluded.blank, vec![2659, 2660, 2661, 2662, 2663]);
    assert!(p3.excluded.blank.is_empty());
    // the dimension labels turned on their side: rotated (their effective size is 0, which must not hide that)
    assert_eq!(p2.excluded.rotated, vec![2800, 2801, 2816, 2817]);
    assert_eq!(p3.excluded.rotated, vec![811, 812, 813]);
    for pb in [p1, p2, p3] {
        for r in pb.excluded.rotated.iter().filter_map(|o| pb.runs.get(o)) {
            assert!(r.size.abs() < 0.01 || pb.styles[&r.object].axis != (1.0, 0.0), "object {} is rotated for a reason", r.object);
        }
        assert!(pb.excluded.invisible.is_empty(), "no alpha-0 text on these pages");
        // what is degenerate has no area (spaces, mostly): nothing that can be clicked is
        assert!(pb.excluded.degenerate.iter().all(|o| !pb.runs.contains_key(o)), "an object with area is degenerate");
    }
    // a click on any twin opens the readable object beneath it
    for pb in [p1, p2] {
        for &t in &pb.excluded.blank {
            let r = pb.runs[&t].rect;
            let (seed, rule) = pick_seed(pb, (r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0, 3.0).expect("a click on a twin finds something");
            assert!(pb.by_object.contains_key(&seed), "a click on twin {t} opened {seed} ({rule:?}), not a block member");
        }
    }
}

#[test]
fn block_input_real_pdf_the_users_paragraph_on_page_1() {
    let Some(pages) = pages() else { return };
    let pb = &pages[0].pb;
    let &(bi, _) = pb.by_object.get(&985).expect("object 985 is a member of a block");
    let block = &pb.blocks[bi];
    let members: BTreeSet<usize> = block.objects().into_iter().collect();
    let want: BTreeSet<usize> = (985..=1043).filter(|o| *o != 1026 && *o != 1035).collect();
    assert_eq!(members, want, "the paragraph is one block");
    assert!(pb.shapes.iter().any(|s| s.object == 1026 && s.depth == 0) && pb.shapes.iter().any(|s| s.object == 1035 && s.depth == 0), "the two outlined words are top-level paths");

    let specs = editor_lines(pb, bi);
    assert_eq!(specs.len(), 13);
    let frozen: Vec<usize> = specs.iter().enumerate().filter(|(_, s)| s.frozen).map(|(i, _)| i).collect();
    assert_eq!(frozen, vec![8, 10], "the two lines that hold an outlined word, and no other");
    assert_eq!(block.lines[8].outlined, vec![1026]);
    assert_eq!(block.lines[10].outlined, vec![1035]);

    let texts = line_texts(pb, &specs);
    assert_eq!(texts[0], "The COB in HSI products sup\u{2}");
    assert_eq!(texts[3], "lighting industry conforming");
    assert_eq!(texts[8], "COB could be even");
    assert_eq!(texts.join("\n").split('\n').count(), 13);
    assert_eq!(check_editor_invariants(pb, bi), Ok(()));

    // a click on the word "could" (object 1028) seeds the paragraph, and the log line says so
    let r = pb.runs[&1028].rect;
    let (x, y) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
    let (seed, rule) = pick_seed(pb, x, y, 3.0).unwrap();
    assert_eq!(pb.by_object[&seed].0, bi);
    let mut t = PickTrace::new(0, (x, y));
    t.page_facts(pb);
    t.block_facts(pb, bi);
    (t.seed, t.rule, t.path) = (Some(seed), Some(rule), PickPath::Block);
    let line = format_pick_line(&t);
    println!("{line}");
    assert!(line.starts_with("page 1 click=(") && line.contains(" path=block ") && line.contains(" lines=13 objects=57 outlined=2 twins=0 frozen=[8,10] "), "{line}");
}

// ------------------------------------------------------------------------------------------------
// Thin glyphs, faux-bold twins, vector art: what the real engine reports
// ------------------------------------------------------------------------------------------------

/// The objects of the datasheet that the engine's own click rule (both sides over half a point) calls
/// "no area" and that are real, visible characters: (page, object, text). Found by listing every text
/// object of pages 1-3 that is not blank and fails that rule: 'l' is 0.40 x 5.94 pt, '-' 2.06 x 0.36 pt,
/// ' . ' 2.63 x 0.46 pt. Poppler reads the words they belong to as "reliable" (the two l's), "Tunable",
/// the hyphens of "2700K - 6500K" and the dots of "HUE . SATURATION . INTENSITY".
const THIN: [(usize, usize, &str); 15] = [
    (0, 19, "-"),
    (0, 644, "l"),
    (0, 648, "l"),
    (0, 709, "-"),
    (1, 1375, "l"),
    (1, 1799, "-"),
    (1, 2345, "l"),
    (1, 2349, "l"),
    (1, 2459, "-"),
    (1, 2782, " . "),
    (2, 1086, "-"),
    (2, 1619, "l"),
    (2, 1623, "l"),
    (2, 1722, "-"),
    (2, 2014, " . "),
];

/// The editor text of the line that holds `object`, with runs of whitespace squeezed to one space.
fn line_text_of(pb: &PageBlocks, object: usize) -> Option<String> {
    let &(bi, li) = pb.by_object.get(&object)?;
    let specs = editor_lines(pb, bi);
    let texts = line_texts(pb, &specs);
    Some(texts[li].split_whitespace().collect::<Vec<_>>().join(" "))
}

#[test]
fn block_input_real_pdf_every_thin_glyph_is_a_member_and_in_the_text_of_its_line() {
    let Some(pages) = pages() else { return };
    for &(page, object, text) in &THIN {
        let pb = &pages[page].pb;
        let run = pb.runs.get(&object).unwrap_or_else(|| panic!("page {} object {object} ({text:?}) is not a run: it has area, however little", page + 1));
        assert_eq!(run.text, text, "page {} object {object} is the character it was measured as", page + 1);
        let w = (run.rect.right - run.rect.left).abs();
        let h = (run.rect.bottom - run.rect.top).abs();
        assert!(w <= 0.5 || h <= 0.5, "page {} object {object} is one of the thin ones (it is {w:.2} x {h:.2})", page + 1);
        assert!(!pb.excluded.degenerate.contains(&object), "page {} object {object} is not 'degenerate': it has area", page + 1);
        assert!(pb.by_object.contains_key(&object), "page {} object {object} ({text:?}, {w:.2} x {h:.2} pt) is a member of its line", page + 1);
        let line = line_text_of(pb, object).expect("a member has a line");
        println!("page {} object {object} {text:?} ({w:.2} x {h:.2} pt): line {line:?}", page + 1);
        assert!(line.contains(text.trim()), "page {} object {object}: its character is in the text of its line: {line:?}", page + 1);
        let (bi, _) = pb.by_object[&object];
        assert_eq!(check_editor_invariants(pb, bi), Ok(()), "page {} object {object}: its block passes the guard", page + 1);
    }
    // the words they belong to, as Poppler reads them, whole in the editor's text
    for (page, object, word) in [(0, 644, "reliable"), (0, 648, "reliable"), (1, 2345, "reliable"), (1, 2349, "reliable"), (2, 1619, "reliable"), (2, 1623, "reliable"), (1, 1375, "Tunable")] {
        let line = line_text_of(&pages[page].pb, object).unwrap();
        assert!(line.contains(word), "page {} object {object}: the editor shows {word:?} whole, not a word with a letter missing: {line:?}", page + 1);
    }
    for (page, object) in [(1, 2782), (2, 2014)] {
        let line = line_text_of(&pages[page].pb, object).unwrap();
        assert!(line.contains("SATURATION . INTENSITY"), "page {} object {object}: {line:?}", page + 1);
    }
    // the six hyphens are the hyphenation marks that end a line whose words are drawn as outlines: the
    // line's only text object, so the line reads "-" (and is frozen: it is never written), where it used
    // to read as a bare "[drawn text]" with the hyphen on the page and in nobody's line
    for (page, object) in [(0, 19), (0, 709), (1, 1799), (1, 2459), (2, 1086), (2, 1722)] {
        let pb = &pages[page].pb;
        let &(bi, li) = pb.by_object.get(&object).unwrap();
        let spec = &editor_lines(pb, bi)[li];
        assert_eq!(spec.objects, vec![object], "page {} object {object}: the hyphen is all the text of its line", page + 1);
        assert!(spec.frozen && spec.placeholder.is_none(), "page {} object {object}: the line also holds an outlined word, so it is frozen", page + 1);
        assert_eq!(line_text_of(pb, object).unwrap(), "-");
    }
    // whatever is still degenerate has no area at all
    for (page, Real { pb, .. }) in pages.iter().enumerate() {
        for &o in &pb.excluded.degenerate {
            assert!(!pb.runs.contains_key(&o), "page {} object {o} is degenerate but has area", page + 1);
        }
    }
}

#[test]
fn block_input_real_pdf_every_faux_bold_twin_is_listed_with_the_line_of_the_object_it_duplicates() {
    let Some(pages) = pages() else { return };
    // the heading "Color Options" is drawn twice: five readable objects and, over them, five blank ones
    for (page, members, twins) in [(0usize, 817..=821usize, 826..=830usize), (1, 2650..=2654, 2659..=2663)] {
        let pb = &pages[page].pb;
        let members: Vec<usize> = members.collect();
        let twins: Vec<usize> = twins.collect();
        assert_eq!(pb.excluded.blank, twins, "page {}: the blank layer", page + 1);
        // each twin is the twin of exactly one member, and each member of the heading has one
        let mut found: Vec<usize> = Vec::new();
        for (&m, t) in members.iter().zip(&twins) {
            assert_eq!(pb.twins.get(&m), Some(&vec![*t]), "page {}: member {m} is duplicated by {t}", page + 1);
            found.extend(&pb.twins[&m]);
        }
        assert_eq!(found, twins);
        // and they all land in the one line of the heading, the first piece's included
        let &(bi, li) = pb.by_object.get(&members[0]).expect("the heading is in a block");
        let specs = editor_lines(pb, bi);
        assert_eq!(specs[li].objects, members, "page {}: the heading is one line of five pieces", page + 1);
        assert_eq!(specs[li].twins, twins, "page {}: the line lists every twin, first piece's included", page + 1);
        for (i, s) in specs.iter().enumerate().filter(|(i, _)| *i != li) {
            assert!(s.twins.is_empty(), "page {}: line {i} of the block is not the heading and has no twin", page + 1);
        }
        let texts = line_texts(pb, &specs);
        assert_eq!(texts[li].split_whitespace().collect::<Vec<_>>().join(" "), "Color Options", "twins are not in the text");
        assert_eq!(check_editor_invariants(pb, bi), Ok(()));
        let mut t = PickTrace::new(page, (0.0, 0.0));
        t.page_facts(pb);
        t.block_facts(pb, bi);
        assert_eq!(t.twins, 5);
        assert!(format_pick_line(&t).contains(" twins=5 "), "{}", format_pick_line(&t));
    }
    // page 3 has no faux bold; and on every page each blank or shadowed object is the twin of some member
    assert!(pages[2].pb.twins.is_empty());
    for Real { pb, .. } in pages.iter() {
        let listed: usize = pb.twins.values().map(Vec::len).sum();
        assert_eq!(listed, pb.excluded.blank.len() + pb.excluded.shadowed.len(), "every blank or shadowed object of these pages is a twin of something");
    }
}

#[test]
fn block_input_real_pdf_the_qr_code_under_rohs_is_vector_art_and_nothing_else_on_these_pages_is() {
    let Some(pages) = pages() else { return };
    let mut art = Vec::new();
    for (page, Real { pb, .. }) in pages.iter().enumerate() {
        for (bi, b) in pb.blocks.iter().enumerate() {
            let with_text = b.lines.iter().filter(|l| !l.objects.is_empty()).count();
            let drawn = b.lines.len() - with_text;
            let is_art = with_text > 0 && drawn > with_text;
            match check_editor_invariants(pb, bi) {
                Ok(()) => assert!(!is_art, "page {} block {bi}: {drawn} drawn lines over {with_text} text lines passed the guard", page + 1),
                Err(e) => {
                    assert!(is_art && e.starts_with("C14:"), "page {} block {bi} was refused for something other than vector art: {e}", page + 1);
                    art.push((page + 1, bi, e));
                }
            }
        }
    }
    println!("vector art refused on pages 1-3: {art:?}");
    // the one the owner reported: 'ROHS' (objects 2683 and 2684) on page 2, four phantom lines over its one line
    let pb = &pages[1].pb;
    let rohs = pb.by_object[&2683].0;
    if let Err(e) = check_editor_invariants(pb, rohs) {
        assert!(e.starts_with("C14: 4 of the block's 5 lines are drawn shapes and only 1 hold text"), "{e}");
        assert_eq!(art.len(), 1, "that block, and no other, is vector art on these pages: {art:?}");
    } else {
        // the detector no longer bridges the QR code into the label's lines: nothing left to refuse
        assert!(art.is_empty(), "{art:?}");
    }
}

/// **The adapter's lists are what the engine needs.** Every line of the datasheet that holds a thin glyph
/// ("reliable", "Tunable", the footer's dots) or a faux-bold twin ("Color Options", five twins) is retyped
/// the way the app retypes a line (`Command::ReplaceTextLines`: the first piece takes the typed words, the
/// rest of the line's objects and the line's twins are removed), on a fresh copy of the datasheet, and held
/// to what apply is held to everywhere: it goes through, the first piece reads as typed, nothing is left
/// of what was removed, and undo restores every object exactly. Before the thin glyphs were members the
/// engine refused three of these lines ("draws something has nothing repositioning it"); before the twins
/// were listed the faux-bold copy went on showing the old words.
#[test]
fn block_input_real_pdf_a_line_with_a_thin_glyph_or_a_twin_is_retyped_and_undone_through_the_engine() {
    use pdf_core::command::Command;
    use pdf_core::document::{TextLineEdit, TextStyle};

    let Some(pages) = pages() else { return };
    let fonts: Option<Vec<Vec<u8>>> = ["Montserrat-Regular.ttf", "Montserrat-Bold.ttf"]
        .iter()
        .map(|name| std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../third_party/fonts").join(name)).ok())
        .collect();
    let Some(fonts) = fonts else {
        eprintln!("skipped: the bundled Montserrat fonts are not at ../../third_party/fonts");
        return;
    };
    let clean = |words: &str| words.replace('\u{2}', "-").split_whitespace().collect::<Vec<_>>().join(" ");
    // the line's longest word spelt backwards: the same letters, so the first piece's font can spell it
    let one_word_changed = |line: &str| {
        let mut words: Vec<String> = line.split(' ').map(str::to_string).collect();
        if let Some(at) = words.iter().enumerate().filter(|(_, w)| w.chars().count() >= 3).max_by_key(|(i, w)| (w.chars().count(), usize::MAX - i)).map(|(i, _)| i) {
            words[at] = words[at].chars().rev().collect();
        }
        words.join(" ")
    };
    // (page, an object of the line): the 'l' of "reliable" on each page, "Tunable", the footers, the two headings
    let mut tried: Vec<(usize, usize, usize)> = Vec::new();
    let mut with_thin = 0;
    let mut with_twins = 0;
    for (page, object) in [(0usize, 644usize), (1, 1375), (1, 2345), (1, 2782), (2, 1619), (2, 2014), (0, 817), (1, 2650)] {
        let pb = &pages[page].pb;
        let &(block, line) = pb.by_object.get(&object).expect("a member");
        if tried.contains(&(page, block, line)) {
            continue;
        }
        tried.push((page, block, line));
        let specs = editor_lines(pb, block);
        let spec = &specs[line];
        assert!(!spec.frozen, "page {} block {block} line {line} is a plain line", page + 1);
        let first = spec.objects[0];
        let mut remove: Vec<usize> = spec.objects[1..].to_vec();
        remove.extend(&spec.twins);
        let thin = THIN.iter().any(|&(p, o, _)| p == page && spec.objects.contains(&o));
        with_thin += usize::from(thin);
        with_twins += usize::from(!spec.twins.is_empty());
        let typed = one_word_changed(&line_texts(pb, &specs)[line]);

        let copy = std::env::temp_dir().join(format!("pagify-b2-thin-{}-{page}-{block}.pdf", std::process::id()));
        std::fs::copy(DATASHEET, &copy).expect("copy the datasheet");
        let outcome = (|| {
            let session = Session::open(&copy).expect("open the copy");
            session.set_typing_fonts(fonts.clone()).expect("typing fonts");
            let before = session.page_text_snapshot(page).expect("snapshot").runs;
            session
                .execute(Command::ReplaceTextLines {
                    page_index: page,
                    edits: vec![TextLineEdit::Retype { first, text: typed.clone(), style: TextStyle::default(), remove: remove.clone(), justify_to: None }],
                })
                .map_err(|e| format!("refused: {e}"))?;
            let after = session.page_text_snapshot(page).expect("snapshot").runs;
            if after.len() != before.len() - remove.len() {
                return Err(format!("{} text objects after, {} expected", after.len(), before.len() - remove.len()));
            }
            let renumbered = |o: usize| o - remove.iter().filter(|r| **r < o).count();
            match after.iter().find(|r| r.object == renumbered(first)) {
                Some(a) if clean(&a.text) == clean(&typed) => {}
                Some(a) => return Err(format!("the first piece reads {:?}, not {:?}", clean(&a.text), clean(&typed))),
                None => return Err("the first piece is gone".into()),
            }
            // nothing of what was removed is left on the page: no object with its box and its words
            for b in before.iter().filter(|b| remove.contains(&b.object)) {
                if after.iter().any(|a| a.rect == b.rect && clean(&a.text) == clean(&b.text)) {
                    return Err(format!("removed object {} ({:?}) is still on the page", b.object, clean(&b.text)));
                }
            }
            // every other object is where it was, with what it said
            for b in before.iter().filter(|b| b.object != first && !remove.contains(&b.object)) {
                match after.iter().find(|r| r.object == renumbered(b.object)) {
                    Some(a) if clean(&a.text) == clean(&b.text) && a.rect == b.rect && a.origin == b.origin && a.size == b.size && a.color == b.color => {}
                    _ => return Err(format!("object {} changed or is gone", b.object)),
                }
            }
            match session.undo() {
                Ok((true, _)) if session.page_text_snapshot(page).expect("snapshot").runs == before => Ok(()),
                Ok((true, _)) => Err("after undo the objects are not exactly what they were".into()),
                other => Err(format!("undo did not undo: {:?}", other.map(|(done, _)| done))),
            }
        })();
        let _ = std::fs::remove_file(&copy);
        println!("page {} block {block} line {line}: {} objects, {} twins, typed {typed:?}: {outcome:?}", page + 1, spec.objects.len(), spec.twins.len());
        assert_eq!(outcome, Ok(()), "page {} block {block} line {line}", page + 1);
    }
    assert!(with_thin >= 5 && with_twins == 2, "the lines tried: {} with a thin glyph, {} with twins", with_thin, with_twins);
}

/// The owner's Arabic payment confirmation (a JasperReports receipt: Arabic words and Latin digits in a
/// table), a copy kept with the verifiers' scratch documents. Skipped, with a line, when it is not there.
const PAYCONF: &str = r"C:\Users\hsili\AppData\Local\Temp\claude\C--Users-hsili\2f819b04-e4a0-440c-abae-e04c9029a62b\scratchpad\paragraphs\V3\docs\payconf.pdf";

#[test]
fn block_input_real_pdf_an_arabic_receipt_never_opens_as_a_paragraph() {
    if !std::path::Path::new(PAYCONF).is_file() {
        eprintln!("skipped: the Arabic receipt is not at {PAYCONF}");
        return;
    }
    let session = Session::open(PAYCONF).expect("open the receipt");
    let pb = build_page_blocks(0, 1, 1, session.page_text_snapshot(0).expect("snapshot"));
    let is_rtl = |c: char| matches!(c as u32, 0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF);
    let (mut right_to_left, mut left_to_right, mut c15) = (0, 0, 0);
    for (bi, b) in pb.blocks.iter().enumerate() {
        // the independent witness: does any text object of the block hold an Arabic character?
        let arabic = b.objects().iter().any(|o| pb.runs.get(o).is_some_and(|r| r.text.chars().any(is_rtl)));
        let verdict = check_editor_invariants(&pb, bi);
        if arabic {
            right_to_left += 1;
            let e = verdict.clone().expect_err("a block with Arabic in it must not open as a paragraph");
            assert!(e.starts_with("C15:"), "block {bi}: refused, but for the wrong reason (the log would not say right-to-left): {e}");
            c15 += 1;
        } else {
            left_to_right += 1;
            if let Err(e) = verdict {
                assert!(!e.starts_with("C15:"), "block {bi} has no Arabic character and was refused as right-to-left: {e}");
            }
        }
    }
    println!("Arabic receipt: {} blocks, {right_to_left} with Arabic (refused C15: {c15}), {left_to_right} without", pb.blocks.len());
    assert!(right_to_left >= 10, "the receipt has many Arabic cells: {right_to_left}");
    assert!(left_to_right >= 1, "and some without");
}
