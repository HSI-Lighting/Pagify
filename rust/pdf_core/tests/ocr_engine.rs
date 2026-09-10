//! The `ocrs` recogniser against a real model and a real scan.
//!
//! Needs two files this repository does not carry — a model is data, and a
//! multi-megabyte one does not belong in git. Point `PAGIFY_OCR_MODELS` at a
//! directory holding `text-detection.rten` and `text-recognition.rten` and
//! these run; without it they skip, loudly enough to notice in the output.

#![cfg(feature = "ocr-engine")]

mod harness;

use std::path::PathBuf;

use pdf_core::document::RenderRequest;
use pdf_core::ocr::engine::{OcrsRecogniser, DEFAULT_ALPHABET};
use pdf_core::ocr::{preprocess, GreyImage, LineBox, Recogniser, Script};
use pdf_core::render::{bitmap::PixelOrder, RenderTarget};

fn models() -> Option<(PathBuf, PathBuf)> {
    let dir = PathBuf::from(std::env::var("PAGIFY_OCR_MODELS").ok()?);
    let (d, r) = (dir.join("text-detection.rten"), dir.join("text-recognition.rten"));
    if d.exists() && r.exists() {
        Some((d, r))
    } else {
        eprintln!("skipping: no models under {}", dir.display());
        None
    }
}

fn engine() -> Option<OcrsRecogniser> {
    let (d, r) = models()?;
    Some(OcrsRecogniser::from_files(&d, &r).expect("load models"))
}

/// A fixture page, rendered to greyscale.
fn scan(name: &str) -> Option<GreyImage> {
    let pdfium = harness::skip_without_pdfium()?;
    let _lock = harness::serial();
    let doc = harness::open_fixture(&pdfium, name);
    let page = doc.page(0).expect("page");

    let scale = 300.0 / 72.0;
    let (w, h) = page.size().pixel_size(scale);
    let mut pixels = vec![0u8; w as usize * h as usize * 4];
    {
        let mut target = RenderTarget::new(w, h, w as usize * 4, PixelOrder::Rgba, &mut pixels)
            .expect("target");
        page.render_into(&RenderRequest { scale, ..Default::default() }, &mut target)
            .expect("render");
    }
    drop(page);

    Some(preprocess::to_grey(&pixels, w, h))
}

fn read(engine: &OcrsRecogniser, image: &GreyImage) -> String {
    engine
        .detect(image)
        .expect("detect")
        .iter()
        .filter_map(|line| engine.recognise(image, line, Script::Latin).ok())
        .map(|line| {
            line.words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// **The assumption the whole decoder rests on.**
///
/// The model's outputs are treated as log probabilities, so a confidence is
/// `exp(score)`. If they were logits instead, every confidence reported would
/// be silently wrong — plausible-looking numbers, no error, no way to notice.
/// Checked against the real model rather than asserted in a comment.
#[test]
fn model_outputs_are_log_probabilities() {
    let Some(engine) = engine() else { return };
    let Some(image) = scan("scan-300dpi.pdf") else { return };

    let lines = engine.detect(&image).expect("detect");
    assert!(!lines.is_empty(), "the detector found nothing on a clean 300 dpi scan");

    let line = engine.recognise(&image, &lines[0], Script::Latin).expect("recognise");
    let word = line.words.first().expect("no words on the first line");

    for (i, c) in word.char_confidence.iter().enumerate() {
        assert!(
            (0.0..=1.0).contains(c),
            "character {i} of {:?} scored {c}, which is not a probability — \
             the model is emitting logits, not log probabilities",
            word.text
        );
    }
    // A greedy pick is the largest of a distribution that sums to one, so it
    // cannot be vanishingly small across a whole word of real text.
    let mean = word.char_confidence.iter().sum::<f32>() / word.char_confidence.len() as f32;
    assert!(mean > 0.3, "mean confidence {mean:.3} on legible text: exp() is being applied to the wrong scale");
}

/// **The risk this backend takes, discharged.**
///
/// The decoder here is not `ocrs`'s. It exists because theirs discards the
/// per-character confidence, and the danger in that is producing different
/// text. Both are run over **the same detected lines**, so the only thing that
/// varies between them is the decoding.
#[test]
fn agrees_with_the_reference_decoder() {
    let Some(engine) = engine() else { return };
    let Some(image) = scan("scan-300dpi.pdf") else { return };

    let lines = engine.detect(&image).expect("detect");
    assert!(!lines.is_empty(), "nothing detected, so nothing is being compared");

    let theirs = engine.reference_read(&image, &lines).expect("reference decode");
    let mine: Vec<String> = lines
        .iter()
        .map(|l| {
            engine
                .recognise(&image, l, Script::Latin)
                .expect("recognise")
                .words
                .iter()
                .map(|w| w.text.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect();

    let mut disagreements = Vec::new();
    for (i, (ours, ref_)) in mine.iter().zip(&theirs).enumerate() {
        if ours.split_whitespace().ne(ref_.split_whitespace()) {
            disagreements.push(format!("  line {i}\n    ours:   {ours}\n    theirs: {ref_}"));
        }
    }
    assert!(
        disagreements.is_empty(),
        "the in-house decoder disagrees with ocrs on {} of {} lines:\n{}",
        disagreements.len(),
        mine.len(),
        disagreements.join("\n")
    );
}

#[test]
fn reads_a_clean_scan() {
    let Some(engine) = engine() else { return };
    let Some(image) = scan("scan-300dpi.pdf") else { return };

    let text = read(&engine, &image).to_uppercase();
    assert!(
        text.contains("LUMINAIRE") || text.contains("ACCEPT"),
        "nothing recognisable came back from a clean scan:\n{text}"
    );
}

/// The reason the field exists: a weak character inside a confident word.
#[test]
fn every_character_carries_its_own_score() {
    let Some(engine) = engine() else { return };
    let Some(image) = scan("scan-lowdpi.pdf") else { return };

    let lines = engine.detect(&image).expect("detect");
    let words: Vec<_> = lines
        .iter()
        .filter_map(|l| engine.recognise(&image, l, Script::Latin).ok())
        .flat_map(|l| l.words)
        .collect();
    assert!(!words.is_empty(), "no words off the 150 dpi scan");

    for w in &words {
        assert_eq!(
            w.char_confidence.len(),
            w.text.chars().count(),
            "{:?} has {} characters but {} scores",
            w.text,
            w.text.chars().count(),
            w.char_confidence.len()
        );
        // The word's score is its weakest character, which is the whole point:
        // a mean would hide the one wrong digit in a part number.
        assert_eq!(w.confidence, w.weakest_character().unwrap());
    }
}

/// A low-resolution scan should read *less* confidently, not differently.
#[test]
fn resolution_moves_confidence_not_the_verdict() {
    let Some(engine) = engine() else { return };
    let (Some(good), Some(poor)) = (scan("scan-300dpi.pdf"), scan("scan-lowdpi.pdf")) else {
        return;
    };

    let mean = |image: &GreyImage| {
        let scores: Vec<f32> = engine
            .detect(image)
            .expect("detect")
            .iter()
            .filter_map(|l| engine.recognise(image, l, Script::Latin).ok())
            .flat_map(|l| l.words)
            .flat_map(|w| w.char_confidence)
            .collect();
        scores.iter().sum::<f32>() / scores.len().max(1) as f32
    };

    let (sharp, blurred) = (mean(&good), mean(&poor));
    assert!(
        blurred < sharp,
        "150 dpi scored {blurred:.3} against 300 dpi's {sharp:.3} — confidence is not tracking legibility"
    );
}

#[test]
fn a_script_the_model_cannot_read_is_refused_rather_than_guessed() {
    let Some(engine) = engine() else { return };
    let image = GreyImage::white(200, 60);
    let line = LineBox::upright(0.0, 0.0, 200.0, 60.0, 1.0);

    let refused = engine.recognise(&image, &line, Script::Arabic);
    assert!(refused.is_err(), "the Latin model accepted Arabic instead of saying it cannot");
}

#[test]
fn the_alphabet_matches_the_class_count_the_model_emits() {
    // 96 characters, so 97 classes with CTC blank at 0. A mismatch shifts every
    // letter by one, which reads as noise rather than as an error — which is
    // why `recognise` checks this against the model at load rather than
    // trusting the constant.
    assert_eq!(DEFAULT_ALPHABET.chars().count(), 96);
}
