//! Recognition on `ocrs`, run through `rten`.
//!
//! Two models, both pure Rust to load and run: a detector that finds words
//! anywhere on the page, and a recogniser that reads one line at a time. That
//! split is the [`Recogniser`] trait's, and it is `ocrs`'s too, which is most
//! of why this backend was chosen at M0.
//!
//! # Why this does not simply call `OcrEngine::recognize_text`
//!
//! Because that function throws away the number this crate cares most about.
//! `ocrs` returns [`TextChar`](ocrs::TextChar) as `{ char, rect }` — no
//! confidence, at any level: not per character, not per word, not per line. Its
//! CTC decoder does compute one, and discards it before the public API.
//!
//! [`RecognisedWord::char_confidence`] is not decoration. Once recognition has
//! run and the per-character numbers are gone, the only way back is to run
//! recognition again over the whole document — which is why that field's
//! doc comment says it is never discarded. A backend that returned nothing for
//! it would make every review tool built on it permanently blind, and `IP65`
//! read as `IP66` would have nothing to flag it by.
//!
//! So the line image is prepared with `ocrs`'s own public
//! [`prepare_recognition_input`](ocrs::OcrEngine::prepare_recognition_input) —
//! the geometry stays theirs, which is the part worth borrowing — and then the
//! recognition model is run here and decoded here, in one pass that yields the
//! characters and their confidences together. Aligned by construction, because
//! they come out of the same argmax.
//!
//! The risk in that is decoding differently from `ocrs` and producing text
//! theirs would not. That is paid down in `agrees_with_the_reference_decoder`,
//! which runs both over the same fixture and compares the strings — a test
//! cost, not a runtime one.

use std::path::Path;
use std::sync::Arc;

use ocrs::{ImageSource, OcrEngine, OcrEngineParams};
use rten::Model;
use rten_imageproc::{Point, RotatedRect};
use rten_tensor::prelude::*;
use rten_tensor::NdTensor;

use super::{GreyImage, LineBox, RecognisedLine, RecognisedWord, Recogniser, Script};
use crate::document::layout::Direction;
use crate::document::Rect;
use crate::error::{PdfError, Result};

/// The label set the stock recognition model was trained on.
///
/// Copied from `ocrs`, where it is private. It is **data about the model file**
/// rather than a choice made here: index `i` of this string is training label
/// `i + 1`, and label `0` is CTC blank. A model trained on a different alphabet
/// needs the matching string passed to [`OcrsRecogniser::with_alphabet`], and
/// the length check in `recognise` is what catches the mismatch rather than
/// letting it come back as plausible wrong letters.
pub const DEFAULT_ALPHABET: &str =
    " 0123456789!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~EABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

/// Recognition backed by the `ocrs` models.
pub struct OcrsRecogniser {
    /// Detection and preprocessing. Owns its own copy of the recognition model
    /// so `recognize_text` stays available to the reference test.
    engine: OcrEngine,
    /// The same recognition model, loaded again and driven directly.
    recognition: Arc<Model>,
    alphabet: Vec<char>,
    scripts: Vec<Script>,
}

impl std::fmt::Debug for OcrsRecogniser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OcrsRecogniser")
            .field("alphabet", &self.alphabet.len())
            .finish_non_exhaustive()
    }
}

fn model_error(what: &str, why: impl std::fmt::Display) -> PdfError {
    PdfError::InvalidArgument(format!("ocr: {what}: {why}"))
}

impl OcrsRecogniser {
    /// Load both models from files.
    ///
    /// Deliberately takes paths rather than bytes. Model files are tens of
    /// megabytes; reading them into a `Vec` to hand over doubles the peak of a
    /// process that the M0 spike already measured at 482 MB.
    pub fn from_files(detection: &Path, recognition: &Path) -> Result<Self> {
        let detection_model =
            Model::load_file(detection).map_err(|e| model_error("detection model", e))?;
        let recognition_model =
            Model::load_file(recognition).map_err(|e| model_error("recognition model", e))?;
        // Loaded twice on purpose: `OcrEngineParams` takes ownership and never
        // gives it back, and the direct pass needs its own handle. Weights are
        // memory-mapped by `rten`, so the second load costs address space
        // rather than another copy of the file.
        let direct =
            Model::load_file(recognition).map_err(|e| model_error("recognition model", e))?;

        let engine = OcrEngine::new(OcrEngineParams {
            detection_model: Some(detection_model),
            recognition_model: Some(recognition_model),
            ..Default::default()
        })
        .map_err(|e| model_error("engine", e))?;

        Ok(OcrsRecogniser {
            engine,
            recognition: Arc::new(direct),
            alphabet: DEFAULT_ALPHABET.chars().collect(),
            scripts: vec![Script::Latin],
        })
    }

    /// Use a different label set, for a model that was not trained on the
    /// stock one.
    pub fn with_alphabet(mut self, alphabet: &str) -> Self {
        self.alphabet = alphabet.chars().collect();
        self
    }

    /// What `ocrs`'s own decoder makes of the **same lines**, for comparison.
    ///
    /// Not part of the [`Recogniser`] contract and not used in the pipeline —
    /// it returns text with no confidences, which is the very thing this
    /// backend exists to avoid. It is here so `agrees_with_the_reference_decoder`
    /// can hold the in-house decoder to the reference's output on a real page.
    /// A wrapper that quietly decodes differently from the library it wraps is
    /// the failure mode worth a public method to rule out.
    ///
    /// It takes the line boxes rather than finding its own, and recognises one
    /// line per call. Both of those matter: `OcrEngine::get_text` would run its
    /// own detection and batch the lines to a common width, so a disagreement
    /// could come from either of those instead of from the decoding, and the
    /// comparison would prove nothing.
    pub fn reference_read(&self, image: &GreyImage, lines: &[LineBox]) -> Result<Vec<String>> {
        let input = self.prepare(image)?;
        let mut out = Vec::with_capacity(lines.len());
        for line in lines {
            let recognised = self
                .engine
                .recognize_text(&input, &[vec![to_rotated(line)]])
                .map_err(|e| model_error("reference", e))?;
            out.push(
                recognised
                    .into_iter()
                    .flatten()
                    .map(|l| l.to_string())
                    .collect::<Vec<_>>()
                    .join(" "),
            );
        }
        Ok(out)
    }

    fn prepare<'a>(&self, image: &'a GreyImage) -> Result<ocrs::OcrInput> {
        let source = ImageSource::from_bytes(&image.data, (image.width, image.height))
            .map_err(|e| model_error("image", e))?;
        self.engine.prepare_input(source).map_err(|e| model_error("prepare", e))
    }
}

/// The quad as `ocrs` wants it.
///
/// Its rotated rect is a centre, an axis and a size, so the conversion goes
/// through the quad's own edges rather than its bounding box — otherwise a
/// skewed line comes back upright and the deskew that `preprocess` does is
/// undone at the last step.
fn to_rotated(line: &LineBox) -> RotatedRect {
    let [tl, tr, br, bl] = line.quad;
    let centre = Point::from_yx(
        (tl.1 + tr.1 + br.1 + bl.1) / 4.0,
        (tl.0 + tr.0 + br.0 + bl.0) / 4.0,
    );
    let along = (tr.0 - tl.0, tr.1 - tl.1);
    let width = (along.0 * along.0 + along.1 * along.1).sqrt();
    let height = {
        let d = (bl.0 - tl.0, bl.1 - tl.1);
        (d.0 * d.0 + d.1 * d.1).sqrt()
    };
    // `up_axis` points from the baseline towards the top of the text, which is
    // the quad's left edge reversed.
    let up = rten_imageproc::Vec2::from_yx(tl.1 - bl.1, tl.0 - bl.0);
    RotatedRect::new(centre, up.normalized(), width.max(1.0), height.max(1.0))
}

/// One character the decoder committed to.
struct Step {
    label: u32,
    /// Timestep it was emitted at, which is what places it along the line.
    pos: usize,
    /// Probability of that label at that timestep, 0–1.
    confidence: f32,
}

/// Greedy CTC: argmax per timestep, drop blanks, collapse runs.
///
/// `logits` is (timesteps, classes) of **log** probabilities — `ocrs`'s own
/// decoder treats them that way, and `probabilities_are_log_probabilities`
/// checks the assumption against the real model rather than trusting this
/// comment.
///
/// A collapsed run keeps its **strongest** timestep, not its first. The two
/// differ where a letter straddles a frame boundary, and the strongest one is
/// both the better confidence and the better position.
fn decode_greedy(logits: &NdTensor<f32, 2>) -> Vec<Step> {
    let [timesteps, classes] = logits.shape();
    let mut out: Vec<Step> = Vec::new();
    let mut previous = 0u32;

    for t in 0..timesteps {
        let mut best = 0usize;
        let mut best_score = f32::NEG_INFINITY;
        for c in 0..classes {
            let score = logits[[t, c]];
            if score > best_score {
                best_score = score;
                best = c;
            }
        }

        let label = best as u32;
        let confidence = best_score.exp().clamp(0.0, 1.0);

        // Blank, or the same label repeated: CTC emits nothing new.
        if label == 0 {
            previous = 0;
            continue;
        }
        if label == previous {
            if let Some(last) = out.last_mut() {
                if confidence > last.confidence {
                    last.confidence = confidence;
                    last.pos = t;
                }
            }
            continue;
        }

        previous = label;
        out.push(Step { label, pos: t, confidence });
    }

    out
}

impl Recogniser for OcrsRecogniser {
    fn detect(&self, image: &GreyImage) -> Result<Vec<LineBox>> {
        if image.is_empty() {
            return Ok(Vec::new());
        }
        let input = self.prepare(image)?;
        let words = self.engine.detect_words(&input).map_err(|e| model_error("detect", e))?;

        // Words into lines here rather than downstream: grouping is where the
        // detector's own layout analysis lives, and a caller reassembling lines
        // from loose word boxes would be guessing at what this already knows.
        Ok(self
            .engine
            .find_text_lines(&input, &words)
            .into_iter()
            .filter_map(|group| {
                let points: Vec<Point<f32>> =
                    group.iter().flat_map(|r| r.corners()).collect();
                if points.is_empty() {
                    return None;
                }
                let rect = rten_imageproc::min_area_rect(&points)?
                    .orient_towards(rten_imageproc::Vec2::from_yx(-1.0, 0.0));
                let c = rect.corners();
                Some(LineBox {
                    quad: [
                        (c[0].x, c[0].y),
                        (c[1].x, c[1].y),
                        (c[2].x, c[2].y),
                        (c[3].x, c[3].y),
                    ],
                    // The detector reports no score of its own. Left at 1.0
                    // rather than invented: this number means "the detector did
                    // not object", and the confidence that decides anything is
                    // the per-character one from recognition.
                    confidence: 1.0,
                })
            })
            .collect())
    }

    fn recognise(
        &self,
        image: &GreyImage,
        line: &LineBox,
        script: Script,
    ) -> Result<RecognisedLine> {
        let empty = RecognisedLine { words: Vec::new(), script, direction: Direction::Ltr };
        if image.is_empty() {
            return Ok(empty);
        }
        if !self.scripts.contains(&script) {
            return Err(PdfError::InvalidArgument(format!(
                "ocr: no recogniser for {script:?} — the stock model reads Latin only"
            )));
        }

        let input = self.prepare(image)?;
        let rect = to_rotated(line);
        let prepared = self
            .engine
            .prepare_recognition_input(&input, &[rect])
            .map_err(|e| model_error("line", e))?;

        let (height, width) = (prepared.size(0), prepared.size(1));
        if height == 0 || width == 0 {
            return Ok(empty);
        }

        let batched = prepared.into_shape([1, 1, height, width]);
        let input_id = self
            .recognition
            .input_ids()
            .first()
            .copied()
            .ok_or_else(|| model_error("recognition model", "no input"))?;
        let output_id = self
            .recognition
            .output_ids()
            .first()
            .copied()
            .ok_or_else(|| model_error("recognition model", "no output"))?;

        let outputs = self
            .recognition
            .run_n(vec![(input_id, (&batched).into())], [output_id], None)
            .map_err(|e| model_error("run", e))?;
        let mut sequence: NdTensor<f32, 3> = outputs
            .into_iter()
            .next()
            .ok_or_else(|| model_error("run", "no output tensor"))?
            .try_into()
            .map_err(|_| model_error("run", "output was not a 3d float tensor"))?;
        // The model emits (sequence, batch, class); everything below reads it
        // as (batch, sequence, class).
        sequence.permute([1, 0, 2]);

        if sequence.size(2) != self.alphabet.len() + 1 {
            return Err(PdfError::InvalidArgument(format!(
                "ocr: model emits {} classes but the alphabet has {} characters — \
                 they must match, or every letter comes back shifted",
                sequence.size(2),
                self.alphabet.len()
            )));
        }

        let logits: NdTensor<f32, 2> = sequence.slice(0).to_tensor();
        let steps = decode_greedy(&logits);
        if steps.is_empty() {
            return Ok(empty);
        }

        // Timestep to position along the line.
        //
        // Not a simple share of the box. The recognition input is scaled to the
        // model's fixed height and then **padded** on the right to a batch
        // width, so the line's real content occupies only the first part of the
        // sequence. Spreading the characters evenly across the whole box
        // therefore drags every one of them leftwards, and lets the model's
        // guesses inside the padding through as real letters — which is exactly
        // the extra `o` on `formed fo` that the reference comparison caught.
        //
        // The arithmetic is `ocrs`'s own, for the same reason the preprocessing
        // is: it is the half of this that has to agree with them.
        let (left, top, right, bottom) = line.bounds();
        let line_width = (right - left).max(1.0);
        let line_height = (bottom - top).max(1.0);

        // What the line was scaled to before padding: its width at the model's
        // input height, **capped at the width actually prepared**.
        //
        // The cap is not defensive tidying. A long line's width at the model's
        // input height can exceed the recogniser's maximum, and it is then
        // squeezed to fit rather than being allowed to run off the end. Without
        // the cap this divides by a width the image never had: measured on the
        // 300 dpi fixture, 3009 against a real 2400, which drags every
        // character box 20% leftwards and lets a letter decoded in the padding
        // count as real. Both symptoms, one cause.
        let resized_width =
            (line_width * (height as f32 / line_height)).round().clamp(1.0, width as f32);
        // How much the model downsamples the width. A whole factor, or close to
        // it when the width is not an exact multiple.
        let downsample = (width as f32 / logits.size(0).max(1) as f32).round().max(1.0);
        let x_scale = line_width / resized_width;
        let at = |pos: usize| left + (pos as f32 * downsample) * x_scale;

        // Characters that start beyond the line's right edge were decoded in
        // the padding. Dropped rather than clamped: they are not badly placed
        // letters, they are letters that are not there.
        let steps: Vec<&Step> = steps.iter().filter(|s| at(s.pos) < right).collect();
        if steps.is_empty() {
            return Ok(empty);
        }

        let mut words: Vec<RecognisedWord> = Vec::new();
        let mut current: Vec<&Step> = Vec::new();

        // Words split on the space character, the same rule `ocrs` uses. Done
        // here because the space's own timestep is what separates the boxes,
        // and it is gone by the time a string is handed over.
        let flush = |chars: &mut Vec<&Step>, words: &mut Vec<RecognisedWord>| {
            if chars.is_empty() {
                return;
            }
            let text: String = chars
                .iter()
                .map(|s| self.alphabet.get((s.label - 1) as usize).copied().unwrap_or('?'))
                .collect();
            let char_confidence: Vec<f32> = chars.iter().map(|s| s.confidence).collect();
            // The word's own score is its **weakest** character, not the mean.
            // A mean of twelve good characters and one bad one still reads as a
            // good word, which is exactly the part number this is meant to
            // catch.
            let confidence =
                char_confidence.iter().copied().fold(1.0f32, f32::min).clamp(0.0, 1.0);

            let start = at(chars.first().map(|s| s.pos).unwrap_or(0));
            let end = at(chars.last().map(|s| s.pos).unwrap_or(0) + 1).min(right);

            words.push(RecognisedWord {
                text,
                rect: Rect { left: start, top, right: end.max(start + 1.0), bottom },
                confidence,
                char_confidence,
            });
            chars.clear();
        };

        for step in steps {
            let ch = self.alphabet.get((step.label - 1) as usize).copied().unwrap_or('?');
            if ch == ' ' {
                flush(&mut current, &mut words);
            } else {
                current.push(step);
            }
        }
        flush(&mut current, &mut words);

        Ok(RecognisedLine { words, script, direction: Direction::Ltr })
    }

    fn supported_scripts(&self) -> &[Script] {
        &self.scripts
    }
}
