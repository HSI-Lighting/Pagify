//! Where a value sits, as bytes FF1 can take as a tweak.
//!
//! # Why this exists as its own thing
//!
//! FF1 takes a tweak by design, and using it is what stops the same value
//! encrypting identically everywhere in a document. Without one, a reader who
//! never decrypts anything still learns which entries in a catalogue share a
//! part number — which is most of what the catalogue was hiding.
//!
//! The tweak is **not secret**. It is a domain separator: it must differ
//! between fields and it must be reproducible, because decrypting needs the
//! same one that encrypted. So it is derived from where the value lives rather
//! than stored anywhere.
//!
//! # What "where it lives" has to mean
//!
//! Page index and page-object index, plus a kind. Deliberately **not** the
//! value itself — a tweak derived from the plaintext cannot be recomputed at
//! decryption time, when the plaintext is what you are trying to get back.
//!
//! The consequence to be honest about: moving or re-ordering the objects on a
//! page changes their indices, and an obfuscated value whose object index has
//! changed will not decrypt. That is a real constraint on when obfuscation may
//! be applied — after the page's content is settled, not before — and it
//! belongs in the caller's design rather than being papered over here.

use serde::{Deserialize, Serialize};

use crate::error::Result;

/// Where a value was read from.
///
/// Part of the tweak so that two fields at the same place but read differently
/// — a part number and the quantity beside it — do not share one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    /// A run of text on the page.
    TextRun,
    /// A form field's value.
    FormField,
    /// An annotation's contents.
    Annotation,
}

impl Source {
    fn tag(self) -> u8 {
        match self {
            Source::TextRun => 1,
            Source::FormField => 2,
            Source::Annotation => 3,
        }
    }
}

/// What sort of value this is, and how it must behave.
///
/// # Why `deterministic` is here and not a global setting
///
/// A tweak that includes the object index means the same part number in two
/// places encrypts to **two different values**. That is the strongest answer to
/// frequency analysis, and it is fatal to the use this mode exists for: a
/// partner whose pipeline matches a catalogue row against their own system
/// cannot join on a value that changes every time it appears.
///
/// Neither answer is right for every field, so it is declared per kind:
///
/// | | Referential integrity | Frequency analysis |
/// |---|---|---|
/// | `deterministic: false` | broken — equal inputs differ | strongest |
/// | `deterministic: true` | joins work | a reader learns a value recurs, and how often |
///
/// A part number probably wants `true` — joining is the entire point. A price
/// probably wants `false`, because nobody joins on a price and repetition leaks
/// a great deal.
///
/// **The question to ask is whether the partner needs to *parse* the value or
/// *join* on it.** Parsing needs only the shape and gets the strong answer for
/// free; joining is a deliberate, documented weakening.
///
/// Adding this flag now is small. Adding it once obfuscated documents exist is
/// a format break, because the tweak that encrypted a value is the tweak that
/// has to decrypt it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldKind {
    /// Names this kind in the envelope — `"partNumber"`, `"price"`.
    pub id: String,
    pub source: Source,
    /// Whether equal values encrypt equally.
    pub deterministic: bool,
}

impl FieldKind {
    /// A kind whose equal values must match after obfuscation, so a partner can
    /// still join on them. **The weaker of the two**, deliberately.
    pub fn joinable(id: &str, source: Source) -> Self {
        FieldKind { id: id.to_string(), source, deterministic: true }
    }

    /// A kind where repetition must not show. The default, and the stronger.
    pub fn opaque(id: &str, source: Source) -> Self {
        FieldKind { id: id.to_string(), source, deterministic: false }
    }
}

/// The tweak for a field at a place.
///
/// Fixed-width and order-explicit rather than a formatted string: a tweak built
/// by concatenating decimal text would give page 1 object 23 and page 12 object 3
/// the same bytes.
///
/// **A deterministic kind omits the page and the object**, which is exactly what
/// makes its equal values encrypt equally — and exactly what a partner needs to
/// join on. Everything else keeps them.
pub fn for_field(document_id: &[u8], page: usize, object: usize, kind: &FieldKind) -> Vec<u8> {
    let mut out = Vec::with_capacity(document_id.len() + kind.id.len() + 20);
    out.extend_from_slice(document_id);
    out.push(0);
    out.extend_from_slice(kind.id.as_bytes());
    out.push(0);
    out.push(kind.source.tag());

    if !kind.deterministic {
        out.extend_from_slice(&(page as u64).to_be_bytes());
        out.extend_from_slice(&(object as u64).to_be_bytes());
    }
    out
}

/// Whether a replacement can actually be drawn.
///
/// Three answers, not two, and the third is the important one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Coverage {
    /// Every character will draw.
    Covered,
    /// These will not.
    Missing(Vec<char>),
    /// **Cannot be determined from here.**
    ///
    /// [`crate::text::covers`] answers for fonts *the app has registered*. A
    /// PDF's own font is embedded in the document and usually subset — it
    /// carries only the glyphs the file already used — and this module has no
    /// way to open it.
    ///
    /// The plan says `pagify_font_covers` already answers this question. It
    /// answers a **different** one, and the difference matters: for a document's
    /// own font it returns `false` for everything, so treating it as the check
    /// would refuse every field, and treating a `false` as "unknown" would
    /// commit obfuscations that render as blanks with the original gone.
    Unknown { font: String },
}

impl Coverage {
    /// Whether it is safe to commit.
    ///
    /// **`Unknown` is not safe.** Obfuscation destroys the original value, so an
    /// unanswerable coverage question has to stop the write — the failure mode
    /// is a document that is both unreadable and unrecoverable.
    pub fn is_safe(&self) -> bool {
        matches!(self, Coverage::Covered)
    }
}

/// Can the font this field is drawn in draw the replacement?
///
/// **Checked before committing, not after.** A value obfuscated into characters
/// the font never contained renders as blanks or tofu, and the original is
/// already gone.
pub fn coverage(font: &str, replacement: &str) -> Coverage {
    if !crate::text::is_registered(font) {
        return Coverage::Unknown { font: font.to_string() };
    }
    let missing: Vec<char> = replacement
        .chars()
        .filter(|c| !crate::text::covers(font, &c.to_string()))
        .collect();
    if missing.is_empty() {
        Coverage::Covered
    } else {
        Coverage::Missing(missing)
    }
}

/// The same question, asked of the font a run is **actually drawn in**.
///
/// [`coverage`] answers for fonts the app registered, which a document's own
/// never is. This takes the embedded font's bytes — see
/// [`crate::document::Document::run_font_data`] — and reads them directly.
///
/// # The self-check, and why it is not optional
///
/// A subset font often carries no usable Unicode `cmap`: the PDF's own
/// `/Encoding` does the mapping, and the font's internal table maps nothing.
/// Asking such a face for `glyph_index('4')` gets `None` for every character,
/// **including the ones it is visibly drawing at that moment**.
///
/// Measured on the HSI catalogue, three pages, by
/// `examples/font_coverage_probe.rs`:
///
/// ```text
///   page 21   65 of 118 runs answer usably   (55%)
///   page 41  209 of 279                      (75%)
///   page 61  359 of 439                      (82%)
/// ```
///
/// Reading the other 18–45% as "cannot draw it" would refuse a great deal of
/// exactly the document this feature exists for, and — worse — would mean the
/// check was answering about a table nobody uses, which gives no confidence in
/// its yeses either.
///
/// So the face is first asked about the text it is drawing **right now**. A
/// face that cannot account for that is being read through the wrong mapping,
/// and the honest answer about anything else is `Unknown` — which
/// [`check_coverage_of_font_data`] then refuses, per the rule that obfuscation
/// destroys the original and an unanswerable question has to stop the write.
pub fn coverage_of_font_data(data: &[u8], drawn_now: &str, replacement: &str) -> Coverage {
    let Ok(face) = ttf_parser::Face::parse(data, 0) else {
        return Coverage::Unknown { font: "(unreadable embedded font)".into() };
    };
    let name = face
        .names()
        .into_iter()
        .find(|n| n.name_id == ttf_parser::name_id::FULL_NAME)
        .and_then(|n| n.to_string())
        .unwrap_or_else(|| "(embedded font)".into());

    // Whitespace is positioning rather than a glyph in many subset fonts, so
    // it is not evidence either way.
    let mut own = drawn_now.chars().filter(|c| !c.is_whitespace()).peekable();
    if own.peek().is_none() || !own.all(|c| face.glyph_index(c).is_some()) {
        return Coverage::Unknown { font: name };
    }

    let missing: Vec<char> = replacement
        .chars()
        .filter(|c| !c.is_whitespace() && face.glyph_index(*c).is_none())
        .collect();
    if missing.is_empty() {
        Coverage::Covered
    } else {
        Coverage::Missing(missing)
    }
}

/// Refuse a replacement that cannot be shown to draw.
pub fn check_coverage(font: &str, replacement: &str) -> Result<()> {
    refuse_unless_covered(coverage(font, replacement))
}

/// [`check_coverage`] against a document's own font.
pub fn check_coverage_of_font_data(
    data: &[u8],
    drawn_now: &str,
    replacement: &str,
) -> Result<()> {
    refuse_unless_covered(coverage_of_font_data(data, drawn_now, replacement))
}

fn refuse_unless_covered(coverage: Coverage) -> Result<()> {
    match coverage {
        Coverage::Covered => Ok(()),
        Coverage::Missing(_) => Err(crate::error::PdfError::Unsupported(
            "this field's font cannot draw the obfuscated value",
        )),
        Coverage::Unknown { .. } => Err(crate::error::PdfError::Unsupported(
            "this field's font cannot be inspected, so the obfuscated value \
             cannot be shown to render",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opaque() -> FieldKind {
        FieldKind::opaque("price", Source::TextRun)
    }

    #[test]
    fn two_fields_on_different_pages_get_different_tweaks() {
        assert_ne!(
            for_field(b"doc", 1, 7, &opaque()),
            for_field(b"doc", 2, 7, &opaque())
        );
    }

    #[test]
    fn two_fields_on_one_page_get_different_tweaks() {
        assert_ne!(
            for_field(b"doc", 1, 7, &opaque()),
            for_field(b"doc", 1, 8, &opaque())
        );
    }

    /// A part number and the price beside it must not share a tweak.
    #[test]
    fn two_kinds_at_the_same_place_get_two_tweaks() {
        assert_ne!(
            for_field(b"doc", 1, 7, &FieldKind::opaque("price", Source::TextRun)),
            for_field(b"doc", 1, 7, &FieldKind::opaque("partNumber", Source::TextRun))
        );
    }

    /// The same place read two ways — a text run and a form field over it — is
    /// two fields.
    #[test]
    fn two_sources_at_the_same_place_get_two_tweaks() {
        assert_ne!(
            for_field(b"doc", 1, 7, &FieldKind::opaque("v", Source::TextRun)),
            for_field(b"doc", 1, 7, &FieldKind::opaque("v", Source::FormField))
        );
    }

    /// Decryption recomputes the tweak; it is never stored. So it has to come
    /// out the same every time.
    #[test]
    fn the_same_place_always_gets_the_same_tweak() {
        assert_eq!(
            for_field(b"doc", 3, 11, &opaque()),
            for_field(b"doc", 3, 11, &opaque())
        );
    }

    /// **The ambiguity a formatted string would introduce.** Page 1 object 23
    /// and page 12 object 3 must not collide, which decimal concatenation would
    /// let them do.
    #[test]
    fn page_and_object_cannot_run_together() {
        assert_ne!(for_field(b"", 1, 23, &opaque()), for_field(b"", 12, 3, &opaque()));
    }

    #[test]
    fn the_document_separates_two_otherwise_identical_fields() {
        assert_ne!(
            for_field(b"catalogue", 1, 7, &opaque()),
            for_field(b"extract", 1, 7, &opaque())
        );
    }

    // -- the join question -------------------------------------------------

    /// **The whole reason `deterministic` exists.** A partner matching a
    /// catalogue row against their own system cannot join on a value that
    /// changes every time it appears.
    #[test]
    fn a_joinable_kind_gets_one_tweak_wherever_it_appears() {
        let kind = FieldKind::joinable("partNumber", Source::TextRun);
        assert_eq!(
            for_field(b"doc", 1, 7, &kind),
            for_field(b"doc", 40, 912, &kind),
            "a joinable field's tweak still depends on where it sits, so joins would fail"
        );
    }

    /// And the default does not, which is the stronger answer.
    #[test]
    fn an_opaque_kind_gets_a_different_tweak_everywhere() {
        assert_ne!(
            for_field(b"doc", 1, 7, &opaque()),
            for_field(b"doc", 40, 912, &opaque())
        );
    }

    /// Joinable is a deliberate weakening, not a default anyone falls into.
    #[test]
    fn the_stronger_answer_is_the_one_you_get_by_default() {
        assert!(!FieldKind::opaque("x", Source::TextRun).deterministic);
        assert!(FieldKind::joinable("x", Source::TextRun).deterministic);
    }

    /// Two kinds that differ only in this flag must not share a tweak — one is
    /// a weakening of the other and they cannot be confused.
    #[test]
    fn determinism_is_part_of_what_separates_two_kinds() {
        let joinable = FieldKind::joinable("partNumber", Source::TextRun);
        let opaque = FieldKind::opaque("partNumber", Source::TextRun);
        assert_ne!(for_field(b"doc", 1, 7, &joinable), for_field(b"doc", 1, 7, &opaque));
    }

    // -- glyph coverage ----------------------------------------------------

    /// **The gap the plan assumed away.**
    ///
    /// `text::covers` answers for fonts the app has *registered*. A PDF's own
    /// font is embedded in the document and this module cannot open it — so the
    /// honest answer is "unknown", not "no".
    #[test]
    fn a_font_that_cannot_be_inspected_answers_unknown_rather_than_no() {
        match coverage("Helvetica", "4821000734") {
            Coverage::Unknown { font } => assert_eq!(font, "Helvetica"),
            other => panic!("expected Unknown for an unregistered font, got {other:?}"),
        }
    }

    /// **And unknown must stop the write.** Obfuscation destroys the original,
    /// so an unanswerable coverage question cannot be treated as a yes: the
    /// failure mode is a document both unreadable and unrecoverable.
    #[test]
    fn an_unanswerable_coverage_question_refuses_to_commit() {
        assert!(!Coverage::Unknown { font: "X".into() }.is_safe());
        assert!(check_coverage("Helvetica", "4821000734").is_err());

        assert!(Coverage::Covered.is_safe());
        assert!(!Coverage::Missing(vec!['\u{4e00}']).is_safe());
    }

    // -- coverage from a document's own font -------------------------------

    /// A real face, with a real `cmap`. Bundled with the app for outlined-text
    /// matching, and reused here because a test that invents font bytes proves
    /// nothing about reading real ones.
    fn a_real_font() -> Option<Vec<u8>> {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../../pagify/third_party/fonts/Montserrat-Regular.ttf"
        );
        std::fs::read(path).ok()
    }

    /// The five tests below skip when the bundled font is not where they
    /// expect, which makes a wrong path look like a clean run — it did once,
    /// by one directory level. This fails instead.
    #[test]
    fn the_font_these_tests_need_is_actually_found() {
        assert!(
            a_real_font().is_some_and(|f| !f.is_empty()),
            "the bundled font was not found, so every coverage test below is skipping"
        );
    }

    #[test]
    fn a_readable_face_answers_for_a_replacement_it_can_draw() {
        let Some(data) = a_real_font() else {
            eprintln!("skipping: the bundled font is not where this test expects it");
            return;
        };
        assert_eq!(
            coverage_of_font_data(&data, "4821000734", "9350021198"),
            Coverage::Covered
        );
        assert!(check_coverage_of_font_data(&data, "4821000734", "9350021198").is_ok());
    }

    /// And says which characters are missing when it cannot.
    #[test]
    fn a_readable_face_names_what_it_cannot_draw() {
        let Some(data) = a_real_font() else { return };
        match coverage_of_font_data(&data, "part 4821", "part 一二三") {
            Coverage::Missing(missing) => assert_eq!(missing, vec!['一', '二', '三']),
            other => panic!("expected Missing, got {other:?}"),
        }
    }

    /// **The measured case.** A subset font with no usable Unicode `cmap`
    /// cannot account for the text it is visibly drawing — 18–45% of the runs
    /// on real catalogue pages, per this module's own notes. Its verdict about
    /// anything else is worthless, so it must come back `Unknown` rather than
    /// as a refusal that looks like a real answer.
    #[test]
    fn a_face_that_cannot_account_for_its_own_text_answers_unknown() {
        let Some(data) = a_real_font() else { return };
        // A face genuinely cannot draw these, which stands in for the same
        // observable state a wrongly-mapped subset font is in: asked about the
        // text on the page, it says no.
        match coverage_of_font_data(&data, "一二三", "9350021198") {
            Coverage::Unknown { .. } => {}
            other => panic!("expected Unknown for a face that cannot draw its own text: {other:?}"),
        }
        assert!(check_coverage_of_font_data(&data, "一二三", "9350021198").is_err());
    }

    /// Bytes that are not a font at all are unknown, not missing — the
    /// difference between "this will not render" and "I could not tell", which
    /// is the whole point of the third answer.
    #[test]
    fn bytes_that_are_not_a_font_answer_unknown() {
        match coverage_of_font_data(b"not a font", "4821", "9350") {
            Coverage::Unknown { .. } => {}
            other => panic!("expected Unknown, got {other:?}"),
        }
    }

    /// Nothing drawn is nothing to calibrate against, so there is no basis for
    /// believing the face either way.
    #[test]
    fn a_run_with_no_drawable_text_gives_no_basis_to_judge() {
        let Some(data) = a_real_font() else { return };
        for drawn in ["", "   "] {
            match coverage_of_font_data(&data, drawn, "9350021198") {
                Coverage::Unknown { .. } => {}
                other => panic!("expected Unknown for {drawn:?}, got {other:?}"),
            }
        }
    }

    /// Whitespace is positioning rather than a glyph in many subset fonts, so
    /// a space in either string must not decide the answer on its own.
    #[test]
    fn a_space_is_not_taken_as_evidence_either_way() {
        let Some(data) = a_real_font() else { return };
        assert_eq!(
            coverage_of_font_data(&data, "part 4821", "part 9350"),
            Coverage::Covered
        );
    }
}
