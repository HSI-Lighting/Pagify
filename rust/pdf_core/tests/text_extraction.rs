//! Extraction, proved against real files rather than against our own model.
//!
//! The house rule is *measure, do not infer* — read the file back with none of
//! our own code in the way and assert on what is actually there. For a text
//! layer that means: write it, save, reopen from disk, and ask ordinary
//! extraction whether the words are there. Anything short of the reopen tests
//! the writer against itself.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test --test text_extraction
//! ```

mod harness;
use harness::{open_fixture, save_and_reopen, serial, skip_without_pdfium};

use pdf_core::document::{Document, PageTextKind, RecognisedWord, Rect};

fn word(text: &str, left: f32, top: f32, right: f32, bottom: f32) -> RecognisedWord {
    RecognisedWord {
        text: text.into(),
        rect: Rect { left, top, right, bottom },
        confidence: 0.95,
        char_confidence: Vec::new(),
    }
}

#[test]
fn a_page_with_text_is_classified_native() {
    let Some(pdfium) = skip_without_pdfium() else { return };
    let _lock = serial();

    let doc = open_fixture(&pdfium, "text-lines.pdf");
    let page = doc.page(0).expect("page 0");
    let verdict = page.classify().expect("classify");

    assert_eq!(verdict.kind, PageTextKind::Native);
    assert!(verdict.chars > 0, "a page with visible text reported no characters");
    assert_eq!(verdict.unmappable, 0, "clean text reported mapping errors");
}

#[test]
fn an_empty_page_is_told_apart_from_a_page_that_failed_to_map() {
    // The distinction the classifier exists for. Both used to arrive as an
    // empty result, and they want opposite responses.
    let Some(pdfium) = skip_without_pdfium() else { return };
    let _lock = serial();

    let doc = open_fixture(&pdfium, "single-page.pdf");
    let verdict = doc.page(0).expect("page").classify().expect("classify");

    assert_eq!(verdict.kind, PageTextKind::Empty);
    assert_eq!(verdict.chars, 0, "an empty page reported characters");
    assert_eq!(
        verdict.unmappable, 0,
        "an empty page reported mapping errors — Empty and Unmappable are now \
         indistinguishable again"
    );
}

#[test]
fn an_invisible_text_layer_survives_a_save_and_reads_back_as_text() {
    // Phase 6's acceptance: recognise once, write invisibly, and every later
    // read goes through ordinary extraction like any other native text.
    let Some(pdfium) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open_fixture(&pdfium, "single-page.pdf");
    let before = doc.page(0).expect("page").classify().expect("classify");
    assert_eq!(before.kind, PageTextKind::Empty, "the fixture already had text");

    let words = vec![
        word("LUMINAIRE", 40.0, 80.0, 140.0, 96.0),
        word("IP65", 40.0, 110.0, 80.0, 126.0),
    ];

    let written = doc
        .as_document_mut()
        .expect("editable")
        .add_text_layer(0, &words)
        .expect("write the layer");
    assert_eq!(written, 2);

    // Off to disk and back, with nothing of ours in the way.
    let reopened = save_and_reopen(&pdfium, &mut doc);
    let page = reopened.page(0).expect("page");

    let text = page.text().expect("extract");
    assert!(text.contains("LUMINAIRE"), "the layer did not survive the save: {text:?}");
    assert!(text.contains("IP65"), "only some of the layer survived: {text:?}");

    // And the page now says it has text, which is what makes it searchable.
    let after = page.classify().expect("classify");
    assert_ne!(after.kind, PageTextKind::Empty, "the page still reports no text layer");
    assert!(after.chars >= 13, "only {} characters came back", after.chars);
}

#[test]
fn the_layer_is_written_where_the_words_were_recognised() {
    // A layer that reads back correctly but sits in the wrong place looks fine
    // in a text dump and is useless on screen — the selection highlight lands
    // over the wrong line. Asserted against the boxes extraction reports.
    let Some(pdfium) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open_fixture(&pdfium, "single-page.pdf");
    doc.as_document_mut()
        .expect("editable")
        .add_text_layer(0, &[word("HERE", 50.0, 200.0, 110.0, 216.0)])
        .expect("write");

    let reopened = save_and_reopen(&pdfium, &mut doc);
    let segments = reopened.page(0).expect("page").text_segments().expect("segments");

    let found = segments
        .iter()
        .find(|s| s.text.contains("HERE"))
        .expect("the word is not in the extracted runs");

    // Within a couple of points of where it was asked for.
    assert!((found.left - 50.0).abs() < 4.0, "x drifted to {}", found.left);
    assert!(
        (found.bottom - 216.0).abs() < 8.0,
        "y drifted to {} — the layer is not over the ink it describes",
        found.bottom
    );
}

#[test]
fn an_empty_word_is_skipped_rather_than_written_as_nothing() {
    let Some(pdfium) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open_fixture(&pdfium, "single-page.pdf");
    let written = doc
        .as_document_mut()
        .expect("editable")
        .add_text_layer(
            0,
            &[word("", 0.0, 0.0, 10.0, 10.0), word("   ", 0.0, 0.0, 10.0, 10.0)],
        )
        .expect("write");

    assert_eq!(written, 0, "blank words were written into the layer");
}

#[test]
fn running_recognition_twice_replaces_the_layer_rather_than_stacking_it() {
    // The trap the OCR plan names: run it twice and every search returns
    // doubled hits.
    //
    // It does not show up where you would look for it. Writing the same layer
    // three times without removing the previous one left `text()` reporting a
    // single copy, because PDFium's extraction dedupes exactly coincident
    // glyphs — so the obvious assertion passes on a page that is genuinely
    // carrying three copies. The saved bytes are where the truth was: 2,023 for
    // one run, 3,037 for three.
    //
    // This asserts on the extracted text after a save and reopen, which is what
    // a reader and a search actually see, and it holds because `add_text_layer`
    // now removes any layer already written under its id.
    let Some(pdfium) = skip_without_pdfium() else { return };
    let _lock = serial();

    let layer = vec![RecognisedWord {
        text: "LUMINAIRE".into(),
        rect: Rect { left: 40.0, top: 80.0, right: 140.0, bottom: 96.0 },
        confidence: 0.99,
        char_confidence: Vec::new(),
    }];

    let mut doc = open_fixture(&pdfium, "text-lines.pdf");
    for _ in 0..3 {
        doc.as_document_mut().expect("editable").add_text_layer(0, &layer).expect("write");
    }

    let reopened = save_and_reopen(&pdfium, &mut doc);
    let text = reopened.page(0).expect("page").text().expect("text");

    assert_eq!(
        text.matches("LUMINAIRE").count(),
        1,
        "three runs left {} copies — every search on this page now returns \
         duplicated hits:\n{text}",
        text.matches("LUMINAIRE").count()
    );

    // What re-running *does* still cost: the removed text objects leave their
    // font resources behind, so the file grows by roughly 500 bytes a run even
    // though the text itself is replaced. Recorded rather than asserted — it is
    // bounded by how many times somebody re-recognises one page, and the fix
    // belongs in PDFium's resource pruning rather than here.
}

#[test]
fn adding_a_text_layer_changes_no_rendered_pixel() {
    // The OCR plan's acceptance test for the whole feature, and the right one:
    // it catches a render mode left at fill, a stray stroke colour, a leaked
    // clip, or a font that shifts the content stream — none of which are
    // visible by eye until someone prints the page.
    use pdf_core::document::RenderRequest;
    use pdf_core::render::{bitmap::PixelOrder, RenderTarget};

    let Some(pdfium) = skip_without_pdfium() else { return };
    let _lock = serial();

    let render = |doc: &dyn Document, scale: f32| -> (u32, u32, Vec<u8>) {
        let page = doc.page(0).expect("page");
        let (w, h) = page.size().pixel_size(scale);
        let mut pixels = vec![0u8; w as usize * h as usize * 4];
        {
            let mut target =
                RenderTarget::new(w, h, w as usize * 4, PixelOrder::Rgba, &mut pixels)
                    .expect("target");
            page.render_into(&RenderRequest { scale, ..Default::default() }, &mut target)
                .expect("render");
        }
        (w, h, pixels)
    };

    // Three scales, because a difference can hide at one and not another.
    for scale in [1.0f32, 2.0, 300.0 / 72.0] {
        let mut doc = open_fixture(&pdfium, "text-lines.pdf");
        let (w, h, before) = render(&*doc, scale);

        doc.as_document_mut()
            .expect("editable")
            .add_text_layer(
                0,
                &[RecognisedWord {
                    text: "INVISIBLE".into(),
                    rect: Rect { left: 40.0, top: 80.0, right: 160.0, bottom: 100.0 },
                    confidence: 1.0,
                    char_confidence: Vec::new(),
                }],
            )
            .expect("layer");

        let (w2, h2, after) = render(&*doc, scale);
        assert_eq!((w, h), (w2, h2), "the page changed size at scale {scale}");
        assert_eq!(
            before, after,
            "the text layer changed {} pixels at scale {scale} — it is not invisible",
            before.iter().zip(&after).filter(|(a, b)| a != b).count()
        );
    }
}

// ---------------------------------------------------------------------------
// The fixtures, and the assertion that matters most
// ---------------------------------------------------------------------------

/// **The single most damaging failure available here.**
///
/// `two-column.pdf` is well authored: its content stream emits the whole left
/// column, then the whole right one, so character order *is* reading order. The
/// trust check firing on this would send every good document through
/// reconstruction — degrading them all in the name of fixing the broken ones.
#[test]
fn a_well_authored_two_column_page_is_not_reconstructed() {
    use pdf_core::document::layout;

    let Some(pdfium) = skip_without_pdfium() else { return };
    let _lock = serial();

    let doc = open_fixture(&pdfium, "two-column.pdf");
    let glyphs = doc.page(0).expect("page").glyphs().expect("glyphs");
    assert!(glyphs.len() > 300, "only {} glyphs — is the fixture right?", glyphs.len());

    let measured = layout::trust(&glyphs);
    assert!(
        !measured.is_noteworthy(),
        "a well-authored two-column page was flagged (disorder {:.2} over {} \
         line transitions)",
        measured.disorder(),
        measured.steps
    );

    let page = layout::page_text(&glyphs, &doc.page(0).unwrap().text().unwrap());
    assert_eq!(page.source, layout::Source::CharacterOrder);
}

/// The same words, emitted in paint order — what a browser print produces.
#[test]
fn a_shredded_page_is_detected_and_rebuilt() {
    use pdf_core::document::layout;

    let Some(pdfium) = skip_without_pdfium() else { return };
    let _lock = serial();

    let doc = open_fixture(&pdfium, "shredded.pdf");
    let glyphs = doc.page(0).expect("page").glyphs().expect("glyphs");

    let measured = layout::trust(&glyphs);
    assert!(
        measured.is_noteworthy(),
        "paint order was not flagged (disorder {:.2} over {} line transitions)",
        measured.disorder(),
        measured.steps
    );

    // Flagged, and still returned as the document has it. Reflowing is asked
    // for, never assumed — four of this machine's real documents score higher
    // than this fixture and read perfectly.
    let page = layout::page_text(&glyphs, &doc.page(0).unwrap().text().unwrap());
    assert_eq!(page.source, layout::Source::CharacterOrder);
}

/// The pair is the point: identical content, different order, opposite verdicts.
#[test]
fn the_two_fixtures_differ_only_in_order() {
    let Some(pdfium) = skip_without_pdfium() else { return };
    let _lock = serial();

    let ordered = open_fixture(&pdfium, "two-column.pdf");
    let shredded = open_fixture(&pdfium, "shredded.pdf");

    let letters = |doc: &Box<dyn Document>| -> String {
        let mut chars: Vec<char> = doc
            .page(0)
            .unwrap()
            .glyphs()
            .unwrap()
            .iter()
            .map(|g| g.ch)
            .collect();
        chars.sort_unstable();
        chars.into_iter().collect()
    };

    assert_eq!(
        letters(&ordered),
        letters(&shredded),
        "the fixtures carry different text, so the comparison proves nothing"
    );
}

#[test]
fn reconstruction_recovers_the_columns_from_a_shredded_page() {
    use pdf_core::document::layout;

    let Some(pdfium) = skip_without_pdfium() else { return };
    let _lock = serial();

    let doc = open_fixture(&pdfium, "shredded.pdf");
    let glyphs = doc.page(0).expect("page").glyphs().expect("glyphs");
    let rebuilt = layout::reconstruct(&glyphs);

    assert_eq!(rebuilt.blocks.len(), 2, "the gutter between the columns was not found");

    let first = rebuilt.blocks[0]
        .lines
        .iter()
        .map(|l| l.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        first.contains("luminaire housing") || first.contains("housing"),
        "the left column did not come back first: {first:?}"
    );
    assert!(
        !first.contains("Control gear"),
        "the columns interleaved: {first:?}"
    );
}

#[test]
fn a_page_of_outlined_type_is_not_reported_as_empty() {
    // It reads as text and selects as nothing. Calling it Empty sends it
    // nowhere; calling it Scanned sends it to OCR, which is a quality loss for
    // glyphs whose exact contours are sitting right there.
    let Some(pdfium) = skip_without_pdfium() else { return };
    let _lock = serial();

    let doc = open_fixture(&pdfium, "outlined.pdf");
    let verdict = doc.page(0).expect("page").classify().expect("classify");

    assert_eq!(verdict.kind, PageTextKind::Outlined);
    assert_eq!(verdict.chars, 0, "outlined type reported characters");
    assert!(verdict.glyph_paths > 100, "only {} glyph paths", verdict.glyph_paths);
}

#[test]
fn a_scan_is_scanned_at_every_resolution() {
    // Low resolution must not turn a scan into something else — confidence
    // should fall, not the verdict.
    let Some(pdfium) = skip_without_pdfium() else { return };
    let _lock = serial();

    for name in ["scan-300dpi.pdf", "scan-skewed.pdf", "scan-lowdpi.pdf"] {
        let doc = open_fixture(&pdfium, name);
        let verdict = doc.page(0).expect("page").classify().expect("classify");

        assert_eq!(verdict.kind, PageTextKind::Scanned, "{name} was not seen as a scan");
        assert!(verdict.image_coverage > 0.9, "{name} covers only {:.0}%", verdict.image_coverage * 100.0);
        assert_eq!(verdict.chars, 0, "{name} has a text layer already");
    }
}

/// OCR must never run on a page that already has text.
///
/// Recognising good text replaces accurate characters with recognised ones.
/// That is a data-loss bug wearing a feature's clothes, and the classifier is
/// the only thing standing in front of it.
#[test]
fn a_page_with_text_would_be_refused_by_the_gate() {
    let Some(pdfium) = skip_without_pdfium() else { return };
    let _lock = serial();

    for name in ["text-lines.pdf", "two-column.pdf", "shredded.pdf"] {
        let doc = open_fixture(&pdfium, name);
        let kind = doc.page(0).expect("page").classify().expect("classify").kind;

        assert!(
            matches!(kind, PageTextKind::Native | PageTextKind::Hybrid),
            "{name} classified as {kind:?} — recognition would run over its text"
        );
    }
}

/// The measurement that made reconstruction a request rather than a decision.
///
/// Real documents on this machine scored **above** the deliberately scrambled
/// fixture — a CAD drawing at 0.637, the 149-page catalogue at 0.393 with 36
/// pages past any workable threshold — and every one of them extracts
/// correctly. The metric cannot tell paint order from positioned layout.
#[test]
fn a_positioned_layout_is_never_silently_reordered() {
    use pdf_core::document::layout;

    let Some(pdfium) = skip_without_pdfium() else { return };
    let _lock = serial();

    // `quadrants.pdf` is four coloured blocks: no linear reading order at all,
    // which is the shape a drawing has.
    for name in ["two-column.pdf", "shredded.pdf", "text-lines.pdf"] {
        let doc = open_fixture(&pdfium, name);
        let glyphs = doc.page(0).expect("page").glyphs().expect("glyphs");
        let page = layout::page_text(&glyphs, &doc.page(0).unwrap().text().unwrap());

        assert_eq!(
            page.source,
            layout::Source::CharacterOrder,
            "{name} was reordered without being asked"
        );
    }
}

/// Editing the words already on a page.
///
/// The unit is a **run**, which is what the file actually holds — one may be a
/// paragraph or a single letter that needed different spacing. Anything finer
/// means rewriting the content stream; anything coarser is a guess about which
/// runs belong together.
mod editing {
    use super::*;
    use pdf_core::document::{Color, TextStyle};

    #[test]
    fn the_runs_on_a_page_can_be_read() {
        let Some(pdfium) = skip_without_pdfium() else { return };
        let _lock = serial();
        let doc = open_fixture(&pdfium, "text-lines.pdf");

        let runs = doc.text_runs(0).expect("read runs");
        assert!(!runs.is_empty(), "a page of text reported no runs");
        assert!(
            runs.iter().all(|r| !r.text.trim().is_empty()),
            "a blank run was offered for editing"
        );
        assert!(
            runs.iter().all(|r| r.rect.right > r.rect.left),
            "a run has no width, so nothing could be clicked on"
        );
    }

    #[test]
    fn a_run_can_be_rewritten_and_reads_back_changed() {
        let Some(pdfium) = skip_without_pdfium() else { return };
        let _lock = serial();
        let mut doc = open_fixture(&pdfium, "text-lines.pdf");

        let (object, before) = {
            let runs = doc.text_runs(0).expect("runs");
            (runs[0].object, runs[0].text.clone())
        };
        assert_ne!(before.trim(), "PAGIFY", "the fixture already says the new text");

        doc.as_document_mut()
            .expect("editable")
            .set_text_run(0, object, "PAGIFY")
            .expect("rewrite");

        let after = doc.text_runs(0).expect("runs");
        assert_eq!(after[0].text.trim(), "PAGIFY", "the run did not change");
    }

    /// The change has to reach the **content stream**, not merely PDFium's
    /// object model — otherwise it survives until the save and then vanishes.
    #[test]
    fn a_rewritten_run_survives_a_save() {
        let Some(pdfium) = skip_without_pdfium() else { return };
        let _lock = serial();
        let mut doc = open_fixture(&pdfium, "text-lines.pdf");

        let object = doc.text_runs(0).expect("runs")[0].object;
        doc.as_document_mut()
            .expect("editable")
            .set_text_run(0, object, "PAGIFY")
            .expect("rewrite");

        let reopened = harness::save_full_copy_and_reopen(&mut doc);
        let text = reopened.page(0).expect("page").text().expect("text");
        assert!(text.contains("PAGIFY"), "the edit did not survive the save:\n{text}");
    }

    /// **The double-handle bug.**
    ///
    /// Reading the previous text used to open the page a *second* time while
    /// the write held it open — two live `FPDF_PAGE` handles for one page, one
    /// of them regenerating the content stream. PDFium does not arbitrate
    /// between them, and a page comes back with its text drawn twice in two
    /// different fonts.
    ///
    /// Caught by counting the objects: a page that gains text objects from an
    /// edit that replaced one is a page that has been drawn on twice.
    #[test]
    fn rewriting_a_run_does_not_add_objects_to_the_page() {
        let Some(pdfium) = skip_without_pdfium() else { return };
        let _lock = serial();
        let mut doc = open_fixture(&pdfium, "text-lines.pdf");

        let before = doc.text_runs(0).expect("runs").len();
        let object = doc.text_runs(0).expect("runs")[0].object;

        doc.as_document_mut()
            .expect("editable")
            .set_text_run(0, object, "PAGIFY")
            .expect("rewrite");

        let after = doc.text_runs(0).expect("runs").len();
        assert_eq!(after, before, "the edit left {} runs where there were {before}", after);

        // And once more, because the damage compounds.
        let object = doc.text_runs(0).expect("runs")[0].object;
        doc.as_document_mut()
            .expect("editable")
            .set_text_run(0, object, "AGAIN")
            .expect("rewrite twice");
        assert_eq!(doc.text_runs(0).expect("runs").len(), before, "editing twice grew the page");
    }

    /// Undo needs what was actually replaced, and it is read through the same
    /// handle that does the writing.
    #[test]
    fn the_write_reports_the_words_it_replaced() {
        let Some(pdfium) = skip_without_pdfium() else { return };
        let _lock = serial();
        let mut doc = open_fixture(&pdfium, "text-lines.pdf");

        let (object, before) = {
            let runs = doc.text_runs(0).expect("runs");
            (runs[0].object, runs[0].text.clone())
        };
        let reported = doc
            .as_document_mut()
            .expect("editable")
            .set_text_run(0, object, "PAGIFY")
            .expect("rewrite");

        assert_eq!(
            reported.trim(),
            before.trim(),
            "the write reported {reported:?} as the previous text, but it was {before:?}"
        );
    }

    /// **The white-text bug.**
    ///
    /// `FPDFText_SetText` writes the object afresh and does not carry the fill
    /// colour with it. White text on a dark banner came back black — which is
    /// to say it vanished. An edit must change what it was asked to change and
    /// nothing else.
    #[test]
    fn rewriting_a_run_keeps_the_colour_it_was_drawn_in() {
        let Some(pdfium) = skip_without_pdfium() else { return };
        let _lock = serial();
        let mut doc = open_fixture(&pdfium, "text-lines.pdf");

        let object = doc.text_runs(0).expect("runs")[0].object;

        // Make it unmistakably not-black first, so a reset to the default is
        // visible rather than a coincidence.
        let white = Color { r: 255, g: 255, b: 255, a: 255 };
        doc.as_document_mut()
            .expect("editable")
            .set_text_run_styled(0, object, "BEFORE", &TextStyle { color: Some(white), ..Default::default() })
            .expect("colour it");
        assert_eq!(doc.text_runs(0).expect("runs")[0].color, white);

        // Now an ordinary edit, which asks for no colour at all.
        doc.as_document_mut()
            .expect("editable")
            .set_text_run(0, object, "AFTER")
            .expect("rewrite");

        assert_eq!(
            doc.text_runs(0).expect("runs")[0].color,
            white,
            "the edit reset the colour it was not asked to change"
        );
    }

    #[test]
    fn a_run_can_be_recoloured_and_moved() {
        let Some(pdfium) = skip_without_pdfium() else { return };
        let _lock = serial();
        let mut doc = open_fixture(&pdfium, "text-lines.pdf");

        let (object, was) = {
            let runs = doc.text_runs(0).expect("runs");
            (runs[0].object, runs[0].rect)
        };
        let red = Color { r: 220, g: 20, b: 20, a: 255 };

        doc.as_document_mut()
            .expect("editable")
            .set_text_run_styled(
                0,
                object,
                "MOVED",
                &TextStyle { color: Some(red), at: Some((120.0, 200.0)), ..Default::default() },
            )
            .expect("restyle");

        let run = &doc.text_runs(0).expect("runs")[0];
        assert_eq!(run.color, red, "the colour did not change");
        assert!(
            (run.rect.left - was.left).abs() > 1.0 || (run.rect.top - was.top).abs() > 1.0,
            "the run did not move: {:?} then {:?}",
            was,
            run.rect
        );
    }

    /// Undo has to restore the **appearance**, not only the words. A run that
    /// came back with its old text in a new colour would still be wrong.
    #[test]
    fn undoing_an_edit_restores_the_colour_as_well_as_the_words() {
        let Some(pdfium) = skip_without_pdfium() else { return };
        let _lock = serial();
        let mut doc = open_fixture(&pdfium, "text-lines.pdf");
        let object = doc.text_runs(0).expect("runs")[0].object;

        let white = Color { r: 255, g: 255, b: 255, a: 255 };
        doc.as_document_mut()
            .expect("editable")
            .set_text_run_styled(0, object, "WHITE", &TextStyle { color: Some(white), ..Default::default() })
            .expect("colour it");

        // An edit that also recolours, then put back.
        let red = Color { r: 220, g: 20, b: 20, a: 255 };
        let (words, appearance) = doc
            .as_document_mut()
            .expect("editable")
            .set_text_run_styled(0, object, "RED", &TextStyle { color: Some(red), ..Default::default() })
            .expect("recolour");
        assert_eq!(words.trim(), "WHITE");
        assert_eq!(appearance.color, Some(white), "the write did not report the old colour");

        doc.as_document_mut()
            .expect("editable")
            .set_text_run_styled(0, object, &words, &appearance)
            .expect("undo");

        let run = &doc.text_runs(0).expect("runs")[0];
        assert_eq!(run.text.trim(), "WHITE", "the words did not come back");
        assert_eq!(run.color, white, "the colour did not come back");
    }

    #[test]
    fn an_object_that_is_not_text_is_refused() {
        let Some(pdfium) = skip_without_pdfium() else { return };
        let _lock = serial();
        let mut doc = open_fixture(&pdfium, "text-lines.pdf");
        let far = doc.text_runs(0).expect("runs").len() + 9_000;

        assert!(
            doc.as_document_mut().expect("editable").set_text_run(0, far, "x").is_err(),
            "an object that is not there was accepted"
        );
    }
}
