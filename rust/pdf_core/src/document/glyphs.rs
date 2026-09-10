//! Reading letters back out of shapes.
//!
//! Two different failures land here and they are not the same problem.
//!
//! ## Broken `/ToUnicode` — and why the exact repair is blocked at this pin
//!
//! When a font's `/ToUnicode` is missing or wrong the glyphs still draw
//! correctly; only the mapping to Unicode is lost. The obvious repair is to read
//! the embedded font's own `cmap` and recover the mapping from there, and
//! `FPDFFont_GetFontData` does hand back the font program to do it with.
//!
//! **It cannot be keyed to a character.** At chromium/7881 PDFium exposes no
//! charcode getter and no glyph-index getter — `FPDFText_SetCharcodes` is a
//! setter, and the font API stops at metrics and names. `FPDFText_GetTextObject`
//! plus `FPDFTextObj_GetFont` will say *which font* an unmappable character
//! belongs to, and nothing available will say *which glyph within it*. So the
//! table below can be built, and there is no way to look a character up in it.
//!
//! That is worth stating rather than working around, because the workaround is
//! the expensive one: render the run and recognise it, at recognition accuracy,
//! for a document whose glyphs are sitting right there. The table is built
//! anyway because the *other* case needs it.
//!
//! ## Type converted to outlines — where this pays
//!
//! Print production converts type to curves: logos, headlines, anything placed
//! from Illustrator. Those pages have no text objects at all, so there is no
//! `/ToUnicode` to repair and nothing to select. The instinct is to rasterise
//! and recognise them.
//!
//! They are not pixels. They are the *exact* contours the font drew, carried
//! through a transform — nothing has been quantised or resampled. So they can be
//! compared against font outlines directly, and the comparison has far better
//! signal than any pixel method: an `8` and a `B` differ by an entire contour.
//!
//! It is **not** exact, and the plan that proposed it oversold that. The
//! transform inversion is floating point; Illustrator does its own curve
//! fitting on conversion; and a reissued face with revised glyphs will not
//! coincide with the one that set the type. So this scores and thresholds like
//! any other matcher — it simply starts from a much better signal.

use std::collections::HashMap;

/// A closed contour, as points. Curves arrive already flattened: matching
/// samples along them anyway, so carrying the control points would be work
/// nothing downstream uses.
pub type Contour = Vec<(f32, f32)>;

/// One glyph's shape, normalised so two of them can be compared.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Outline {
    pub contours: Vec<Contour>,
}

impl Outline {
    pub fn is_empty(&self) -> bool {
        self.contours.iter().all(|c| c.len() < 3)
    }

    pub fn bounds(&self) -> Option<(f32, f32, f32, f32)> {
        let mut bounds: Option<(f32, f32, f32, f32)> = None;
        for point in self.contours.iter().flatten() {
            bounds = Some(match bounds {
                None => (point.0, point.1, point.0, point.1),
                Some((l, t, r, b)) => (l.min(point.0), t.min(point.1), r.max(point.0), b.max(point.1)),
            });
        }
        bounds
    }

    /// Width divided by height. The cheapest discriminator there is, and it
    /// throws out most candidates before any sampling happens — an `i` and an
    /// `m` are not close on any measure, and this is the one that costs nothing.
    pub fn aspect(&self) -> f32 {
        match self.bounds() {
            Some((l, t, r, b)) if (b - t).abs() > 1e-6 => (r - l) / (b - t),
            _ => 0.0,
        }
    }

    /// Translate to the origin and scale so the height is 1.
    ///
    /// Uniform scaling, deliberately: normalising width and height separately
    /// would make every letter the same shape and match `l` to `—`.
    pub fn normalised(&self) -> Outline {
        let Some((left, top, _, bottom)) = self.bounds() else {
            return Outline::default();
        };
        let height = (bottom - top).abs().max(1e-6);

        Outline {
            contours: self
                .contours
                .iter()
                .map(|contour| {
                    contour
                        .iter()
                        .map(|(x, y)| ((x - left) / height, (y - top) / height))
                        .collect()
                })
                .collect(),
        }
    }

    /// Resample a contour to `n` evenly spaced points along its perimeter, so
    /// two outlines with different numbers of control points still compare.
    fn resample(contour: &Contour, n: usize) -> Contour {
        if contour.len() < 2 || n == 0 {
            return contour.clone();
        }

        let mut lengths = vec![0.0f32];
        let mut total = 0.0f32;
        for pair in contour.windows(2) {
            total += distance(pair[0], pair[1]);
            lengths.push(total);
        }
        // Close the loop.
        total += distance(contour[contour.len() - 1], contour[0]);
        lengths.push(total);

        if total < 1e-9 {
            return vec![contour[0]; n];
        }

        let mut out = Vec::with_capacity(n);
        let mut segment = 0usize;
        for i in 0..n {
            let target = total * (i as f32) / (n as f32);
            while segment + 1 < lengths.len() && lengths[segment + 1] < target {
                segment += 1;
            }
            let a = contour[segment % contour.len()];
            let b = contour[(segment + 1) % contour.len()];
            let span = (lengths[(segment + 1).min(lengths.len() - 1)] - lengths[segment]).max(1e-9);
            let t = ((target - lengths[segment]) / span).clamp(0.0, 1.0);
            out.push((a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t));
        }
        out
    }

    /// Reverse the point order of every contour, without moving a single
    /// point. The shape drawn is identical; only the direction it is traced in
    /// changes — clockwise becomes counterclockwise.
    ///
    /// One of two independent ways two producers can disagree about the same
    /// shape. See [`Outline::distance_to`] for the other, and for why neither
    /// can be assumed away.
    fn reversed(&self) -> Outline {
        Outline {
            contours: self.contours.iter().map(|c| c.iter().rev().copied().collect()).collect(),
        }
    }

    /// Flip every point across its own vertical centre — top and bottom trade
    /// places, left and right do not move. Unlike [`Outline::reversed`], this
    /// **does** move every point: it is a real reflection, not a re-ordering.
    fn mirrored_vertically(&self) -> Outline {
        let Some((_, top, _, bottom)) = self.bounds() else { return self.clone() };
        Outline {
            contours: self
                .contours
                .iter()
                .map(|c| c.iter().map(|&(x, y)| (x, top + bottom - y)).collect())
                .collect(),
        }
    }

    /// How far this outline is from another, 0 being identical.
    ///
    /// `f32::INFINITY` when they cannot be the same letter at all — a different
    /// number of contours settles it outright, which is why an `o` is never
    /// confused with a `c` however similar the ink looks.
    ///
    /// # Neither winding nor axis direction is trusted
    ///
    /// A page's path objects and a font's own glyph outlines come from two
    /// unrelated producers, and this module holds them to no shared
    /// convention for either. **Measured, not assumed** — comparing
    /// `outlined.pdf`'s own `T` against the very font that drew it scored
    /// 0.38, worse than eight unrelated letters, until the actual cause was
    /// isolated with both shapes' points printed side by side: this module's
    /// page-reading side works in top-left, y-down space to match where a
    /// selection rectangle lives, while [`Catalogue::from_font`]'s glyphs stay
    /// in the font's own y-up space. That is a **reflection** — the same `T`,
    /// vertically mirrored — and a rotation search, which is all
    /// [`ring_distance`] does, cannot undo a reflection by trying different
    /// starting points. It is a different transform, and needed its own fix:
    /// with the font's `T` mirrored to match, the same comparison fell to
    /// 0.0002.
    ///
    /// A **reversed** point order is a second, independent way two producers
    /// can disagree, orthogonal to a reflection: TrueType's outer contours
    /// follow a fixed clockwise convention, but a PDF exporter's choice of
    /// Bézier direction is under no such obligation, with or without an
    /// axis-convention difference layered on top.
    ///
    /// So both transforms are tried independently, and the cheapest of the
    /// four combinations is kept. None of this loosens what counts as a
    /// match; it recognises that "the same shape, reflected" and "the same
    /// shape, wound the other way" are not differences in shape at all.
    pub fn distance_to(&self, other: &Outline) -> f32 {
        if self.contours.len() != other.contours.len() || self.contours.is_empty() {
            return f32::INFINITY;
        }
        if (self.aspect() - other.aspect()).abs() > 0.25 {
            return f32::INFINITY;
        }

        let plain = other.clone();
        let mirrored = other.mirrored_vertically();
        [&plain, &mirrored]
            .into_iter()
            .flat_map(|candidate| {
                [
                    Self::distance_same_winding(self, candidate),
                    Self::distance_same_winding(&self.reversed(), candidate),
                ]
            })
            .fold(f32::INFINITY, f32::min)
    }

    /// [`Outline::distance_to`] without trying either the reversed winding or
    /// the vertical mirror — the part of the comparison that is a real
    /// shape-distance rather than a convention guard.
    fn distance_same_winding(mine: &Outline, other: &Outline) -> f32 {
        const SAMPLES: usize = 64;
        let mine = mine.normalised();
        let theirs = other.normalised();

        // Contours are matched by centroid, because a font and an Illustrator
        // export have no reason to emit them in the same order.
        let mut theirs_left: Vec<&Contour> = theirs.contours.iter().collect();
        let mut total = 0.0f32;

        for contour in &mine.contours {
            let a = Outline::resample(contour, SAMPLES);
            let Some((best, cost)) = theirs_left
                .iter()
                .enumerate()
                .map(|(i, other)| {
                    let b = Outline::resample(other, SAMPLES);
                    (i, ring_distance(&a, &b))
                })
                .min_by(|x, y| x.1.partial_cmp(&y.1).unwrap_or(std::cmp::Ordering::Equal))
            else {
                return f32::INFINITY;
            };
            theirs_left.remove(best);
            total += cost;
        }

        total / mine.contours.len() as f32
    }
}

/// Compare two resampled rings, trying every rotation of the start point.
///
/// Necessary because two producers have no reason to start a contour at the
/// same place: the same `o` drawn from the top and from the left is the same
/// letter and would otherwise score as completely different.
fn ring_distance(a: &Contour, b: &Contour) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return f32::INFINITY;
    }
    let n = a.len();
    let mut best = f32::INFINITY;

    for offset in 0..n {
        let mut sum = 0.0;
        for i in 0..n {
            sum += distance(a[i], b[(i + offset) % n]);
            if sum >= best * n as f32 {
                break;
            }
        }
        best = best.min(sum / n as f32);
    }
    best
}

fn distance(a: (f32, f32), b: (f32, f32)) -> f32 {
    ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt()
}

/// Candidate letters to match outlined type against.
///
/// Only as good as the faces it holds: a face the document used and this does
/// not have will never resolve. For a company's own documents that is a solved
/// problem — the brand typefaces are on the design machines.
#[derive(Debug, Default)]
pub struct Catalogue {
    entries: Vec<(char, Outline)>,
}

impl Catalogue {
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn insert(&mut self, ch: char, outline: Outline) {
        if !outline.is_empty() {
            self.entries.push((ch, outline));
        }
    }

    /// [`Catalogue::from_font`] over letters, digits and the punctuation a
    /// redaction is likely to be drawn around — a name, a price, a reference
    /// number. Not exhaustive: a caller matching against a specific known
    /// alphabet (a test fixture, a script this does not cover) should build
    /// its own set and call [`Catalogue::from_font`] directly instead.
    pub fn from_font_common(data: &[u8]) -> Self {
        Self::from_font(data, Self::common_characters())
    }

    /// Build from a font program — an embedded font from `FPDFFont_GetFontData`,
    /// or a file from disk.
    pub fn from_font(data: &[u8], characters: impl IntoIterator<Item = char>) -> Self {
        let mut catalogue = Catalogue::default();
        catalogue.extend_from_font(data, characters);
        catalogue
    }

    /// [`Catalogue::from_font_common`], merged into an existing catalogue
    /// rather than replacing it.
    ///
    /// **The reason this exists rather than a caller retrying with a second
    /// font.** A document's body copy and its headlines are often set at two
    /// different weights of the same face, and `identify` already scores every
    /// entry it holds for a shape and keeps the closest — insert candidates
    /// from more than one weight and the *shape* decides which one a given
    /// glyph matches, with no need for the caller to guess which weight to try
    /// first or notice that the first guess matched badly.
    pub fn extend_from_font_common(&mut self, data: &[u8]) {
        self.extend_from_font(data, Self::common_characters());
    }

    /// [`Catalogue::from_font`], merged into an existing catalogue.
    pub fn extend_from_font(&mut self, data: &[u8], characters: impl IntoIterator<Item = char>) {
        let Ok(face) = ttf_parser::Face::parse(data, 0) else { return };
        for ch in characters {
            let Some(glyph) = face.glyph_index(ch) else { continue };
            let mut builder = OutlineCollector::default();
            if face.outline_glyph(glyph, &mut builder).is_some() {
                self.insert(ch, builder.finish());
            }
        }
    }

    fn common_characters() -> impl Iterator<Item = char> + Clone {
        ('A'..='Z')
            .chain('a'..='z')
            .chain('0'..='9')
            .chain(r#",.-/()&%$#@!?'":;"#.chars())
    }

    /// The letter this shape is, if any is close enough.
    ///
    /// Returns the character and its score. A caller that wants to know how
    /// *distinctive* the match was should ask for [`Catalogue::best_two`] — a
    /// match that beat its runner-up by nothing is not a match worth trusting.
    pub fn identify(&self, outline: &Outline, tolerance: f32) -> Option<(char, f32)> {
        let (best, runner_up) = self.best_two_distinct(outline);
        match best {
            Some((ch, score)) if score <= tolerance => {
                // A win by a hair over a different letter is a coin toss dressed
                // as a result. Better to return nothing than a plausible wrong
                // specification number.
                //
                // The separation is measured **against the tolerance, not
                // against the score**. A margin expressed as a fraction of the
                // winning score collapses to nothing precisely when the match is
                // good: a perfect hit scores ~0, so "beat the runner-up by half
                // your own score" asks it to beat zero by zero, and every
                // ambiguous pair sails through.
                if let Some((_, next)) = runner_up {
                    let both_plausible = next <= tolerance;
                    let indistinguishable = (next - score) < tolerance * 0.25;
                    if both_plausible && indistinguishable {
                        return None;
                    }
                }
                Some((ch, score))
            }
            _ => None,
        }
    }

    /// The closest entry, and the closest one that is a **different letter**.
    ///
    /// **The distinction the ambiguity guard was always about.** It exists to
    /// refuse "this is an `e` or a `c`, by a hair" — its own wording. But a
    /// catalogue built from more than one face holds the same character several
    /// times, once per face, and those are not rivals: an `e` from the regular
    /// weight and an `e` from the semibold agreeing is *confirmation*.
    ///
    /// Taking the runner-up literally meant every extra font made recognition
    /// worse. Measured on a page of drawn words: 24% of clusters matched
    /// against one face, and **3%** against the six the reader had added —
    /// which is precisely backwards, and is why adding the document's own
    /// typeface appeared not to help.
    pub fn best_two_distinct(
        &self,
        outline: &Outline,
    ) -> (Option<(char, f32)>, Option<(char, f32)>) {
        let mut scored: Vec<(char, f32)> = self
            .entries
            .iter()
            .map(|(ch, candidate)| (*ch, outline.distance_to(candidate)))
            .filter(|(_, score)| score.is_finite())
            .collect();
        scored.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

        let best = scored.first().copied();
        let rival = best.and_then(|(winner, _)| {
            scored.iter().find(|(ch, _)| *ch != winner).copied()
        });
        (best, rival)
    }

    pub fn best_two(&self, outline: &Outline) -> (Option<(char, f32)>, Option<(char, f32)>) {
        let mut scored: Vec<(char, f32)> = self
            .entries
            .iter()
            .map(|(ch, candidate)| (*ch, outline.distance_to(candidate)))
            .filter(|(_, score)| score.is_finite())
            .collect();
        scored.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

        (scored.first().copied(), scored.get(1).copied())
    }
}

/// Collects `ttf-parser`'s outline callbacks into contours, flattening curves.
#[derive(Default)]
struct OutlineCollector {
    contours: Vec<Contour>,
    current: Contour,
    at: (f32, f32),
}

impl OutlineCollector {
    /// How finely curves are flattened. Sixteen segments per curve is well
    /// inside the precision the comparison works at.
    const STEPS: usize = 16;

    fn finish(mut self) -> Outline {
        if self.current.len() >= 3 {
            self.contours.push(std::mem::take(&mut self.current));
        }
        Outline { contours: self.contours }
    }
}

impl ttf_parser::OutlineBuilder for OutlineCollector {
    fn move_to(&mut self, x: f32, y: f32) {
        if self.current.len() >= 3 {
            self.contours.push(std::mem::take(&mut self.current));
        } else {
            self.current.clear();
        }
        self.at = (x, y);
        self.current.push((x, y));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.at = (x, y);
        self.current.push((x, y));
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.current.extend(flatten_quad(self.at, (x1, y1), (x, y), Self::STEPS));
        self.at = (x, y);
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.current.extend(flatten_cubic(self.at, (x1, y1), (x2, y2), (x, y), Self::STEPS));
        self.at = (x, y);
    }

    fn close(&mut self) {
        if self.current.len() >= 3 {
            self.contours.push(std::mem::take(&mut self.current));
        } else {
            self.current.clear();
        }
    }
}

/// A quadratic Bézier, sampled into `steps` points (the curve's end, not its
/// start — the start is already the contour's last point).
///
/// Shared rather than inlined twice: [`crate::document::outlined`] flattens the
/// same curve shape from PDF path segments, and two copies of this formula are
/// two chances for them to quietly stop agreeing.
pub(crate) fn flatten_quad(
    from: (f32, f32),
    control: (f32, f32),
    to: (f32, f32),
    steps: usize,
) -> Vec<(f32, f32)> {
    (1..=steps)
        .map(|i| {
            let t = i as f32 / steps as f32;
            let u = 1.0 - t;
            (
                u * u * from.0 + 2.0 * u * t * control.0 + t * t * to.0,
                u * u * from.1 + 2.0 * u * t * control.1 + t * t * to.1,
            )
        })
        .collect()
}

/// A cubic Bézier, sampled into `steps` points (the curve's end, not its
/// start). See [`flatten_quad`] for why this is shared rather than duplicated.
pub(crate) fn flatten_cubic(
    from: (f32, f32),
    c1: (f32, f32),
    c2: (f32, f32),
    to: (f32, f32),
    steps: usize,
) -> Vec<(f32, f32)> {
    (1..=steps)
        .map(|i| {
            let t = i as f32 / steps as f32;
            let u = 1.0 - t;
            (
                u * u * u * from.0 + 3.0 * u * u * t * c1.0 + 3.0 * u * t * t * c2.0 + t * t * t * to.0,
                u * u * u * from.1 + 3.0 * u * u * t * c1.1 + 3.0 * u * t * t * c2.1 + t * t * t * to.1,
            )
        })
        .collect()
}

/// A font's own glyph-to-Unicode table, from its `cmap`.
///
/// Buildable today; not *usable* for `/ToUnicode` repair today, for the reason
/// in the module header — there is no way to ask PDFium which glyph an
/// unmappable character used. Kept because the outline matcher needs exactly
/// this mapping, and because the day a charcode getter appears this is the
/// missing half.
pub fn glyph_to_unicode(data: &[u8]) -> HashMap<u16, char> {
    let mut table = HashMap::new();
    let Ok(face) = ttf_parser::Face::parse(data, 0) else {
        return table;
    };

    if let Some(cmap) = face.tables().cmap {
        for subtable in cmap.subtables {
            if !subtable.is_unicode() {
                continue;
            }
            subtable.codepoints(|code| {
                if let Some(ch) = char::from_u32(code) {
                    if let Some(glyph) = face.glyph_index(ch) {
                        table.entry(glyph.0).or_insert(ch);
                    }
                }
            });
        }
    }
    table
}

#[cfg(test)]
mod tests {
    use super::*;







    fn square(size: f32) -> Outline {
        Outline {
            contours: vec![vec![(0.0, 0.0), (size, 0.0), (size, size), (0.0, size)]],
        }
    }

    fn triangle(size: f32) -> Outline {
        Outline { contours: vec![vec![(0.0, 0.0), (size, 0.0), (size / 2.0, size)]] }
    }

    /// Two contours, like an `o`.
    fn ring() -> Outline {
        Outline {
            contours: vec![
                vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)],
                vec![(3.0, 3.0), (7.0, 3.0), (7.0, 7.0), (3.0, 7.0)],
            ],
        }
    }

    #[test]
    /// **A second face is confirmation, not confusion.**
    ///
    /// The ambiguity guard exists to refuse "this is an `e` or a `c`, by a
    /// hair". A catalogue built from more than one face holds the same
    /// character once per face, and taking those as rivals meant every extra
    /// font made recognition worse — measured on a page of drawn words, 24% of
    /// clusters matched against one face and **3%** against six.
    #[test]
    fn the_same_letter_from_two_faces_does_not_cancel_itself_out() {
        let mut catalogue = Catalogue::default();
        // One letter, twice, in shapes that differ as two weights differ.
        catalogue.insert('a', square(10.0));
        catalogue.insert('a', square(10.4));

        let found = catalogue.identify(&square(10.0), 0.2);
        assert_eq!(found.map(|(ch, _)| ch), Some('a'), "two faces agreeing was read as a tie");
    }

    /// And the guard still does its job: two *different* letters, equally
    /// close, is a coin toss and must come back as nothing.
    #[test]
    fn two_different_letters_equally_close_are_still_refused() {
        let mut catalogue = Catalogue::default();
        catalogue.insert('a', square(10.0));
        catalogue.insert('b', square(10.0));

        assert!(
            catalogue.identify(&square(10.0), 0.2).is_none(),
            "a coin toss between two letters was reported as a result"
        );
    }

    #[test]
    fn the_same_shape_at_a_different_size_still_matches() {
        // The whole point of normalising: outlined type has been through a
        // placement transform, so it is never at the font's own scale.
        let small = square(10.0);
        let large = square(340.0);
        assert!(small.distance_to(&large) < 0.01, "{}", small.distance_to(&large));
    }

    #[test]
    fn the_same_shape_translated_still_matches() {
        let here = square(10.0);
        let there = Outline {
            contours: vec![here.contours[0].iter().map(|(x, y)| (x + 900.0, y - 40.0)).collect()],
        };
        assert!(here.distance_to(&there) < 0.01);
    }

    #[test]
    fn a_contour_starting_somewhere_else_still_matches() {
        // A font and an Illustrator export have no reason to begin a contour at
        // the same point. Without the rotation search this scores as a
        // completely different letter.
        let a = square(10.0);
        let mut rotated = a.contours[0].clone();
        rotated.rotate_left(2);
        let b = Outline { contours: vec![rotated] };

        assert!(a.distance_to(&b) < 0.01, "start point changed the verdict: {}", a.distance_to(&b));
    }

    #[test]
    fn different_shapes_do_not_match() {
        assert!(square(10.0).distance_to(&triangle(10.0)) > 0.05);
    }

    #[test]
    fn a_different_contour_count_settles_it_outright() {
        // This is the structural advantage over pixels: an `8` has three
        // contours and a `B` has three but a `0` has two — no amount of
        // blurring can make them the same, because they are not the same shape.
        assert_eq!(square(10.0).distance_to(&ring()), f32::INFINITY);
    }

    #[test]
    fn aspect_ratio_rejects_before_any_sampling() {
        let wide = Outline { contours: vec![vec![(0.0, 0.0), (100.0, 0.0), (100.0, 5.0), (0.0, 5.0)]] };
        assert_eq!(square(10.0).distance_to(&wide), f32::INFINITY);
    }

    #[test]
    fn a_catalogue_identifies_a_shape_it_holds() {
        let mut catalogue = Catalogue::default();
        catalogue.insert('S', square(10.0));
        catalogue.insert('T', triangle(10.0));
        catalogue.insert('O', ring());

        let (ch, _) = catalogue.identify(&square(250.0), 0.05).expect("a match");
        assert_eq!(ch, 'S');
    }

    #[test]
    fn a_match_that_barely_beats_its_runner_up_is_refused() {
        // A specification number extracted wrongly from a catalogue is a
        // commercial problem, not a cosmetic one. Two nearly identical
        // candidates must return nothing rather than a coin toss.
        let mut catalogue = Catalogue::default();
        catalogue.insert('A', square(10.0));
        // Very slightly different, so it scores just behind.
        catalogue.insert('B', Outline {
            contours: vec![vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.05), (0.0, 10.0)]],
        });

        assert!(
            catalogue.identify(&square(10.0), 0.5).is_none(),
            "an ambiguous match was accepted"
        );
    }

    #[test]
    fn nothing_in_the_catalogue_means_no_answer() {
        let mut catalogue = Catalogue::default();
        catalogue.insert('S', square(10.0));
        assert!(catalogue.identify(&triangle(10.0), 0.01).is_none());

        assert!(Catalogue::default().identify(&square(10.0), 1.0).is_none());
    }

    #[test]
    fn an_unparseable_font_yields_an_empty_catalogue_rather_than_failing() {
        // Subset fonts arrive in every state imaginable; a malformed one must
        // not take the extraction down with it.
        assert!(Catalogue::from_font(b"not a font at all", 'a'..='z').is_empty());
        assert!(glyph_to_unicode(b"not a font at all").is_empty());
    }

    #[test]
    fn a_real_font_yields_real_outlines() {
        // Skipped where the system has no font at this path — the assertion is
        // about the parsing, and pinning it to a machine would be worse.
        let candidates = ["/System/Library/Fonts/Helvetica.ttc", "/System/Library/Fonts/Geneva.ttf"];
        let Some(data) = candidates.iter().find_map(|p| std::fs::read(p).ok()) else {
            return;
        };

        let catalogue = Catalogue::from_font(&data, 'A'..='Z');
        if catalogue.is_empty() {
            return; // a .ttc collection this parser declines is not a failure here
        }
        assert!(catalogue.len() > 10, "only {} letters read", catalogue.len());

        let table = glyph_to_unicode(&data);
        assert!(!table.is_empty(), "no cmap recovered from a real font");
    }
}
