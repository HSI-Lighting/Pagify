//! Can PDFium save incrementally through this binding, given the flag?
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=/tmp/pdfw/bin/pdfium.dll \
//!   cargo run --example incremental_probe -- <pdf>
//! ```
//!
//! `save_probe` shows the safe API rewrites the file. This one answers the
//! follow-on question that decides how much Phase A costs: is incremental save
//! *available* and merely unexposed, or absent?
//!
//! It goes straight to `FPDF_SaveWithVersion` with `FPDF_INCREMENTAL`, supplying
//! the `FPDF_FILEWRITE` callback the safe path builds internally. If the output
//! keeps the original bytes as an exact prefix, the capability is there and the
//! only thing missing is a parameter — a small patch to the binding rather than a
//! change of engine.
use pdfium_render::prelude::*;
use std::os::raw::{c_int, c_ulong, c_void};

/// Our writer, laid out so a `*mut FPDF_FILEWRITE` can be cast back to it.
///
/// PDFium hands the callback a pointer to the struct it was given, so the
/// interface header must come first and the payload after it.
#[repr(C)]
struct Sink {
    base: FPDF_FILEWRITE,
    bytes: *mut Vec<u8>,
}

unsafe extern "C" fn write_block(
    this: *mut FPDF_FILEWRITE,
    data: *const c_void,
    size: c_ulong,
) -> c_int {
    let sink = this as *mut Sink;
    let out = &mut *(*sink).bytes;
    out.extend_from_slice(std::slice::from_raw_parts(data as *const u8, size as usize));
    1
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).expect("usage: incremental_probe <pdf>");

    let lib = std::env::var("PAGIFY_PDFIUM_LIB").expect("set PAGIFY_PDFIUM_LIB");
    let bindings = Pdfium::bind_to_library(&lib).expect("bind pdfium");

    let original = std::fs::read(path).expect("read input");
    println!("input  : {} bytes", original.len());

    unsafe {
        bindings.FPDF_InitLibrary();

        let doc = bindings.FPDF_LoadMemDocument64(&original, None);
        assert!(!doc.is_null(), "could not load the document");

        for (label, flags) in [("FPDF_INCREMENTAL", 1u32), ("flags = 0 (what the safe API sends)", 0u32)] {
            let mut bytes: Vec<u8> = Vec::new();
            let mut sink = Sink {
                base: FPDF_FILEWRITE {
                    version: 1,
                    WriteBlock: Some(write_block),
                },
                bytes: &mut bytes,
            };

            let ok = bindings.FPDF_SaveWithVersion(
                doc,
                &mut sink as *mut Sink as *mut FPDF_FILEWRITE,
                flags,
                17,
            );

            let prefix_kept =
                bytes.len() >= original.len() && bytes[..original.len()] == original[..];
            let eofs = bytes.windows(5).filter(|w| *w == b"%%EOF").count();

            println!(
                "\n{label}\n  saved ok            : {}\n  output              : {} bytes ({:+})\n  original is a prefix: {}\n  %%EOF markers       : {}\n  verdict             : {}",
                ok != 0,
                bytes.len(),
                bytes.len() as i64 - original.len() as i64,
                prefix_kept,
                eofs,
                if prefix_kept { "APPENDED — signature byte ranges survive" } else { "REWRITTEN — signatures break" },
            );
        }

        bindings.FPDF_CloseDocument(doc);
        bindings.FPDF_DestroyLibrary();
    }
}
