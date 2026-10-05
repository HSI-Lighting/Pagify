//! Editing a line of text by **replacing its pieces**: the first piece takes the
//! new words and the line's other pieces come off the page.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test --release --test replace_lines -- --nocapture
//! ```
//!
//! `--nocapture` is the point of the flag, not a convenience: a test here that
//! cannot find PDFium or the real datasheet prints why and returns, and still
//! counts as passed. Look for the numbers each test prints.
//!
//! **Why pieces come off rather than being painted over.** Hiding the other
//! pieces of an edited line in the page's colour leaves their words in the file
//! (white text is still text: it is searched, copied and read by anything that
//! reads the file); paints them *after* the new words, so a piece that a `Td`
//! places over the new text erases part of it — 23% of the ink of a line, measured;
//! leaves them for the next pick to find as members of the line; and is undone by
//! typing the old words back, which cannot bring back the kerning they had.
//!
//! **Every edit on the datasheet is made to a copy of it**, written to the
//! temporary directory and removed afterwards. The original is only ever read.

mod harness;
use harness::{serial, skip_without_pdfium};

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

use pdf_core::command::{Command, CommandHistory};
use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Color, Document, DocumentMut, Rect, RegionRequest, TextLineEdit, TextRun, TextStyle};
use pdf_core::pdf::content;
use pdf_core::render::{Bitmap, PixelOrder};

/// The datasheet this was built against: three A4 pages from Illustrator. Not in
/// the repository (42 MB), so a machine without it skips.
const DATASHEET: &str = r"C:\Users\hsili\Desktop\Datasheets - Editors market - Marina mall.pdf";

// ---------------------------------------------------------------- the copies --

/// A copy of the datasheet this test is free to edit, removed when dropped.
struct Copy(PathBuf);

impl Copy {
    fn of_the_datasheet(tag: &str) -> Option<Self> {
        skip_without_pdfium()?;
        if !std::path::Path::new(DATASHEET).exists() {
            eprintln!("skipped: the real datasheet is not at {DATASHEET}");
            return None;
        }
        let path = std::env::temp_dir().join(format!("pagify-replace-lines-{}-{tag}.pdf", std::process::id()));
        std::fs::copy(DATASHEET, &path).expect("copy the datasheet");
        Some(Copy(path))
    }

    fn open(&self) -> PdfiumDocument {
        PdfiumDocument::open_path(self.0.to_str().expect("path"), None).expect("open the copy")
    }
}

impl Drop for Copy {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

// ------------------------------------------------------- reading the document --

/// Every text object of page 1 — none filtered — as the page says it now.
fn runs(doc: &PdfiumDocument) -> Vec<TextRun> {
    doc.text_runs_unfiltered(0).expect("runs")
}

/// Words as the editor compares them: PDFium adds a space of its own to what it
/// reads back, and reports a hyphen drawn at a line's end as U+0002.
fn clean(words: &str) -> String {
    words.replace('\u{2}', "-").trim_end().to_string()
}

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325u64, |h, b| (h ^ *b as u64).wrapping_mul(0x100000001b3))
}

/// Page 1's decoded content stream, as the document would write it: the thing an
/// edit changes. Two documents with the same bytes here have the same page.
fn stream_of(doc: &mut PdfiumDocument) -> Vec<u8> {
    stream_of_page(doc, 0)
}

/// The same for page `n` (counting from 0).
fn stream_of_page(doc: &mut PdfiumDocument, n: usize) -> Vec<u8> {
    use pdf_core::pdf::Object;
    let mut bytes = Vec::new();
    DocumentMut::save_full_copy(doc, &mut bytes).expect("save");
    let file = pdf_core::pdf::File::parse(&bytes).expect("parse");
    let root = file.resolve(file.trailer().get(b"Root").expect("root")).expect("catalogue");
    let pages = file.resolve(root.as_dict().and_then(|d| d.get(b"Pages")).expect("pages")).expect("tree");
    fn pages_of(file: &pdf_core::pdf::File<'_>, node: &Object, out: &mut Vec<Object>) {
        let Some(dict) = node.as_dict() else { return };
        if dict.get(b"Type").and_then(Object::as_name) == Some(&b"Page"[..]) {
            out.push(node.clone());
            return;
        }
        if let Some(Ok(Object::Array(kids))) = dict.get(b"Kids").map(|k| file.resolve(k)) {
            for kid in kids {
                if let Ok(kid) = file.resolve(&kid) {
                    pages_of(file, &kid, out);
                }
            }
        }
    }
    let mut all_pages = Vec::new();
    pages_of(&file, &pages, &mut all_pages);
    let page = all_pages.get(n).cloned().unwrap_or_else(|| panic!("no page {}", n + 1));
    let contents = page.as_dict().and_then(|d| d.get(b"Contents")).expect("contents");
    let mut stream = Vec::new();
    match file.resolve(contents).expect("contents") {
        Object::Stream(d, r) => stream.extend_from_slice(&content::decode(&d, &bytes[r]).expect("decode")),
        Object::Array(parts) => {
            for part in parts {
                if let Ok(Object::Stream(d, r)) = file.resolve(&part) {
                    stream.extend_from_slice(&content::decode(&d, &bytes[r]).expect("decode"));
                    stream.push(b'\n');
                }
            }
        }
        _ => panic!("odd contents"),
    }
    stream
}

/// Each show-text operator of a stream as the bytes it occupies, operands included.
fn shows_of(stream: &[u8]) -> Vec<Vec<u8>> {
    content::parse(stream).expect("parse").iter().filter(|o| o.shows_text()).map(|o| stream[o.span.clone()].to_vec()).collect()
}

/// Each operation as one line of text with its whitespace squeezed.
fn operation_texts(stream: &[u8]) -> Vec<String> {
    content::parse(stream)
        .expect("parse")
        .iter()
        .map(|o| String::from_utf8_lossy(&stream[o.span.clone()]).split_whitespace().collect::<Vec<_>>().join(" "))
        .collect()
}

fn timing_line(doc: &mut PdfiumDocument) -> String {
    doc.take_last_batch_timing()
        .iter()
        .map(|(what, took)| format!("{} {:.1} ms", what.trim(), took.as_secs_f64() * 1000.0))
        .collect::<Vec<_>>()
        .join(" | ")
}

// ------------------------------------------------------------- the pictures --

fn render(doc: &PdfiumDocument, scale: f32) -> Bitmap {
    let size = doc.page_size(0).expect("size");
    doc.page(0)
        .expect("page")
        .render_region(&RegionRequest {
            crop: Rect { left: 0.0, top: 0.0, right: size.width_pt, bottom: size.height_pt },
            scale,
            render_annotations: false,
            render_form_data: false,
        })
        .expect("render")
}

fn rgb(b: &Bitmap, x: u32, y: u32) -> (u8, u8, u8) {
    let at = y as usize * b.stride + x as usize * 4;
    match b.order {
        PixelOrder::Rgba => (b.data[at], b.data[at + 1], b.data[at + 2]),
        PixelOrder::Bgra => (b.data[at + 2], b.data[at + 1], b.data[at]),
    }
}

/// A rectangle in points as a pixel box `(left, top, right, bottom)`.
fn pixel_box(b: &Bitmap, scale: f32, r: &Rect) -> (u32, u32, u32, u32) {
    let clamp = |v: f32, max: u32| (v.max(0.0) as u32).min(max);
    (
        clamp((r.left * scale).floor(), b.width),
        clamp((r.top * scale).floor(), b.height),
        clamp((r.right * scale).ceil(), b.width),
        clamp((r.bottom * scale).ceil(), b.height),
    )
}

/// How many pixels of a region differ from the page's own colour by enough to be ink.
fn ink(b: &Bitmap, scale: f32, r: &Rect, page: (u8, u8, u8)) -> usize {
    let (l, t, rr, bb) = pixel_box(b, scale, r);
    let mut n = 0;
    for y in t..bb {
        for x in l..rr {
            let (pr, pg, pb) = rgb(b, x, y);
            let d = (pr as i32 - page.0 as i32).abs().max((pg as i32 - page.1 as i32).abs()).max((pb as i32 - page.2 as i32).abs());
            if d > 24 {
                n += 1;
            }
        }
    }
    n
}

/// How many pixels differ between two renders of the same page, outside `skip`.
fn differing_pixels(a: &Bitmap, b: &Bitmap, scale: f32, skip: &[Rect]) -> usize {
    assert_eq!((a.width, a.height), (b.width, b.height), "the two renders are not the same size");
    let skip: Vec<(u32, u32, u32, u32)> = skip.iter().map(|r| pixel_box(a, scale, r)).collect();
    let mut n = 0;
    for y in 0..a.height {
        for x in 0..a.width {
            if skip.iter().any(|&(l, t, r, bt)| x >= l && x < r && y >= t && y < bt) {
                continue;
            }
            if rgb(a, x, y) != rgb(b, x, y) {
                n += 1;
            }
        }
    }
    n
}

fn union(rects: &[Rect]) -> Rect {
    let mut u = rects[0];
    for r in rects {
        u = Rect { left: u.left.min(r.left), top: u.top.min(r.top), right: u.right.max(r.right), bottom: u.bottom.max(r.bottom) };
    }
    u
}

fn grown(r: Rect, by: f32) -> Rect {
    Rect { left: r.left - by, top: r.top - by, right: r.right + by, bottom: r.bottom + by }
}

// ----------------------------------------------------- the datasheet paragraph --

/// The paragraph under page 1's "The Light Source - COB" heading: objects
/// 985..=1043, less 1026 and 1035, which Illustrator converted to outlines. 57
/// text objects in 13 lines of 2 to 7 pieces.
fn paragraph() -> Vec<usize> {
    (985..=1043).filter(|o| *o != 1026 && *o != 1035).collect()
}

/// The paragraph's lines, top to bottom, each with its objects in stream order.
/// The first is the piece the app types a line's new words into.
fn lines_of(all: &[TextRun]) -> Vec<Vec<usize>> {
    let mut by_baseline: BTreeMap<i64, Vec<usize>> = BTreeMap::new();
    for object in paragraph() {
        let run = all.iter().find(|r| r.object == object).expect("a paragraph object");
        by_baseline.entry((run.origin.y * 2.0).round() as i64).or_default().push(object);
    }
    by_baseline
        .into_values()
        .map(|mut line| {
            line.sort_unstable();
            line
        })
        .collect()
}

/// What typing a line's words into its first piece looks like, three ways: a
/// line **longer** than it was (its words and its first word again), the same
/// words, and a **shorter** one (the first half of them). Made of the line's own
/// letters in its first piece's own font, so that font can spell all of it.
fn new_text(doc: &PdfiumDocument, all: &[TextRun], line: &[usize], kind: usize) -> String {
    let styles = doc.run_styles(0).expect("styles");
    let font = styles[&line[0]].font;
    let text_of = |o: usize| all.iter().find(|r| r.object == o).expect("object").text.clone();
    let joined: String = line.iter().filter(|o| styles[o].font == font).map(|o| text_of(*o)).collect::<String>();
    let joined = clean(&joined);
    let words: Vec<&str> = joined.split(' ').filter(|w| !w.is_empty()).collect();
    match kind % 3 {
        0 => format!("{joined} {}", words[0]),
        1 => words[..(words.len() / 2).max(1)].join(" "),
        _ => joined,
    }
}

/// What the app sends to apply an edited paragraph: each line's first piece
/// retyped and every other piece of it removed.
fn paragraph_edits(doc: &PdfiumDocument, all: &[TextRun], lines: &[Vec<usize>]) -> (Vec<TextLineEdit>, Vec<String>) {
    let mut edits = Vec::new();
    let mut texts = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let text = new_text(doc, all, line, i);
        texts.push(text.clone());
        edits.push(TextLineEdit::Retype {
            first: line[0],
            text,
            style: TextStyle::default(),
            remove: line[1..].to_vec(),
            justify_to: None,
        });
    }
    (edits, texts)
}

/// Where an object is numbered after `removed` have come off the page.
fn renumbered(object: usize, removed: &[usize]) -> usize {
    object - removed.iter().filter(|r| **r < object).count()
}

fn same_but_object(a: &TextRun, b: &TextRun) -> bool {
    a.text == b.text && a.rect == b.rect && a.origin == b.origin && a.size == b.size && a.color == b.color
}

// ============================================================ the datasheet ==

/// **The paragraph, applied as the app will apply it.** 13 lines, each typed onto
/// its first piece — longer than the line was for five of them, shorter for four,
/// the same words for four — and each line's other 44 pieces removed.
///
/// What it holds the result to:
///
/// - the words and the objects: every first piece reads what was typed, every
///   removed object is gone (883 text objects become 839), and every other object
///   on the page is unchanged in its words, colour, box, origin and size, at its
///   new number;
/// - the stream: 44 show-text operators fewer; every one that is not a first piece
///   byte for byte what it was;
/// - the picture at 1.5x: nothing differs from a reference — the same new words
///   with the other pieces' codes taken out, which is what the removal is meant to
///   look like — anywhere on the page, and no old word of a removed piece shows;
/// - undo, in one step: the page's content stream **byte for byte** as it was, and
///   a render pixel for pixel the same, all 13 lines, the 4 kerned ones too;
/// - redo: the same bytes as the apply.
#[test]
fn the_whole_paragraph_is_replaced_and_undone_byte_for_byte() {
    let Some(copy) = Copy::of_the_datasheet("paragraph") else { return };
    let _lock = serial();
    let mut doc = copy.open();

    let before = runs(&doc);
    assert_eq!(before.len(), 883, "page 1 has 883 text objects");
    let lines = lines_of(&before);
    let firsts: Vec<usize> = lines.iter().map(|l| l[0]).collect();
    assert_eq!(firsts, [985, 989, 991, 995, 1001, 1007, 1014, 1018, 1025, 1030, 1033, 1039, 1041]);
    let removed: Vec<usize> = lines.iter().flat_map(|l| l[1..].iter().copied()).collect();
    assert_eq!(removed.len(), 44);
    let (edits, texts) = paragraph_edits(&doc, &before, &lines);
    for (i, text) in texts.iter().enumerate() {
        println!("line {:>2}: first piece {:>4}, {:>7}: {text:?}", i + 1, lines[i][0], ["longer", "shorter", "same"][i % 3]);
    }

    let stream_before = stream_of(&mut doc);
    let image_before = render(&doc, 1.5);

    // ---- the reference: the same new words, the other pieces' codes taken out
    let reference = {
        let mut other = copy.open();
        let mut list: Vec<(usize, String, TextStyle)> = Vec::new();
        for (i, line) in lines.iter().enumerate() {
            list.push((line[0], texts[i].clone(), TextStyle::default()));
            for o in &line[1..] {
                list.push((*o, " ".to_string(), TextStyle::default()));
            }
        }
        other.set_text_runs_styled(0, &list).unwrap_or_else(|e| panic!("the reference was refused: {e}"));
        render(&other, 1.5)
    };

    // ---- apply, as one undoable step
    let mut history = CommandHistory::new(8);
    let started = Instant::now();
    let applied = history.execute(Command::ReplaceTextLines { page_index: 0, edits: edits.clone() }, &mut doc);
    let took = started.elapsed();
    let timing = timing_line(&mut doc);
    if let Err(error) = &applied {
        panic!("the paragraph was refused after {:.0} ms: {error}", took.as_secs_f64() * 1000.0);
    }
    println!("13 lines typed, 44 pieces removed, applied in {:.0} ms   {timing}", took.as_secs_f64() * 1000.0);

    // ---- the objects
    let after = runs(&doc);
    assert_eq!(after.len(), 883 - 44, "883 text objects less the 44 removed");
    let first_set: HashSet<usize> = firsts.iter().copied().collect();
    let removed_set: HashSet<usize> = removed.iter().copied().collect();
    let (mut typed_n, mut other_n) = (0, 0);
    for b in &before {
        if removed_set.contains(&b.object) {
            continue;
        }
        let now = renumbered(b.object, &removed);
        let a = after.iter().find(|r| r.object == now).unwrap_or_else(|| panic!("object {} (now {now}) is gone", b.object));
        if first_set.contains(&b.object) {
            let line = lines.iter().position(|l| l[0] == b.object).expect("a line");
            assert_eq!(clean(&a.text), texts[line], "first piece {} should read what was typed", b.object);
            assert_eq!(a.color, b.color, "first piece {} changed colour", b.object);
            typed_n += 1;
        } else {
            assert!(
                same_but_object(a, b),
                "object {} (now {now}) is not part of the paragraph and changed:\n  {b:?}\n  {a:?}",
                b.object
            );
            other_n += 1;
        }
    }
    println!("{typed_n} first pieces read what was typed; {other_n} other objects identical in words, colour, box, origin and size at their new numbers");
    assert_eq!((typed_n, other_n), (13, 826));

    // ---- the stream
    let stream_after = stream_of(&mut doc);
    let (shows_before, shows_after) = (shows_of(&stream_before), shows_of(&stream_after));
    assert_eq!(shows_after.len(), shows_before.len() - 44, "44 show-text operators come out");
    let ordinal = |o: usize| before.iter().position(|r| r.object == o).expect("ordinal");
    let first_ordinals: HashSet<usize> = firsts.iter().map(|o| ordinal(*o)).collect();
    let removed_ordinals: HashSet<usize> = removed.iter().map(|o| ordinal(*o)).collect();
    let survivors: Vec<usize> = (0..shows_before.len()).filter(|n| !removed_ordinals.contains(n)).collect();
    assert_eq!(survivors.len(), shows_after.len());
    for (k, n) in survivors.iter().enumerate() {
        if first_ordinals.contains(n) {
            assert!(shows_after[k] != shows_before[*n], "show-text operator {n} is a first piece and was not retyped");
        } else {
            assert!(shows_after[k] == shows_before[*n], "show-text operator {n} is not a first piece and its bytes changed");
        }
    }

    // ---- the picture
    let image_after = render(&doc, 1.5);
    let against_reference = differing_pixels(&image_after, &reference, 1.5, &[]);
    let against_before = differing_pixels(&image_after, &image_before, 1.5, &[]);
    println!("pixels that differ from the reference (the same words, the other pieces' codes taken out): {against_reference}; from the page as it was: {against_before}");
    assert_eq!(against_reference, 0, "the page does not look as the new words alone should look");
    assert!(against_before > 1000, "the page does not look edited at all");
    // No old word of a removed piece shows where it was, except where a new line
    // is drawn over the place (the reference has that ink too).
    let page = (255u8, 255u8, 255u8);
    let mut checked = 0;
    for &o in &removed {
        let r = before.iter().find(|r| r.object == o).expect("object").rect;
        let place = Rect { left: r.left + 0.4, top: r.top + 0.4, right: r.right - 0.4, bottom: r.bottom - 0.4 };
        if place.right - place.left < 2.0 {
            continue;
        }
        let (now, ideal) = (ink(&image_after, 1.5, &place, page), ink(&reference, 1.5, &place, page));
        assert_eq!(now, ideal, "removed piece {o}: its place holds {now} ink, the reference's {ideal}");
        checked += 1;
    }
    println!("{checked} removed pieces checked: their places hold exactly the ink of the reference");

    // ---- the re-pick: what a later block detection sees
    let (old_left, old_right) = {
        let u = union(&paragraph().iter().map(|o| before.iter().find(|r| r.object == *o).expect("object").rect).collect::<Vec<_>>());
        (u.left, u.right)
    };
    for (i, line) in lines.iter().enumerate() {
        let baseline = after.iter().find(|r| r.object == renumbered(line[0], &removed)).expect("first piece").origin.y;
        let here: Vec<&TextRun> = after
            .iter()
            .filter(|r| (r.origin.y - baseline).abs() < 0.5 && r.origin.x >= old_left - 1.0 && r.origin.x <= old_right + 1.0)
            .filter(|r| !clean(&r.text).is_empty())
            .collect();
        assert_eq!(here.len(), 1, "line {}: after the apply {} pieces sit on its baseline: {:?}", i + 1, here.len(), here.iter().map(|r| clean(&r.text)).collect::<Vec<_>>());
    }
    println!("every edited line is one piece on its baseline: no leftover piece for a later pick to find");

    // ---- undo, in one step, byte for byte
    let undone = history.undo(&mut doc).expect("undo");
    assert!(undone.is_some(), "the whole paragraph is one step to undo");
    let stream_undone = stream_of(&mut doc);
    println!(
        "content stream: {} bytes before, {} after the apply, {} after undo; fingerprints {:016x} / {:016x} / {:016x}",
        stream_before.len(),
        stream_after.len(),
        stream_undone.len(),
        fnv(&stream_before),
        fnv(&stream_after),
        fnv(&stream_undone)
    );
    assert!(stream_undone == stream_before, "after undo the content stream is not byte for byte what it was");
    let restored = runs(&doc);
    assert_eq!(restored.len(), 883);
    assert!(restored == before, "after undo the objects are not exactly what they were");
    let image_undone = render(&doc, 1.5);
    let off = differing_pixels(&image_before, &image_undone, 1.5, &[]);
    println!("pixels that differ after undo, anywhere on the page: {off}");
    assert_eq!(off, 0, "after undo the render is not pixel for pixel what it was");

    // ---- redo: the same bytes as the apply
    let redone = history.redo(&mut doc).expect("redo");
    assert!(redone.is_some(), "the paragraph can be redone");
    let stream_redone = stream_of(&mut doc);
    assert!(stream_redone == stream_after, "redo wrote different bytes from the apply");
    assert!(runs(&doc) == after, "redo did not put back the objects the apply made");
    println!("redo wrote the same {} bytes as the apply", stream_redone.len());
}

/// **A line taken off outright.** The last line's three pieces, removed: gone, and
/// nothing else on the page changes, at its new numbers; undo puts the page back
/// byte for byte.
#[test]
fn a_whole_line_can_be_taken_off_and_put_back() {
    let Some(copy) = Copy::of_the_datasheet("line") else { return };
    let _lock = serial();
    let mut doc = copy.open();

    let before = runs(&doc);
    let lines = lines_of(&before);
    let line = lines.last().expect("a line").clone();
    assert_eq!(line, [1041, 1042, 1043]);
    let stream_before = stream_of(&mut doc);
    let image_before = render(&doc, 1.5);
    let rect = union(&line.iter().map(|o| before.iter().find(|r| r.object == *o).expect("object").rect).collect::<Vec<_>>());

    let mut history = CommandHistory::new(4);
    history
        .execute(
            Command::ReplaceTextLines { page_index: 0, edits: vec![TextLineEdit::Remove { objects: line.clone() }] },
            &mut doc,
        )
        .unwrap_or_else(|e| panic!("the line was refused: {e}"));
    let after = runs(&doc);
    assert_eq!(after.len(), 880);
    for b in before.iter().filter(|b| !line.contains(&b.object)) {
        let a = after.iter().find(|r| r.object == renumbered(b.object, &line)).expect("object");
        assert!(same_but_object(a, b), "object {} changed", b.object);
    }
    let image_after = render(&doc, 1.5);
    let outside = differing_pixels(&image_before, &image_after, 1.5, &[grown(rect, 2.0)]);
    let page = (255u8, 255u8, 255u8);
    let (was, now) = (ink(&image_before, 1.5, &rect, page), ink(&image_after, 1.5, &rect, page));
    println!("the line's box held {was} ink and holds {now}; pixels that differ outside it: {outside}");
    assert_eq!(outside, 0);
    assert!(was > 100 && now == 0, "the line is not gone from the page");

    history.undo(&mut doc).expect("undo").expect("a step to undo");
    assert!(stream_of(&mut doc) == stream_before, "after undo the content stream is not byte for byte what it was");
    assert!(runs(&doc) == before);
    assert_eq!(differing_pixels(&image_before, &render(&doc, 1.5), 1.5, &[]), 0);
}

/// **A bad object id refuses the whole batch and writes nothing** — the last of
/// many good edits and removals — and the history is as it was: nothing to undo.
#[test]
fn a_bad_object_refuses_the_whole_batch_and_leaves_the_history_alone() {
    let Some(copy) = Copy::of_the_datasheet("bad") else { return };
    let _lock = serial();
    let mut doc = copy.open();

    let before = runs(&doc);
    let lines = lines_of(&before);
    let stream_before = stream_of(&mut doc);
    let (good, _) = paragraph_edits(&doc, &before, &lines);

    for (bad, why) in [(1026usize, "an outline, not text"), (99_999, "no such object")] {
        let mut edits = good.clone();
        edits.push(TextLineEdit::Remove { objects: vec![bad] });
        let mut history = CommandHistory::new(4);
        let error = history
            .execute(Command::ReplaceTextLines { page_index: 0, edits }, &mut doc)
            .err()
            .unwrap_or_else(|| panic!("object {bad} ({why}) was accepted"));
        println!("object {bad} ({why}): refused: {error}");
        assert!(stream_of(&mut doc) == stream_before, "object {bad}: a refused batch changed the stream");
        assert!(runs(&doc) == before, "object {bad}: a refused batch changed the objects");
        assert!(history.undo(&mut doc).expect("undo").is_none(), "object {bad}: a refused batch left something to undo");
    }

    // A bad id inside a Retype's own list, and an object named twice.
    let mut edits = good.clone();
    if let TextLineEdit::Retype { remove, .. } = &mut edits[0] {
        remove.push(1026);
    }
    assert!(doc.replace_text_lines(0, &edits).is_err(), "a bad id in a remove list was accepted");
    let mut edits = good.clone();
    edits.push(TextLineEdit::Remove { objects: vec![986] }); // already in line 1's remove list
    let error = doc.replace_text_lines(0, &edits).err().expect("an object named twice must be refused");
    println!("an object named twice: refused: {error}");
    assert!(stream_of(&mut doc) == stream_before);
    assert!(runs(&doc) == before);
}

// ============================================================= synthetic pages ==

/// A one-page PDF drawing `content`, with Helvetica as `/F1`.
fn page_with(content: &[u8]) -> Vec<u8> {
    page_with_extra(content, "", &[])
}

/// The same, with `resources` added to the page's resource dictionary and
/// `extra` as objects 6, 7, ...
fn page_with_extra(content: &[u8], resources: &str, extra: &[Vec<u8>]) -> Vec<u8> {
    let mut objects: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R \
             /Resources << /Font << /F1 5 0 R >> {resources} >> >>"
        )
        .into_bytes(),
        [format!("<< /Length {} >>\nstream\n", content.len()).as_bytes(), content, b"\nendstream"].concat(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
    ];
    objects.extend(extra.iter().cloned());
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref_at = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in &offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF", objects.len() + 1).as_bytes(),
    );
    out
}

fn open(content: &[u8]) -> PdfiumDocument {
    PdfiumDocument::open_bytes(page_with(content), None).expect("open")
}

fn words(doc: &PdfiumDocument) -> Vec<String> {
    runs(doc).iter().map(|r| clean(&r.text)).collect()
}

fn remove(objects: &[usize]) -> TextLineEdit {
    TextLineEdit::Remove { objects: objects.to_vec() }
}

fn retype(first: usize, text: &str, remove: &[usize]) -> TextLineEdit {
    TextLineEdit::Retype { first, text: text.to_string(), style: TextStyle::default(), remove: remove.to_vec(), justify_to: None }
}

/// **The quote operators move to the next line, and what they leave behind is
/// what keeps the lines after them in place.** `(Bravo) '` is `T* (Bravo) Tj`;
/// cut out whole it would pull every line after it up. Here lines 2 and 4 are
/// quotes (4 a `"`, which also sets the word and character spacing the last line
/// is drawn with), and taking them out leaves the other lines exactly where they
/// were — and the last line exactly as wide, because the `Tc` is still in force.
#[test]
fn removing_a_quote_operator_keeps_the_lines_after_it_in_place() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    let mut doc = open(
        b"BT /F1 12 Tf 14 TL 72 700 Td (Alpha) Tj (Bravo) ' (Charlie) ' 3 1 (Delta) \" (Echo) ' ET",
    );
    let before = runs(&doc);
    assert_eq!(words(&doc), ["Alpha", "Bravo", "Charlie", "Delta", "Echo"]);
    let stream_before = stream_of(&mut doc);

    doc.replace_text_lines(0, &[remove(&[1]), remove(&[3])]).unwrap_or_else(|e| panic!("refused: {e}"));
    let after = runs(&doc);
    assert_eq!(words(&doc), ["Alpha", "Charlie", "Echo"]);
    for (kept, now) in [(0usize, 0usize), (2, 1), (4, 2)] {
        let (b, a) = (&before[kept], &after[now]);
        println!("{:>8}: origin {:?} -> {:?}, width {:.2} -> {:.2}", clean(&b.text), b.origin, a.origin, b.rect.right - b.rect.left, a.rect.right - a.rect.left);
        assert!(same_but_object(a, b), "{} moved or changed", clean(&b.text));
    }
    let texts = operation_texts(&stream_of(&mut doc));
    assert!(texts.iter().any(|t| t == "T*"), "the line a `'` moved to is not kept: {texts:?}");
    assert!(texts.iter().any(|t| t == "3 Tw" || t == "3.0 Tw"), "the `\"`'s word spacing is not kept: {texts:?}");
    assert!(texts.iter().any(|t| t == "1 Tc" || t == "1.0 Tc"), "the `\"`'s character spacing is not kept: {texts:?}");
    assert!(stream_of(&mut doc) != stream_before);
}

/// **A quote operator that is retyped keeps its line too**: the `TJ` written in
/// its place does not move to the next line, so the move is written first.
#[test]
fn retyping_a_quote_operator_keeps_its_line_and_the_lines_after_it() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    let mut doc = open(
        b"BT /F1 12 Tf 14 TL 72 700 Td (Alpha) Tj (Bravo) ' (Charlie) ' 3 1 (Delta) \" (Echo) ' ET",
    );
    let before = runs(&doc);

    doc.replace_text_lines(0, &[retype(1, "Bravo!", &[]), retype(3, "Delta?", &[])])
        .unwrap_or_else(|e| panic!("refused: {e}"));
    let after = runs(&doc);
    assert_eq!(words(&doc), ["Alpha", "Bravo!", "Charlie", "Delta?", "Echo"]);
    for kept in [0usize, 2, 4] {
        assert!(same_but_object(&after[kept], &before[kept]), "{} moved or changed", clean(&before[kept].text));
    }
    for typed in [1usize, 3] {
        assert!(
            (after[typed].origin.y - before[typed].origin.y).abs() < 0.01 && (after[typed].origin.x - before[typed].origin.x).abs() < 0.01,
            "{} left its line: {:?} -> {:?}",
            clean(&before[typed].text),
            before[typed].origin,
            after[typed].origin
        );
    }
}

/// **A piece that the pen places cannot lose the piece before it.** In
/// `(Alpha) Tj (beta) Tj (gamma) Tj` each piece starts where the one before
/// ended; taking `Alpha` out would move `beta` to the line's start. Refused, and
/// nothing is written. A piece placed by its own `Td` or `'` is not moved by what
/// is taken out before it, and pieces that all go together move nothing.
#[test]
fn a_piece_the_pen_places_is_not_left_behind_to_move() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    let content = b"BT /F1 12 Tf 72 700 Td (Alpha) Tj (beta) Tj (gamma) Tj ET";
    let mut doc = open(content);
    let before = stream_of(&mut doc);
    let runs_before = runs(&doc);

    let refused: [(&str, Vec<TextLineEdit>); 4] = [
        ("Alpha out, beta and gamma kept", vec![remove(&[0])]),
        ("beta out, gamma kept", vec![remove(&[1])]),
        ("a retyped first piece, beta out, gamma kept", vec![retype(0, "Alpha!", &[1])]),
        ("Alpha out, beta retyped and kept", vec![retype(1, "beta!", &[0])]),
    ];
    for (name, edits) in refused {
        let error = doc.replace_text_lines(0, &edits).err().unwrap_or_else(|| panic!("{name}: accepted: {:?}", words(&doc)));
        println!("{name}: refused: {error}");
        assert_eq!(words(&doc), ["Alpha", "beta", "gamma"], "{name}: a refused batch changed the page");
        assert!(stream_of(&mut doc) == before, "{name}: a refused batch changed the stream");
        assert!(runs(&doc) == runs_before);
    }

    // Allowed: everything after goes with it.
    doc.replace_text_lines(0, &[retype(0, "Alpha!", &[1, 2])]).unwrap_or_else(|e| panic!("refused: {e}"));
    assert_eq!(words(&doc), ["Alpha!"]);

    // Allowed: the pieces after are placed by operators of their own.
    for (name, content) in [
        ("a Td", &b"BT /F1 12 Tf 72 700 Td (Alpha) Tj 60 0 Td (beta) Tj 60 0 Td (gamma) Tj ET"[..]),
        ("a Tm", &b"BT /F1 12 Tf 1 0 0 1 72 700 Tm (Alpha) Tj 1 0 0 1 150 700 Tm (beta) Tj ET"[..]),
        ("a quote", &b"BT /F1 12 Tf 14 TL 72 700 Td (Alpha) Tj (beta) ' ET"[..]),
        ("a new text object", &b"BT /F1 12 Tf 72 700 Td (Alpha) Tj ET BT /F1 12 Tf 150 700 Td (beta) Tj ET"[..]),
    ] {
        let mut doc = open(content);
        let was = runs(&doc);
        doc.replace_text_lines(0, &[remove(&[0])]).unwrap_or_else(|e| panic!("{name}: refused: {e}"));
        let now = runs(&doc);
        assert_eq!(now.len(), was.len() - 1, "{name}");
        for (i, b) in was.iter().enumerate().skip(1) {
            assert!(same_but_object(&now[i - 1], b), "{name}: {} moved: {:?} -> {:?}", clean(&b.text), b.origin, now[i - 1].origin);
        }
    }
}

/// **Spaces between the pieces of a line do not hold up taking the pieces out.**
/// Illustrator writes a space as a `( ) Tj` of its own between the words of a
/// justified line. Each advances the pen like any piece but has no ink — PDFium
/// reports no text and a box of no area — and the app, which lists only what it
/// can see, does not name them. Left behind they move with the pen and move no
/// pixel; a *seen* piece left behind after them still moves, and still refuses.
#[test]
fn spaces_between_the_pieces_of_a_line_do_not_hold_up_a_removal() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    let content = b"BT /F1 12 Tf 72 700 Td (Alpha) Tj ( ) Tj (beta) Tj ( ) Tj (gamma) Tj ET";
    let mut doc = open(content);
    let all = runs(&doc);
    let space = &all[1];
    println!("the space: text {:?}, box {:?}", space.text, space.rect);
    assert!(
        (space.rect.right - space.rect.left).abs() < 0.01 && (space.rect.bottom - space.rect.top).abs() < 0.01,
        "PDFium gives a space an area, so there is nothing here to test: {:?}",
        space.rect
    );
    let before = stream_of(&mut doc);

    // The words go; the spaces, which nobody named, stay and draw nothing.
    doc.replace_text_lines(0, &[remove(&[0, 2, 4])]).unwrap_or_else(|e| panic!("the words were refused: {e}"));
    assert_eq!(words(&doc), ["", ""], "only the two spaces are left");

    // A seen piece left behind after a space still refuses.
    let mut doc = open(content);
    let error = doc.replace_text_lines(0, &[remove(&[0, 2])]).err().unwrap_or_else(|| panic!("accepted: {:?}", words(&doc)));
    println!("a seen piece left after a space: refused: {error}");
    assert_eq!(words(&doc), ["Alpha", "", "beta", "", "gamma"]);
    assert!(stream_of(&mut doc) == before);

    // And a retyped line whose other words go, the spaces left between.
    let mut doc = open(content);
    doc.replace_text_lines(0, &[retype(0, "Alpha beta gamma", &[2, 4])]).unwrap_or_else(|e| panic!("refused: {e}"));
    let now = words(&doc);
    assert_eq!(now.iter().filter(|w| !w.is_empty()).collect::<Vec<_>>(), ["Alpha beta gamma"]);
}

/// **Text that clips cannot be taken out.** Rendering modes 4 to 7 add the glyphs
/// to the clipping path, so taking such a piece out changes what everything after
/// it is clipped to. Refused, and nothing is written.
#[test]
fn text_that_adds_to_the_clipping_path_is_not_taken_out() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    let mut doc = open(b"BT /F1 12 Tf 72 700 Td (Alpha) Tj ET BT /F1 12 Tf 7 Tr 72 650 Td (Clip) Tj ET");
    let before = stream_of(&mut doc);
    let error = doc.replace_text_lines(0, &[remove(&[1])]).err().unwrap_or_else(|| panic!("accepted: {:?}", words(&doc)));
    println!("refused: {error}");
    assert!(stream_of(&mut doc) == before);
    // The text before it is not clipping, and comes out.
    doc.replace_text_lines(0, &[remove(&[0])]).unwrap_or_else(|e| panic!("refused: {e}"));
}

/// **What a retype cannot do in the same step is refused, not half done.**
#[test]
fn a_retype_that_asks_for_a_colour_or_a_position_is_refused() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    let mut doc = open(b"BT /F1 12 Tf 72 700 Td (Alpha) Tj ET BT /F1 12 Tf 72 650 Td (Bravo) Tj ET");
    let before = stream_of(&mut doc);
    let coloured = TextLineEdit::Retype {
        first: 0,
        text: "Alpha!".into(),
        style: TextStyle { color: Some(Color { r: 1, g: 2, b: 3, a: 255 }), ..Default::default() },
        remove: vec![],
        justify_to: None,
    };
    let moved = TextLineEdit::Retype {
        first: 0,
        text: "Alpha!".into(),
        style: TextStyle { at: Some((10.0, 10.0)), ..Default::default() },
        remove: vec![],
        justify_to: None,
    };
    for (name, edit) in [("a colour", coloured), ("a position", moved)] {
        let error = doc.replace_text_lines(0, &[edit]).err().unwrap_or_else(|| panic!("{name}: accepted"));
        println!("{name}: refused: {error}");
        assert_eq!(words(&doc), ["Alpha", "Bravo"]);
        assert!(stream_of(&mut doc) == before);
    }
    // An empty batch is nothing to do, not an error.
    doc.replace_text_lines(0, &[]).expect("an empty batch");
    assert!(stream_of(&mut doc) == before);
}

/// **A size and a face travel with the words**, as they do through
/// `set_text_runs_styled`, and the piece after the retyped one that has a `Td`
/// of its own is where it was.
#[test]
fn a_retyped_line_takes_its_size() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    let mut doc = open(b"BT /F1 12 Tf 72 700 Td (Alpha) Tj (beta) Tj 100 0 Td (gamma) Tj ET");
    let before = runs(&doc);
    doc.replace_text_lines(
        0,
        &[TextLineEdit::Retype {
            first: 0,
            text: "Alpha beta".into(),
            style: TextStyle { size: Some(18.0), ..Default::default() },
            remove: vec![1],
            justify_to: None,
        }],
    )
    .unwrap_or_else(|e| panic!("refused: {e}"));
    let after = runs(&doc);
    assert_eq!(words(&doc), ["Alpha beta", "gamma"]);
    assert!((after[0].size - 18.0).abs() < 0.01, "the retyped piece is {} pt", after[0].size);
    assert!(same_but_object(&after[1], &before[2]), "the piece placed by its own Td moved or changed");
}

/// **Text inside a form is not a page object**, so there is nothing to name it by:
/// the form is one object, and naming it asks for something that is not a piece
/// of text. Refused, and nothing is written.
#[test]
fn text_inside_a_form_cannot_be_named_and_the_form_is_not_text() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    let body = b"BT /F1 10 Tf 0 0 Td (inside) Tj ET";
    let form = [
        format!(
            "<< /Type /XObject /Subtype /Form /BBox [0 0 100 100] /Resources << /Font << /F1 5 0 R >> >> /Length {} >>\nstream\n",
            body.len()
        )
        .as_bytes(),
        body.as_slice(),
        b"\nendstream",
    ]
    .concat();
    let mut doc = PdfiumDocument::open_bytes(
        page_with_extra(b"BT /F1 12 Tf 72 700 Td (Alpha) Tj ET\nq 1 0 0 1 72 600 cm /Fm1 Do Q", "/XObject << /Fm1 6 0 R >>", &[form]),
        None,
    )
    .expect("open");
    assert_eq!(words(&doc), ["Alpha"], "the form's text is not a text object of the page");
    let before = stream_of(&mut doc);
    let error = doc.replace_text_lines(0, &[remove(&[1])]).err().expect("the form was taken for a piece of text");
    println!("the form object: refused: {error}");
    assert!(stream_of(&mut doc) == before);
}

/// An Arial to type with, if this machine has one: a letter a document's own
/// subset fonts cannot spell has to come from a font written into the file.
fn arial() -> Option<Vec<u8>> {
    [r"C:\Windows\Fonts\arial.ttf", "/System/Library/Fonts/Supplemental/Arial.ttf", "/usr/share/fonts/truetype/msttcorefonts/Arial.ttf"]
        .iter()
        .find_map(|path| std::fs::read(path).ok())
}

/// **Lines that each need a new font are replaced one at a time, and still all or
/// nothing.** Typing `é` into one line and `ñ` into another makes each write a
/// font into the file, which a single pass does not do twice; the lines then go
/// one at a time and the removals in one pass after them, on a copy of the page
/// that is put back if any of it fails.
///
/// - one line needing a font goes through the single pass;
/// - two do not, and still arrive: their words, every piece removed, every other
///   object unchanged, undone byte for byte;
/// - a third that no font can spell fails *after* the first two were written, and
///   the page is put back exactly as it was.
#[test]
fn lines_that_each_need_a_new_font_are_replaced_one_at_a_time_and_atomically() {
    let Some(copy) = Copy::of_the_datasheet("fonts") else { return };
    let Some(font) = arial() else {
        eprintln!("skipped: no Arial on this machine to type letters the page cannot spell");
        return;
    };
    let _lock = serial();

    let setup = || {
        let mut doc = copy.open();
        doc.set_typing_fonts(vec![font.clone()]);
        doc
    };
    let mut doc = setup();
    let before = runs(&doc);
    let lines = lines_of(&before);
    let (a, b, c) = (&lines[1], &lines[2], &lines[3]);
    let stream_before = stream_of(&mut doc);

    // One line: the single pass.
    doc.replace_text_lines(0, &[retype(a[0], "plié by the COB", &a[1..])]).unwrap_or_else(|e| panic!("one line was refused: {e}"));
    let now = runs(&doc);
    assert_eq!(now.iter().find(|r| r.object == a[0]).map(|r| clean(&r.text)).as_deref(), Some("plié by the COB"));
    assert_eq!(now.len(), before.len() - a[1..].len());
    println!("one line needing a font: written in one pass");

    // Two lines: the one-at-a-time way.
    let mut doc = setup();
    let edits = vec![retype(a[0], "plié by the COB", &a[1..]), retype(b[0], "which ñare market", &b[1..])];
    let mut history = CommandHistory::new(4);
    let started = Instant::now();
    history
        .execute(Command::ReplaceTextLines { page_index: 0, edits: edits.clone() }, &mut doc)
        .unwrap_or_else(|e| panic!("two lines needing fonts were refused: {e}"));
    println!("two lines needing a font each: written one at a time in {:.0} ms", started.elapsed().as_secs_f64() * 1000.0);
    let removed: Vec<usize> = a[1..].iter().chain(b[1..].iter()).copied().collect();
    let after = runs(&doc);
    assert_eq!(after.len(), before.len() - removed.len());
    for (line, text) in [(a, "plié by the COB"), (b, "which ñare market")] {
        let got = after.iter().find(|r| r.object == renumbered(line[0], &removed)).expect("first piece");
        assert_eq!(clean(&got.text), text);
    }
    for o in before.iter().filter(|o| !removed.contains(&o.object) && o.object != a[0] && o.object != b[0]) {
        let now = after.iter().find(|r| r.object == renumbered(o.object, &removed)).expect("object");
        assert!(same_but_object(now, o), "object {} changed", o.object);
    }
    history.undo(&mut doc).expect("undo").expect("a step to undo");
    assert!(stream_of(&mut doc) == stream_before, "after undo the content stream is not byte for byte what it was");

    // Three lines, the last not spellable by any font: all or nothing.
    let mut doc = setup();
    let edits = vec![
        retype(a[0], "plié by the COB", &a[1..]),
        retype(b[0], "which ñare market", &b[1..]),
        retype(c[0], "lighting \u{2603} industry", &c[1..]),
    ];
    let mut history = CommandHistory::new(4);
    let error = history
        .execute(Command::ReplaceTextLines { page_index: 0, edits }, &mut doc)
        .err()
        .unwrap_or_else(|| panic!("a line no font can spell was accepted"));
    println!("a third line no font can spell: refused: {error}");
    assert!(stream_of(&mut doc) == stream_before, "the first two lines were left written");
    assert!(runs(&doc) == before, "the page was not put back as it was");
    assert!(history.undo(&mut doc).expect("undo").is_none(), "a refused command left something to undo");
}

/// What the zero-width text objects between the pieces of a line are. A probe.
#[test]
#[ignore = "a probe: what sits between the pieces of a line on page 2"]
fn probe_what_sits_between_the_pieces() {
    let Some(copy) = Copy::of_the_datasheet("between") else { return };
    let _lock = serial();
    let mut doc = copy.open();
    let all = doc.text_runs_unfiltered(1).expect("runs");
    let stream = stream_of_page(&mut doc, 1);
    let shows = shows_of(&stream);
    println!("page 2: {} text objects, {} show-text operators", all.len(), shows.len());
    for object in 1971..=1984usize {
        let Some(at) = all.iter().position(|r| r.object == object) else {
            println!("object {object}: not a text object");
            continue;
        };
        let r = &all[at];
        let op = String::from_utf8_lossy(&shows[at]).replace('\n', " ");
        println!(
            "object {object}: text {:?} size {} rect {:.3}..{:.3} x {:.3}..{:.3} colour {:?}  op {}",
            r.text,
            r.size,
            r.rect.left,
            r.rect.right,
            r.rect.top,
            r.rect.bottom,
            r.color,
            &op[..op.len().min(90)]
        );
    }
}

// ================================================================ measuring ==

/// What the paragraph costs, and where it goes: all 57 objects (13 retyped, 44
/// removed) as one command, three times. Not asserted. Run on demand:
///
/// ```text
/// PAGIFY_PDFIUM_LIB=<pdfium> cargo test --release --test replace_lines -- --ignored --nocapture
/// ```
#[test]
#[ignore = "a measurement, not a check; run with --ignored --nocapture"]
fn what_the_paragraph_costs() {
    let Some(copy) = Copy::of_the_datasheet("cost") else { return };
    let _lock = serial();
    for round in 1..=3 {
        let mut doc = copy.open();
        let before = runs(&doc);
        let lines = lines_of(&before);
        let (edits, _) = paragraph_edits(&doc, &before, &lines);
        let objects: usize = edits
            .iter()
            .map(|e| match e {
                TextLineEdit::Retype { remove, .. } => 1 + remove.len(),
                TextLineEdit::Remove { objects } => objects.len(),
            })
            .sum();

        let mut history = CommandHistory::new(2);
        let started = Instant::now();
        let outcome = history.execute(Command::ReplaceTextLines { page_index: 0, edits: edits.clone() }, &mut doc);
        let took = started.elapsed().as_secs_f64() * 1000.0;
        match outcome {
            Ok(_) => println!("round {round}: {objects} objects as one command in {took:7.1} ms   {}", timing_line(&mut doc)),
            Err(e) => println!("round {round}: {objects} objects REFUSED after {took:7.1} ms: {e}"),
        }

        let started = Instant::now();
        let undone = history.undo(&mut doc);
        println!("round {round}: undo (a page snapshot put back) in {:7.1} ms ({})", started.elapsed().as_secs_f64() * 1000.0, if undone.is_ok() { "ok" } else { "failed" });
        let started = Instant::now();
        let _ = history.redo(&mut doc);
        println!("round {round}: redo in {:7.1} ms", started.elapsed().as_secs_f64() * 1000.0);

        // The same edits through the trait, without the snapshot the command takes.
        let mut doc = copy.open();
        let started = Instant::now();
        let outcome = doc.replace_text_lines(0, &edits);
        let took = started.elapsed().as_secs_f64() * 1000.0;
        println!("round {round}: replace_text_lines alone in {took:7.1} ms ({})", if outcome.is_ok() { "ok" } else { "refused" });
    }
}
