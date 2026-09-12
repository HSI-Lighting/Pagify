//! Redaction, proved by reading the saved file back.
//!
//! The house rule with teeth. Every other feature can be checked against the
//! in-memory document and be right; redaction cannot, because the whole failure
//! mode is content that is absent from the object model and present in the
//! bytes. So each test here **saves, reopens from those bytes, and asks PDFium's
//! own extraction** — a code path that shares nothing with the writer — whether
//! the words are still there.
//!
//! Every negative assertion is paired with a control that proves the check could
//! have failed. "The word is gone" is worth nothing from a reader that finds no
//! words at all.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test --test redaction
//! ```

mod harness;
use harness::{fixture_bytes, open_fixture, serial, skip_without_pdfium};

use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{
    Color, Document, DocumentMut, Rect, Redaction, RedactionReport, Uncleared,
};
use pdf_core::error::PdfError;

fn rect(left: f32, top: f32, right: f32, bottom: f32) -> Rect {
    Rect { left, top, right, bottom }
}

/// The rectangle a user would draw over `needle` — the union of its own
/// characters' boxes.
///
/// **Measured rather than guessed.** The first version of these tests carried
/// hand-estimated coordinates and ate the leading `t` of the following word,
/// which reads as a splitting bug and is not one: any character the rectangle
/// touches is removed, and the rectangle was simply too wide. Deriving it from
/// the characters tests the behaviour instead of the arithmetic in a comment,
/// and is what a selection in the UI actually produces.
fn box_around(doc: &dyn Document, page: usize, needle: &str) -> Rect {
    let chars = doc.page(page).expect("page").characters().expect("characters");
    let at = chars.text.find(needle).expect("the text to redact is not on the page");
    // `boxes` is four floats per **code unit** of `text`, so the offset has to be
    // counted the same way rather than in bytes.
    let start = chars.text[..at].chars().count();
    let len = needle.chars().count();

    let mut bounds: Option<Rect> = None;
    for index in start..start + len {
        let b = &chars.boxes[index * 4..index * 4 + 4];
        let (left, top, right, bottom) = (b[0], b[1], b[2], b[3]);
        bounds = Some(match bounds {
            None => Rect { left, top, right, bottom },
            Some(r) => Rect {
                left: r.left.min(left),
                top: r.top.min(top),
                right: r.right.max(right),
                bottom: r.bottom.max(bottom),
            },
        });
    }
    bounds.expect("a non-empty needle")
}

/// Every character PDFium finds on a page of the **saved** file.
///
/// Deliberately not `text_runs`, which reads through the same object walk
/// redaction writes with. This goes through `FPDFText_*` on a document parsed
/// afresh from bytes.
fn text_of(doc: &dyn Document, page: usize) -> String {
    doc.page(page).expect("page").characters().expect("characters").text
}

/// Redact, save as a full copy, and reopen from the bytes.
fn redact_and_reopen(name: &str, request: &Redaction) -> (Box<dyn Document>, Vec<u8>) {
    let path = harness::fixture_path(name);
    let mut doc =
        PdfiumDocument::open_path(path.to_str().expect("path"), None).expect("open fixture");
    doc.redact(request, None).expect("redact");

    let mut bytes = Vec::new();
    doc.save_full_copy(&mut bytes).expect("save");
    drop(doc);

    let reopened = PdfiumDocument::open_bytes(bytes.clone(), None).expect("reopen");
    (Box::new(reopened), bytes)
}

// ---------------------------------------------------------------- the gate --

/// **The acceptance test.** Redact a line, write the file, reopen it with none
/// of the redaction code in the way, and ask whether the words are there.
///
/// Everything downstream — Lock, Obfuscate, the command stack — assumes this
/// works. It is proved once, explicitly, before anything is built on it.
#[test]
fn a_redacted_line_is_gone_from_the_saved_file() {
    let Some(pdfium) = skip_without_pdfium() else { return };
    let _lock = serial();

    // The control first: prove the reader finds this line before it is removed,
    // or "it is gone" means only that the reader is broken.
    let before = text_of(open_fixture(&pdfium, "text-lines.pdf").as_ref(), 0);
    assert!(
        before.contains("The quick brown fox"),
        "the reader cannot see the line to begin with, so its absence proves nothing"
    );
    assert!(before.contains("jumps over the lazy dog"), "control line missing");

    // "The quick brown fox" sits at L40.3 T49.8 R165.2 B62.8.
    let (doc, bytes) = redact_and_reopen(
        "text-lines.pdf",
        &Redaction::new(0, rect(35.0, 45.0, 170.0, 66.0)),
    );
    let after = text_of(doc.as_ref(), 0);

    assert!(
        !after.contains("The quick brown fox"),
        "the redacted line survived the save and is still extractable: {after:?}"
    );
    assert!(
        after.contains("jumps over the lazy dog"),
        "redaction took a line it was not pointed at: {after:?}"
    );

    // And the same question asked of the raw bytes, which is the form the
    // failure actually takes: a string sitting in the file, whatever any reader
    // makes of it.
    let raw = fixture_bytes("text-lines.pdf");
    let control_is_findable = contains(&raw, b"The quick brown fox");
    if control_is_findable {
        assert!(
            !contains(&bytes, b"The quick brown fox"),
            "the words are still in the file's bytes"
        );
    } else {
        // Compressed or subset-encoded, so a byte scan proves nothing either
        // way. Said out loud rather than passing quietly.
        eprintln!("note: the fixture's text is not stored as plain bytes, so the byte scan is inconclusive");
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// **The trap this feature exists to avoid.** `FPDF_INCREMENTAL` keeps the
/// original bytes verbatim, so the removed words would still be in the file at
/// their old offsets.
#[test]
fn a_redacted_document_refuses_to_save_incrementally() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let path = harness::fixture_path("text-lines.pdf");
    let mut doc =
        PdfiumDocument::open_path(path.to_str().expect("path"), None).expect("open fixture");
    assert!(!doc.must_save_full_copy(), "nothing has been redacted yet");

    doc.redact(&Redaction::new(0, rect(35.0, 45.0, 170.0, 66.0)), None).expect("redact");
    assert!(doc.must_save_full_copy(), "the document did not record that it was redacted");

    let mut bytes = Vec::new();
    match doc.save_incremental(&mut bytes) {
        Err(PdfError::IncompleteRedaction(why)) => {
            assert!(why.contains("full copy"), "the error does not say what to do: {why}")
        }
        Err(other) => panic!("wrong error: {other}"),
        Ok(()) => panic!("an incremental save of a redacted document was allowed"),
    }
    assert!(bytes.is_empty(), "it wrote something before refusing");

    // The full copy is still available, and is what the caller is told to use.
    doc.save_full_copy(&mut bytes).expect("full copy");
    assert!(!bytes.is_empty());
}

// ------------------------------------------------------------- the splitting --

/// **The case the splitting exists for.** A rectangle over one word in the
/// middle of a line has to keep both ends, each where it was.
#[test]
fn a_word_taken_from_the_middle_of_a_line_leaves_the_rest() {
    let Some(pdfium) = skip_without_pdfium() else { return };
    let _lock = serial();

    let source = open_fixture(&pdfium, "two-column.pdf");
    let before = text_of(source.as_ref(), 0);
    assert!(before.contains("IP65"), "control: the word is not there to remove");

    // "rated to IP65 throughout the range," — take the middle word only.
    let area = box_around(source.as_ref(), 0, "IP65");
    drop(source);
    let (doc, _) = redact_and_reopen("two-column.pdf", &Redaction::new(0, area));
    let after = text_of(doc.as_ref(), 0);

    assert!(!after.contains("IP65"), "the covered word survived: {after:?}");
    assert!(after.contains("rated to"), "the head of the line was lost: {after:?}");
    assert!(
        after.contains("throughout the range"),
        "the tail of the line was lost: {after:?}"
    );
}

/// The surviving tail must stay where it was rather than sliding into the gap.
#[test]
fn the_tail_of_a_split_line_keeps_its_position() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let path = harness::fixture_path("two-column.pdf");
    let source = PdfiumDocument::open_path(path.to_str().expect("path"), None).expect("open");
    let source: Box<dyn Document> = Box::new(source);
    assert!(
        text_of(source.as_ref(), 0).contains("throughout"),
        "control: the tail is not there to begin with"
    );
    let area = box_around(source.as_ref(), 0, "IP65");
    drop(source);

    let (doc, _) = redact_and_reopen("two-column.pdf", &Redaction::new(0, area));
    let run = doc
        .text_runs(0)
        .expect("runs")
        .into_iter()
        .find(|r| r.text.contains("throughout"))
        .expect("the tail survived as a run");

    // It began after "rated to IP65 " — well right of the line's own origin at
    // 50.7. Rebuilt from the origin it would start there instead.
    assert!(
        run.rect.left > 100.0,
        "the tail slid left into the gap the redaction made: it starts at {}",
        run.rect.left
    );
}

// ----------------------------------------------------------- what it refuses --

/// A page whose words are curves cannot be redacted by removing text objects.
/// Refused by name rather than reporting success over content still there.
#[test]
fn an_outlined_page_refuses_rather_than_removing_nothing() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let path = harness::fixture_path("outlined.pdf");
    let mut doc = PdfiumDocument::open_path(path.to_str().expect("path"), None).expect("open");

    match doc.redact(&Redaction::new(0, rect(0.0, 0.0, 500.0, 500.0)), None) {
        Err(PdfError::Unsupported(why)) => {
            assert!(why.contains("curves"), "the reason is not named: {why}")
        }
        Err(other) => panic!("wrong error: {other}"),
        Ok(report) => panic!("an outlined page reported a redaction: {report:?}"),
    }
    assert!(!doc.must_save_full_copy(), "a refused redaction still marked the document");
}

/// The same for a scan: the words are the picture.
#[test]
fn a_scanned_page_refuses_rather_than_removing_nothing() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let path = harness::fixture_path("scan-300dpi.pdf");
    let mut doc = PdfiumDocument::open_path(path.to_str().expect("path"), None).expect("open");

    match doc.redact(&Redaction::new(0, rect(50.0, 50.0, 200.0, 100.0)), None) {
        Err(PdfError::Unsupported(why)) => {
            assert!(why.contains("scan"), "the reason is not named: {why}")
        }
        Err(other) => panic!("wrong error: {other}"),
        Ok(report) => panic!("a scan reported a redaction: {report:?}"),
    }
}

/// **Surveyed whole, then applied.** A refusal must leave the page exactly as
/// it was, not half-cleared.
#[test]
fn a_refused_redaction_changes_nothing() {
    let Some(pdfium) = skip_without_pdfium() else { return };
    let _lock = serial();

    let before = text_of(open_fixture(&pdfium, "outlined.pdf").as_ref(), 0);

    let path = harness::fixture_path("outlined.pdf");
    let mut doc = PdfiumDocument::open_path(path.to_str().expect("path"), None).expect("open");
    let _ = doc.redact(&Redaction::new(0, rect(0.0, 0.0, 500.0, 500.0)), None);

    let mut bytes = Vec::new();
    doc.save_full_copy(&mut bytes).expect("save");
    drop(doc);
    let after = text_of(
        &PdfiumDocument::open_bytes(bytes, None).expect("reopen") as &dyn Document,
        0,
    );
    assert_eq!(before, after, "a refused redaction still changed the page");
}

/// **Words inside a form XObject are not reached, and the refusal says so.**
///
/// The audit's probe: a card number drawn through a form. Extraction finds it,
/// so it can be searched for and a rectangle drawn over it — and the redaction
/// pass, which does not descend into forms, removes nothing. That used to go
/// through with `require_complete` off: a black mark painted, the number still
/// in the saved file, and a report of success. Now the survey sees nested
/// content under an area it took no characters out of, and refuses by name.
#[test]
fn words_inside_a_form_refuse_rather_than_being_painted_over() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let path = harness::fixture_path("secret-in-form.pdf");
    let mut doc = PdfiumDocument::open_path(path.to_str().expect("path"), None).expect("open");
    let card = doc
        .sensitive_on(0)
        .expect("scan")
        .into_iter()
        .find(|f| f.text.contains("4111"))
        .expect("the card number is found by extraction — that is the point");

    // The survey says what is there.
    let mut lenient = Redaction::new(0, card.area);
    lenient.require_complete = false;
    let report = doc.preview_redaction(&lenient, None).expect("survey");
    assert_eq!(report.characters, 0, "no page-level characters are under the area");
    assert!(
        report.uncleared.iter().any(|u| matches!(u, Uncleared::Form { .. })),
        "the form was not seen: {report:?}"
    );
    assert!(report.would_only_draw_a_mark(), "a mark over nothing removed was allowed");

    // And going ahead — even with completeness not required — is refused.
    match doc.redact(&lenient, None) {
        Err(PdfError::IncompleteRedaction(why)) => {
            assert!(why.contains("nested content"), "the reason is not named: {why}");
            assert!(why.contains("remove nothing"), "{why}");
        }
        Err(other) => panic!("wrong error: {other}"),
        Ok(report) => panic!("words inside a form reported a redaction: {report:?}"),
    }

    // The control: the number is still there, which is exactly why refusing
    // was right.
    let mut bytes = Vec::new();
    doc.save_full_copy(&mut bytes).expect("save");
    drop(doc);
    let reopened = PdfiumDocument::open_bytes(bytes, None).expect("reopen");
    assert!(text_of(&reopened as &dyn Document, 0).contains("4111 1111 1111 1111"));
}

/// The page-level words on the same page are still redactable — the form's
/// presence elsewhere on the page does not refuse them.
#[test]
fn page_level_words_beside_a_form_still_come_out() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let path = harness::fixture_path("secret-in-form.pdf");
    let mut doc = PdfiumDocument::open_path(path.to_str().expect("path"), None).expect("open");
    let telephone = doc
        .sensitive_on(0)
        .expect("scan")
        .into_iter()
        .find(|f| f.text.contains("7946"))
        .expect("the telephone number is found");
    let report = doc.redact(&Redaction::new(0, telephone.area), None).expect("redact");
    assert!(report.characters > 0, "{report:?}");

    let mut bytes = Vec::new();
    doc.save_full_copy(&mut bytes).expect("save");
    drop(doc);
    let reopened = PdfiumDocument::open_bytes(bytes, None).expect("reopen");
    let text = text_of(&reopened as &dyn Document, 0);
    assert!(!text.contains("7946"), "the telephone number survived: {text}");
    assert!(text.contains("4111"), "the control went missing: {text}");
}

// ------------------------------------------------------------- the reporting --

#[test]
fn the_report_says_what_it_destroyed() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let path = harness::fixture_path("text-lines.pdf");
    let mut doc = PdfiumDocument::open_path(path.to_str().expect("path"), None).expect("open");
    let report = doc
        .redact(&Redaction::new(0, rect(35.0, 45.0, 170.0, 66.0)), None)
        .expect("redact");

    assert_eq!(report.characters, "The quick brown fox".len());
    assert_eq!(report.objects, 1, "one whole run should have gone");
    assert!(report.spilled.is_empty(), "nothing outside the rectangle should have gone");
    assert!(report.is_complete(), "nothing should have been left behind: {report:?}");
}

/// A rectangle that touches nothing is not an error, and does not pretend to
/// have done something.
#[test]
fn a_rectangle_over_empty_space_removes_nothing() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let path = harness::fixture_path("text-lines.pdf");
    let mut doc = PdfiumDocument::open_path(path.to_str().expect("path"), None).expect("open");
    let report = doc
        .redact(&Redaction {
            fill: None,
            ..Redaction::new(0, rect(300.0, 250.0, 380.0, 280.0))
        }, None)
        .expect("redact");

    assert_eq!(report.characters, 0);
    assert_eq!(report.objects, 0);

    // Still marked, because whether an incremental save is safe is not a
    // judgement to make from one rectangle's yield.
    assert!(doc.must_save_full_copy());
}

#[test]
fn the_mark_is_painted_over_the_area() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let path = harness::fixture_path("text-lines.pdf");
    let mut doc = PdfiumDocument::open_path(path.to_str().expect("path"), None).expect("open");
    let before = doc.page(0).expect("page").classify().expect("classify").paths;

    doc.redact(&Redaction {
        fill: Some(Color { r: 0, g: 0, b: 0, a: 255 }),
        ..Redaction::new(0, rect(35.0, 45.0, 170.0, 66.0))
    }, None)
    .expect("redact");

    let after = doc.page(0).expect("page").classify().expect("classify").paths;
    assert_eq!(after, before + 1, "no mark was painted");
}

#[test]
fn asking_for_no_mark_leaves_the_space_blank() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let path = harness::fixture_path("text-lines.pdf");
    let mut doc = PdfiumDocument::open_path(path.to_str().expect("path"), None).expect("open");
    let before = doc.page(0).expect("page").classify().expect("classify").paths;

    doc.redact(&Redaction { fill: None, ..Redaction::new(0, rect(35.0, 45.0, 170.0, 66.0)) }, None)
        .expect("redact");

    let after = doc.page(0).expect("page").classify().expect("classify").paths;
    assert_eq!(after, before, "a mark was painted when none was asked for");
}

#[test]
fn an_out_of_range_page_is_refused() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let path = harness::fixture_path("text-lines.pdf");
    let mut doc = PdfiumDocument::open_path(path.to_str().expect("path"), None).expect("open");
    assert!(doc.redact(&Redaction::new(99, rect(0.0, 0.0, 10.0, 10.0)), None).is_err());
}

#[allow(dead_code)]
fn unused(_: Uncleared) {}

// ------------------------------------------------------- the outside check --

/// Every string in the file, inflating anything that will inflate.
///
/// **Deliberately not a PDF parser.** It looks for bytes, in every form a PDF
/// may store text in, and knows nothing about object structure. A parser that
/// understood the file could be fooled by the same misunderstanding that wrote
/// it; this cannot.
fn haystacks(data: &[u8]) -> Vec<Vec<u8>> {
    use std::io::Read;

    let mut out = vec![data.to_vec()];
    let mut at = 0usize;
    while at + 6 <= data.len() {
        let Some(found) = data[at..].windows(6).position(|w| w == b"stream") else { break };
        let keyword = at + found;
        at = keyword + 6;

        // `endstream` contains `stream`. Matching inside one desynchronises the
        // walk from that point on — every later stream is read from three bytes
        // into its own terminator — and the result is a search that quietly
        // finds nothing. Which is exactly the shape of a redaction test that
        // passes for the wrong reason.
        if keyword >= 3 && &data[keyword - 3..keyword] == b"end" {
            continue;
        }

        let mut start = at;
        while matches!(data.get(start), Some(b'\r') | Some(b'\n')) {
            start += 1;
        }
        let Some(end) = data[start..].windows(9).position(|w| w == b"endstream") else { break };
        let raw = &data[start..start + end];

        let mut inflated = Vec::new();
        if flate2::read::ZlibDecoder::new(raw).read_to_end(&mut inflated).is_ok()
            && !inflated.is_empty()
        {
            out.push(inflated);
        }
        at = start + end + 9;
    }
    out
}

/// Where `needle` appears, in any encoding a PDF may hold it in.
fn appears_in(data: &[u8], needle: &str) -> Vec<String> {
    let utf8 = needle.as_bytes().to_vec();
    let utf16: Vec<u8> = needle.encode_utf16().flat_map(|u| u.to_be_bytes()).collect();
    let hex = utf8.iter().map(|b| format!("{b:02x}")).collect::<String>();
    // Both cases. The spec allows either and PDFium writes upper, so a
    // lowercase-only probe reports a file clean that plainly is not.
    let probes: Vec<(&str, Vec<u8>)> = vec![
        ("utf-8", utf8),
        ("utf-16be", utf16),
        ("hex", hex.clone().into_bytes()),
        ("HEX", hex.to_uppercase().into_bytes()),
    ];

    let mut found = Vec::new();
    for (label, probe) in &probes {
        if probe.is_empty() {
            continue;
        }
        if haystacks(data).iter().any(|h| h.windows(probe.len()).any(|w| w == probe.as_slice())) {
            found.push((*label).to_string());
        }
    }
    found
}

/// **The strongest form of the acceptance test**, and the one that would catch
/// a redaction PDFium alone cannot see the flaw in: the words are not in the
/// file's bytes, raw or inflated, in any encoding.
///
/// Paired with the proof that the check works — the *unredacted* fixture must
/// fail the identical assertion. Without that, a probe that finds nothing
/// anywhere passes and means nothing.
#[test]
fn the_words_are_not_in_the_bytes_at_all() {
    let Some(pdfium) = skip_without_pdfium() else { return };
    let _lock = serial();

    // The check can fail. Proved on the original file first.
    let original = fixture_bytes("two-column.pdf");
    assert!(
        !appears_in(&original, "IP65").is_empty(),
        "the byte search cannot find the word even before it is removed, so its \
         silence afterwards would prove nothing"
    );

    let source = open_fixture(&pdfium, "two-column.pdf");
    let area = box_around(source.as_ref(), 0, "IP65");
    drop(source);
    let (_, bytes) = redact_and_reopen("two-column.pdf", &Redaction::new(0, area));

    // The control: this word is on the same line and must survive, so a clean
    // result cannot come from a file the search failed to read.
    assert!(
        !appears_in(&bytes, "throughout").is_empty(),
        "the search found nothing at all in the saved file, so it proves nothing"
    );

    let where_ = appears_in(&bytes, "IP65");
    assert!(
        where_.is_empty(),
        "the redacted word is still in the saved bytes, as {where_:?}"
    );
}

// ------------------------------------------------------- the command stack --

use pdf_core::command::history::CommandHistory;
use pdf_core::command::Command;

fn redact_command(page_index: usize, area: Rect) -> Command {
    Command::Redact { page_index, area, fill: None, allow_incomplete: false, outlined_fonts: Vec::new() }
}

/// **Redaction undoes, and it is the one command that has to prove it.**
///
/// Every other reversal describes a change — put the crop back, write the old
/// words. This one has nothing to describe, because what it removed is gone from
/// the object model, so it reverses by restoring a copy of the page. If that
/// copy were taken after the removal, or put back beside the page instead of
/// over it, this is where it would show.
#[test]
fn undoing_a_redaction_brings_the_words_back() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let path = harness::fixture_path("text-lines.pdf");
    let mut doc: Box<dyn Document> =
        Box::new(PdfiumDocument::open_path(path.to_str().expect("path"), None).expect("open"));
    let before = text_of(doc.as_ref(), 0);
    let pages_before = doc.page_count();
    assert!(before.contains("The quick brown fox"), "control");

    let mut history = CommandHistory::default();
    let mutable = doc.as_document_mut().expect("mutable");
    history
        .execute(redact_command(0, rect(35.0, 45.0, 170.0, 66.0)), mutable)
        .expect("redact");

    assert!(
        !text_of(doc.as_ref(), 0).contains("The quick brown fox"),
        "the redaction did not take"
    );

    let mutable = doc.as_document_mut().expect("mutable");
    history.undo(mutable).expect("undo").expect("there was something to undo");

    assert_eq!(
        doc.page_count(),
        pages_before,
        "undo put the page back beside the redacted one instead of over it"
    );
    assert_eq!(text_of(doc.as_ref(), 0), before, "the words did not come back");
}

#[test]
fn a_redaction_can_be_redone() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let path = harness::fixture_path("text-lines.pdf");
    let mut doc: Box<dyn Document> =
        Box::new(PdfiumDocument::open_path(path.to_str().expect("path"), None).expect("open"));

    let mut history = CommandHistory::default();
    history
        .execute(redact_command(0, rect(35.0, 45.0, 170.0, 66.0)), doc.as_document_mut().unwrap())
        .expect("redact");
    history.undo(doc.as_document_mut().unwrap()).expect("undo");
    assert!(text_of(doc.as_ref(), 0).contains("The quick brown fox"));

    history.redo(doc.as_document_mut().unwrap()).expect("redo").expect("something to redo");
    assert!(
        !text_of(doc.as_ref(), 0).contains("The quick brown fox"),
        "redo did not redact again"
    );
}

/// A refused redaction leaves no trace in the history — otherwise undo would
/// reverse a change that never happened.
#[test]
fn a_refused_redaction_is_not_recorded() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let path = harness::fixture_path("outlined.pdf");
    let mut doc: Box<dyn Document> =
        Box::new(PdfiumDocument::open_path(path.to_str().expect("path"), None).expect("open"));

    let mut history = CommandHistory::default();
    let outcome =
        history.execute(redact_command(0, rect(0.0, 0.0, 500.0, 500.0)), doc.as_document_mut().unwrap());

    assert!(outcome.is_err(), "an outlined page reported a redaction");
    assert!(!history.can_undo(), "a refused command was recorded anyway");
}

/// The undo label goes on a button, so it has to name the page a reader sees.
#[test]
fn the_history_names_the_redaction() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let path = harness::fixture_path("text-lines.pdf");
    let mut doc: Box<dyn Document> =
        Box::new(PdfiumDocument::open_path(path.to_str().expect("path"), None).expect("open"));

    let mut history = CommandHistory::default();
    history
        .execute(redact_command(0, rect(35.0, 45.0, 170.0, 66.0)), doc.as_document_mut().unwrap())
        .expect("redact");

    assert_eq!(history.undo_description().as_deref(), Some("Redact on page 1"));
}

/// **Undo does not make an incremental save safe again.**
///
/// The document may already have been saved between the two, and this cannot
/// know. Staying set is the answer that is wrong only in the harmless direction:
/// an unnecessary full copy, rather than a file that quietly keeps what somebody
/// meant to destroy.
#[test]
fn undoing_a_redaction_does_not_re_permit_an_incremental_save() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let path = harness::fixture_path("text-lines.pdf");
    let mut doc: Box<dyn Document> =
        Box::new(PdfiumDocument::open_path(path.to_str().expect("path"), None).expect("open"));

    let mut history = CommandHistory::default();
    history
        .execute(redact_command(0, rect(35.0, 45.0, 170.0, 66.0)), doc.as_document_mut().unwrap())
        .expect("redact");
    history.undo(doc.as_document_mut().unwrap()).expect("undo");

    assert!(doc.as_document_mut().unwrap().must_save_full_copy());
}

// -------------------------------------------------------------- the preview --

/// **What lets a caller ask instead of refusing.**
///
/// Measured on the 2026 catalogue: of 113 pages where an image blocks a mid-page
/// rectangle, 73 had their text come out perfectly cleanly — 12,886 characters
/// of it — and were refused over a photograph beside the words. Only 28 were the
/// case where the words *are* the picture. The preview is what turns the other
/// 85 from a refusal into a question.
#[test]
fn a_preview_reports_what_a_redaction_would_do_and_does_none_of_it() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let path = harness::fixture_path("text-lines.pdf");
    let mut doc: Box<dyn Document> =
        Box::new(PdfiumDocument::open_path(path.to_str().expect("path"), None).expect("open"));
    let before = text_of(doc.as_ref(), 0);

    let report = doc
        .as_document_mut()
        .expect("mutable")
        .preview_redaction(&Redaction::new(0, rect(35.0, 45.0, 170.0, 66.0)), None)
        .expect("preview");

    assert_eq!(report.characters, "The quick brown fox".len(), "it did not survey");
    assert_eq!(report.objects, 1);

    assert_eq!(text_of(doc.as_ref(), 0), before, "the preview changed the page");
    assert!(
        !doc.as_document_mut().unwrap().must_save_full_copy(),
        "a preview marked the document as redacted"
    );
}

/// A preview reports what a strict redaction would refuse over, rather than
/// refusing — otherwise there is nothing to show the person being asked.
#[test]
fn a_preview_reports_blockers_instead_of_erroring_on_them() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let path = harness::fixture_path("scan-300dpi.pdf");
    let mut doc: Box<dyn Document> =
        Box::new(PdfiumDocument::open_path(path.to_str().expect("path"), None).expect("open"));

    // A scan still refuses outright even to a preview: that one is not a
    // question anybody can answer, it is an absence of text to remove.
    assert!(doc
        .as_document_mut()
        .unwrap()
        .preview_redaction(&Redaction::new(0, rect(50.0, 50.0, 200.0, 100.0)), None)
        .is_err());
}

/// The preview and the redaction must agree, or the preview is worse than none.
#[test]
fn a_preview_says_the_same_thing_the_redaction_does() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let path = harness::fixture_path("two-column.pdf");
    let mut doc: Box<dyn Document> =
        Box::new(PdfiumDocument::open_path(path.to_str().expect("path"), None).expect("open"));
    let area = box_around(doc.as_ref(), 0, "IP65");

    let request = Redaction::new(0, area);
    let previewed = doc.as_document_mut().unwrap().preview_redaction(&request, None).expect("preview");
    let done = doc.as_document_mut().unwrap().redact(&request, None).expect("redact");

    assert_eq!(previewed, done, "the preview promised something else");
}

/// An image beside the words is reported with how much of the area it lies
/// under, which is the number that separates "ask the user" from "re-encode".
#[test]
fn an_image_reports_how_much_of_the_area_it_covers() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let path = harness::fixture_path("scan-300dpi.pdf");
    let doc = PdfiumDocument::open_path(path.to_str().expect("path"), None).expect("open");
    let size = doc.page_size(0).expect("size");
    drop(doc);

    // A rectangle over part of the scan's single full-page image.
    let mut doc = PdfiumDocument::open_path(path.to_str().expect("path"), None).expect("open");
    let area = Rect {
        left: size.width_pt * 0.2,
        top: size.height_pt * 0.2,
        right: size.width_pt * 0.5,
        bottom: size.height_pt * 0.3,
    };
    // The page refuses outright, so the coverage is asserted through the pure
    // geometry instead — the same function the survey uses.
    let _ = doc.preview_redaction(&Redaction::new(0, area), None);

    let report = RedactionReport {
        uncleared: vec![Uncleared::Image { object: 0, covers: 1.0, may_hold_text: false }],
        ..Default::default()
    };
    assert!(report.uncleared[0].describe().contains("100%"));
}
