//! Editing the pieces of a line: a text object that continues the one before it.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test --release --test continuation_edit -- --nocapture
//! ```
//!
//! `--nocapture` is the point of the flag, not a convenience: a test here that
//! cannot find PDFium or the real datasheet prints why and returns, and still
//! counts as passed. Look for the numbers each test prints.
//!
//! **Why these pieces are the hard case.** A producer writes a line as several
//! show-text operators with nothing repositioning between them (`(Th) Tj
//! [(e COB in )] TJ`), and the walk over the stream cannot know how far the first
//! one advanced the pen — so it reports every one of them at the line's start. A
//! piece found by *where it is* is then either not found (the editor refuses the
//! page) or, when the piece before it is narrow, found *wrongly*: the operator of
//! its neighbour. PDFium makes one text object per show-text operator, in the
//! order the stream draws them, so the *n*th object is the *n*th operator — and
//! the stream confirms it before the edit is allowed to rely on it.
//!
//! **Every edit on the datasheet is made to a copy of it**, written to the
//! temporary directory and removed afterwards. The original is only ever read.

mod harness;
use harness::{serial, skip_without_pdfium};

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

use pdf_core::command::{Command, CommandHistory};
use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Color, Document, DocumentMut, Rect, RegionRequest, TextRun, TextStyle};
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
        let path = std::env::temp_dir().join(format!("pagify-continuation-{}-{tag}.pdf", std::process::id()));
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
    use pdf_core::pdf::Object;
    let mut bytes = Vec::new();
    DocumentMut::save_full_copy(doc, &mut bytes).expect("save");
    let file = pdf_core::pdf::File::parse(&bytes).expect("parse");
    let root = file.resolve(file.trailer().get(b"Root").expect("root")).expect("catalogue");
    let pages = file.resolve(root.as_dict().and_then(|d| d.get(b"Pages")).expect("pages")).expect("tree");
    fn first_page(file: &pdf_core::pdf::File<'_>, node: &Object) -> Option<Object> {
        let dict = node.as_dict()?;
        if dict.get(b"Type").and_then(Object::as_name) == Some(&b"Page"[..]) {
            return Some(node.clone());
        }
        if let Ok(Object::Array(kids)) = file.resolve(dict.get(b"Kids")?) {
            for kid in kids {
                if let Some(page) = file.resolve(&kid).ok().and_then(|kid| first_page(file, &kid)) {
                    return Some(page);
                }
            }
        }
        None
    }
    let page = first_page(&file, &pages).expect("page 1");
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

/// Each operation of a stream as the bytes it occupies, operands included.
fn raw_operations(stream: &[u8]) -> Vec<Vec<u8>> {
    content::parse(stream).expect("parse").iter().map(|o| stream[o.span.clone()].to_vec()).collect()
}

/// Each operation as one line of text with its whitespace squeezed, for reading
/// what sits next to what.
fn operation_texts(stream: &[u8]) -> Vec<String> {
    content::parse(stream)
        .expect("parse")
        .iter()
        .map(|o| String::from_utf8_lossy(&stream[o.span.clone()]).split_whitespace().collect::<Vec<_>>().join(" "))
        .collect()
}

/// The index, among a stream's operations, of the `ordinal`th show-text one.
fn show_operation(stream: &[u8], ordinal: usize) -> usize {
    content::parse(stream)
        .expect("parse")
        .iter()
        .enumerate()
        .filter(|(_, o)| o.shows_text())
        .nth(ordinal)
        .map(|(i, _)| i)
        .unwrap_or_else(|| panic!("the stream has no show-text operator number {ordinal}"))
}

/// Where two operation lists first differ and how many operations on each side
/// the difference spans: `(index, removed, added)`. Everything before `index` and
/// after the span is byte for byte the same.
fn changed_operations(before: &[Vec<u8>], after: &[Vec<u8>]) -> (usize, usize, usize) {
    let mut p = 0;
    while p < before.len() && p < after.len() && before[p] == after[p] {
        p += 1;
    }
    let mut s = 0;
    while s < before.len() - p && s < after.len() - p && before[before.len() - 1 - s] == after[after.len() - 1 - s] {
        s += 1;
    }
    (p, before.len() - p - s, after.len() - p - s)
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

/// A rectangle in points as a pixel box `(left, top, right, bottom)`, widened to
/// whole pixels.
fn pixel_box(b: &Bitmap, scale: f32, r: &Rect) -> (u32, u32, u32, u32) {
    let clamp = |v: f32, max: u32| (v.max(0.0) as u32).min(max);
    (
        clamp((r.left * scale).floor(), b.width),
        clamp((r.top * scale).floor(), b.height),
        clamp((r.right * scale).ceil(), b.width),
        clamp((r.bottom * scale).ceil(), b.height),
    )
}

/// The colour most pixels of a region have: what the page is made of there.
fn dominant(b: &Bitmap, scale: f32, r: &Rect) -> (u8, u8, u8) {
    let (l, t, rr, bb) = pixel_box(b, scale, r);
    let mut seen: HashMap<(u8, u8, u8), usize> = HashMap::new();
    for y in t..bb {
        for x in l..rr {
            *seen.entry(rgb(b, x, y)).or_default() += 1;
        }
    }
    seen.into_iter().max_by_key(|(_, n)| *n).map(|(c, _)| c).expect("an empty region")
}

/// How many pixels of a region differ from the page's own colour by enough to be
/// ink — the old words of a hidden piece would be, a hidden piece is not.
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

/// A fingerprint of a region's pixels.
fn crop_hash(b: &Bitmap, scale: f32, r: &Rect) -> u64 {
    let (l, t, rr, bb) = pixel_box(b, scale, r);
    let mut bytes = Vec::new();
    for y in t..bb {
        for x in l..rr {
            let (pr, pg, pb) = rgb(b, x, y);
            bytes.extend_from_slice(&[pr, pg, pb]);
        }
    }
    fnv(&bytes)
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

/// How many pixels differ between two renders of the same page inside `zone`.
fn differing_within(a: &Bitmap, b: &Bitmap, scale: f32, zone: &Rect) -> usize {
    let (l, t, r, bt) = pixel_box(a, scale, zone);
    let mut n = 0;
    for y in t..bt {
        for x in l..r {
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
/// text objects, 33 of which have an operator of their own and 24 of which
/// continue the piece before them.
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

/// New words for a line's first piece: its own letters the other way round.
/// Spellable in its font by construction, and about as wide, so the retyped piece
/// stays inside the place the old one held.
fn retyped(original: &str) -> String {
    let words = clean(original);
    let reversed: String = words.chars().rev().collect();
    if reversed != words {
        reversed
    } else if words == "e" {
        "t".into()
    } else {
        "e".into()
    }
}

/// What the app sends to apply an edited paragraph: the first piece of each line
/// retyped, every other piece of it hidden by colour, its words left as they were.
fn paragraph_batch(all: &[TextRun], lines: &[Vec<usize>], hide: Color) -> Vec<(usize, String, TextStyle)> {
    let text_of = |o: usize| all.iter().find(|r| r.object == o).expect("object").text.clone();
    let mut edits = Vec::new();
    for line in lines {
        for (j, &object) in line.iter().enumerate() {
            if j == 0 {
                edits.push((object, retyped(&text_of(object)), TextStyle::default()));
            } else {
                edits.push((object, text_of(object), TextStyle { color: Some(hide), ..Default::default() }));
            }
        }
    }
    edits
}

fn timing_line(doc: &mut PdfiumDocument) -> String {
    doc.take_last_batch_timing()
        .iter()
        .map(|(what, took)| format!("{} {:.1} ms", what.trim(), took.as_secs_f64() * 1000.0))
        .collect::<Vec<_>>()
        .join(" | ")
}

// ============================================================ the datasheet ==

/// **The paragraph the user can open and apply, applied.** One batch: the first
/// piece of each of 13 lines retyped, the other 44 pieces hidden by colour — 33
/// pieces that have an operator of their own and 24 that continue another.
///
/// It was refused whole, "that text is drawn in a way this cannot edit", because
/// a continuation piece cannot be found by where it is.
///
/// What this holds it to, from every angle the page can be looked at from:
///
/// - the words: every first piece reads what was typed, every hidden piece reads
///   what it read;
/// - the colour: every hidden piece is the page's own colour and nothing else is
///   touched, nor is any other object on the page changed — not in its words, its
///   colour, its box or its size, all 883 of them;
/// - the picture, at 1.5x: nothing outside the paragraph moved by a pixel; in the
///   paragraph the old words are gone from the places of the hidden pieces and the
///   first pieces show ink;
/// - undo: one step, and every object reads again as it did.
#[test]
fn the_whole_paragraph_applies_in_one_batch_and_undoes_cleanly() {
    let Some(copy) = Copy::of_the_datasheet("whole") else { return };
    let _lock = serial();
    let mut doc = copy.open();

    let before = runs(&doc);
    assert_eq!(before.len(), 883, "page 1 has 883 text objects");
    let lines = lines_of(&before);
    let firsts: Vec<usize> = lines.iter().map(|l| l[0]).collect();
    assert_eq!(
        firsts,
        [985, 989, 991, 995, 1001, 1007, 1014, 1018, 1025, 1030, 1033, 1039, 1041],
        "the paragraph's 13 lines: {lines:?}"
    );
    assert_eq!(lines.iter().map(Vec::len).sum::<usize>(), 57);
    let by_object = |all: &[TextRun], o: usize| all.iter().find(|r| r.object == o).expect("object").clone();

    let image_before = render(&doc, 1.5);
    let rect_of = |o: usize| by_object(&before, o).rect;
    let paragraph_rect = union(&paragraph().iter().map(|o| rect_of(*o)).collect::<Vec<_>>());
    let page = dominant(&image_before, 1.5, &paragraph_rect);
    let hide = Color { r: page.0, g: page.1, b: page.2, a: 255 };
    println!("the page behind the paragraph is {page:?}");

    let edits = paragraph_batch(&before, &lines, hide);
    assert_eq!(edits.len(), 57);
    let stream_before = stream_of(&mut doc);

    // ---- apply, as one undoable step
    let mut history = CommandHistory::new(8);
    let started = Instant::now();
    let applied = history.execute(Command::SetTextRuns { page_index: 0, edits: edits.clone() }, &mut doc);
    let took = started.elapsed();
    let timing = timing_line(&mut doc);
    if let Err(error) = &applied {
        panic!("the paragraph was refused after {:.0} ms: {error}", took.as_secs_f64() * 1000.0);
    }
    println!("57 objects (13 retyped, 44 hidden) applied in {:.0} ms   {timing}", took.as_secs_f64() * 1000.0);

    // ---- the words and colours, all 883 objects
    let after = runs(&doc);
    assert_eq!(after.len(), 883, "an apply must not add or remove an object");
    let first_set: HashSet<usize> = firsts.iter().copied().collect();
    let hidden: Vec<usize> = lines.iter().flat_map(|l| l[1..].iter().copied()).collect();
    let hidden_set: HashSet<usize> = hidden.iter().copied().collect();
    let (mut retyped_n, mut hidden_n, mut untouched_n) = (0, 0, 0);
    for (b, a) in before.iter().zip(&after) {
        assert_eq!(a.object, b.object);
        if first_set.contains(&b.object) {
            assert_eq!(clean(&a.text), retyped(&b.text), "first piece {} should read what was typed", b.object);
            assert_eq!(a.color, b.color, "first piece {} changed colour", b.object);
            retyped_n += 1;
        } else if hidden_set.contains(&b.object) {
            // Compared through `clean`: PDFium reports a hyphen that ends a line
            // as U+0002 only when the next line starts with a letter, and the
            // retyped first piece of the next line starts with a full stop, so
            // the same codes read as "-" now. The codes themselves are not
            // touched — a colour-only edit never reads them (checked below, in
            // the stream, for the whole batch).
            assert_eq!(clean(&a.text), clean(&b.text), "hidden piece {}: its words must be left alone", b.object);
            assert_eq!(a.color, hide, "hidden piece {} should be the page's own colour", b.object);
            assert!((a.size - b.size).abs() < 1e-4, "hidden piece {} changed size", b.object);
            hidden_n += 1;
        } else {
            assert!(a == b, "object {} is not part of the paragraph and changed:\n  {b:?}\n  {a:?}", b.object);
            untouched_n += 1;
        }
    }
    println!("{retyped_n} first pieces read what was typed, {hidden_n} pieces hidden, {untouched_n} other objects identical in words, colour, box, origin and size");
    assert_eq!((retyped_n, hidden_n, untouched_n), (13, 44, 826));

    // ---- the stream: every show-text operator but the 13 first pieces is byte
    // for byte what it was, so no hidden piece had its codes touched, and what
    // was added is exactly a `Tf` per retyped piece and a colour operator on each
    // side of each hidden one
    let stream_after = stream_of(&mut doc);
    let shows = |s: &[u8]| -> Vec<Vec<u8>> {
        content::parse(s).expect("parse").iter().filter(|o| o.shows_text()).map(|o| s[o.span.clone()].to_vec()).collect()
    };
    let (shows_before, shows_after) = (shows(&stream_before), shows(&stream_after));
    assert_eq!(shows_before.len(), shows_after.len(), "the number of show-text operators changed");
    let first_ordinals: HashSet<usize> =
        firsts.iter().map(|o| before.iter().position(|r| r.object == *o).expect("ordinal")).collect();
    for (n, (b, a)) in shows_before.iter().zip(&shows_after).enumerate() {
        if first_ordinals.contains(&n) {
            assert!(a != b, "show-text operator {n} is a first piece and was not retyped");
        } else {
            assert!(a == b, "show-text operator {n} is not a first piece and its bytes changed");
        }
    }
    let (ops_before, ops_after) = (raw_operations(&stream_before).len(), raw_operations(&stream_after).len());
    println!("operations in the stream: {ops_before} -> {ops_after} (13 `Tf` + 44 x 2 colour operators = {})", 13 + 44 * 2);
    assert_eq!(ops_after, ops_before + 13 + 44 * 2, "something other than the `Tf`s and colour operators was added");

    // ---- the picture
    let image_after = render(&doc, 1.5);
    let zone = grown(paragraph_rect, 8.0);
    let outside = differing_pixels(&image_before, &image_after, 1.5, &[zone]);
    println!("pixels that differ outside the paragraph (plus 8 pt): {outside}");
    assert_eq!(outside, 0, "something outside the paragraph changed on the page");

    let mut checked = 0;
    for line in &lines {
        // The ink in the line's own pieces, piece by piece: not in the box round
        // them all, which on two of these lines holds a word the page draws as
        // outlines — a path, not text, and never touched by an edit of text.
        let on_page = |image: &Bitmap| -> usize { line.iter().map(|o| ink(image, 1.5, &rect_of(*o), page)).sum() };
        let (ink_before, ink_after) = (on_page(&image_before), on_page(&image_after));
        let first = rect_of(line[0]);
        let (first_before, first_after) = (ink(&image_before, 1.5, &first, page), ink(&image_after, 1.5, &first, page));
        println!(
            "line of {:>2} pieces from object {}: ink {ink_before} -> {ink_after}   first piece ink {first_before} -> {first_after}",
            line.len(),
            line[0]
        );
        assert!(ink_after * 2 < ink_before, "line {}: the old words are still on the page ({ink_before} -> {ink_after})", line[0]);
        assert!(first_after > 0, "line {}: the retyped first piece shows no ink", line[0]);
        assert_ne!(
            crop_hash(&image_before, 1.5, &first),
            crop_hash(&image_after, 1.5, &first),
            "line {}: the first piece looks as it did",
            line[0]
        );
        // Each hidden piece, where it was: nothing of the old words is left. The
        // piece's first 3 pt are left out, which a retyped neighbour may spill into.
        for &object in &line[1..] {
            let r = rect_of(object);
            let place = Rect { left: r.left + 3.0, top: r.top + 0.4, right: r.right - 0.4, bottom: r.bottom - 0.4 };
            if place.right - place.left < 3.0 {
                continue;
            }
            let (was, now) = (ink(&image_before, 1.5, &place, page), ink(&image_after, 1.5, &place, page));
            assert!(was > 0, "hidden piece {object} had no ink to begin with");
            assert!(now * 10 <= was, "hidden piece {object} still shows its old words where it was ({was} -> {now})");
            checked += 1;
        }
    }
    println!("{checked} hidden pieces were wide enough to check; none shows its old words");
    assert!(checked >= 25, "only {checked} hidden pieces were checked");

    // ---- undo: one step, and every object reads again as it did
    let undone = history.undo(&mut doc).expect("undo");
    assert!(undone.is_some(), "the whole paragraph is one step to undo");
    let restored = runs(&doc);
    assert_eq!(restored.len(), 883);
    // A first piece whose operator carries kerning loses it when it is retyped:
    // the new words bring their own spacing, so putting the old ones back cannot
    // put the kerning back. Its line's pieces then sit a little off — see below.
    for (b, r) in before.iter().zip(&restored) {
        let name = if first_set.contains(&b.object) { "first piece" } else if hidden_set.contains(&b.object) { "hidden piece" } else { "object" };
        assert_eq!(clean(&r.text), clean(&b.text), "{name} {}: its words after undo", b.object);
        assert_eq!(r.color, b.color, "{name} {}: its colour after undo", b.object);
    }
    println!("after undo every one of the 883 objects reads as it did: words and colour");

    let image_undone = render(&doc, 1.5);
    let mut drifted: Vec<Rect> = Vec::new();
    let original_stream = {
        let mut fresh = copy.open();
        stream_of(&mut fresh)
    };
    let original_ops = content::parse(&original_stream).expect("parse");
    let mut kerned_lines: Vec<&Vec<usize>> = Vec::new();
    for line in &lines {
        let ordinal = before.iter().position(|r| r.object == line[0]).expect("ordinal");
        let op = &original_ops[show_operation(&original_stream, ordinal)];
        let kerned = content::pieces(op).iter().any(|p| matches!(p, content::Piece::Kern(_)));
        if kerned {
            kerned_lines.push(line);
            drifted.push(grown(union(&line.iter().map(|o| rect_of(*o)).collect::<Vec<_>>()), 6.0));
        }
    }
    println!("{} of the 13 lines open with a kerned operator (a TJ with spacing numbers), whose spacing a retype cannot bring back", drifted.len());
    let off = differing_pixels(&image_before, &image_undone, 1.5, &drifted);
    println!("pixels that differ after undo, outside those lines: {off}");
    assert_eq!(off, 0, "after undo the page is not pixel for pixel as it was, outside the lines whose first piece was kerned");
    // What it comes to on the kerned lines: how far the pieces after the first sit
    // from where they were, and how many pixels that is.
    let mut within = 0;
    for line in &kerned_lines {
        let drift = line
            .iter()
            .map(|o| (by_object(&restored, *o).rect.left - by_object(&before, *o).rect.left).abs())
            .fold(0.0f32, f32::max);
        let zone = grown(union(&line.iter().map(|o| rect_of(*o)).collect::<Vec<_>>()), 6.0);
        let n = differing_within(&image_before, &image_undone, 1.5, &zone);
        within += n;
        let (was, now) = (by_object(&before, line[0]).rect, by_object(&restored, line[0]).rect);
        let wider = (now.right - now.left) - (was.right - was.left);
        println!(
            "  line from object {}: its pieces start up to {drift:.2} pt from where they did; the first is {wider:+.2} pt wider; {n} pixels differ in its box",
            line[0]
        );
    }
    println!("pixels that differ after undo, in the kerned lines: {within}");
}

/// **One continuation piece, alone.** Object 986, `e COB in `, is drawn by an
/// operator with no position of its own — the piece after `Th` on the first line
/// — and retyping it changes that operator and nothing else.
///
/// Before: "that text is drawn in a way this cannot edit".
#[test]
fn a_continuation_piece_alone_is_retyped_and_only_its_operator_changes() {
    let Some(copy) = Copy::of_the_datasheet("alone") else { return };
    let _lock = serial();
    let mut doc = copy.open();

    let before = runs(&doc);
    let stream_before = stream_of(&mut doc);
    let target = 986;
    let ordinal = before.iter().position(|r| r.object == target).expect("object 986");
    let old = clean(&before[ordinal].text);
    assert_eq!(old, "e COB in", "the piece the page has after `Th`");
    let new = "e COB i".to_string();

    let result = doc.set_text_runs_styled(0, &[(target, new.clone(), TextStyle::default())]);
    let results = result.unwrap_or_else(|e| panic!("retyping the continuation piece was refused: {e}"));
    assert_eq!(clean(&results[0].0), old, "it reports the words it replaced");

    let after = runs(&doc);
    assert_eq!(after.len(), before.len());
    let mut changed = Vec::new();
    for (b, a) in before.iter().zip(&after) {
        if a != b {
            changed.push(b.object);
        }
    }
    println!("objects whose words, colour, box, origin or size differ: {changed:?}");
    assert_eq!(changed, [target], "only the retyped piece may differ");
    assert_eq!(clean(&after[ordinal].text), new);

    let stream_after = stream_of(&mut doc);
    let (index, removed, added) = changed_operations(&raw_operations(&stream_before), &raw_operations(&stream_after));
    println!("the stream changed at operation {index}: {removed} operation replaced by {added}");
    assert_eq!(index, show_operation(&stream_before, ordinal), "the edit landed on another operator");
    assert_eq!((removed, added), (1, 2), "one show-text operator becomes a `Tf` and a `TJ`, and nothing else moves");
}

/// **Two continuation pieces on different lines, in one batch.**
#[test]
fn two_continuation_pieces_in_one_batch_change_two_operators() {
    let Some(copy) = Copy::of_the_datasheet("two") else { return };
    let _lock = serial();
    let mut doc = copy.open();

    let before = runs(&doc);
    let stream_before = stream_of(&mut doc);
    let targets = [986usize, 992];
    let edits: Vec<(usize, String, TextStyle)> = targets
        .iter()
        .map(|t| {
            let mut words: Vec<char> = clean(&before.iter().find(|r| r.object == *t).expect("object").text).chars().collect();
            words.pop();
            (*t, words.into_iter().collect(), TextStyle::default())
        })
        .collect();

    doc.set_text_runs_styled(0, &edits).unwrap_or_else(|e| panic!("two continuation pieces were refused: {e}"));
    let after = runs(&doc);
    let changed: Vec<usize> = before.iter().zip(&after).filter(|(b, a)| a != b).map(|(b, _)| b.object).collect();
    assert_eq!(changed, targets, "only the two retyped pieces may differ");
    for (object, text, _) in &edits {
        assert_eq!(&clean(&after.iter().find(|r| r.object == *object).expect("object").text), text);
    }

    let stream_after = stream_of(&mut doc);
    let (b, a) = (raw_operations(&stream_before), raw_operations(&stream_after));
    assert_eq!(a.len(), b.len() + 2, "each retyped operator gains a `Tf` in front of it and nothing else is added");
    let first = show_operation(&stream_before, before.iter().position(|r| r.object == 986).unwrap());
    let second = show_operation(&stream_before, before.iter().position(|r| r.object == 992).unwrap());
    let (p, removed, added) = changed_operations(&b, &a);
    assert_eq!((p, removed), (first, second - first + 1), "the two edits are at the two operators");
    assert_eq!(added, removed + 2);
}

/// **A lone thin letter is a text object like any other.** Retyped to `l`, object
/// 1033 (`lu`) is 0.4 pt wide — under the 0.5 pt of ink `text_runs` asks before it
/// calls something a run — and a batch that names it must still find its operator
/// and write it, because it has a place in the page's order whatever it draws.
///
/// Before: "that is not a text run", and a whole paragraph apply with such a
/// letter on one of its lines was refused with it.
#[test]
fn a_lone_thin_letter_in_a_batch_is_resolved_like_any_other_piece() {
    let Some(copy) = Copy::of_the_datasheet("thin") else { return };
    let _lock = serial();
    let mut doc = copy.open();

    // Make it: `lu` retyped as `l`, with another line beside it in the batch.
    let first = runs(&doc);
    let text = |all: &[TextRun], o: usize| clean(&all.iter().find(|r| r.object == o).expect("object").text);
    assert_eq!(text(&first, 1033), "lu");
    doc.set_text_runs_styled(
        0,
        &[(1033, "l".into(), TextStyle::default()), (1036, text(&first, 1036), TextStyle::default())],
    )
    .unwrap_or_else(|e| panic!("making the thin letter was refused: {e}"));
    let thin = runs(&doc).into_iter().find(|r| r.object == 1033).expect("it is still a text object");
    let (w, h) = (thin.rect.right - thin.rect.left, thin.rect.bottom - thin.rect.top);
    println!("object 1033 reads {:?}, {w:.2} x {h:.2} pt", thin.text);
    assert!(w <= 0.5 || h <= 0.5, "the letter is not thin: {w} x {h}");
    assert!(
        !doc.text_runs(0).expect("text_runs").iter().any(|r| r.object == 1033),
        "text_runs still lists it, so it is not the no-ink case this is about"
    );

    // Now a batch that retypes it and hides its neighbour by colour.
    let now = runs(&doc);
    let edits = vec![
        (1033, "lu".to_string(), TextStyle::default()),
        (1036, text(&now, 1036), TextStyle { color: Some(Color { r: 255, g: 255, b: 255, a: 255 }), ..Default::default() }),
    ];
    doc.set_text_runs_styled(0, &edits).unwrap_or_else(|e| panic!("a batch naming the thin letter was refused: {e}"));
    let after = runs(&doc);
    assert_eq!(text(&after, 1033), "lu", "the thin letter was retyped");
    assert_eq!(after.iter().find(|r| r.object == 1036).expect("object").color, Color { r: 255, g: 255, b: 255, a: 255 });
    let changed: Vec<usize> = now.iter().zip(&after).filter(|(b, a)| a != b).map(|(b, _)| b.object).collect();
    println!("objects that differ after it: {changed:?}");
    assert!(changed.contains(&1033) && changed.contains(&1036));
}

/// **And on its own**, through the single-edit path: a thin letter retyped alone,
/// and hidden alone.
#[test]
fn a_lone_thin_letter_alone_is_resolved_too() {
    let Some(copy) = Copy::of_the_datasheet("thin-alone") else { return };
    let _lock = serial();
    let mut doc = copy.open();

    let text = |all: &[TextRun], o: usize| clean(&all.iter().find(|r| r.object == o).expect("object").text);
    let first = runs(&doc);
    assert_eq!(text(&first, 1033), "lu");
    doc.set_text_runs_styled(
        0,
        &[(1033, "l".into(), TextStyle::default()), (1036, text(&first, 1036), TextStyle::default())],
    )
    .unwrap_or_else(|e| panic!("making the thin letter was refused: {e}"));
    assert!(!doc.text_runs(0).expect("text_runs").iter().any(|r| r.object == 1033), "it is not thin");

    // Hidden alone: the colour-only path of its own.
    let paper = Color { r: 200, g: 30, b: 30, a: 255 };
    doc.set_text_run_styled(0, 1033, "l", &TextStyle { color: Some(paper), ..Default::default() })
        .unwrap_or_else(|e| panic!("hiding the thin letter alone was refused: {e}"));
    assert_eq!(runs(&doc).iter().find(|r| r.object == 1033).expect("object").color, paper);

    // Retyped alone: the words path.
    doc.set_text_run_styled(0, 1033, "lu", &TextStyle::default())
        .unwrap_or_else(|e| panic!("retyping the thin letter alone was refused: {e}"));
    assert_eq!(text(&runs(&doc), 1033), "lu");
}

/// **An object with no words and no ink is a text object too** — a lone space
/// drawn as a piece of a justified line — and hiding it by colour in a batch must
/// not stop the batch.
#[test]
fn a_blank_text_object_in_a_batch_is_resolved_like_any_other_piece() {
    let Some(copy) = Copy::of_the_datasheet("blank") else { return };
    let _lock = serial();
    let mut doc = copy.open();

    let before = runs(&doc);
    let no_ink = |r: &TextRun| (r.rect.right - r.rect.left).abs() <= 0.5 || (r.rect.bottom - r.rect.top).abs() <= 0.5;
    let listed: HashSet<usize> = doc.text_runs(0).expect("text_runs").iter().map(|r| r.object).collect();
    let blank = before
        .iter()
        .find(|r| no_ink(r) && !listed.contains(&r.object))
        .unwrap_or_else(|| panic!("no text object on page 1 is missing from text_runs"))
        .clone();
    println!(
        "{} of {} text objects have no ink area; using object {} ({:?}, {:.2} x {:.2} pt)",
        before.iter().filter(|r| no_ink(r)).count(),
        before.len(),
        blank.object,
        blank.text,
        blank.rect.right - blank.rect.left,
        blank.rect.bottom - blank.rect.top
    );
    let ordinary = before.iter().find(|r| r.object == 989).expect("object 989").clone();
    let hide = Color { r: 10, g: 200, b: 10, a: 255 };

    let mut words: Vec<char> = clean(&ordinary.text).chars().collect();
    words.pop();
    let edits = vec![
        (ordinary.object, words.into_iter().collect::<String>(), TextStyle::default()),
        (blank.object, blank.text.clone(), TextStyle { color: Some(hide), ..Default::default() }),
    ];
    doc.set_text_runs_styled(0, &edits).unwrap_or_else(|e| panic!("a batch naming a blank object was refused: {e}"));
    let after = runs(&doc);
    assert_eq!(after.iter().find(|r| r.object == blank.object).expect("object").color, hide);
    let changed: Vec<usize> = before.iter().zip(&after).filter(|(b, a)| a != b).map(|(b, _)| b.object).collect();
    println!("objects that differ after it: {changed:?}");
    assert!(changed.contains(&ordinary.object) && changed.contains(&blank.object));
}

// ============================================================= synthetic pages ==

/// A one-page PDF drawing `content`, with Helvetica as `/F1`; `fonts` is added to
/// the page's font dictionary, `resources` to its resource dictionary, and
/// `extra` become objects 6, 7, ...
fn page_with(content: &[u8], fonts: &str, resources: &str, extra: &[Vec<u8>]) -> Vec<u8> {
    let mut objects: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R \
             /Resources << /Font << /F1 5 0 R {fonts} >> {resources} >> >>"
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
    PdfiumDocument::open_bytes(page_with(content, "", "", &[]), None).expect("open")
}

fn words(doc: &PdfiumDocument) -> Vec<String> {
    runs(doc).iter().map(|r| clean(&r.text)).collect()
}

fn edit(object: usize, text: &str) -> (usize, String, TextStyle) {
    (object, text.to_string(), TextStyle::default())
}

fn hide(object: usize, text: &str, colour: Color) -> (usize, String, TextStyle) {
    (object, text.to_string(), TextStyle { color: Some(colour), ..Default::default() })
}

/// **A piece close behind the one before it is found by count, not by where it
/// is.** Each of these pieces starts within four points of the line's start, which
/// is where the stream walk reports all of them — so the nearest operator to the
/// second piece is the *first*, and an edit that goes by position retypes the
/// wrong word without a word of complaint.
#[test]
fn a_piece_close_behind_the_one_before_it_is_found_by_count_not_by_position() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    let mut doc = open(b"BT /F1 5 Tf 72 700 Td (ii) Tj (jj) Tj (ll) Tj (ii) Tj ET");
    assert_eq!(words(&doc), ["ii", "jj", "ll", "ii"]);
    let all = runs(&doc);
    let behind = all[1].origin.x - all[0].origin.x;
    println!("the second piece starts {behind:.2} pt after the first");
    assert!(behind < 4.0, "the second piece is further than the 4 pt a lookup by position allows");

    doc.set_text_run_styled(0, 1, "xx", &TextStyle::default())
        .unwrap_or_else(|e| panic!("retyping the second piece was refused: {e}"));
    assert_eq!(words(&doc), ["ii", "xx", "ll", "ii"], "the edit landed on another piece");

    // And as one batch, three pieces at once.
    doc.set_text_runs_styled(0, &[edit(1, "yy"), edit(2, "zz"), edit(3, "ww")])
        .unwrap_or_else(|e| panic!("a batch of three pieces was refused: {e}"));
    assert_eq!(words(&doc), ["ii", "yy", "zz", "ww"]);
}

/// **A page whose operators and objects do not match one to one is not counted.**
/// The empty string in `() Tj` makes no text object, so there is one operator
/// more than there are objects and the *n*th object is no longer the *n*th
/// operator. Everything here is on one line, so no position test could tell the
/// pieces apart either: the batch is refused, and nothing is written.
#[test]
fn a_page_whose_objects_do_not_match_its_operators_is_not_counted() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    let mut doc = open(b"BT /F1 12 Tf 72 700 Td () Tj (Same) Tj (Same) Tj (Same) Tj ET");
    assert_eq!(words(&doc), ["Same", "Same", "Same"], "the empty string makes no text object");
    let stream_before = stream_of(&mut doc);
    let runs_before = runs(&doc);

    let result = doc.set_text_runs_styled(0, &[edit(1, "Aaaa"), edit(2, "Bbbb")]);
    let error = result.err().unwrap_or_else(|| {
        panic!("the batch was accepted, and the words are now {:?}", words(&doc));
    });
    println!("refused: {error}");
    assert_eq!(words(&doc), ["Same", "Same", "Same"], "a refused batch changed the page");
    assert!(runs(&doc) == runs_before);
    assert!(stream_of(&mut doc) == stream_before, "a refused batch changed the stream");
}

/// **Two edits that land on one operator are refused**, not applied by half.
/// Splicing two edits into one operator keeps the first and drops the second
/// without a word, so a batch that said both were done would have done one.
#[test]
fn two_edits_that_resolve_to_one_operator_are_refused() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    let mut doc = open(b"BT /F1 5 Tf 72 700 Td (ii) Tj (ii) Tj () Tj ET");
    assert_eq!(words(&doc), ["ii", "ii"]);
    let stream_before = stream_of(&mut doc);

    let result = doc.set_text_runs_styled(0, &[edit(0, "xx"), edit(1, "yy")]);
    let error = result.err().unwrap_or_else(|| {
        panic!("both edits were reported done, and the words are now {:?}", words(&doc));
    });
    println!("refused: {error}");
    assert_eq!(words(&doc), ["ii", "ii"]);
    assert!(stream_of(&mut doc) == stream_before);
}

/// **A kerned start is not confirmed by the stream.** `[500 (Hello)] TJ` moves
/// the text half an em to the left before drawing it; PDFium places the object
/// there and the walk does not know, so the operator's origin is behind where the
/// object starts. The count is right here — one object, one operator — but the
/// stream does not confirm it, and an edit that went ahead on the count alone is
/// an edit the page has not agreed to.
#[test]
fn a_run_the_stream_does_not_confirm_is_refused_even_when_the_count_is_right() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    // One object and one operator: the count is as right as it can be, and no
    // other line is there for any fallback to count against.
    let mut doc = open(b"BT /F1 12 Tf 72 700 Td [500 (Hello)] TJ ET");
    let all = runs(&doc);
    println!("origin {:?}, where the stream puts the pen: x = 72", all[0].origin);
    assert!(all[0].origin.x < 71.0, "PDFium did not move the object by the kerning");
    let before = stream_of(&mut doc);

    let single = doc.set_text_run_styled(0, 0, "Jello", &TextStyle::default());
    let error = single.err().unwrap_or_else(|| panic!("the edit was accepted: {:?}", words(&doc)));
    println!("refused: {error}");
    assert_eq!(words(&doc), ["Hello"], "a refused edit changed the page");
    assert!(stream_of(&mut doc) == before, "a refused edit changed the stream");
}

/// **A form and an inline image between the text do not disturb the count** —
/// neither is a text object on the page, and neither is a show-text operator in
/// its stream — so the pieces after them are still found.
#[test]
fn forms_and_inline_images_between_text_do_not_disturb_the_count() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    let mut content = b"BT /F1 12 Tf 72 700 Td (Alpha one) Tj (alpha two) Tj ET\n/Fm1 Do\n\
                        q 20 0 0 20 300 300 cm BI /W 1 /H 1 /CS /G /BPC 8 ID "
        .to_vec();
    content.push(0x80);
    content.extend_from_slice(b" EI Q\nBT /F1 12 Tf 72 660 Td (Bravo one) Tj (bravo two) Tj ET");
    let form = {
        let body = b"BT /F1 10 Tf 0 0 Td (inside) Tj ET";
        [
            format!(
                "<< /Type /XObject /Subtype /Form /BBox [0 0 100 100] \
                 /Resources << /Font << /F1 5 0 R >> >> /Length {} >>\nstream\n",
                body.len()
            )
            .as_bytes(),
            body.as_slice(),
            b"\nendstream",
        ]
        .concat()
    };
    let mut doc =
        PdfiumDocument::open_bytes(page_with(&content, "", "/XObject << /Fm1 6 0 R >>", &[form]), None).expect("open");
    let all = runs(&doc);
    assert_eq!(all.len(), 4, "the form's text and the image are not text objects of the page");
    let objects: Vec<usize> = all.iter().map(|r| r.object).collect();
    println!("text objects at page-object indices {objects:?}");
    assert_eq!(objects, [0, 1, 4, 5]);

    doc.set_text_runs_styled(0, &[edit(1, "alpha tw"), edit(5, "bravo tw")])
        .unwrap_or_else(|e| panic!("the pieces after a form and an image were refused: {e}"));
    assert_eq!(words(&doc), ["Alpha one", "alpha tw", "Bravo one", "bravo tw"]);
}

/// **An object that is not text, or not there, refuses cleanly and writes
/// nothing** — even as the last of many good edits.
#[test]
fn an_object_that_is_not_text_refuses_the_whole_batch() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    let mut doc = open(b"BT /F1 12 Tf 72 700 Td (Alpha one) Tj (alpha two) Tj ET\n0 0 50 50 re f");
    assert_eq!(words(&doc), ["Alpha one", "alpha two"]);
    let before = stream_of(&mut doc);
    for bad in [2usize, 99] {
        let result = doc.set_text_runs_styled(0, &[edit(0, "Alpha on"), edit(1, "alpha tw"), edit(bad, "x")]);
        let error = result.err().unwrap_or_else(|| panic!("object {bad} was accepted"));
        println!("object {bad}: refused: {error}");
        assert_eq!(words(&doc), ["Alpha one", "alpha two"]);
        assert!(stream_of(&mut doc) == before, "object {bad}: a refused batch changed the stream");
    }
}

/// **Where the count is not available, the position match still works** for the
/// pieces that have an operator of their own, and edits the right ones. An empty
/// string draws no text object, so there is an operator more than there are
/// objects; each line here stands alone, so each object is found where it is.
#[test]
fn where_the_count_is_unavailable_the_position_match_finds_the_pieces_with_a_place_of_their_own() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    let mut doc = open(
        b"BT /F1 12 Tf 72 740 Td () Tj ET\nBT /F1 12 Tf 72 700 Td (Alpha) Tj ET\nBT /F1 12 Tf 72 660 Td (Bravo) Tj ET",
    );
    assert_eq!(words(&doc), ["Alpha", "Bravo"], "the empty string makes no text object");
    doc.set_text_runs_styled(0, &[edit(0, "Alphb"), edit(1, "Bravx")])
        .unwrap_or_else(|e| panic!("the position match was not used: {e}"));
    assert_eq!(words(&doc), ["Alphb", "Bravx"], "an edit landed on the wrong operator");
}

/// **Rotated text, a Type 3 font and a piece of zero size between the pieces do
/// not break the count.** The pieces after them are still found, and the right
/// ones are edited; where the page's operators and objects stop matching one to
/// one the batch is refused instead, never applied to the wrong operator.
#[test]
fn rotated_text_and_odd_fonts_between_the_pieces_do_not_break_the_count() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    let type3 = b"<< /Type /Font /Subtype /Type3 /FontBBox [0 0 1000 1000] /FontMatrix [0.001 0 0 0.001 0 0] \
                  /CharProcs << /sq 7 0 R >> /Encoding << /Type /Encoding /Differences [97 /sq] >> \
                  /FirstChar 97 /LastChar 97 /Widths [1000] >>"
        .to_vec();
    let glyph = {
        let body = b"1000 0 0 0 800 800 d1 0 0 800 800 re f";
        [format!("<< /Length {} >>\nstream\n", body.len()).as_bytes(), body.as_slice(), b"\nendstream"].concat()
    };
    let content = b"BT /F1 12 Tf 72 700 Td (Alpha one) Tj (alpha two) Tj ET\n\
                    BT /F1 12 Tf 0 1 -1 0 300 400 Tm (Rotated one) Tj (rotated two) Tj ET\n\
                    BT /F3 10 Tf 72 560 Td (aa) Tj ET\n\
                    BT /F1 12 Tf 72 520 Td (Omega one) Tj (omega two) Tj ET";
    let mut doc = PdfiumDocument::open_bytes(page_with(content, "/F3 6 0 R", "", &[type3, glyph]), None).expect("open");
    let all = runs(&doc);
    println!("{} text objects; the rotated ones run along {:?}", all.len(), all[2].origin);
    assert_eq!(all.len(), 7, "seven text objects");
    let before = stream_of(&mut doc);

    doc.set_text_runs_styled(0, &[edit(1, "alpha tw"), edit(3, "rotated tw"), edit(6, "omega tw")])
        .unwrap_or_else(|e| panic!("the pieces after rotated text and a Type 3 font were refused: {e}"));
    let now = words(&doc);
    assert_eq!(now[0], "Alpha one");
    assert_eq!(now[1], "alpha tw");
    assert_eq!(now[2], "Rotated one");
    assert_eq!(now[3], "rotated tw");
    assert_eq!(now[5], "Omega one");
    assert_eq!(now[6], "omega tw");
    let (index, removed, added) = changed_operations(&raw_operations(&before), &raw_operations(&stream_of(&mut doc)));
    println!("the stream changed from operation {index}: {removed} operations replaced by {added}");

    // A piece of zero size is a text object like the rest — or it is not one at
    // all, and then the count is off and the batch must be refused. Either way
    // the edit goes where it was meant to or nowhere.
    let mut zero = open(
        b"BT /F1 12 Tf 72 700 Td (Alpha one) Tj (alpha two) Tj ET\n\
          BT /F1 0 Tf 72 600 Td (hidden) Tj ET\n\
          BT /F1 12 Tf 72 520 Td (Omega one) Tj (omega two) Tj ET",
    );
    let objects = runs(&zero).len();
    println!("with a piece of zero size: {objects} text objects for 5 operators");
    let outcome = zero.set_text_runs_styled(0, &[edit(1, "alpha tw"), edit(objects - 1, "omega tw")]);
    match outcome {
        Ok(_) => {
            let now = words(&zero);
            assert_eq!(now[1], "alpha tw");
            assert_eq!(now[0], "Alpha one");
            assert_eq!(now[objects - 1], "omega tw");
            assert_eq!(now[objects - 2], "Omega one");
            println!("applied: {now:?}");
        }
        Err(e) => {
            assert_eq!(words(&zero).len(), objects);
            assert_eq!(words(&zero)[1], "alpha two", "a refused batch changed the page");
            println!("refused, as the count is off: {e}");
        }
    }
}

// ---------------------------------------------------------- the colour state --

/// What the stream says right after the `ordinal`th show-text operator, as text.
fn after_the_operator(doc: &mut PdfiumDocument, ordinal: usize, count: usize) -> Vec<String> {
    let stream = stream_of(doc);
    let at = show_operation(&stream, ordinal);
    operation_texts(&stream)[at + 1..at + 1 + count].to_vec()
}

/// **Hiding a piece puts the colour back as the stream had it — not as an
/// approximation of it.** Whatever set the fill colour before the piece, that is
/// what is written after it, so every piece after it that draws in the inherited
/// colour draws in exactly the colour it did: not a DeviceRGB stand-in for a
/// CMYK black, not the colour of a block that was closed before the piece began.
#[test]
fn hiding_a_piece_puts_the_colour_state_back_exactly() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    let paper = Color { r: 200, g: 30, b: 30, a: 255 };
    let body = "BT /F1 12 Tf 72 700 Td (first) Tj (second) Tj (third) Tj ET";
    let cases: [(&str, String, Vec<&str>); 5] = [
        ("a CMYK fill", format!("0 0 0 1 k {body}"), vec!["0 0 0 1 k"]),
        ("a colour space and its value", format!("/DeviceRGB cs 0.2 0.4 0.6 sc {body}"), vec!["/DeviceRGB cs", "0.2 0.4 0.6 sc"]),
        ("no fill colour set at all", body.to_string(), vec!["0 g"]),
        ("a colour set in a block that was closed", format!("0.5 g q 1 0 0 rg Q {body}"), vec!["0.5 g"]),
        ("a colour set inside the block it is drawn in", format!("0.9 g q 0.2 g {body} Q"), vec!["0.2 g"]),
    ];
    for (name, content, expected) in cases {
        // As a single edit, which takes the colour-only path of its own...
        let mut doc = open(content.as_bytes());
        let before = runs(&doc);
        assert_eq!(words(&doc), ["first", "second", "third"], "{name}");
        doc.set_text_run_styled(0, 1, "second", &TextStyle { color: Some(paper), ..Default::default() })
            .unwrap_or_else(|e| panic!("{name}: hiding the second piece was refused: {e}"));
        let after = after_the_operator(&mut doc, 1, expected.len());
        println!("{name}: after the hidden piece the stream says {after:?}");
        assert_eq!(after, expected, "{name}: the colour is not put back as the stream had it");
        let now = runs(&doc);
        assert_eq!(now[1].color, paper, "{name}: the piece is not the colour it was asked to be");
        assert_eq!(now[2].color, before[2].color, "{name}: the piece after it changed colour");
        assert_eq!(now[0].color, before[0].color, "{name}: the piece before it changed colour");

        // ...and inside a batch, with the first piece retyped beside it.
        let mut doc = open(content.as_bytes());
        doc.set_text_runs_styled(0, &[edit(0, "firs"), hide(1, "second", paper)])
            .unwrap_or_else(|e| panic!("{name}: the batch was refused: {e}"));
        let after = after_the_operator(&mut doc, 1, expected.len());
        // The retyped first piece adds a `Tf` in front of its own operator, which
        // does not move the second piece's operator among the show-text ones.
        assert_eq!(after, expected, "{name}, in a batch: the colour is not put back as the stream had it");
        assert_eq!(runs(&doc)[2].color, before[2].color, "{name}, in a batch: the piece after it changed colour");
    }
}

// ================================================================= probing ==

/// What area redaction does with a piece that continues another. Not asserted: a
/// probe, kept so that the answer can be looked at again. Redaction finds the
/// operators of a run by position alone — it has no count to go by — and this
/// shows what that comes to.
///
/// ```text
/// PAGIFY_PDFIUM_LIB=<pdfium> cargo test --release --test continuation_edit probe -- --ignored --nocapture
/// ```
#[test]
#[ignore = "a probe, not a check: what redaction does with a piece that continues another"]
fn probe_redaction_of_a_continuation_piece() {
    use pdf_core::document::Redaction;
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    // The narrow pieces: a lookup by position takes the second for the first.
    let mut doc = open(b"BT /F1 5 Tf 72 700 Td (ii) Tj (jj) Tj (ll) Tj (ii) Tj ET");
    let target = runs(&doc)[1].rect;
    let area = Rect { left: target.left + 0.3, top: target.top + 0.3, right: target.right - 0.3, bottom: target.bottom - 0.3 };
    let outcome = doc.redact(&Redaction::new(0, area), None);
    let mut bytes = Vec::new();
    DocumentMut::save_full_copy(&mut doc, &mut bytes).expect("save");
    let reopened = PdfiumDocument::open_bytes(bytes, None).expect("reopen");
    println!("narrow pieces, redacting the second ('jj'): {:?} -> words now {:?}", outcome.as_ref().map(|r| format!("{r:?}").len()), words(&reopened));
    if let Err(e) = &outcome {
        println!("  refused: {e}");
    }

    // The datasheet: `e COB in ` (object 986) continues `Th` (985).
    let Some(copy) = Copy::of_the_datasheet("probe") else { return };

    // The control: nothing redacted, only saved and reopened, as the redactions
    // below are. What differs here differs for a reason that is not redaction.
    {
        let mut doc = copy.open();
        let before = runs(&doc);
        let mut bytes = Vec::new();
        DocumentMut::save_full_copy(&mut doc, &mut bytes).expect("save");
        let after = runs(&PdfiumDocument::open_bytes(bytes, None).expect("reopen"));
        let mut left: HashMap<String, usize> = HashMap::new();
        for r in &before {
            *left.entry(clean(&r.text)).or_default() += 1;
        }
        let mut added: Vec<String> = Vec::new();
        for r in &after {
            match left.get_mut(&clean(&r.text)) {
                Some(n) if *n > 0 => *n -= 1,
                _ => added.push(clean(&r.text)),
            }
        }
        let gone = left.into_iter().filter(|(w, n)| *n > 0 && !w.is_empty()).count();
        println!("control, saved and reopened with nothing redacted: {} -> {} text objects, {gone} words gone, {} new", before.len(), after.len(), added.len());
    }
    for object in [986usize, 1017, 992] {
        let mut doc = copy.open();
        let before = runs(&doc);
        let target = before.iter().find(|r| r.object == object).expect("object").clone();
        let area = Rect {
            left: target.rect.left + 0.3,
            top: target.rect.top + 0.3,
            right: target.rect.right - 0.3,
            bottom: target.rect.bottom - 0.3,
        };
        let outcome = doc.redact(&Redaction::new(0, area), None);
        let mut bytes = Vec::new();
        DocumentMut::save_full_copy(&mut doc, &mut bytes).expect("save");
        if object == 986 {
            // Is a changed word a fact of the file, or of how this crate reads
            // words? Read the redacted copy with the one call that reads them
            // the old way, straight through pdfium-render, and compare.
            use pdf_core::document::pdfium_doc::pdfium;
            use pdfium_render::prelude::*;
            let path = std::env::temp_dir().join(format!("pagify-continuation-probe-{}.pdf", std::process::id()));
            std::fs::write(&path, &bytes).expect("write the redacted copy");
            let old_way: Vec<String> = {
                let document = pdfium().expect("pdfium").load_pdf_from_file(&path, None).expect("load");
                let page = document.pages().get(0).expect("page");
                page.objects().iter().filter_map(|o| o.as_text_object().map(|t| clean(&t.text()))).collect()
            };
            let _ = std::fs::remove_file(&path);
            let new_way: Vec<String> =
                runs(&PdfiumDocument::open_bytes(bytes.clone(), None).expect("reopen")).iter().map(|r| clean(&r.text)).collect();
            println!(
                "the redacted copy read the old way and this crate's way: {} and {} text objects, {} of them differ",
                old_way.len(),
                new_way.len(),
                old_way.iter().zip(&new_way).filter(|(a, b)| a != b).count()
            );
        }
        let reopened = PdfiumDocument::open_bytes(bytes, None).expect("reopen");
        let after = runs(&reopened);
        // The words the page had that it no longer has, by text alone (a save
        // and a reopen move a box by a rounding, so boxes are not compared), and
        // the words it has that it had not.
        let mut left: HashMap<String, usize> = HashMap::new();
        for r in &before {
            *left.entry(clean(&r.text)).or_default() += 1;
        }
        let mut added: Vec<String> = Vec::new();
        for r in &after {
            match left.get_mut(&clean(&r.text)) {
                Some(n) if *n > 0 => *n -= 1,
                _ => added.push(clean(&r.text)),
            }
        }
        let mut gone: Vec<(String, usize)> = left.into_iter().filter(|(w, n)| *n > 0 && !w.is_empty()).collect();
        gone.sort();
        match outcome {
            Ok(_) => println!(
                "datasheet object {object} ({:?}) redacted: {} text objects before, {} after; words gone: {gone:?}; words new: {added:?}",
                clean(&target.text),
                before.len(),
                after.len()
            ),
            Err(e) => println!("datasheet object {object} ({:?}) redaction refused: {e}", clean(&target.text)),
        }
    }
}

/// A PNG of `width` x `height` RGB pixels, uncompressed, for looking at.
fn write_png(path: &std::path::Path, width: u32, height: u32, rgb: &[u8]) {
    fn crc(bytes: &[u8]) -> u32 {
        let mut c = 0xffff_ffffu32;
        for &b in bytes {
            c ^= u32::from(b);
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        !c
    }
    fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        out.extend_from_slice(&body);
        out.extend_from_slice(&crc(&body).to_be_bytes());
    }
    let mut raw = Vec::with_capacity((width as usize * 3 + 1) * height as usize);
    for row in rgb.chunks(width as usize * 3) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut z = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(65535).collect();
    for (i, block) in blocks.iter().enumerate() {
        z.push(u8::from(i + 1 == blocks.len()));
        z.extend_from_slice(&(block.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
        z.extend_from_slice(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in &raw {
        a = (a + u32::from(byte)) % 65521;
        b = (b + a) % 65521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    let mut header = Vec::new();
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(&mut png, b"IHDR", &header);
    chunk(&mut png, b"IDAT", &z);
    chunk(&mut png, b"IEND", &[]);
    std::fs::write(path, png).expect("write the png");
}

/// What a retyped first piece does under the pieces hidden after it. In the app
/// the typed line goes onto the first piece, which is short, and the others are
/// hidden by painting them in the page's colour — *after* it in the stream.
/// This retypes a line as one piece and compares three pictures of it: as it was;
/// as the app hides it (every other piece in the page colour); and the same with
/// the other pieces' codes taken out altogether, which is what the hiding is
/// trying to look like. Written to a PNG, three crops one above the other. Not
/// asserted.
fn overpaint_case(copy: &Copy, name: &str, objects: &[usize], typed: &str) {
    let scale = 4.0;
    let white = Color { r: 255, g: 255, b: 255, a: 255 };
    let picture = |edit: &dyn Fn(&[TextRun]) -> Vec<(usize, String, TextStyle)>| -> (Bitmap, Vec<TextRun>) {
        let mut doc = copy.open();
        let all = runs(&doc);
        let list = edit(&all);
        if !list.is_empty() {
            doc.set_text_runs_styled(0, &list).unwrap_or_else(|e| panic!("{name}: refused: {e}"));
        }
        (render(&doc, scale), runs(&doc))
    };
    let all = {
        let doc = copy.open();
        runs(&doc)
    };
    let rect = union(&objects.iter().map(|o| all.iter().find(|r| r.object == *o).expect("object").rect).collect::<Vec<_>>());
    let others: Vec<usize> = objects[1..].to_vec();
    let (original, _) = picture(&|_| Vec::new());
    let (hidden, after) = picture(&|all| {
        let mut list = vec![(objects[0], typed.to_string(), TextStyle::default())];
        for o in &others {
            let text = all.iter().find(|r| r.object == *o).expect("object").text.clone();
            list.push((*o, text, TextStyle { color: Some(white), ..Default::default() }));
        }
        list
    });
    let (removed, _) = picture(&|_| {
        let mut list = vec![(objects[0], typed.to_string(), TextStyle::default())];
        for o in &others {
            list.push((*o, " ".to_string(), TextStyle::default()));
        }
        list
    });
    let page = (255u8, 255u8, 255u8);
    let zone = grown(rect, 4.0);
    let (ink_original, ink_hidden, ink_removed) =
        (ink(&original, scale, &zone, page), ink(&hidden, scale, &zone, page), ink(&removed, scale, &zone, page));
    let painted_over = differing_within(&hidden, &removed, scale, &zone);
    println!("{name}: typed {typed:?}; the first piece now reads {:?}", after.iter().find(|r| r.object == objects[0]).map(|r| clean(&r.text)));
    println!("  ink in the line's box: as it was {ink_original}; hidden as the app hides it {ink_hidden}; the other pieces' codes removed {ink_removed}");
    println!(
        "  pixels that differ between 'hidden' and 'codes removed': {painted_over} ({:.0}% of the ink the line should show)",
        100.0 * painted_over as f32 / ink_removed.max(1) as f32
    );
    let (l, t, r, b) = pixel_box(&original, scale, &zone);
    let (w, h) = (r - l, b - t);
    let mut pixels = Vec::with_capacity((w * h * 3 * 3) as usize);
    for image in [&original, &hidden, &removed] {
        for y in t..b {
            for x in l..r {
                let (pr, pg, pb) = rgb(image, x, y);
                pixels.extend_from_slice(&[pr, pg, pb]);
            }
        }
    }
    let path = std::env::temp_dir().join(format!("pagify-continuation-overpaint-{name}.png"));
    write_png(&path, w, h * 3, &pixels);
    println!("  picture (as it was / as the app hides it / as it should look): {}", path.display());
}

#[test]
#[ignore = "a probe, not a check: the picture of a longer first piece over its hidden neighbours"]
fn probe_a_longer_first_piece_under_its_hidden_neighbours() {
    let Some(copy) = Copy::of_the_datasheet("overpaint") else { return };
    let _lock = serial();
    // Line 2: `plie` then a piece that continues it with nothing repositioning.
    overpaint_case(&copy, "line2", &[989, 990], "plied by the COB manufacturers");
    // Line 1: `Th`, a continuation, and then two pieces each placed by a `Td` of
    // its own from the start of the line.
    overpaint_case(&copy, "line1", &[985, 986, 987, 988], "The COB in products sup");
}

// ================================================================ measuring ==

/// What applying the paragraph costs, and where it goes: the whole 57 objects
/// (13 retyped, 44 hidden) and the 33 that have an operator of their own, each as
/// the app sends it. Not asserted. Run on demand:
///
/// ```text
/// PAGIFY_PDFIUM_LIB=<pdfium> cargo test --release --test continuation_edit -- --ignored --nocapture
/// ```
#[test]
#[ignore = "a measurement, not a check; run with --ignored --nocapture"]
fn what_the_paragraph_costs() {
    let Some(copy) = Copy::of_the_datasheet("cost") else { return };
    let _lock = serial();

    let own: Vec<usize> = vec![
        985, 987, 988, 989, 991, 993, 995, 997, 999, 1001, 1003, 1005, 1007, 1008, 1010, 1012, 1014, 1016, 1018, 1019, 1021,
        1023, 1025, 1027, 1028, 1030, 1031, 1033, 1036, 1037, 1039, 1041, 1042,
    ];
    for round in 1..=3 {
        // All 57, as the app sends them.
        let mut doc = copy.open();
        let before = runs(&doc);
        let lines = lines_of(&before);
        let edits = paragraph_batch(&before, &lines, Color { r: 255, g: 255, b: 255, a: 255 });
        let started = Instant::now();
        let outcome = doc.set_text_runs_styled(0, &edits);
        let took = started.elapsed().as_secs_f64() * 1000.0;
        match outcome {
            Ok(_) => println!("round {round}: all 57 objects applied in {took:7.1} ms   {}", timing_line(&mut doc)),
            Err(e) => println!("round {round}: all 57 objects REFUSED after {took:7.1} ms: {e}"),
        }

        // The 33 with an operator of their own, each one retyped one letter shorter.
        let mut doc = copy.open();
        let before = runs(&doc);
        let edits: Vec<(usize, String, TextStyle)> = own
            .iter()
            .map(|o| {
                let mut chars: Vec<char> = clean(&before.iter().find(|r| r.object == *o).expect("object").text).chars().collect();
                if chars.len() > 2 {
                    chars.pop();
                }
                edit(*o, &chars.into_iter().collect::<String>())
            })
            .collect();
        let started = Instant::now();
        let outcome = doc.set_text_runs_styled(0, &edits);
        let took = started.elapsed().as_secs_f64() * 1000.0;
        match outcome {
            Ok(_) => println!("round {round}: the 33 own-operator objects applied in {took:7.1} ms   {}", timing_line(&mut doc)),
            Err(e) => println!("round {round}: the 33 own-operator objects REFUSED after {took:7.1} ms: {e}"),
        }

        // One continuation piece alone, through the single-edit path.
        let mut doc = copy.open();
        let started = Instant::now();
        let outcome = doc.set_text_run_styled(0, 986, "e COB i", &TextStyle::default());
        let took = started.elapsed().as_secs_f64() * 1000.0;
        match outcome {
            Ok(_) => println!("round {round}: one continuation piece alone applied in {took:7.1} ms"),
            Err(e) => println!("round {round}: one continuation piece alone REFUSED after {took:7.1} ms: {e}"),
        }
    }
}
