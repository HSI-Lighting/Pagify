//! What an edit costs on a CAD set, step by step: `DWTC C3, Unit 110 -
//! Arch.Pack.pdf`, two of its six pages ~720,000 lines. A byte-safe edit saves
//! the whole document to bytes, reads them, rewrites a stream and reopens the
//! whole document (`adopt_edit`); this times each part and an edit on a light
//! page and on a heavy one. A measurement, not a check:
//!
//! `cargo test --release --test edit_cost -- --ignored --nocapture`

mod harness;
use harness::{serial, skip_without_pdfium};

use pdf_core::command::{Command, CommandHistory};
use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Document, Point};
use std::time::Instant;

const SET: &str = r"C:\Users\hsili\Desktop\DWTC C3, Unit 110 - Arch.Pack.pdf";

#[test]
#[ignore]
fn what_an_edit_costs_on_a_cad_set() {
    let Some(()) = skip_without_pdfium() else { return };
    if !std::path::Path::new(SET).is_file() {
        eprintln!("skipped: the set is not at {SET}");
        return;
    }
    let _serial = serial();

    let t = Instant::now();
    let mut doc = PdfiumDocument::open_path(SET, None).expect("open");
    eprintln!("open: {:?}", t.elapsed());
    let t = Instant::now();
    let bytes = doc.readable_bytes_for_probe().expect("save");
    eprintln!("the whole document saved to bytes: {:?} ({:.1} MB)", t.elapsed(), bytes.len() as f64 / 1e6);
    let t = Instant::now();
    let parsed = pdf_core::pdf::File::parse(&bytes).map(|_| ());
    eprintln!("read by Pagify's own reader: {:?} ({parsed:?})", t.elapsed());
    let t = Instant::now();
    let reopened = PdfiumDocument::open_bytes(bytes, None).expect("reopen");
    eprintln!("reopened: {:?}", t.elapsed());
    drop(reopened);

    // The parts of a move on the heavy page that are not PDFium's.
    let t = Instant::now();
    let stream = doc.page_stream(1).expect("the page's content");
    eprintln!("page 2: its content read and decoded: {:?} ({:.1} MB)", t.elapsed(), stream.len() as f64 / 1e6);
    let t = Instant::now();
    let operations = pdf_core::pdf::content::parse(&stream).expect("parse");
    eprintln!("page 2: {} operators parsed: {:?}", operations.len(), t.elapsed());
    let t = Instant::now();
    let states = pdf_core::pdf::content::states(&operations);
    eprintln!("page 2: their states: {:?}", t.elapsed());
    let t = Instant::now();
    let placed = pdf_core::pdf::content::placed(&operations);
    eprintln!("page 2: where text is placed: {:?} ({} show operators)", t.elapsed(), placed.len());
    drop((states, placed));
    let t = Instant::now();
    let packed = pdf_core::pdf::content::encode(&stream).expect("encode");
    eprintln!("page 2: the content compressed again: {:?} ({:.1} MB)", t.elapsed(), packed.len() as f64 / 1e6);
    let t = Instant::now();
    doc.page_scale(1).expect("scale");
    eprintln!("page 2: one load of it by PDFium: {:?}", t.elapsed());

    let mut history = CommandHistory::default();
    for page in [0usize, 1] {
        let t = Instant::now();
        let run = doc.text_runs(page).expect("runs").into_iter().next().expect("a word");
        eprintln!("page {}: its words read in {:?}", page + 1, t.elapsed());
        let t = Instant::now();
        let moved = history.execute(
            Command::MoveObject { page_index: page, object: run.object, by: Point { x: 2.0, y: 0.0 } },
            &mut doc,
        );
        eprintln!("page {}: {:?} moved in {:?} ({:?})", page + 1, run.text, t.elapsed(), moved.map(|_| ()));
        let t = Instant::now();
        let undone = history.undo(&mut doc);
        eprintln!("page {}: undone in {:?} ({:?})", page + 1, t.elapsed(), undone.map(|_| ()));
    }
}
