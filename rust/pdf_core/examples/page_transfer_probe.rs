//! Do pages survive being moved between documents?
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=/tmp/pdfw/bin/pdfium.dll \
//!   cargo run --example page_transfer_probe -- <scratch-dir>
//! ```
//!
//! Export takes chosen pages out into a new file; import brings another file's
//! pages into this one. Both are `FPDF_ImportPagesByIndex`, and the question is
//! what it takes *with* the page. A page is not just its content stream — it has
//! annotations, fonts and images in a resource dictionary, and a size — and an
//! import that quietly drops any of those produces a file that opens fine and is
//! missing exactly the thing somebody wanted to keep.
//!
//! Four things are asked, because each fails differently and silently:
//!
//!   1. do the right *number* of pages arrive, in the order asked for,
//!   2. does each page keep its own size — a mixed-size document must not have
//!      everything squashed to the first page's box,
//!   3. does the page's text come with it — the fonts live in a resource
//!      dictionary that has to be copied too,
//!   4. do our own marked-content tags survive, since that is what makes saved
//!      text an editable mark and an erasable one.
//!
//! Number four is the one most likely to fail. Nothing in PDFium's contract
//! promises that a private marked-content tag survives a cross-document import.

use pdf_core::document::pdfium_doc::pdfium;
use pdfium_render::prelude::*;
use std::os::raw::c_ulong;

/// The tag every text object the app writes carries.
const MARK_NAME: &str = "PagifyText";

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let scratch = args
        .get(1)
        .expect("usage: page_transfer_probe <scratch-dir>");
    std::env::var("PAGIFY_PDFIUM_LIB").expect("set PAGIFY_PDFIUM_LIB");

    let pdfium = pdfium().expect("pdfium");
    let bindings = pdfium.bindings();

    // The fixture is built here rather than taken from whatever file is lying
    // around. The first run of this probe used a blank notebook and passed with
    // "pages that kept all their text: 0" — the two assertions that matter had
    // nothing to check. A fixture has to *contain* the thing being tested.
    let fixture = format!("{scratch}/transfer-source.pdf");
    build_fixture(&fixture);
    let original = pdfium.load_pdf_from_file(&fixture, None).expect("open source");
    let total = original.pages().len() as usize;
    println!("source: {total} pages");
    assert!(total >= 3, "this probe needs a source of at least three pages");

    // Deliberately out of order and non-contiguous: "pages 3, 1 and 2" is a
    // thing a reader can ask for, and an implementation that quietly sorts the
    // list gives them the wrong document without saying so.
    let wanted: Vec<i32> = vec![2, 0, 1];
    let before: Vec<PageFacts> = wanted
        .iter()
        .map(|&index| facts(bindings, &original, index))
        .collect();
    for (slot, facts) in before.iter().enumerate() {
        println!(
            "  wanted[{slot}] = source page {}: {:.0}x{:.0}pt, {} objects, \
             {} chars, {} of ours tagged",
            wanted[slot],
            facts.width,
            facts.height,
            facts.objects,
            facts.characters,
            facts.tagged,
        );
    }

    // ------------------------------------------------------------- export --
    let exported = format!("{scratch}/exported.pdf");
    {
        let fresh = pdfium.create_new_pdf().expect("new document");

        // Safety: both documents are live for the call, and the index list is
        // valid for its stated length.
        let ok = unsafe {
            bindings.FPDF_ImportPagesByIndex(
                fresh.handle(),
                original.handle(),
                wanted.as_ptr(),
                wanted.len() as c_ulong,
                0,
            )
        };
        assert!(ok != 0, "the pages were refused");
        fresh.save_to_file(&exported).expect("save");
    }

    let reopened = pdfium.load_pdf_from_file(&exported, None).expect("reopen");
    let arrived = reopened.pages().len() as usize;
    println!("\nexported: {arrived} pages");
    assert_eq!(arrived, wanted.len(), "wrong number of pages arrived");

    let mut kept_text = 0;
    let mut kept_tags = 0;
    for slot in 0..arrived {
        let after = facts(bindings, &reopened, slot as i32);
        let want = &before[slot];
        println!(
            "  page {slot}: {:.0}x{:.0}pt, {} objects, {} chars, {} of ours tagged",
            after.width, after.height, after.objects, after.characters, after.tagged,
        );

        // Size, per page. A document of mixed sizes is the case that catches an
        // import that applies the first page's box to everything.
        assert!(
            (after.width - want.width).abs() < 1.0 && (after.height - want.height).abs() < 1.0,
            "page {slot} changed size: wanted {:.0}x{:.0}, got {:.0}x{:.0}",
            want.width,
            want.height,
            after.width,
            after.height,
        );

        if want.characters > 0 && after.characters == want.characters {
            kept_text += 1;
        }
        if want.tagged > 0 && after.tagged == want.tagged {
            kept_tags += 1;
        }
        assert!(
            after.characters >= want.characters,
            "page {slot} lost text: {} chars became {}",
            want.characters,
            after.characters,
        );
        assert!(
            after.tagged >= want.tagged,
            "page {slot} lost {} of our marked text objects",
            want.tagged - after.tagged,
        );
    }
    println!("pages that kept all their text: {kept_text}");
    println!("pages that kept all our tags:   {kept_tags}");
    // The counts, not just the per-page comparisons. Every page of the fixture
    // has words and a tag, so anything less than all of them means a page came
    // across hollow — and the per-page assertions above would not catch it if
    // the fixture itself were empty.
    assert_eq!(kept_text, arrived, "some pages arrived without their text");
    assert_eq!(kept_tags, arrived, "some pages arrived without our tags");

    // ------------------------------------------------------------- import --
    // The other direction, and the one that has to be undoable: pages arrive in
    // the middle of a document that already has its own.
    let combined = format!("{scratch}/imported.pdf");
    {
        let host = pdfium.load_pdf_from_file(&fixture, None).expect("open host");
        let at = 1;

        // Safety: as above.
        let ok = unsafe {
            bindings.FPDF_ImportPagesByIndex(
                host.handle(),
                reopened.handle(),
                std::ptr::null(),
                0,
                at,
            )
        };
        assert!(ok != 0, "the import was refused");
        let after = host.pages().len() as usize;
        println!("\nimported all {arrived} pages at index {at}: {total} -> {after} pages");
        assert_eq!(
            after,
            total + arrived,
            "a null index list should mean every page",
        );
        host.save_to_file(&combined).expect("save");
    }

    // And the inverse an undo has to perform: take those pages back out.
    {
        let host = pdfium.load_pdf_from_file(&combined, None).expect("reopen combined");
        // Backwards: deleting shifts every index after it.
        // Safety: the document is live and every index is inside it.
        unsafe {
            for index in (1..1 + arrived).rev() {
                bindings.FPDFPage_Delete(host.handle(), index as i32);
            }
        }
        let left = host.pages().len() as usize;
        println!("undo removed them again: {left} pages");
        assert_eq!(left, total, "undoing the import did not restore the document");
    }

    println!("\nVERDICT: pages transfer with their size, text and our tags intact");
}

/// What a page is, in the numbers that would show it had been damaged.
struct PageFacts {
    width: f32,
    height: f32,
    objects: i32,
    characters: usize,
    tagged: i32,
}

fn facts(bindings: &dyn PdfiumLibraryBindings, doc: &PdfDocument, index: i32) -> PageFacts {
    let characters = doc
        .pages()
        .get(index)
        .ok()
        .and_then(|page| page.text().ok().map(|t| t.all()))
        .map(|text| text.chars().filter(|c| !c.is_whitespace()).count())
        .unwrap_or(0);

    // Safety: the page is loaded here and closed before returning.
    unsafe {
        let page = bindings.FPDF_LoadPage(doc.handle(), index);
        assert!(!page.is_null(), "page {index} would not load");
        let facts = PageFacts {
            width: bindings.FPDF_GetPageWidthF(page),
            height: bindings.FPDF_GetPageHeightF(page),
            objects: bindings.FPDFPage_CountObjects(page),
            characters,
            tagged: tagged_objects(bindings, page),
        };
        bindings.FPDF_ClosePage(page);
        facts
    }
}

/// How many objects on this page carry our own marked-content tag.
///
/// Safety: `page` must be live.
unsafe fn tagged_objects(bindings: &dyn PdfiumLibraryBindings, page: FPDF_PAGE) -> i32 {
    let mut found = 0;
    for index in 0..bindings.FPDFPage_CountObjects(page) {
        let object = bindings.FPDFPage_GetObject(page, index);
        if object.is_null() {
            continue;
        }
        for slot in 0..bindings.FPDFPageObj_CountMarks(object) {
            let mark = bindings.FPDFPageObj_GetMark(object, slot as c_ulong);
            if mark.is_null() {
                continue;
            }
            let mut buffer = [0u16; 64];
            let mut length: c_ulong = 0;
            if bindings.FPDFPageObjMark_GetName(
                mark,
                buffer.as_mut_ptr(),
                (buffer.len() * 2) as c_ulong,
                &mut length,
            ) == 0
            {
                continue;
            }
            // The length is in bytes and includes the terminator.
            let characters = (length as usize / 2).saturating_sub(1);
            if String::from_utf16_lossy(&buffer[..characters]) == MARK_NAME {
                found += 1;
                break;
            }
        }
    }
    found
}

/// Build a document that actually contains what the probe claims to check:
/// three pages of different sizes, each with real text, each carrying one of our
/// marked-content tags.
///
/// Different sizes because a mixed-size document is what catches an import that
/// applies the first page's box to everything. Tagged text because that is what
/// makes saved words an editable mark, and nothing in PDFium's contract promises
/// a private tag survives a cross-document import.
fn build_fixture(path: &str) {
    let pdfium = pdfium().expect("pdfium");
    let bindings = pdfium.bindings();
    let document = pdfium.create_new_pdf().expect("new document");
    let handle = document.handle();

    // A4, landscape A4, and a square: three visibly different boxes.
    let sizes = [(595.0, 842.0), (842.0, 595.0), (500.0, 500.0)];

    // Safety: every handle is checked, and each page is closed before the next.
    unsafe {
        let font = bindings.FPDFText_LoadStandardFont(handle, "Helvetica");
        assert!(!font.is_null(), "Helvetica would not load");

        for (index, (width, height)) in sizes.iter().enumerate() {
            let page = bindings.FPDFPage_New(handle, index as i32, *width, *height);
            assert!(!page.is_null(), "fixture page {index} was not made");

            let object = bindings.FPDFPageObj_CreateTextObj(handle, font, 24.0);
            assert!(!object.is_null());
            let words = format!("Page {}", index + 1);
            let encoded: Vec<u16> = words.encode_utf16().chain(std::iter::once(0)).collect();
            assert!(bindings.FPDFText_SetText(object, encoded.as_ptr()) != 0);
            bindings.FPDFPageObj_SetFillColor(object, 20, 20, 20, 255);
            bindings.FPDFPageObj_Transform(object, 1.0, 0.0, 0.0, 1.0, 60.0, *height - 100.0);

            let mark = bindings.FPDFPageObj_AddMark(object, MARK_NAME);
            assert!(!mark.is_null(), "the tag was not added");
            bindings.FPDFPageObjMark_SetIntParam(handle, object, mark, "id", index as i32);

            bindings.FPDFPage_InsertObject(page, object);
            assert!(bindings.FPDFPage_GenerateContent(page) != 0);
            bindings.FPDF_ClosePage(page);
        }
    }

    document.save_to_file(path).expect("save the fixture");
}
