//! What actually corrupts page geometry when two documents live at once?
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=/tmp/pdfw/bin/pdfium.dll \
//!   cargo run --example handle_race_probe
//! ```
//!
//! `handle_reuse_probe` ruled out the first theory: PDFium hands the same address
//! to document after document, and each one still reports its own page sizes. So
//! stale cache entries do not simply outlive a close.
//!
//! That leaves overlap. `PdfPageIndexCache` is keyed on `(FPDF_DOCUMENT, FPDF_PAGE)`
//! raw addresses and maps *back* from `(document, index)` to a page handle. If one
//! thread is still tearing a document down while another opens a new one at the same
//! address, the two share a key space for a moment — and a lookup can be answered
//! with the other document's page. That would explain the shape of the original
//! failure exactly: `[200, 612, 612, 612]`, one correct width followed by pages that
//! belong to something else.
//!
//! This drives that overlap on purpose. It reports how often a document is asked for
//! its own page sizes and answers with someone else's.
use pdfium_render::prelude::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

static WRONG: AtomicUsize = AtomicUsize::new(0);
static CHECKED: AtomicUsize = AtomicUsize::new(0);
/// Opens that failed outright, which the first run showed to be the larger symptom.
static FAILED_OPENS: AtomicUsize = AtomicUsize::new(0);

fn widths(doc: &PdfDocument) -> Vec<i32> {
    (0..doc.pages().len())
        .filter_map(|i| doc.pages().get(i).ok())
        .map(|p| p.width().value.round() as i32)
        .collect()
}

/// `cargo run --example handle_race_probe -- serialised`
fn serialised() -> bool {
    std::env::args().any(|a| a == "serialised")
}

fn main() {
    let lib = std::env::var("PAGIFY_PDFIUM_LIB").expect("set PAGIFY_PDFIUM_LIB");
    let pdfium = Arc::new(Pdfium::new(
        Pdfium::bind_to_library(&lib).expect("bind pdfium"),
    ));

    let fixtures = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures");
    let ladder = format!("{fixtures}/pages-ladder.pdf");
    let mixed = format!("{fixtures}/mixed-sizes.pdf");

    let truth_ladder = vec![200, 250, 300, 350, 400];
    let truth_mixed = vec![595, 420, 612, 842];

    // Two threads, two different files, both cycling open -> read -> close as fast
    // as they can. Different page sizes are what makes a mix-up visible at all.
    let handles: Vec<_> = [
        (ladder, truth_ladder, "ladder"),
        (mixed, truth_mixed, "mixed"),
    ]
    .into_iter()
    .map(|(path, truth, label)| {
        let pdfium = Arc::clone(&pdfium);
        std::thread::spawn(move || {
            for round in 0..400 {
                let _guard =
                    serialised().then(|| LIFECYCLE.lock().unwrap_or_else(|p| p.into_inner()));
                let Ok(doc) = pdfium.load_pdf_from_file(&path, None) else {
                    FAILED_OPENS.fetch_add(1, Ordering::Relaxed);
                    continue;
                };
                let observed = widths(&doc);
                CHECKED.fetch_add(1, Ordering::Relaxed);
                if observed != truth {
                    let count = WRONG.fetch_add(1, Ordering::Relaxed);
                    if count < 6 {
                        println!(
                            "{label} round {round}: expected {truth:?}, got {observed:?} \
                             (handle {:#x})",
                            doc.handle() as usize,
                        );
                    }
                }
            }
        })
    })
    .collect();

    for handle in handles {
        handle.join().expect("thread");
    }

    let wrong = WRONG.load(Ordering::Relaxed);
    let checked = CHECKED.load(Ordering::Relaxed);
    let failed = FAILED_OPENS.load(Ordering::Relaxed);
    println!("\nserialised   : {}", serialised());
    println!("opens failed : {failed}");
    println!("wrong reads  : {wrong} of {checked} that succeeded");
    if wrong == 0 && failed == 0 {
        println!("every document reported its own pages, and nothing failed to open");
    }
}

/// Held across a whole open -> read -> close cycle when the probe is run as
/// `cargo run --example handle_race_probe -- serialised`.
///
/// The point is to test the *proposed fix* rather than assume it: if one lock
/// around the document lifecycle removes both the failed opens and the missing
/// pages, then serialising lifecycle in the registry is the answer, and the
/// per-call locking that `thread_safe` already does is not enough on its own.
static LIFECYCLE: std::sync::Mutex<()> = std::sync::Mutex::new(());
