//! Type converted to outlines, read back by matching against a real font.
//!
//! The house rule applies here exactly as it does to redaction: prove it
//! against a real file with a real font, not against the pure geometry alone.
//! `outlined.pdf` was written by `examples/make_text_fixtures.rs` from an
//! actual system font's own glyph contours — see that file's `outlined()` —
//! so the text this test expects back is known independently of anything in
//! `pdf_core` itself.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test --test outlined_type
//! ```

mod harness;
use harness::{open_fixture, save_full_copy_and_reopen, serial, skip_without_pdfium};

use pdf_core::document::glyphs::Catalogue;
use pdf_core::document::{Document, DocumentMut, PageTextKind, Redaction};

/// The candidate fonts `make_text_fixtures.rs` tries, in the same order — the
/// fixture was built from whichever of these existed on the machine that ran
/// it. Tried in order here for the same reason: this machine may not be that
/// one, and the fixture does not say which font it used.
const CANDIDATES: &[&str] = &[
    "/System/Library/Fonts/Supplemental/Arial.ttf",
    "/System/Library/Fonts/Geneva.ttf",
];

fn any_candidate_catalogue() -> Option<Catalogue> {
    let chars = ('A'..='Z').chain('a'..='z').chain(",.".chars());
    CANDIDATES.iter().find_map(|path| {
        let data = std::fs::read(path).ok()?;
        let catalogue = Catalogue::from_font(&data, chars.clone());
        (!catalogue.is_empty()).then_some(catalogue)
    })
}

/// The exact sixteen lines `outlined()` in `make_text_fixtures.rs` draws.
///
/// **A single stacked column, not two side-by-side ones** — `LEFT` and
/// `RIGHT` are two arrays in the generator's own source, chained end to end
/// with `LEFT.iter().chain(RIGHT.iter())`, every line starting at the same
/// `pen_x` with the baseline stepping down by one row each time. The names
/// describe how the strings are organised in that file, not the geometry of
/// the page, and this test compared against half the page until that was
/// checked directly rather than assumed from the names.
const EXPECTED: &str = "The luminaire housing is formed from
extruded aluminium with a powder
coated finish. Ingress protection is
rated to IP65 throughout the range,
and the diffuser is opal polycarbonate
with a nominal transmission of eighty
two percent measured at the centre
of the emitting surface.
Control gear is supplied loose or
integral depending on the variant
ordered. DALI-2 dimming is available
across every output, and emergency
versions carry a three hour battery
tested to the relevant standard for
self contained luminaires used in
commercial installations.";

#[test]
fn the_page_is_classified_outlined() {
    let Some(pdfium) = skip_without_pdfium() else { return };
    let _lock = serial();

    let doc = open_fixture(&pdfium, "outlined.pdf");
    let verdict = doc.page(0).expect("page 0").classify().expect("classify");
    assert_eq!(verdict.kind, PageTextKind::Outlined, "the fixture is not outlined type: {verdict:?}");
}

/// Ordinary Levenshtein edit distance, with no crate pulled in for one
/// function used in exactly one test.
fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i];
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            cur.push((prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost));
        }
        prev = cur;
    }
    prev[b.len()]
}

/// How much of `expected` survived in `text`, 0 to 1.
fn similarity(text: &str, expected: &str) -> f32 {
    let max_len = text.chars().count().max(expected.chars().count()).max(1);
    1.0 - edit_distance(text, expected) as f32 / max_len as f32
}

/// **The acceptance test.** Real path objects, read through PDFium, matched
/// against a real font's real glyph contours — no synthetic geometry anywhere
/// in this one.
///
/// Scored by similarity rather than exact substrings, and deliberately so.
/// This module's own docs name two real, measured limitations — a disjoint
/// `i` or `j` splits into two clusters that rarely both match anything, and a
/// small number of tightly kerned neighbours merge into one shape that
/// matches nothing — and an exact-match test would either hide those behind a
/// generous `contains` here and there, or fail permanently on a limitation
/// this module already knows about and names. A quantified score says
/// instead exactly how good this is, catches a real regression (a change
/// that makes the number fall), and does not pretend the number is 100.
///
/// **No separate check for reading order** — measured, not assumed, to be
/// unnecessary: shuffling this exact expected text's sixteen lines into a
/// random order, with every character otherwise perfect, scores 0.22 against
/// itself under this same metric. Edit distance is an alignment, so getting
/// the order wrong is expensive in exactly the currency this test already
/// charges; a real reordering bug would fail the one assertion below on its
/// own, and a second, cruder check built on exact substrings only reintroduces
/// the brittleness the first paragraph explains why this test avoids.
#[test]
fn outlined_type_is_read_back_by_matching_a_real_font() {
    let Some(pdfium) = skip_without_pdfium() else { return };
    let _lock = serial();

    let Some(catalogue) = any_candidate_catalogue() else {
        eprintln!("skipped: none of {CANDIDATES:?} exist on this machine");
        return;
    };
    // Twenty-six letters plus digits and punctuation is not a large
    // catalogue; if this is suspiciously small, the font parsed but barely
    // resolved any glyphs, and the rest of the test would only be measuring
    // that.
    assert!(catalogue.len() > 20, "the candidate font yielded almost no glyphs: {}", catalogue.len());

    let doc = open_fixture(&pdfium, "outlined.pdf");
    let page = doc.page(0).expect("page 0");
    let result = page.recognise_outlined(&catalogue).expect("recognise");
    let text = result.plain();

    let score = similarity(&text, EXPECTED);
    eprintln!("=== RECOVERED (score {score:.2}) ===\n{text}\n=== EXPECTED ===\n{EXPECTED}\n=== END ===");

    // Measured at 0.74 against Arial on the machine this was written on.
    // Margin left for a different candidate font resolving on someone else's
    // machine, not for a future regression to hide in.
    assert!(
        score > 0.55,
        "recognition quality fell to {score:.2} (expected comfortably above 0.55). \
         Recovered:\n{text}\nExpected:\n{EXPECTED}"
    );
}

/// A catalogue with nothing in it recognises nothing — and says so by
/// returning empty, not by failing.
#[test]
fn an_empty_catalogue_recognises_nothing() {
    let Some(pdfium) = skip_without_pdfium() else { return };
    let _lock = serial();

    let doc = open_fixture(&pdfium, "outlined.pdf");
    let result = doc.page(0).expect("page").recognise_outlined(&Catalogue::default()).expect("recognise");
    assert!(result.blocks.is_empty());
}

/// A page with real text objects — not outlines — matches nothing against a
/// letter catalogue, because there are no paths shaped like type on it at all.
/// This is the negative control: it proves the pipeline does not hallucinate
/// text from an ordinary page's furniture.
#[test]
fn a_native_text_page_has_no_outlined_letters_to_find() {
    let Some(pdfium) = skip_without_pdfium() else { return };
    let _lock = serial();

    let Some(catalogue) = any_candidate_catalogue() else { return };
    let doc = open_fixture(&pdfium, "two-column.pdf");
    let result = doc.page(0).expect("page").recognise_outlined(&catalogue).expect("recognise");
    assert!(
        result.plain().is_empty(),
        "found outlined letters on a page that has none: {:?}",
        result.plain()
    );
}

// --------------------------------------------------- redaction integration --

/// **The acceptance test for wiring this matcher into redaction.** Before
/// this, [`PageTextKind::Outlined`] refused a redaction outright, with no way
/// past it — the plan's own words were "which redaction cannot remove yet".
/// With a real font supplied, the same page now redacts, and the proof is the
/// same one the rest of redaction already lives by: save it, reopen it with
/// nothing of this test's own code in the way, and ask the same matcher —
/// itself a real, external check, since it has no memory of what this test
/// just did — whether the words are still findable.
#[test]
fn a_redaction_with_a_font_actually_removes_outlined_words() {
    let Some(pdfium) = skip_without_pdfium() else { return };
    let _lock = serial();

    let Some(catalogue) = any_candidate_catalogue() else {
        eprintln!("skipped: none of {CANDIDATES:?} exist on this machine");
        return;
    };
    const FIRST_LINE: &str = "The luminaire housing is formed from";
    const LAST_LINE: &str = "commercial installations.";

    let mut doc = open_fixture(&pdfium, "outlined.pdf");
    let before = doc.page(0).expect("page").recognise_outlined(&catalogue).expect("recognise");

    // Scored, not matched exactly — this file's own earlier test already
    // measured that individual letters (every disjoint `i`) do not survive
    // recognition even before anything is redacted. A brittle exact-substring
    // control here would be testing that known, named gap, not this feature.
    let before_score = similarity(&before.blocks[0].lines[0].text, FIRST_LINE);
    assert!(before_score > 0.5, "control: the line barely reads at all yet ({before_score:.2})");

    // The first line's own rectangle — not a hand-measured number carried in
    // from earlier debugging, which is exactly the kind of brittle constant
    // this whole session has tried not to leave behind. `recognise_outlined`
    // is the thing under test elsewhere in this file; reusing its own output
    // to find where the words are is using an already-proven-correct oracle,
    // not assuming a coordinate.
    //
    // `layout::Rect` and `document::Rect` are two distinct types with
    // matching fields, not one — the same distinction `redact_inner` itself
    // has to bridge by hand where `Identified::bounds` meets `Redaction`.
    let line_rect = before.blocks[0].lines[0].rect;
    let area = pdf_core::document::Rect {
        left: line_rect.left,
        top: line_rect.top,
        right: line_rect.right,
        bottom: line_rect.bottom,
    };

    // **Not `Redaction::new`'s default.** That refuses the whole operation
    // unless *every* path the rectangle touches clears — and this feature's
    // own honestly-measured ceiling is well under every letter matching. A
    // disjoint dot that cannot be identified must still block on its own
    // account, which `an_unidentifiable_outlined_path_still_blocks_rather_
    // than_being_guessed_at` already covers; this test is about the many
    // letters that *do* identify, acknowledged the same way an image beside
    // real text already can be.
    let request = pdf_core::document::Redaction { require_complete: false, ..Redaction::new(0, area) };
    let outcome = doc.as_document_mut().unwrap().redact(&request, Some(&catalogue));
    assert!(outcome.is_ok(), "a font-assisted redaction on an outlined page was refused: {outcome:?}");
    let report = outcome.unwrap();
    assert!(report.objects > 0, "nothing was actually removed");

    let mut doc = doc;
    let mut reopened = save_full_copy_and_reopen(&mut doc);
    let after = reopened
        .as_mut()
        .page(0)
        .expect("page")
        .recognise_outlined(&catalogue)
        .expect("recognise");

    // The saved-and-reopened file, read by the same matcher fresh — no memory
    // of this test's own in-memory state. The redacted line must read
    // dramatically worse than it did; it does not have to read as nothing,
    // because a handful of unidentified fragments are expected to remain and
    // `recognise_outlined` may still stitch a few surviving letters into
    // *something*, just not the original sentence.
    let after_text = after.plain();
    let after_first_line_score = after
        .blocks
        .first()
        .and_then(|b| b.lines.first())
        .map(|l| similarity(&l.text, FIRST_LINE))
        .unwrap_or(0.0);
    eprintln!("first line score: {before_score:.2} -> {after_first_line_score:.2}");
    eprintln!("objects removed: {}", report.objects);
    assert!(
        after_first_line_score < before_score - 0.2,
        "the redacted line still reads about as well as before ({before_score:.2} -> {after_first_line_score:.2}):\n{after_text}"
    );

    // And nothing else on the page was touched: the last line, nowhere near
    // the rectangle, must read exactly as well as it always did. Scored
    // against that one line's own text, not the whole page's — comparing a
    // single short line against sixteen lines of accumulated text would tank
    // the score on length alone, whatever that line actually says.
    let last_line_score = after
        .blocks
        .iter()
        .flat_map(|b| &b.lines)
        .map(|l| similarity(&l.text, LAST_LINE))
        .fold(0.0f32, f32::max);
    assert!(
        last_line_score > 0.5,
        "redaction reached a line it was never pointed at (best line score {last_line_score:.2}):\n{after_text}"
    );
}

/// **The regression this integration must not cause.** Exactly the same call,
/// with no font — the refusal has to be precisely what it always was, not
/// weakened by the new parameter merely existing.
#[test]
fn without_a_font_the_same_page_still_refuses_exactly_as_before() {
    let Some(pdfium) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open_fixture(&pdfium, "outlined.pdf");
    let area = pdf_core::document::Rect { left: 0.0, top: 0.0, right: 500.0, bottom: 500.0 };
    let outcome = doc.as_document_mut().unwrap().redact(&Redaction::new(0, area), None);

    assert!(outcome.is_err(), "an outlined page redacted with no font at all");
    let message = format!("{}", outcome.unwrap_err());
    assert!(
        message.contains("curves") && message.contains("font"),
        "the refusal no longer names why: {message}"
    );
}

/// A path that cannot be identified — the catalogue given is empty — is left
/// exactly where it was: reported, not guessed at and not removed.
#[test]
fn an_unidentifiable_outlined_path_still_blocks_rather_than_being_guessed_at() {
    let Some(pdfium) = skip_without_pdfium() else { return };
    let _lock = serial();

    let doc = open_fixture(&pdfium, "outlined.pdf");
    let area = pdf_core::document::Rect { left: 55.0, top: 85.0, right: 180.0, bottom: 105.0 };
    let empty = Catalogue::default();

    let mut doc = doc;
    let outcome = doc.as_document_mut().unwrap().preview_redaction(&Redaction::new(0, area), Some(&empty));
    let report = outcome.expect("a survey, not a hard refusal, once a catalogue exists at all");
    assert!(
        !report.blockers().is_empty(),
        "an unmatchable shape was treated as removable"
    );
    assert!(
        report
            .uncleared
            .iter()
            .any(|u| matches!(u, pdf_core::document::Uncleared::OutlinedText { .. })),
        "the report no longer says the blocker is outlined type: {report:?}"
    );
}

// --------------------------------------------------------------- as_words --

/// **The real-fixture proof for the word-level entry point.** Everything
/// above proves `recognise_outlined`'s line-level reassembly against real
/// path objects; this proves the *other* consumer of the same identified
/// glyphs — `recognise_outlined_words`, the one an "Extract Text" feature
/// would actually call — independently, since the two share the survey but
/// not the assembly, and a bug in word-grouping specifically would not show
/// up in a test that only ever asks for the whole page as one string.
#[test]
fn recognise_outlined_words_finds_real_words_with_sane_boxes() {
    let Some(pdfium) = skip_without_pdfium() else { return };
    let _lock = serial();

    let Some(catalogue) = any_candidate_catalogue() else {
        eprintln!("skipped: none of {CANDIDATES:?} exist on this machine");
        return;
    };

    let doc = open_fixture(&pdfium, "outlined.pdf");
    let page = doc.page(0).expect("page 0");
    let size = doc.page_size(0).expect("size");
    let words = page.recognise_outlined_words(&catalogue).expect("recognise");

    assert!(words.len() > 50, "suspiciously few words for a sixteen-line page: {}", words.len());

    // "The" is the very first word on the page and short enough to survive
    // the disjoint-dot limitation this module's own docs name — a safe,
    // specific thing to look for rather than asserting on the count alone.
    let the = words.iter().find(|w| w.text == "The").expect("the opening word was not found at all");
    assert!(the.rect.left < 100.0 && the.rect.top < 150.0, "'The' was found somewhere implausible: {:?}", the.rect);

    for word in &words {
        assert!(!word.text.is_empty(), "an empty word was returned");
        assert!(
            (0.0..=1.0).contains(&word.confidence),
            "confidence out of range for {:?}: {}",
            word.text,
            word.confidence
        );
        assert_eq!(
            word.char_confidence.len(),
            word.text.chars().count(),
            "char_confidence does not line up with its own word's text: {:?}",
            word.text
        );
        // Every box has to sit on the page it was read from — the coordinate
        // space this module works in throughout, not the font's or the raw
        // PDF's.
        assert!(
            word.rect.left >= -1.0
                && word.rect.right <= size.width_pt + 1.0
                && word.rect.top >= -1.0
                && word.rect.bottom <= size.height_pt + 1.0,
            "a word's box falls off the page: {:?} on a {}x{} page",
            word.rect,
            size.width_pt,
            size.height_pt
        );
    }
}

/// The two entry points share the same survey, so they must agree on *which*
/// words exist — reassembled into one string, `recognise_outlined_words`'
/// own output has to be the same text `recognise_outlined` already produces
/// and this whole file already trusts.
#[test]
fn recognise_outlined_words_agrees_with_recognise_outlined() {
    let Some(pdfium) = skip_without_pdfium() else { return };
    let _lock = serial();

    let Some(catalogue) = any_candidate_catalogue() else { return };

    let doc = open_fixture(&pdfium, "outlined.pdf");
    let page = doc.page(0).expect("page 0");

    let as_text = page.recognise_outlined(&catalogue).expect("recognise").plain();
    let words = page.recognise_outlined_words(&catalogue).expect("recognise words");
    let joined = words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");

    assert!(
        similarity(&joined, &as_text) > 0.9,
        "the two entry points disagree about what is on the page:\nwords: {joined}\ntext:  {as_text}"
    );
}
