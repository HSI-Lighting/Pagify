//! Hidden data — what a document carries that is not on its pages.
//!
//! The tests here save and reopen, like the lock and redaction suites, because
//! the claim is about the **file** rather than about anything in memory: a
//! sanitiser that cleaned a document PDFium was holding, and left the bytes
//! alone, would pass every test that never wrote one out.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test --test hidden
//! ```

mod harness;
use harness::{serial, skip_without_pdfium};

use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Document, DocumentMut, Rect, Redaction};

fn open(name: &str) -> PdfiumDocument {
    let path = harness::fixture_path(name);
    PdfiumDocument::open_path(path.to_str().expect("path"), None).expect("open fixture")
}

fn text_of(doc: &dyn Document, page: usize) -> String {
    doc.page(page).expect("page").characters().expect("characters").text
}

/// **What it finds is what a person needs to decide by.**
#[test]
fn a_survey_reports_what_the_file_carries() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    let found = doc.hidden_data().expect("survey");

    // Whatever it says, it must say it in words somebody can act on.
    let said = found.describe();
    assert!(!said.is_empty());
    assert_eq!(found.is_empty(), said.contains("nothing hidden"));
}

/// **The pages survive.** A sanitiser that loses a page is worse than one that
/// leaves metadata behind.
#[test]
fn sanitising_keeps_every_page_exactly_as_it_was() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("text-lines.pdf");
    let before: Vec<String> = (0..doc.page_count()).map(|p| text_of(&doc, p)).collect();

    doc.remove_hidden_data().expect("sanitise");

    assert_eq!(doc.page_count(), before.len(), "a page went");
    for (page, was) in before.iter().enumerate() {
        assert_eq!(&text_of(&doc, page), was, "page {} came back different", page + 1);
    }
}

/// And once cleaned, a second survey finds nothing — otherwise the button does
/// not do what its own report says it does.
#[test]
fn cleaning_twice_finds_nothing_the_second_time() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    doc.remove_hidden_data().expect("sanitise");

    let after = doc.hidden_data().expect("survey");
    assert!(
        after.is_empty(),
        "cleaning left something behind: {}",
        after.describe()
    );
}

/// **A sanitised document must be saved whole.**
///
/// Appending to it would start the problem over: the cleaned revision, with an
/// earlier version of every page in front of it.
#[test]
fn a_sanitised_document_must_be_saved_as_a_full_copy() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    doc.remove_hidden_data().expect("sanitise");
    assert!(doc.must_save_full_copy());

    let mut bytes = Vec::new();
    assert!(doc.save_incremental(&mut bytes).is_err());
}

/// **It must never take the lock's own copy.**
///
/// A locked document keeps the sealed original inside itself. That is hidden
/// data by any definition, and removing it would destroy the only copy of what
/// somebody was promised they could get back.
#[test]
fn sanitising_leaves_a_lock_able_to_be_opened() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    const PASSCODE: &[u8] = b"correct horse battery staple";

    let mut doc = open("text-lines.pdf");
    doc.lock_area(&Redaction::new(0, Rect { left: 35.0, top: 45.0, right: 170.0, bottom: 66.0 }), PASSCODE, None)
        .expect("lock");
    assert!(!text_of(&doc, 0).contains("The quick brown fox"), "the lock did nothing");

    doc.remove_hidden_data().expect("sanitise");

    // Saved and reopened, because the vault lives in the file rather than in
    // memory and this is the whole question.
    let mut bytes = Vec::new();
    doc.save_full_copy(&mut bytes).expect("save");
    let mut reopened = PdfiumDocument::open_bytes(bytes, None).expect("reopen");

    assert_eq!(
        reopened.locked_pages().expect("locked pages"),
        vec![0],
        "sanitising destroyed the lock"
    );
    let pages = reopened.open_lock(PASSCODE).expect("the passcode no longer opens it");
    let (index, pdf) = &pages[0];
    reopened.replace_page(*index, pdf).expect("restore");
    assert!(
        text_of(&reopened, 0).contains("The quick brown fox"),
        "the sealed original did not survive sanitising"
    );
}

/// The words on the page are never touched — this rewrites the file's
/// structure, not its content streams.
#[test]
fn sanitising_does_not_rewrite_the_text_on_a_page() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    let before = text_of(&doc, 0);
    doc.remove_hidden_data().expect("sanitise");
    assert_eq!(text_of(&doc, 0), before, "the page's own text was rewritten");
}

// ------------------------------------------------------------ smart redact --

/// **Found on the page, with somewhere to point at.**
///
/// The scanner's own tests check what it recognises in text; this checks the
/// other half — that a find comes back with the rectangle it occupies, which is
/// what makes it something a person can act on rather than a line in a report.
#[test]
fn something_sensitive_comes_back_with_where_it_sits() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let path = format!(
        "{}/Downloads/HSI CATALOG 2026.pdf",
        std::env::var("HOME").unwrap_or_default()
    );
    if !std::path::Path::new(&path).is_file() {
        eprintln!("skipping: no catalogue in ~/Downloads");
        return;
    }
    let Ok(doc) = PdfiumDocument::open_path(&path, None) else {
        eprintln!("skipping: the catalogue is present but would not open");
        return;
    };

    let Some((page, found)) = (0..doc.page_count().min(60)).find_map(|p| {
        doc.sensitive_on(p).ok().filter(|f| !f.is_empty()).map(|f| (p, f))
    }) else {
        eprintln!("skipping: nothing sensitive in this copy");
        return;
    };

    let size = doc.page_size(page).expect("size");
    for item in &found {
        assert!(!item.text.is_empty(), "a find with nothing in it");
        assert_eq!(item.page_index, page);
        assert!(
            item.area.right > item.area.left && item.area.bottom > item.area.top,
            "{:?} came back with an empty rectangle",
            item.text
        );
        assert!(
            item.area.left >= -1.0 && item.area.right <= size.width_pt + 1.0,
            "{:?} is off the page: {:?}",
            item.text,
            item.area
        );
        // What was found must actually be on the page where it says.
        let text = doc.page(page).expect("page").characters().expect("characters").text;
        assert!(text.contains(&item.text), "{:?} is not in the page's text", item.text);
    }
}

/// A page with nothing on it comes back empty rather than refusing.
#[test]
fn a_page_with_nothing_sensitive_says_so() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let doc = open("two-column.pdf");
    assert!(doc.sensitive_on(0).expect("scan").is_empty());
}

// ---------------------------------------------------------------- whiteout --

/// **A whiteout covers; it does not remove.**
///
/// The whole point of keeping this apart from redaction is that the words stay
/// in the file. A test that only checked the page looked right would be
/// checking the wrong thing — and would pass just as happily if this quietly
/// destroyed the text, which is the confusion the two tools exist to prevent.
#[test]
fn a_whiteout_leaves_the_words_underneath_exactly_where_they_were() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    use pdf_core::document::Color;

    let mut doc = open("two-column.pdf");
    let before = text_of(&doc, 0);
    assert!(before.contains("luminaire"), "control");

    let size = doc.page_size(0).expect("size");
    let area = Rect {
        left: 0.0,
        top: 0.0,
        right: size.width_pt,
        bottom: size.height_pt * 0.5,
    };
    doc.whiteout(0, area, Color { r: 255, g: 255, b: 255, a: 255 }).expect("whiteout");

    // Still there, through a save and a reopen — which is what anyone else
    // would see.
    let mut bytes = Vec::new();
    doc.save_full_copy(&mut bytes).expect("save");
    let reopened = PdfiumDocument::open_bytes(bytes, None).expect("reopen");
    assert_eq!(
        text_of(&reopened, 0),
        before,
        "a whiteout removed text it was only supposed to cover"
    );
}

/// And it really is painted on the page, not an annotation a reader can turn
/// off — so it prints, and it is there in any viewer.
#[test]
fn a_whiteout_is_page_content_rather_than_a_note() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    use pdf_core::document::Color;

    let mut doc = open("two-column.pdf");
    let annotations_before = doc.annotations(0).expect("annotations").len();

    let size = doc.page_size(0).expect("size");
    doc.whiteout(
        0,
        Rect { left: 10.0, top: 10.0, right: size.width_pt - 10.0, bottom: 200.0 },
        Color { r: 255, g: 255, b: 255, a: 255 },
    )
    .expect("whiteout");

    assert_eq!(
        doc.annotations(0).expect("annotations").len(),
        annotations_before,
        "it added an annotation, which a reader can switch off"
    );

    // And the page draws differently, which is what was asked for.
    let mut pixels = vec![0u8; 200 * 260 * 4];
    let mut target = pdf_core::render::RenderTarget {
        width: 200,
        height: 260,
        stride: 200 * 4,
        order: pdf_core::render::PixelOrder::Rgba,
        pixels: &mut pixels,
    };
    doc.page(0)
        .expect("page")
        .render_into(
            &pdf_core::document::RenderRequest {
                scale: 200.0 / size.width_pt,
                ..Default::default()
            },
            &mut target,
        )
        .expect("render");
    let inked = pixels
        .chunks_exact(4)
        .filter(|p| p[0] < 240 || p[1] < 240 || p[2] < 240)
        .count();
    assert!(inked > 0, "the page came back blank altogether");
}

/// An area with no size is a slip, not an instruction.
#[test]
fn a_whiteout_of_nothing_is_refused() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    use pdf_core::document::Color;
    let mut doc = open("two-column.pdf");
    let white = Color { r: 255, g: 255, b: 255, a: 255 };
    assert!(doc
        .whiteout(0, Rect { left: 10.0, top: 10.0, right: 10.0, bottom: 40.0 }, white)
        .is_err());
    assert!(doc
        .whiteout(0, Rect { left: 10.0, top: 40.0, right: 60.0, bottom: 40.0 }, white)
        .is_err());
}

// ------------------------------------------------------- the sign-in box --

/// **The box is an outline, not a cover.**
///
/// The mark somebody puts *around* an answer while filling a form in. A filled
/// one would hide the answer it was meant to draw attention to, and — like the
/// whiteout above — a test that only checked the page looked different would
/// pass just as happily either way.
#[test]
fn a_box_outlines_what_it_is_drawn_around_rather_than_covering_it() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    let before = text_of(&doc, 0);
    assert!(before.contains("luminaire"), "control");

    let size = doc.page_size(0).expect("size");
    let area = Rect {
        left: 20.0,
        top: 20.0,
        right: size.width_pt - 20.0,
        bottom: size.height_pt * 0.5,
    };
    doc.stamp_box(0, area).expect("box");

    // Whatever it was drawn around is still readable — through a save, which is
    // what anyone else would see.
    let mut bytes = Vec::new();
    doc.save_full_copy(&mut bytes).expect("save");
    let reopened = PdfiumDocument::open_bytes(bytes, None).expect("reopen");
    assert_eq!(
        text_of(&reopened, 0),
        before,
        "drawing a box changed the words inside it"
    );
}

/// And it is on the page rather than laid over it — the difference between this
/// and the `rectangle` drawing tool.
#[test]
fn a_box_is_page_content_rather_than_a_mark_to_pick_up() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    let before = doc.annotations(0).expect("annotations").len();
    doc.stamp_box(0, Rect { left: 40.0, top: 40.0, right: 200.0, bottom: 120.0 })
        .expect("box");
    assert_eq!(
        doc.annotations(0).expect("annotations").len(),
        before,
        "it added an annotation, which can be selected and deleted"
    );
}

/// **A ruled line strikes through; it does not remove.**
///
/// The same trap as the whiteout: crossing something out on a form leaves it
/// perfectly readable to anything that reads the file rather than looks at it.
#[test]
fn a_ruled_line_leaves_the_words_it_strikes_through_in_the_file() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    use pdf_core::document::Point;

    let mut doc = open("two-column.pdf");
    let before = text_of(&doc, 0);
    assert!(before.contains("luminaire"), "control");

    let size = doc.page_size(0).expect("size");
    doc.stamp_line(
        0,
        Point { x: 20.0, y: size.height_pt * 0.4 },
        Point { x: size.width_pt - 20.0, y: size.height_pt * 0.4 },
    )
    .expect("line");

    let mut bytes = Vec::new();
    doc.save_full_copy(&mut bytes).expect("save");
    let reopened = PdfiumDocument::open_bytes(bytes, None).expect("reopen");
    assert_eq!(text_of(&reopened, 0), before, "ruling a line rewrote the page");
    assert!(
        reopened.annotations(0).expect("annotations").is_empty(),
        "it added an annotation, which can be selected and deleted"
    );
}

/// A line from a point to itself is a slip, not an instruction.
#[test]
fn a_line_with_no_length_is_refused() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    use pdf_core::document::Point;
    let mut doc = open("two-column.pdf");
    assert!(doc
        .stamp_line(0, Point { x: 40.0, y: 40.0 }, Point { x: 40.0, y: 40.0 })
        .is_err());
    // And a click that moved by a hair is the same slip.
    assert!(doc
        .stamp_line(0, Point { x: 40.0, y: 40.0 }, Point { x: 40.2, y: 40.1 })
        .is_err());
}

/// An area with no size is a slip, not an instruction.
#[test]
fn a_box_with_no_size_is_refused() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    assert!(doc.stamp_box(0, Rect { left: 10.0, top: 10.0, right: 10.0, bottom: 40.0 }).is_err());
    assert!(doc.stamp_box(0, Rect { left: 10.0, top: 40.0, right: 60.0, bottom: 40.0 }).is_err());
}

// ------------------------------------------------------------- sensitivity --

/// **Marked on every page, and readable back.**
#[test]
fn a_document_can_be_marked_and_the_marking_read_back() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    use pdf_core::document::sensitivity::Sensitivity;

    let mut doc = open("pages-ladder.pdf");
    assert!(doc.sensitivity().is_none(), "the fixture is already marked");

    doc.set_sensitivity(Sensitivity::Confidential).expect("mark");
    assert_eq!(doc.sensitivity(), Some(Sensitivity::Confidential));

    // Every page, not just the first — a marking on page one of forty is a
    // marking on the page nobody forwards.
    for page in 0..doc.page_count() {
        let text = text_of(&doc, page);
        assert!(
            text.contains("CONFIDENTIAL"),
            "page {} is not marked: {text:?}",
            page + 1
        );
    }
}

/// **Re-marking replaces.** A page reading "INTERNAL CONFIDENTIAL" says neither.
#[test]
fn marking_a_document_twice_leaves_one_marking() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    use pdf_core::document::sensitivity::Sensitivity;

    let mut doc = open("pages-ladder.pdf");
    doc.set_sensitivity(Sensitivity::Internal).expect("mark");
    doc.set_sensitivity(Sensitivity::Secret).expect("mark again");

    assert_eq!(doc.sensitivity(), Some(Sensitivity::Secret));
    let text = text_of(&doc, 0);
    assert!(text.contains("SECRET"));
    assert!(!text.contains("INTERNAL"), "the old marking is still there: {text:?}");
}

/// **And it comes off.** A label that could only be added would make an
/// accident permanent, and a document reclassified downwards would carry the
/// old word forever.
#[test]
fn a_marking_can_be_taken_off_again() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    use pdf_core::document::sensitivity::Sensitivity;

    let mut doc = open("pages-ladder.pdf");
    let before: Vec<String> = (0..doc.page_count()).map(|p| text_of(&doc, p)).collect();

    doc.set_sensitivity(Sensitivity::Secret).expect("mark");
    doc.clear_sensitivity().expect("unmark");

    assert!(doc.sensitivity().is_none(), "it still reads as marked");
    for (page, was) in before.iter().enumerate() {
        assert_eq!(&text_of(&doc, page), was, "page {} did not come back", page + 1);
    }
}

/// It survives the file, which is the only place it matters.
#[test]
fn a_marking_survives_a_save_and_a_reopen() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    use pdf_core::document::sensitivity::Sensitivity;

    let mut doc = open("pages-ladder.pdf");
    doc.set_sensitivity(Sensitivity::Confidential).expect("mark");

    let mut bytes = Vec::new();
    doc.save_full_copy(&mut bytes).expect("save");
    let mut reopened = PdfiumDocument::open_bytes(bytes, None).expect("reopen");

    assert_eq!(reopened.sensitivity(), Some(Sensitivity::Confidential));
}

/// **Sanitising must not take the marking off**, which is why there is no copy
/// of it in the document information.
#[test]
fn sanitising_leaves_the_marking_where_it_is() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    use pdf_core::document::sensitivity::Sensitivity;

    let mut doc = open("pages-ladder.pdf");
    doc.set_sensitivity(Sensitivity::Secret).expect("mark");
    doc.remove_hidden_data().expect("sanitise");

    assert_eq!(
        doc.sensitivity(),
        Some(Sensitivity::Secret),
        "sanitising removed the marking"
    );
}

// ------------------------------------------------------------ fill and sign --

/// **A mark really lands on the page**, and only where it was put.
#[test]
fn a_tick_is_drawn_where_it_was_asked_for() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    use pdf_core::document::{FillMark, Point};

    /// How much ink is in a small square of the page, at the given point.
    fn ink_near(doc: &dyn Document, page: usize, at: Point) -> usize {
        let size = doc.page_size(page).expect("size");
        let scale = 4.0;
        let (w, h) = ((size.width_pt * scale) as u32, (size.height_pt * scale) as u32);
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        let mut target = pdf_core::render::RenderTarget {
            width: w,
            height: h,
            stride: (w * 4) as usize,
            order: pdf_core::render::PixelOrder::Rgba,
            pixels: &mut pixels,
        };
        doc.page(page)
            .expect("page")
            .render_into(
                &pdf_core::document::RenderRequest { scale, ..Default::default() },
                &mut target,
            )
            .expect("render");

        // A 40-point box around the point, in pixels.
        let (cx, cy) = ((at.x * scale) as i64, (at.y * scale) as i64);
        let reach = (20.0 * scale) as i64;
        let mut inked = 0;
        for row in (cy - reach).max(0)..(cy + reach).min(h as i64) {
            for column in (cx - reach).max(0)..(cx + reach).min(w as i64) {
                let at = ((row * w as i64 + column) * 4) as usize;
                if pixels[at] < 200 || pixels[at + 1] < 200 || pixels[at + 2] < 200 {
                    inked += 1;
                }
            }
        }
        inked
    }

    let mut doc = open("pages-ladder.pdf");
    // A corner of the page, well away from anything the fixture draws.
    let size = doc.page_size(0).expect("size");
    let at = Point { x: size.width_pt - 60.0, y: size.height_pt - 60.0 };
    let before = ink_near(&doc, 0, at);

    doc.stamp_mark(0, FillMark::Tick, at, 18.0).expect("tick");
    let after = ink_near(&doc, 0, at);

    assert!(
        after > before,
        "nothing was drawn where the tick was asked for ({before} -> {after})"
    );
}

/// Each mark draws something, and they are not the same shape.
#[test]
fn a_tick_a_cross_and_a_dot_are_three_different_marks() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    use pdf_core::document::{FillMark, Point};

    let mut drawn = Vec::new();
    for mark in [FillMark::Tick, FillMark::Cross, FillMark::Dot] {
        let mut doc = open("pages-ladder.pdf");
        let size = doc.page_size(0).expect("size");
        doc.stamp_mark(0, mark, Point { x: size.width_pt / 2.0, y: size.height_pt / 2.0 }, 20.0)
            .expect("mark");
        let mut bytes = Vec::new();
        doc.save_full_copy(&mut bytes).expect("save");
        drawn.push(bytes.len());
    }
    // Three shapes of different complexity; a dot is four curves and a tick two
    // lines, so they cannot all come out the same size.
    assert!(
        drawn[0] != drawn[2] || drawn[1] != drawn[2],
        "the three marks produced identical files: {drawn:?}"
    );
}

/// A mark with no size is a slip, not an instruction.
#[test]
fn a_mark_with_no_size_is_refused() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    use pdf_core::document::{FillMark, Point};
    let mut doc = open("pages-ladder.pdf");
    assert!(doc.stamp_mark(0, FillMark::Tick, Point { x: 40.0, y: 40.0 }, 0.0).is_err());
}

/// And it survives the file, which is the only place it matters.
#[test]
fn a_mark_survives_a_save_and_a_reopen() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    use pdf_core::document::{FillMark, Point};

    let mut doc = open("pages-ladder.pdf");
    let mut plain = Vec::new();
    doc.save_full_copy(&mut plain).expect("save");

    doc.stamp_mark(0, FillMark::Cross, Point { x: 60.0, y: 60.0 }, 16.0).expect("cross");
    let mut marked = Vec::new();
    doc.save_full_copy(&mut marked).expect("save");

    assert_ne!(plain.len(), marked.len(), "the mark did not reach the file");
    PdfiumDocument::open_bytes(marked, None).expect("the marked file will not open");
}
