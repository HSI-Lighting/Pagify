//! Can text we wrote be found again after a save, and taken back out?
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=/tmp/pdfw/bin/pdfium.dll \
//!   cargo run --example text_mark_probe -- <in.pdf> <scratch-dir>
//! ```
//!
//! Written because the app has a bug whose fix depends entirely on the answer.
//! Text placed on a page is page content, not an annotation — so once the file is
//! saved the app has no mark for it any more, and the eraser cannot touch it. A
//! clouded caption came apart: the ring erased (it *is* an annotation) and the
//! words stayed behind.
//!
//! The proposed fix is marked content: tag each text object we write with a name
//! of our own, then after any number of saves walk the page's objects, recognise
//! ours by that tag, and remove exactly those. Three things have to be true and
//! none of them is obvious:
//!
//!   1. the tag survives a save and a reopen,
//!   2. our objects can be told apart from the document's own,
//!   3. removing one takes it off the page for good, and leaves the rest alone.
//!
//! If any of them fails, text has to stop being page content and the feature is
//! built a different way — so this asks before anything is built on top.

use pdfium_render::prelude::*;
use std::os::raw::c_ulong;

/// The tag every text object the app writes carries.
const MARK_NAME: &str = "PagifyText";

/// The parameter holding the mark's own id, so one caption can be found alone.
const ID_KEY: &str = "id";

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let source = args.get(1).expect("usage: text_mark_probe <in.pdf> <scratch-dir>");
    let scratch = args.get(2).expect("usage: text_mark_probe <in.pdf> <scratch-dir>");
    let written = format!("{scratch}/marked.pdf");
    let erased = format!("{scratch}/erased.pdf");

    let lib = std::env::var("PAGIFY_PDFIUM_LIB").expect("set PAGIFY_PDFIUM_LIB");
    let pdfium = Pdfium::new(Pdfium::bind_to_library(&lib).expect("bind pdfium"));
    let bindings = pdfium.bindings();

    // ---------------------------------------------------------------- write --
    {
        let doc = pdfium.load_pdf_from_file(source, None).expect("open");
        let handle = doc.handle();

        // Safety: handles live for the block; the page is closed before it ends.
        unsafe {
            let font = bindings.FPDFText_LoadStandardFont(handle, "Helvetica");
            assert!(!font.is_null(), "Helvetica would not load");

            let page = bindings.FPDF_LoadPage(handle, 0);
            assert!(!page.is_null(), "page 0 would not load");

            // Two captions, so erasing one must leave the other. A fix that took
            // out every tagged object would pass a single-object test and then
            // wipe the page in use.
            for (id, y, words) in [(7i32, 500.0f64, "Keeper"), (9, 440.0, "Doomed")] {
                let object = bindings.FPDFPageObj_CreateTextObj(handle, font, 24.0);
                assert!(!object.is_null(), "text object was not created");

                let text: Vec<u16> = words.encode_utf16().chain(std::iter::once(0)).collect();
                assert!(bindings.FPDFText_SetText(object, text.as_ptr()) != 0);
                bindings.FPDFPageObj_SetFillColor(object, 200, 30, 30, 255);
                bindings.FPDFPageObj_Transform(object, 1.0, 0.0, 0.0, 1.0, 72.0, y);

                let mark = bindings.FPDFPageObj_AddMark(object, MARK_NAME);
                assert!(!mark.is_null(), "the mark was not added");
                let set = bindings.FPDFPageObjMark_SetIntParam(
                    handle,
                    object,
                    mark,
                    ID_KEY,
                    id,
                );
                println!("tagged {words} with id {id}: {}", set != 0);

                bindings.FPDFPage_InsertObject(page, object);
            }

            assert!(bindings.FPDFPage_GenerateContent(page) != 0, "content not regenerated");
            bindings.FPDF_ClosePage(page);
        }

        doc.save_to_file(&written).expect("save");
    }

    // -------------------------------------------------- find again and erase --
    let removed = {
        let doc = pdfium.load_pdf_from_file(&written, None).expect("reopen");
        let handle = doc.handle();
        let mut removed = 0;

        // Safety: as above.
        unsafe {
            let page = bindings.FPDF_LoadPage(handle, 0);
            assert!(!page.is_null(), "page 0 would not load");

            let total = bindings.FPDFPage_CountObjects(page);
            println!("objects on the page: {total}");

            let mut ours = 0;
            // Backwards: removing an object shifts every index after it.
            for index in (0..total).rev() {
                let object = bindings.FPDFPage_GetObject(page, index);
                if object.is_null() {
                    continue;
                }

                let Some(id) = pagify_id(bindings, object) else {
                    continue;
                };
                ours += 1;
                if id != 9 {
                    continue;
                }

                let taken = bindings.FPDFPage_RemoveObject(page, object);
                println!("removed id {id}: {}", taken != 0);
                if taken != 0 {
                    bindings.FPDFPageObj_Destroy(object);
                    removed += 1;
                }
            }

            println!("objects carrying our tag: {ours}");
            assert!(ours == 2, "the tag did not survive the save: found {ours} of 2");

            assert!(bindings.FPDFPage_GenerateContent(page) != 0, "content not regenerated");
            bindings.FPDF_ClosePage(page);
        }

        doc.save_to_file(&erased).expect("save");
        removed
    };

    // ------------------------------------------------------------ read back --
    let extracted = pdfium
        .load_pdf_from_file(&erased, None)
        .expect("reopen erased")
        .pages()
        .get(0)
        .expect("page 0")
        .text()
        .expect("text page")
        .all();

    let keeper = extracted.contains("Keeper");
    let doomed = extracted.contains("Doomed");
    println!("removed {removed} object(s)");
    println!("the one we kept is still there: {keeper}");
    println!("the one we erased is gone: {}", !doomed);

    assert!(keeper, "erasing one caption took the other with it");
    assert!(!doomed, "the erased text is still in the file");
    println!("VERDICT: marked content survives a save and can be erased by id");
}

/// The Pagify id on this object, if it is one of ours.
///
/// Safety: `object` must be a live page object.
unsafe fn pagify_id(bindings: &dyn PdfiumLibraryBindings, object: FPDF_PAGEOBJECT) -> Option<i32> {
    for slot in 0..bindings.FPDFPageObj_CountMarks(object) {
        let mark = bindings.FPDFPageObj_GetMark(object, slot as c_ulong);
        if mark.is_null() {
            continue;
        }

        let mut buffer = [0u16; 64];
        let mut length: c_ulong = 0;
        let read = bindings.FPDFPageObjMark_GetName(
            mark,
            buffer.as_mut_ptr(),
            (buffer.len() * 2) as c_ulong,
            &mut length,
        );
        if read == 0 {
            continue;
        }

        // The length is bytes and includes the terminator.
        let characters = (length as usize / 2).saturating_sub(1);
        let name = String::from_utf16_lossy(&buffer[..characters]);
        if name != MARK_NAME {
            continue;
        }

        let mut value = 0;
        if bindings.FPDFPageObjMark_GetParamIntValue(mark, ID_KEY, &mut value) != 0 {
            return Some(value);
        }
    }
    None
}
