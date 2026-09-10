//! How well does recognition read a page whose text we already know?
//!
//! Takes a **native** PDF, rasterises it, reads the image back with OCR, and
//! compares that against the text the document actually contains. The document
//! is the ground truth, which is the thing a scanned fixture can never provide:
//! on a real scan there is nothing to check the answer against except a person.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> PAGIFY_OCR_MODELS=<dir> \
//!   cargo run --release --example ocr_accuracy -- <pdf> [page]
//! ```

use std::path::PathBuf;

use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::Document;
use pdf_core::ocr::engine::OcrsRecogniser;
use pdf_core::ocr::pipeline::{read_page, Options};

/// Levenshtein distance, two rows rather than a full matrix.
fn distance(a: &[char], b: &[char]) -> usize {
    if a.is_empty() {
        return b.len();
    }
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0usize; b.len() + 1];

    for (i, ca) in a.iter().enumerate() {
        current[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            current[j + 1] = (previous[j] + cost).min(previous[j + 1] + 1).min(current[j] + 1);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[b.len()]
}

/// Collapse runs of whitespace. Line breaks differ between a content stream and
/// a detector's idea of a line, and counting those as errors would measure the
/// wrong thing.
fn flatten(text: &str) -> Vec<char> {
    text.split_whitespace().collect::<Vec<_>>().join(" ").chars().collect()
}

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: ocr_accuracy <pdf> [page]");
    let only: Option<usize> = args.next().and_then(|s| s.parse().ok());

    let dir = PathBuf::from(std::env::var("PAGIFY_OCR_MODELS").expect("PAGIFY_OCR_MODELS"));
    let recogniser =
        OcrsRecogniser::from_files(&dir.join("text-detection.rten"), &dir.join("text-recognition.rten"))
            .expect("load models");

    let doc: Box<dyn Document> =
        Box::new(PdfiumDocument::open_path(&path, None).expect("open"));

    println!("{}\n", path);
    println!("  page   truth     read    lines    dist   accuracy   skew");

    let (mut total_truth, mut total_distance) = (0usize, 0usize);
    for index in 0..doc.page_count() {
        if only.is_some_and(|p| p != index + 1) {
            continue;
        }
        let Ok(page) = doc.page(index) else { continue };

        let truth = flatten(&page.characters().map(|c| c.text).unwrap_or_default());
        if truth.is_empty() {
            continue;
        }

        let reading = read_page(&*page, &recogniser, &Options::default()).expect("read");
        let read: Vec<char> = flatten(
            &reading.words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" "),
        );

        let d = distance(&truth, &read);
        total_truth += truth.len();
        total_distance += d;

        println!(
            "  {:>4}   {:>5}   {:>6}   {:>6}   {:>5}    {:>6.2}%   {:>5.2}°",
            index + 1,
            truth.len(),
            read.len(),
            reading.lines,
            d,
            100.0 * (1.0 - d as f32 / truth.len() as f32),
            reading.skew.to_degrees(),
        );

        if only.is_some() {
            println!("\n--- the document says ---\n{}", truth.iter().collect::<String>());
            println!("\n--- recognition read ---\n{}", read.iter().collect::<String>());
        }
    }

    if total_truth > 0 {
        println!(
            "\n  {:.2}% of {total_truth} characters, across the document",
            100.0 * (1.0 - total_distance as f32 / total_truth as f32)
        );
    }
}
