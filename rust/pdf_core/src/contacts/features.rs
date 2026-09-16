//! What a line looks like, as numbers a classifier can learn from.
//!
//! # Why this exists in Rust rather than in the training script
//!
//! **This is the most likely silent failure in the whole classifier plan.**
//! Features computed twice — once in Python to build the training set, once
//! here to run the model — drift apart on a different median, a different
//! normalisation or a different hash seed. Accuracy collapses and nothing
//! errors, because both halves are individually correct. The failure has no
//! symptom except a model that is quietly worse than the rules it replaced.
//!
//! So there is one implementation, and it is this one. Training reads its
//! feature matrix from a Rust binary that calls [`extract`]; inference calls
//! [`extract`]. Drift is then impossible rather than unlikely.
//!
//! # The version is part of the contract
//!
//! [`VERSION`] is bumped whenever the meaning or the order of the vector
//! changes, and it is written beside any model trained against it. A model
//! loaded against a different version must be refused rather than run: the
//! numbers would still be numbers, the inference would still succeed, and the
//! answers would be nonsense.
//!
//! # What is deliberately absent
//!
//! No word lists. The rules this is meant to replace already fail on trade
//! words — `Lighting` is not a legal suffix, so a lockup reading
//! `HSI LIGHTING` passes every company test there is — and adding industries to
//! a list is the brittle approach the plan warns against. The lexical signal
//! here is character n-grams, which learn that distinction from data instead of
//! being told it.

/// The shape and meaning of a feature vector. Bump it when either changes.
///
/// Written beside a trained model. Loading a model whose recorded version
/// differs from this must fail loudly — see the module note.
pub const VERSION: u32 = 2;

/// How many buckets the character n-grams are hashed into.
///
/// 2^12. Large enough that unrelated n-grams rarely collide, small enough that
/// the model stays well under the megabyte the plan budgets for it.
pub const GRAM_BUCKETS: usize = 4096;

/// How many features come before the hashed n-grams.
pub const DENSE: usize = 18;

/// The full width of one line's feature vector.
pub const WIDTH: usize = DENSE + GRAM_BUCKETS;

/// How many geometric features come first, and are absent from a text-only row.
pub const GEOMETRY: usize = 13;

/// The width of a text-only vector: character class, then the n-grams.
///
/// **The training data has no geometry and cannot be given any.** A company
/// register and a corpus of personal names are lists of strings; they were
/// never on a card, so they have no box, no neighbours and no position. There
/// are only two honest responses to that, and inventing plausible geometry for
/// them is not one of them.
///
/// So the lexical model is trained and run on *this* narrower vector, and the
/// geometry stays where it already works — in the heuristics. That is not a
/// compromise forced by the data; it is §11.4's requirement arriving as a
/// structural fact. The two paths are independent precisely because one of them
/// cannot see position and the other cannot see language.
pub const LEXICAL_WIDTH: usize = (DENSE - GEOMETRY) + GRAM_BUCKETS;

/// One line, and where it sits among the others on its card.
///
/// Deliberately not `parse::Line` — that type is private to the parser and
/// carries what the parser needs. This carries what a classifier needs, so the
/// two can move independently, and so a training set can be built from lines
/// that never came from a card at all.
#[derive(Debug, Clone)]
pub struct LineView<'a> {
    pub text: &'a str,
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
    /// Median glyph height of the line's own boxes — the proxy for font size.
    pub glyph_height: f32,
}

/// The card the lines were read from, for the normalisations.
#[derive(Debug, Clone, Copy)]
pub struct CardView {
    pub width: f32,
    pub height: f32,
}

/// Every line of one card, as feature vectors in the order given.
///
/// Whole-card at once rather than line-by-line, because half the geometry is
/// relative: a line's height means nothing until it is divided by the median,
/// and "how far from the bottom" cannot be known from the line alone.
pub fn extract(lines: &[LineView<'_>], card: CardView) -> Vec<Vec<f32>> {
    let median = median_glyph_height(lines);
    (0..lines.len())
        .map(|index| extract_one(lines, index, card, median))
        .collect()
}

/// One line's vector. Public so a training binary can emit a single row.
pub fn extract_one(
    lines: &[LineView<'_>],
    index: usize,
    card: CardView,
    median_height: f32,
) -> Vec<f32> {
    let line = &lines[index];
    let mut features = Vec::with_capacity(WIDTH);

    // ---- geometry -------------------------------------------------------
    // Every one of these is a ratio, so a card photographed close up and the
    // same card across a desk give the same numbers.
    let width = card.width.max(1.0);
    let height = card.height.max(1.0);
    let median = median_height.max(1.0);

    features.push(line.glyph_height / median);
    features.push(((line.top + line.bottom) / 2.0) / height);
    features.push(line.left / width);
    features.push(line.right / width);
    features.push((line.right - line.left) / width);
    features.push((line.bottom - line.top) / height);

    // Alignment, as three flags rather than one number: a card's columns are
    // left-aligned, right-aligned or centred, and those are categories rather
    // than points on a scale.
    let left_edge = line.left / width;
    let right_edge = 1.0 - (line.right / width);
    features.push(flag(left_edge < 0.08));
    features.push(flag(right_edge < 0.08));
    features.push(flag((left_edge - right_edge).abs() < 0.06));

    // Distance to the neighbours, in line heights. A title sits tight under its
    // name; an address block is evenly spaced.
    features.push(gap_before(lines, index) / median);
    features.push(gap_after(lines, index) / median);

    // Where in the card, counted both ways. The name is near the top on most
    // cards and last on some, and "third from the end" is a different fact from
    // "third from the start".
    let count = lines.len().max(1) as f32;
    features.push(index as f32 / count);
    features.push((count - 1.0 - index as f32) / count);

    debug_assert_eq!(features.len(), GEOMETRY, "GEOMETRY does not match what is pushed");

    // Everything from here down is what a bare string also has, so it comes
    // from the one function the lexical model is trained on. Calling it rather
    // than repeating it is the whole anti-drift argument in miniature: there is
    // no second copy to diverge.
    features.extend(extract_lexical(line.text));

    debug_assert_eq!(features.len(), WIDTH);
    features
}

/// Everything about a line that a bare string also has: character class, then
/// the hashed n-grams.
///
/// **This is what the lexical classifier is trained and run on.** A row of
/// training data is a name from a register or a person-name corpus — a string
/// with no card behind it — and this is the most that can honestly be computed
/// from one. Inference calls the same function, so a model cannot be fed
/// differently-shaped numbers than it learned from.
pub fn extract_lexical(text: &str) -> Vec<f32> {
    let mut features = Vec::with_capacity(LEXICAL_WIDTH);

    // ---- character class ------------------------------------------------
    let chars = text.chars().count().max(1) as f32;
    features.push(text.chars().filter(char::is_ascii_digit).count() as f32 / chars);
    features.push(
        text.chars().filter(|c| c.is_ascii_punctuation()).count() as f32 / chars,
    );
    // Both scaled into 0..1 rather than pushed raw. A word count of 4 and a
    // character count of 40 sit beside n-gram values around 0.1, and a
    // multinomial model reads all of them as counts of the same kind — so a raw
    // length feature silently becomes almost the whole model, and what gets
    // learned is how long the string is. Found by a test that expected a name
    // it had been trained on and got the other class.
    features.push((text.split_whitespace().count() as f32 / 10.0).min(1.0));
    features.push((chars / 60.0).min(1.0));
    features.push(capitalisation(text));

    // ---- lexical --------------------------------------------------------
    // The one channel the heuristics cannot see, and so the only axis on which
    // the two paths are genuinely independent, per §11.4.
    let mut buckets = vec![0.0f32; GRAM_BUCKETS];
    let grams = character_grams(text);
    let scale = 1.0 / (grams.len().max(1) as f32).sqrt();
    for gram in &grams {
        buckets[bucket_of(gram)] += scale;
    }
    features.extend(buckets);

    debug_assert_eq!(features.len(), LEXICAL_WIDTH);
    features
}

/// The n-grams of a line, as the lexical channel sees it.
///
/// Case is folded and spaces become `^`, so a word boundary is itself a
/// character the model can learn from — `^wu$` is a short surname, `^wu` inside
/// a longer run is not. Public because a training set is built from the same
/// function, and because it is the piece most worth testing directly.
pub fn character_grams(text: &str) -> Vec<String> {
    let folded = format!("^{}$", text.to_lowercase().replace(' ', "^"));
    let chars: Vec<char> = folded.chars().collect();
    let mut grams = Vec::new();
    for size in 2..=4 {
        if chars.len() < size {
            continue;
        }
        for window in chars.windows(size) {
            grams.push(window.iter().collect());
        }
    }
    grams
}

/// Which bucket an n-gram hashes into.
///
/// `DefaultHasher` is not stable across Rust releases, so this is written out:
/// a model trained today must bucket identically on a toolchain from next year,
/// or every feature silently moves.
fn bucket_of(gram: &str) -> usize {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in gram.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    (hash % GRAM_BUCKETS as u64) as usize
}

/// How the line is capitalised: 0 lower, 0.5 title, 1 upper.
///
/// A number rather than three flags because these do sit on a scale — a lockup
/// shouts, a name is titled, a stray fragment is neither.
fn capitalisation(text: &str) -> f32 {
    let letters: Vec<char> = text.chars().filter(|c| c.is_alphabetic()).collect();
    if letters.is_empty() {
        return 0.0;
    }
    let upper = letters.iter().filter(|c| c.is_uppercase()).count() as f32;
    let ratio = upper / letters.len() as f32;
    if ratio > 0.9 {
        1.0
    } else if ratio > 0.1 {
        0.5
    } else {
        0.0
    }
}

fn median_glyph_height(lines: &[LineView<'_>]) -> f32 {
    if lines.is_empty() {
        return 1.0;
    }
    let mut heights: Vec<f32> = lines.iter().map(|l| l.glyph_height).collect();
    heights.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    heights[heights.len() / 2]
}

fn gap_before(lines: &[LineView<'_>], index: usize) -> f32 {
    if index == 0 {
        return 0.0;
    }
    (lines[index].top - lines[index - 1].bottom).max(0.0)
}

fn gap_after(lines: &[LineView<'_>], index: usize) -> f32 {
    if index + 1 >= lines.len() {
        return 0.0;
    }
    (lines[index + 1].top - lines[index].bottom).max(0.0)
}

fn flag(yes: bool) -> f32 {
    if yes {
        1.0
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str, top: f32, height: f32) -> LineView<'_> {
        LineView {
            text,
            left: 100.0,
            right: 100.0 + text.len() as f32 * 10.0,
            top,
            bottom: top + height,
            glyph_height: height,
        }
    }

    fn card() -> CardView {
        CardView { width: 1000.0, height: 600.0 }
    }

    /// The width is fixed and matches what the constants promise.
    ///
    /// A vector one element short of what a model expects does not fail — it
    /// shifts every feature by one and produces confident nonsense.
    #[test]
    fn every_vector_is_exactly_the_promised_width() {
        let lines = vec![line("Sam Reyes", 60.0, 30.0), line("Northwind Ltd", 110.0, 18.0)];
        for vector in extract(&lines, card()) {
            assert_eq!(vector.len(), WIDTH);
            assert_eq!(WIDTH, DENSE + GRAM_BUCKETS);
        }
    }

    /// **The anti-drift property, asserted.** The same input gives the same
    /// numbers, every time and in any order of iteration.
    ///
    /// The hash is written out by hand rather than taken from `DefaultHasher`
    /// precisely so this holds across toolchains; this test would not catch a
    /// toolchain change on its own, but it catches the far likelier mistake of
    /// someone reaching for a `HashMap` and letting iteration order in.
    #[test]
    fn extraction_is_deterministic() {
        let lines = vec![
            line("Priya Raman", 60.0, 34.0),
            line("Head of Purchasing", 110.0, 20.0),
            line("priya@northwind.example", 300.0, 16.0),
        ];
        let once = extract(&lines, card());
        for _ in 0..8 {
            assert_eq!(extract(&lines, card()), once);
        }
    }

    /// The bucket for an n-gram is a fixed number, not whatever the standard
    /// library hashes to this year.
    ///
    /// These values are pinned deliberately. If they change, every model
    /// trained before the change is invalid, and `VERSION` must be bumped —
    /// which is the whole reason the constant exists.
    #[test]
    fn gram_buckets_are_pinned() {
        assert_eq!(bucket_of("^s"), bucket_of("^s"));
        assert!(bucket_of("^sam") < GRAM_BUCKETS);
        // A change here is a breaking change to every trained model, and so
        // requires a VERSION bump. It is written out rather than computed so
        // that a silent change to the hash cannot pass unnoticed.
        assert_eq!(bucket_of("^wu$"), 1379);
    }

    /// Geometry is expressed in ratios, so the same card at two sizes agrees.
    ///
    /// This is the pose-stability property from the real-card tests, at the
    /// feature level: a classifier that learned absolute pixels would score the
    /// same card differently depending on how close the camera was.
    #[test]
    fn a_card_photographed_larger_gives_the_same_geometry() {
        let small = vec![line("Sam Reyes", 60.0, 30.0), line("Northwind Ltd", 110.0, 18.0)];
        let small_card = CardView { width: 1000.0, height: 600.0 };

        let scaled: Vec<LineView> = small
            .iter()
            .map(|l| LineView {
                text: l.text,
                left: l.left * 3.0,
                right: l.right * 3.0,
                top: l.top * 3.0,
                bottom: l.bottom * 3.0,
                glyph_height: l.glyph_height * 3.0,
            })
            .collect();
        let big_card = CardView { width: 3000.0, height: 1800.0 };

        let a = extract(&small, small_card);
        let b = extract(&scaled, big_card);

        for (row_a, row_b) in a.iter().zip(b.iter()) {
            // Every dense feature is either a ratio of the geometry or derived
            // from the text, and the text does not change with the camera.
            for i in 0..DENSE {
                assert!(
                    (row_a[i] - row_b[i]).abs() < 1e-4,
                    "feature {i} moved when the card was photographed larger: \
                     {} against {}",
                    row_a[i],
                    row_b[i],
                );
            }
        }
    }

    /// A word boundary is a character the model can learn from.
    #[test]
    fn grams_carry_the_word_boundaries() {
        let grams = character_grams("Sam Wu");
        assert!(grams.contains(&"^s".to_string()), "the opening boundary is missing");
        assert!(grams.contains(&"wu$".to_string()), "the closing boundary is missing");
        assert!(grams.contains(&"m^w".to_string()), "the space became no boundary at all");
    }

    /// The lexical block is normalised, so a long address does not simply
    /// outweigh a short name by having more n-grams in it.
    #[test]
    fn the_lexical_block_does_not_grow_with_the_line() {
        let short = extract(&[line("Wu", 0.0, 20.0)], card());
        let long = extract(
            &[line("Add: Room 501, Innovation Industrial Park, Nanshan", 0.0, 20.0)],
            card(),
        );

        let magnitude = |v: &Vec<f32>| v[DENSE..].iter().map(|x| x * x).sum::<f32>().sqrt();
        let (a, b) = (magnitude(&short[0]), magnitude(&long[0]));
        assert!(
            (a - b).abs() < 0.35,
            "the lexical block scales with length: {a} against {b}",
        );
    }

    /// Capitalisation is the three cases a card actually prints.
    #[test]
    fn capitalisation_separates_a_lockup_from_a_name() {
        assert_eq!(capitalisation("HSI LIGHTING"), 1.0);
        assert_eq!(capitalisation("Michael Peng"), 0.5);
        assert_eq!(capitalisation("www.example.com"), 0.0);
    }

    /// **The full vector ends with exactly the text-only vector.**
    ///
    /// The lexical model is trained on strings that were never on a card and
    /// run on lines that were. If those two paths ever computed the tail
    /// differently, the model would be fed numbers it had never learned from
    /// and nothing would report an error — the failure Part 13 names as the
    /// most likely in the project. One function, asserted.
    #[test]
    fn a_lines_vector_ends_with_what_its_text_alone_would_give() {
        let lines = vec![line("Michael Peng", 60.0, 34.0), line("HSI LIGHTING", 110.0, 48.0)];
        let full = extract(&lines, card());

        for (row, view) in full.iter().zip(lines.iter()) {
            assert_eq!(
                &row[GEOMETRY..],
                extract_lexical(view.text).as_slice(),
                "the tail of the full vector is not the text-only vector for {:?}",
                view.text,
            );
        }
    }

    /// A bare string yields a vector of the promised narrower width.
    #[test]
    fn a_string_with_no_card_behind_it_still_yields_features() {
        assert_eq!(extract_lexical("Northwind Traders Ltd").len(), LEXICAL_WIDTH);
        assert_eq!(extract_lexical("Priya Raman").len(), LEXICAL_WIDTH);
        assert_eq!(LEXICAL_WIDTH + GEOMETRY, WIDTH);
    }
}
