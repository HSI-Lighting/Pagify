//! Can we put real, selectable text onto a page?
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=/tmp/pdfw/bin/pdfium.dll \
//!   cargo run --example text_object_probe -- <in.pdf> <out.pdf>
//! ```
//!
//! Written before any of the text feature exists, because its whole shape depends
//! on the answer. Every mark the app makes today is an *annotation*; text that is
//! genuinely searchable has to be **page content** instead, which is a different
//! PDFium call with its own ways of going wrong — a standard font that will not
//! load, a matrix in the wrong space, content that is never regenerated so the
//! object is in the document but not on the page.
//!
//! So this does the smallest version end to end and then reads it back with
//! PDFium's own text extraction. If the string comes out, the feature is
//! buildable on this foundation; if it does not, better to know before the tools
//! and the toolbars are written on top of it.
//!
//! Two objects are written, one flat and one rotated, because the rotated one is
//! the question curved text actually asks: every glyph gets its own matrix.


use pdfium_render::prelude::*;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let source = args.get(1).expect("usage: text_object_probe <in.pdf> <out.pdf>");
    let target = args.get(2).expect("usage: text_object_probe <in.pdf> <out.pdf>");

    let lib = std::env::var("PAGIFY_PDFIUM_LIB").expect("set PAGIFY_PDFIUM_LIB");
    let pdfium = Pdfium::new(Pdfium::bind_to_library(&lib).expect("bind pdfium"));
    let bindings = pdfium.bindings();

    {
        let doc = pdfium.load_pdf_from_file(source, None).expect("open");
        println!("pages: {}", doc.pages().len());
        let handle = doc.handle();

        // Safety: the document and page handles are live for the whole block, and
        // the page is closed before it ends.
        unsafe {
            let font = bindings.FPDFText_LoadStandardFont(handle, "Helvetica");
            println!("Helvetica loaded: {}", !font.is_null());
            assert!(!font.is_null(), "Helvetica would not load");

            let page = bindings.FPDF_LoadPage(handle, 0);
            assert!(!page.is_null(), "page 0 would not load");

            for (index, (x, y, radians)) in
                [(72.0f32, 500.0f32, 0.0f32), (72.0, 440.0, 0.4)].iter().enumerate()
            {
                let object = bindings.FPDFPageObj_CreateTextObj(handle, font, 24.0);
                assert!(!object.is_null(), "text object {index} was not created");

                // FPDF_WIDESTRING is UTF-16, null terminated.
                let text: Vec<u16> = "Pagify text 123"
                    .encode_utf16()
                    .chain(std::iter::once(0))
                    .collect();
                let set = bindings.FPDFText_SetText(object, text.as_ptr());
                println!("object {index}: text set = {}", set != 0);

                bindings.FPDFPageObj_SetFillColor(object, 200, 30, 30, 255);

                // PDF user space has y running up from the bottom left, which is
                // the opposite of every coordinate in the app. Worth proving here
                // rather than discovering it later as upside-down text.
                let (sin, cos) = radians.sin_cos();
                bindings.FPDFPageObj_Transform(
                    object,
                    cos as f64,
                    sin as f64,
                    -sin as f64,
                    cos as f64,
                    *x as f64,
                    *y as f64,
                );

                bindings.FPDFPage_InsertObject(page, object);
            }

            // Without this the objects are in the document but not in the page's
            // content stream: present in memory, absent from the file.
            let generated = bindings.FPDFPage_GenerateContent(page);
            println!("content generated: {}", generated != 0);

            bindings.FPDF_ClosePage(page);
        }

        doc.save_to_file(target).expect("save");
        println!("saved: {target}");
    }

    // The point of the whole probe: does a reader see it as *text*?
    let reopened = pdfium.load_pdf_from_file(target, None).expect("reopen");
    let extracted = reopened
        .pages()
        .get(0)
        .expect("page 0")
        .text()
        .expect("text page")
        .all();

    let found = extracted.contains("Pagify text 123");
    println!("text extraction finds it: {found}");
    println!("--- first 400 characters of the page ---");
    println!("{}", extracted.chars().take(400).collect::<String>());

    assert!(found, "the text was written but is not extractable");
}
