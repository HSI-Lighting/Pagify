//! What text marks does this file actually carry?
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=/tmp/pdfw/bin/pdfium.dll \
//!   cargo run --example text_mark_dump -- <file.pdf> [page]
//! ```
//!
//! The app writes each caption's objects with a marked-content tag, and reads
//! them back to make saved words an editable mark again. When that comes back
//! empty there are three places it can have gone — the write, the save, or the
//! read — and this separates them: it looks at the file on disk with none of the
//! app's code in the way.

use pdfium_render::prelude::*;
use std::os::raw::c_ulong;

const MARK_NAME: &str = "PagifyText";

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let file = args.get(1).expect("usage: text_mark_dump <file.pdf> [page]");
    let page_index: i32 = args.get(2).map(|p| p.parse().expect("page")).unwrap_or(0);

    let lib = std::env::var("PAGIFY_PDFIUM_LIB").expect("set PAGIFY_PDFIUM_LIB");
    let pdfium = Pdfium::new(Pdfium::bind_to_library(&lib).expect("bind pdfium"));
    let bindings = pdfium.bindings();

    let doc = pdfium.load_pdf_from_file(file, None).expect("open");
    let handle = doc.handle();

    // Safety: the page is live for the loop and closed before the end.
    unsafe {
        let page = bindings.FPDF_LoadPage(handle, page_index);
        assert!(!page.is_null(), "page {page_index} would not load");

        println!("pages in the file: {}", bindings.FPDF_GetPageCount(handle));
        let total = bindings.FPDFPage_CountObjects(page);
        println!("page {page_index}: {total} objects");

        let mut tagged = 0;
        let mut marked_at_all = 0;
        for index in 0..total {
            let object = bindings.FPDFPage_GetObject(page, index);
            if object.is_null() {
                continue;
            }
            let marks = bindings.FPDFPageObj_CountMarks(object);
            if marks > 0 {
                marked_at_all += 1;
            }
            for slot in 0..marks {
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
                let characters = (length as usize / 2).saturating_sub(1);
                let name = String::from_utf16_lossy(&buffer[..characters]);
                if name != MARK_NAME {
                    println!("  object {index}: some other tag {name:?}");
                    continue;
                }

                tagged += 1;
                let mut id = 0;
                let has_id = bindings.FPDFPageObjMark_GetParamIntValue(mark, "id", &mut id) != 0;

                let mut needed: c_ulong = 0;
                let has_blob = bindings.FPDFPageObjMark_GetParamStringValue(
                    mark,
                    "restore",
                    std::ptr::null_mut(),
                    0,
                    &mut needed,
                ) != 0;
                println!("  object {index}: ours, id={id} (read={has_id}), blob={has_blob} ({needed} bytes)");
            }
        }

        println!("objects carrying any tag: {marked_at_all}");
        println!("objects carrying ours:    {tagged}");
        bindings.FPDF_ClosePage(page);
    }
}
