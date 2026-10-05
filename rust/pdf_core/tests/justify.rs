//! A retyped line spread to a width: `TextLineEdit::Retype { justify_to }`.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test --release --test justify -- --nocapture
//! ```
//!
//! `--nocapture` is the point of the flag, not a convenience: a test here that
//! cannot find PDFium or the real datasheet prints why and returns, and still
//! counts as passed. Look for the numbers each test prints.
//!
//! **What it is for.** The datasheet's paragraph is justified: the producer
//! spaced the words of every line but the last until the line reached the right
//! margin. Retyping a line writes plain words, which come out a few points short
//! of the margin — and a line that ends short reads, to the detector, as the end
//! of its paragraph, so the next click opens only the lines above it. `justify_to`
//! is the width the line is to span; the engine opens or closes the line's word
//! gaps with `TJ` spacing numbers until PDFium's own box for it is that wide.
//!
//! **Every edit on the datasheet is made to a copy of it**, written to the
//! temporary directory and removed afterwards. The original is only ever read.

mod harness;
use harness::{serial, skip_without_pdfium};

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

use pdf_core::command::{Command, CommandHistory};
use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Document, DocumentMut, Rect, RegionRequest, TextLineEdit, TextRun, TextStyle};
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
        let path = std::env::temp_dir().join(format!("pagify-justify-{}-{tag}.pdf", std::process::id()));
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

/// Page 1's decoded content stream, as the document would write it.
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
        if let Some(Ok(Object::Array(kids))) = dict.get(b"Kids").map(|k| file.resolve(k)) {
            for kid in kids {
                if let Some(page) = file.resolve(&kid).ok().and_then(|k| first_page(file, &k)) {
                    return Some(page);
                }
            }
        }
        None
    }
    let page = first_page(&file, &pages).expect("a page");
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

/// Each operation of a stream as one line of text, whitespace squeezed.
fn operation_texts(stream: &[u8]) -> Vec<String> {
    content::parse(stream)
        .expect("parse")
        .iter()
        .map(|o| String::from_utf8_lossy(&stream[o.span.clone()]).split_whitespace().collect::<Vec<_>>().join(" "))
        .collect()
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

fn run_of(all: &[TextRun], object: usize) -> &TextRun {
    all.iter().find(|r| r.object == object).unwrap_or_else(|| panic!("no object {object}"))
}

/// A line's rect: the union of its pieces'. Its width is what the app asks the
/// engine to keep.
fn line_rect(all: &[TextRun], line: &[usize]) -> Rect {
    union(&line.iter().map(|o| run_of(all, *o).rect).collect::<Vec<_>>())
}

/// A line's words as the editor would show them: the pieces in the order they are
/// drawn, as one string.
fn line_words(all: &[TextRun], line: &[usize]) -> String {
    clean(&line.iter().map(|o| run_of(all, *o).text.clone()).collect::<String>())
}

/// What typing a line's words into its first piece looks like with its second word
/// dropped — the line comes out **narrower**, and everything it needs is on the
/// page in its first piece's own font.
fn without_second_word(all: &[TextRun], line: &[usize]) -> String {
    let words: Vec<String> = line_words(all, line).split(' ').filter(|w| !w.is_empty()).map(str::to_string).collect();
    assert!(words.len() >= 3, "a line of at least three words: {words:?}");
    let mut kept = words;
    kept.remove(1);
    kept.join(" ")
}

/// Where an object is numbered after `removed` have come off the page.
fn renumbered(object: usize, removed: &[usize]) -> usize {
    object - removed.iter().filter(|r| **r < object).count()
}

fn same_but_object(a: &TextRun, b: &TextRun) -> bool {
    a.text == b.text && a.rect == b.rect && a.origin == b.origin && a.size == b.size && a.color == b.color
}

// ============================================================ the datasheet ==

/// **The datasheet's paragraph, retyped line by line with a word dropped and
/// spread back to the width each line had.** For each of its twelve justified
/// lines: the first piece is retyped, the others come off, and the line's own
/// piece comes out within half a point of where the whole line ended — left edge
/// where it was, right edge where the line's last piece had its. The closing line,
/// retyped without a width, stays as short as its words make it.
///
/// And the rest of the page is exactly as it was: every other object in its words,
/// colour, box, origin and size at its new number; the picture outside the
/// paragraph pixel for pixel; no `Tw` or `Tc` anywhere new in the stream — the
/// spacing is written into the operators it belongs to and nothing is left in force
/// for what is drawn after them. One undo puts the page back byte for byte, in
/// words, colour, box, origin and pixels.
#[test]
fn the_justified_paragraph_keeps_the_width_of_every_line() {
    let Some(copy) = Copy::of_the_datasheet("paragraph") else { return };
    let _lock = serial();
    let mut doc = copy.open();

    let before = runs(&doc);
    let lines = lines_of(&before);
    assert_eq!(lines.len(), 13);
    let removed: Vec<usize> = lines.iter().flat_map(|l| l[1..].iter().copied()).collect();
    let image_before = render(&doc, 1.5);
    let stream_before = stream_of(&mut doc);

    // ---- one edit for each line: every line but the last spread to its old width
    let mut edits = Vec::new();
    let mut typed = Vec::new();
    let mut widths = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let last = i == lines.len() - 1;
        let text = if last {
            // The closing line: its first piece's own words, cut at the middle.
            let words = line_words(&before, line);
            let cut = (words.chars().count() * 2 / 3).max(1);
            words.chars().take(cut).collect::<String>().trim_end().to_string()
        } else {
            without_second_word(&before, line)
        };
        let rect = line_rect(&before, line);
        let width = rect.right - rect.left;
        typed.push(text.clone());
        widths.push(width);
        edits.push(TextLineEdit::Retype {
            first: line[0],
            text,
            style: TextStyle::default(),
            remove: line[1..].to_vec(),
            justify_to: if last { None } else { Some(width) },
        });
    }

    // ---- apply, as one undoable step
    let mut history = CommandHistory::new(8);
    let started = std::time::Instant::now();
    history
        .execute(Command::ReplaceTextLines { page_index: 0, edits }, &mut doc)
        .unwrap_or_else(|e| panic!("the paragraph was refused: {e}"));
    println!("13 lines typed, 12 spread to their width, {} pieces removed: applied in {:.0} ms", removed.len(), started.elapsed().as_secs_f64() * 1000.0);

    // ---- every line ends where it ended
    let after = runs(&doc);
    assert_eq!(after.len(), before.len() - removed.len());
    let mut worst = 0.0f32;
    for (i, line) in lines.iter().enumerate() {
        let now = run_of(&after, renumbered(line[0], &removed));
        let was = line_rect(&before, line);
        assert_eq!(clean(&now.text), typed[i], "line {}: the first piece should read what was typed", i + 1);
        let (dr, dl) = (now.rect.right - was.right, now.rect.left - was.left);
        println!(
            "line {:>2}: {:>6.2} pt wide was {:>6.2}; right edge {:+.3} pt, left edge {:+.3} pt from the original line's{}",
            i + 1,
            now.rect.right - now.rect.left,
            widths[i],
            dr,
            dl,
            if i == lines.len() - 1 { "  (the closing line, not spread)" } else { "" }
        );
        if i + 1 < lines.len() {
            assert!(dr.abs() <= 0.5, "line {}: ends {dr:+.2} pt from where it ended", i + 1);
            assert!(dl.abs() <= 0.5, "line {}: moved sideways by {dl:+.2} pt", i + 1);
            worst = worst.max(dr.abs());
        } else {
            assert!(now.rect.right < was.right - 3.0, "the closing line was stretched: it ends {dr:+.2} pt from where it ended");
        }
    }
    println!("worst right edge off by {worst:.3} pt");

    // ---- the rest of the page, untouched
    let paragraph_objects: HashSet<usize> = paragraph().into_iter().collect();
    let mut others = 0;
    for b in &before {
        if paragraph_objects.contains(&b.object) {
            continue;
        }
        let a = run_of(&after, renumbered(b.object, &removed));
        assert!(same_but_object(a, b), "object {} is not part of the paragraph and changed:\n  {b:?}\n  {a:?}", b.object);
        others += 1;
    }
    println!("{others} objects outside the paragraph identical in words, colour, box, origin and size");
    assert_eq!(others, before.len() - paragraph_objects.len());

    let image_after = render(&doc, 1.5);
    let paragraph_box = grown(union(&lines.iter().map(|l| line_rect(&before, l)).collect::<Vec<_>>()), 3.0);
    let outside = differing_pixels(&image_before, &image_after, 1.5, &[paragraph_box]);
    println!("pixels that differ outside the paragraph: {outside}");
    assert_eq!(outside, 0);

    // ---- nothing is left in force
    let stream_after = stream_of(&mut doc);
    let count = |texts: &[String], op: &str| texts.iter().filter(|t| t.ends_with(&format!(" {op}")) || **t == op).count();
    let (b, a) = (operation_texts(&stream_before), operation_texts(&stream_after));
    for op in ["Tw", "Tc", "Tz"] {
        println!("{op} operators: {} before, {} after", count(&b, op), count(&a, op));
        assert_eq!(count(&a, op), count(&b, op), "the edit wrote a {op} that stays in force for what is drawn after it");
    }

    // ---- undo: byte for byte
    history.undo(&mut doc).expect("undo").expect("a step to undo");
    assert!(stream_of(&mut doc) == stream_before, "after undo the content stream is not byte for byte what it was");
    assert!(runs(&doc) == before, "after undo the objects are not exactly what they were");
    let off = differing_pixels(&image_before, &render(&doc, 1.5), 1.5, &[]);
    println!("pixels that differ after undo, anywhere on the page: {off}");
    assert_eq!(off, 0);
}

/// **One line of the paragraph spread, and nothing after it moves**: the lines
/// below it and everything else on the page, in words, colour, box, origin, size
/// and number, are what they were. (The first test retypes every line, so a `Tw`
/// that leaked into the next line would be hidden by that line being retyped; here
/// the lines after it are left alone, and they draw with whatever is in force.)
#[test]
fn spreading_one_line_leaves_the_lines_after_it_alone() {
    let Some(copy) = Copy::of_the_datasheet("one") else { return };
    let _lock = serial();
    let mut doc = copy.open();
    let before = runs(&doc);
    let lines = lines_of(&before);
    let image_before = render(&doc, 1.5);
    let stream_before = stream_of(&mut doc);

    let at = 3usize;
    let line = &lines[at];
    let rect = line_rect(&before, line);
    let text = without_second_word(&before, line);
    let removed: Vec<usize> = line[1..].to_vec();
    doc.replace_text_lines(
        0,
        &[TextLineEdit::Retype {
            first: line[0],
            text: text.clone(),
            style: TextStyle::default(),
            remove: removed.clone(),
            justify_to: Some(rect.right - rect.left),
        }],
    )
    .unwrap_or_else(|e| panic!("refused: {e}"));
    let after = runs(&doc);

    let now = run_of(&after, renumbered(line[0], &removed));
    println!("line {}: {:?} spans {:.3} pt, was {:.3}", at + 1, clean(&now.text), now.rect.right - now.rect.left, rect.right - rect.left);
    assert!(((now.rect.right - now.rect.left) - (rect.right - rect.left)).abs() <= 0.1, "the line does not span its old width");
    for b in before.iter().filter(|b| b.object != line[0] && !removed.contains(&b.object)) {
        let a = run_of(&after, renumbered(b.object, &removed));
        assert!(same_but_object(a, b), "object {} (line {:?}) changed", b.object, lines.iter().position(|l| l.contains(&b.object)));
    }
    let image_after = render(&doc, 1.5);
    let outside = differing_pixels(&image_before, &image_after, 1.5, &[grown(rect, 3.0)]);
    println!("pixels that differ outside the line: {outside}");
    assert_eq!(outside, 0);

    // The stream: the only operators that changed are this line's first piece (the
    // spread version of it) and the pieces taken off.
    let (b, a) = (operation_texts(&stream_before), operation_texts(&stream_of(&mut doc)));
    // One operator in, `Tf` and `TJ` out: the edit selects the font it writes in.
    assert_eq!(a.len(), b.len() - removed.len() + 1, "operators: {} before, {} after", b.len(), a.len());
    let changed = a.iter().filter(|t| !b.contains(t)).count();
    println!("{changed} operator(s) are new in the stream: {:?}", a.iter().filter(|t| !b.contains(t)).map(|t| t.chars().take(90).collect::<String>()).collect::<Vec<_>>());
    assert!(changed <= 2, "{changed} operators changed for one line");
}

/// CAMINO's COB paragraph (page 1, objects 418..=435): 13 lines of its Light CID
/// font (Identity-H, spaces and a line-ending hyphen made of pieces of their own),
/// the first with four pieces. Where the datasheet's fonts are simple TrueType
/// subsets, these are composite: a stretch that depended on a single-byte space
/// would not reach them.
const CAMINO: &str = r"C:\Users\hsili\Downloads\CAMINO elitee-plus 3.0.pdf";

fn camino_lines() -> Vec<Vec<usize>> {
    vec![
        vec![418, 419, 420, 421],
        vec![422],
        vec![423],
        vec![424],
        vec![425, 426],
        vec![427],
        vec![428],
        vec![429],
        vec![430],
        vec![431],
        vec![432, 433],
        vec![434],
        vec![435],
    ]
}

/// **A composite font's justified lines keep their width too**: each of CAMINO's
/// twelve justified lines is retyped on a copy of the file, one at a time, with
/// its second word dropped and spread to the width the line had; the line's first
/// piece comes out within half a point of where it ended. A line whose retype
/// the engine refuses for a reason of its own (a letter its subset does not carry,
/// which is not what is being asked here) is counted and left out.
#[test]
fn camino_s_composite_lines_keep_their_width() {
    let Some(_) = skip_without_pdfium() else { return };
    if !std::path::Path::new(CAMINO).exists() {
        eprintln!("skipped: {CAMINO} is not on this machine");
        return;
    }
    let _lock = serial();
    let path = std::env::temp_dir().join(format!("pagify-justify-{}-camino.pdf", std::process::id()));
    std::fs::copy(CAMINO, &path).expect("copy CAMINO");
    let copy = Copy(path);
    let lines = camino_lines();
    let (mut done, mut refused, mut worst) = (0, 0, 0.0f32);
    for (i, line) in lines.iter().enumerate().take(lines.len() - 1) {
        let mut doc = copy.open();
        let before = runs(&doc);
        let words: Vec<String> = line_words(&before, line).split_whitespace().map(str::to_string).collect();
        if words.len() < 3 {
            continue;
        }
        let mut kept = words.clone();
        kept.remove(1);
        let text = kept.join(" ");
        let rect = line_rect(&before, line);
        let removed: Vec<usize> = line[1..].to_vec();
        let outcome = doc.replace_text_lines(0, &[spread(line[0], &text, &removed, rect.right - rect.left)]);
        match outcome {
            Err(e) => {
                println!("line {:>2}: {text:?}: refused: {e}", i + 1);
                refused += 1;
            }
            Ok(()) => {
                let after = runs(&doc);
                let now = run_of(&after, renumbered(line[0], &removed));
                let (dr, dl) = (now.rect.right - rect.right, now.rect.left - rect.left);
                println!("line {:>2}: {text:?}: {:.3} pt wide, was {:.3}; right edge {dr:+.3}, left edge {dl:+.3}", i + 1, now.rect.right - now.rect.left, rect.right - rect.left);
                assert!(dr.abs() <= 0.5 && dl.abs() <= 0.5, "line {}: right edge {dr:+.2}, left edge {dl:+.2} pt from the line's", i + 1);
                assert_eq!(clean(&now.text), text);
                worst = worst.max(dr.abs());
                done += 1;
            }
        }
    }
    println!("{done} lines spread, {refused} refused for their own reasons; the worst right edge is {worst:.3} pt off");
    assert!(done >= 8, "only {done} of CAMINO's lines could be retyped at all");
}

/// An Arial to type with, if this machine has one: a letter a document's own
/// subset fonts cannot spell has to come from a font written into the file.
fn arial() -> Option<Vec<u8>> {
    [r"C:\Windows\Fonts\arial.ttf", "/System/Library/Fonts/Supplemental/Arial.ttf", "/usr/share/fonts/truetype/msttcorefonts/Arial.ttf"]
        .iter()
        .find_map(|path| std::fs::read(path).ok())
}

/// **Spread lines that each need a new font written into the file** — the one-at-a-
/// time way the plain edit already has for two of them — still end where the line
/// ended, with the font's own widths (the measuring is of the page as written), and
/// still all or nothing: a third line asking for what cannot be spread fails after
/// the first two were written, and the page is put back exactly as it was.
#[test]
fn spread_lines_that_each_need_a_new_font_are_measured_with_that_font() {
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
    let (wa, wb) = (line_rect(&before, a), line_rect(&before, b));
    let (ta, tb) = ("plié by the COB conforming", "which ñare market leading");
    let edits = vec![
        spread(a[0], ta, &a[1..], wa.right - wa.left),
        spread(b[0], tb, &b[1..], wb.right - wb.left),
    ];
    let mut history = CommandHistory::new(4);
    history
        .execute(Command::ReplaceTextLines { page_index: 0, edits: edits.clone() }, &mut doc)
        .unwrap_or_else(|e| panic!("two spread lines needing fonts were refused: {e}"));
    let removed: Vec<usize> = a[1..].iter().chain(b[1..].iter()).copied().collect();
    let after = runs(&doc);
    for (line, text, rect) in [(a, ta, wa), (b, tb, wb)] {
        let now = run_of(&after, renumbered(line[0], &removed));
        println!("{text:?}: {:.3} pt wide, was {:.3}; right edge {:+.3} pt", now.rect.right - now.rect.left, rect.right - rect.left, now.rect.right - rect.right);
        assert_eq!(clean(&now.text), text);
        assert!((now.rect.right - rect.right).abs() <= 0.5, "{text:?} ends {:+.2} pt from where the line ended", now.rect.right - rect.right);
    }
    history.undo(&mut doc).expect("undo").expect("a step to undo");
    assert!(stream_of(&mut doc) == stream_before, "after undo the content stream is not byte for byte what it was");

    // A third line that cannot be spread (a single letter), after two that were written.
    let mut doc = setup();
    let mut with_a_third = edits.clone();
    with_a_third.push(spread(c[0], "x", &c[1..], 200.0));
    let mut history = CommandHistory::new(4);
    let error = history
        .execute(Command::ReplaceTextLines { page_index: 0, edits: with_a_third }, &mut doc)
        .err()
        .unwrap_or_else(|| panic!("a line that cannot be spread was accepted"));
    println!("a third line that cannot be spread: refused: {error}");
    assert!(stream_of(&mut doc) == stream_before, "the first two lines were left written");
    assert!(runs(&doc) == before, "the page was not put back as it was");
    assert!(history.undo(&mut doc).expect("undo").is_none(), "a refused command left something to undo");
}

// ============================================================ synthetic pages ==

/// A PDF made of these numbered objects (the first is object 1, the catalogue),
/// with a classic cross-reference table.
fn pdf_of(objects: &[String]) -> Vec<u8> {
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
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

/// A one-page PDF drawing `content`, with Helvetica as `/F1`. Helvetica is not
/// embedded, so every letter the page has is every letter there is: the retyped
/// lines below can be of any words.
fn page_with(content: &str) -> PdfiumDocument {
    let stream = format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len());
    let pdf = pdf_of(&[
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>"
            .to_string(),
        stream,
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_string(),
    ]);
    PdfiumDocument::open_bytes(pdf, None).expect("open")
}

fn spread(first: usize, text: &str, remove: &[usize], width: f32) -> TextLineEdit {
    TextLineEdit::Retype { first, text: text.to_string(), style: TextStyle::default(), remove: remove.to_vec(), justify_to: Some(width) }
}

fn width_of(doc: &PdfiumDocument, object: usize) -> f32 {
    let all = runs(doc);
    let r = run_of(&all, object);
    r.rect.right - r.rect.left
}

/// What every synthetic test draws: lines to spread and a line after each to be
/// left alone.
const LINES: &str = "\
    BT /F1 12 Tf 72 700 Td (alpha beta gamma delta) Tj ET \n\
    BT /F1 12 Tf 72 680 Td (below it) Tj ET \n\
    BT /F1 12 Tf 72 660 Td (single) Tj ET \n\
    BT /F1 12 Tf 72 640 Td (x) Tj ET \n\
    BT /F1 12 Tf 0 1 -1 0 400 300 Tm (turned words here) Tj ET \n\
    BT /F1 12 Tf 2 Tc 6 Tw 72 600 Td (spacing in force here) Tj 0 Tc 0 Tw ET \n\
    BT /F1 12 Tf 150 Tz 72 580 Td (wide scaled line here) Tj 100 Tz ET \n\
    BT /F1 12 Tf 72 560 Td (the last line) Tj ET";

/// **A line is made exactly as wide as it is asked to be**, wider or narrower, to
/// a hundredth of a point — whatever is in force where it is drawn: plain, with a
/// `Tc` and a `Tw` set (which are in the plain line's width and so in the amount to
/// add), under a horizontal scaling of 150%. The words come out as typed, the
/// origin does not move, and the lines drawn after it are what they were.
#[test]
fn a_line_is_spread_to_exactly_the_width_asked_for() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    for (what, object, words) in [
        ("plain", 0usize, "alpha beta gamma delta"),
        ("with a Tc and a Tw in force", 5, "spacing in force here"),
        ("under a 150% horizontal scaling", 6, "wide scaled line here"),
    ] {
        for ask in [-4.0f32, 0.0, 17.5, 60.0] {
            let mut doc = page_with(LINES);
            let before = runs(&doc);
            let was = run_of(&before, object).clone();
            let target = (was.rect.right - was.rect.left) + ask;
            doc.replace_text_lines(0, &[spread(object, words, &[], target)]).unwrap_or_else(|e| panic!("{what} {ask:+}: refused: {e}"));
            let after = runs(&doc);
            let now = run_of(&after, object);
            let got = now.rect.right - now.rect.left;
            println!("{what:<34} asked {target:>7.2} pt ({ask:+5.1}): got {got:.4}");
            assert!((got - target).abs() < 0.01, "{what} {ask:+}: asked for {target:.3} pt, got {got:.3}");
            assert_eq!(clean(&now.text), words, "{what}: the words are not as typed");
            assert!((now.origin.x - was.origin.x).abs() < 0.001 && (now.origin.y - was.origin.y).abs() < 0.001, "{what}: the line moved");
            assert!((now.rect.left - was.rect.left).abs() < 0.01, "{what}: the left edge moved");
            for (a, b) in after.iter().zip(&before).filter(|(_, b)| b.object != object) {
                assert!(same_but_object(a, b), "{what} {ask:+}: object {} changed", b.object);
            }
        }
    }
}

/// **A word gap opens or closes by the same amount everywhere** — the gaps of the
/// line are equal, as a word-spacing operator would make them — and a single word
/// opens between its letters instead.
#[test]
fn the_gaps_of_a_line_are_equal_and_a_single_word_opens_between_its_letters() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    let mut doc = page_with(LINES);
    let before = runs(&doc);
    let w = |o| {
        let r = run_of(&before, o);
        r.rect.right - r.rect.left
    };
    doc.replace_text_lines(0, &[spread(0, "alpha beta gamma delta", &[], w(0) + 30.0), spread(2, "single", &[], w(2) + 3.0)])
        .unwrap_or_else(|e| panic!("refused: {e}"));
    let stream = stream_of(&mut doc);
    let texts = operation_texts(&stream);
    let tj = |needle: &str| texts.iter().find(|t| t.ends_with("TJ") && t.contains(needle)).unwrap_or_else(|| panic!("no TJ with {needle}")).clone();
    // The spacing numbers of a TJ: what is outside its hex strings.
    let numbers_of = |text: &str| -> Vec<f32> {
        let (mut outside, mut inside) = (String::new(), false);
        for c in text.chars() {
            match c {
                '<' => inside = true,
                '>' => inside = false,
                c if !inside => outside.push(c),
                _ => {}
            }
        }
        outside.split(['[', ']', ' ']).filter_map(|p| p.parse::<f32>().ok()).collect()
    };
    // alpha, beta, gamma, delta: three gaps, three equal numbers.
    let line = tj("616C706861"); // "alpha" as hex
    let numbers = numbers_of(&line);
    println!("the four-word line: {line}   numbers {numbers:?}");
    assert_eq!(numbers.len(), 3, "three gaps");
    assert!(numbers.iter().all(|n| (n - numbers[0]).abs() < 0.01) && numbers[0] < 0.0, "equal, negative (opening) gaps: {numbers:?}");
    // The single word: a number between every pair of letters (5 gaps in six letters).
    let word = tj("<73> "); // "single", its letters now apart: s, i, n, g, l, e
    println!("the single word: {word}");
    assert_eq!(numbers_of(&word).len(), 5, "five gaps between six letters");
}

/// **What cannot be spread is refused, with a message, and nothing is written** —
/// not the stream, not the objects, not the history; and one refused line refuses
/// the whole batch it is in: text drawn at an angle, a line of one character, a
/// line far too long for its width, a single word asked to open to a sentence, a
/// width that is not a width.
#[test]
fn what_cannot_be_spread_is_refused_and_nothing_is_written() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    let mut doc = page_with(LINES);
    let stream_before = stream_of(&mut doc);
    let before = runs(&doc);
    let w = |o: usize| {
        let r = run_of(&before, o);
        r.rect.right - r.rect.left
    };
    let cases: Vec<(&str, Vec<TextLineEdit>, &str)> = vec![
        ("text drawn at an angle", vec![spread(4, "turned words here", &[], w(4) + 10.0)], "angle"),
        ("a line of one character", vec![spread(3, "x", &[], w(3) + 10.0)], "one character"),
        ("a line far too long for its width", vec![spread(0, "alpha beta gamma delta", &[], 40.0)], "on top of each other"),
        ("a single word asked to fill a sentence", vec![spread(2, "single", &[], 300.0)], "single word"),
        ("a width of zero", vec![spread(0, "alpha beta gamma delta", &[], 0.0)], "width"),
        ("a negative width", vec![spread(0, "alpha beta gamma delta", &[], -5.0)], "width"),
        ("a width that is not a number", vec![spread(0, "alpha beta gamma delta", &[], f32::NAN)], "width"),
        (
            "one line that cannot, with a line that can",
            vec![spread(0, "alpha beta gamma delta", &[], w(0) + 10.0), spread(3, "x", &[], w(3) + 10.0)],
            "one character",
        ),
    ];
    for (what, edits, says) in cases {
        let error = doc.replace_text_lines(0, &edits).err().unwrap_or_else(|| panic!("{what}: accepted"));
        let message = error.to_string();
        println!("{what}: refused: {message}");
        assert!(message.contains(says), "{what}: the message {message:?} does not say {says:?}");
        assert!(message.len() > 20, "{what}: a message that says nothing");
        assert!(stream_of(&mut doc) == stream_before, "{what}: a refused edit changed the stream");
        assert!(runs(&doc) == before, "{what}: a refused edit changed the objects");
    }
    // Through the command stack: nothing to undo afterwards.
    let mut history = CommandHistory::new(4);
    let refused = history.execute(Command::ReplaceTextLines { page_index: 0, edits: vec![spread(3, "x", &[], w(3) + 10.0)] }, &mut doc);
    assert!(refused.is_err());
    assert!(history.undo(&mut doc).expect("undo").is_none(), "a refused edit left something to undo");
}

/// **Spreading is undone exactly**: words, colour, box, origin and pixels, the
/// stream byte for byte, and redone to the same bytes — the same guarantee the
/// plain edit has.
#[test]
fn a_spread_line_is_undone_and_redone_exactly() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    let mut doc = page_with(LINES);
    let before = runs(&doc);
    let stream_before = stream_of(&mut doc);
    let image_before = render(&doc, 1.5);
    let target = width_of(&doc, 0) + 25.0;
    let mut history = CommandHistory::new(4);
    history
        .execute(Command::ReplaceTextLines { page_index: 0, edits: vec![spread(0, "alpha beta gamma delta", &[], target)] }, &mut doc)
        .unwrap_or_else(|e| panic!("refused: {e}"));
    let stream_after = stream_of(&mut doc);
    let after = runs(&doc);
    assert!(stream_after != stream_before);
    assert!((width_of(&doc, 0) - target).abs() < 0.01);

    history.undo(&mut doc).expect("undo").expect("a step");
    assert!(stream_of(&mut doc) == stream_before, "undo: the stream is not byte for byte what it was");
    assert!(runs(&doc) == before, "undo: the objects are not what they were");
    assert_eq!(differing_pixels(&image_before, &render(&doc, 1.5), 1.5, &[]), 0, "undo: the pixels are not what they were");
    history.redo(&mut doc).expect("redo").expect("a step");
    assert!(stream_of(&mut doc) == stream_after, "redo wrote different bytes");
    assert!(runs(&doc) == after);
}

/// **A stretched line and the pieces it replaces, together**: three pieces on one
/// line (each placed by its own `Td`), the first retyped and spread to the width
/// the whole line had, the other two taken off — what the app sends for a justified
/// paragraph's line.
#[test]
fn a_line_of_pieces_is_replaced_by_one_piece_as_wide_as_the_line_was() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    let mut doc = page_with(
        "BT /F1 12 Tf 72 700 Td (alpha beta) Tj 90 0 Td (gamma delta) Tj 90 0 Td (epsilon) Tj ET \n\
         BT /F1 12 Tf 72 680 Td (below) Tj ET",
    );
    let before = runs(&doc);
    let line = line_rect(&before, &[0, 1, 2]);
    let target = line.right - line.left;
    doc.replace_text_lines(0, &[spread(0, "alpha beta gamma delta epsilon", &[1, 2], target)]).unwrap_or_else(|e| panic!("refused: {e}"));
    let after = runs(&doc);
    assert_eq!(after.len(), 2);
    let now = &after[0];
    println!("the line: {:?} {:.3} pt wide, was {:.3}", clean(&now.text), now.rect.right - now.rect.left, target);
    assert!(((now.rect.right - now.rect.left) - target).abs() < 0.01);
    assert!((now.rect.right - line.right).abs() < 0.05, "its right edge is not where the line's last piece ended");
    assert!(same_but_object(&after[1], &before[3]), "the line below changed");
}
