//! open → command → save → reopen → assert.
//!
//! The primary defence for the whole write path. A command that appears to work
//! but does not survive a save is the most common editing bug there is, and it is
//! invisible to any test that stops before the reopen.
//!
//! **Written before the code they test**, so a week of page-tree wiring has a
//! signal from day one rather than day five. They are `#[ignore]`d rather than
//! left red: a suite that always fails teaches people to stop reading it. Run the
//! remaining work with `cargo test -- --ignored`, and delete an `#[ignore]` as
//! each operation lands.
//!
//! `fixtures_are_present_and_readable` is **not** ignored, and it is the one that
//! matters most right now: it separates "not implemented yet" from "the fixtures
//! or the binding are broken", which look identical from a failure message.
//!
//! Requires a desktop PDFium of the pinned build:
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=/tmp/pdfw/bin/pdfium.dll cargo test --test round_trip
//! ```
//!
//! Every fixture page has a distinct width, so the page tree alone says which
//! page ended up where — no text, no pixels.

use pdf_core::command::{Command, CommandHistory};
use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Annotation, Color, Document, Point, Rect};
use pdf_core::registry;

mod harness;
use harness::{open_fixture, page_widths, save_and_reopen, skip_without_pdfium};

#[test]
fn fixtures_are_present_and_readable() {
    let Some(pdfium) = skip_without_pdfium() else {
        return;
    };
    let _serial = harness::serial();
    let doc = open_fixture(&pdfium, "pages-ladder.pdf");
    assert_eq!(5, doc.page_count());
    assert_eq!(
        vec![200, 250, 300, 350, 400],
        page_widths(&doc),
        "the ladder is what every other test reads positions from",
    );
}

#[test]
fn a_deleted_page_is_still_gone_after_a_save() {
    let Some(pdfium) = skip_without_pdfium() else {
        return;
    };
    let _serial = harness::serial();
    let mut doc = open_fixture(&pdfium, "pages-ladder.pdf");

    let mut history = CommandHistory::default();
    history
        .execute(
            Command::DeletePage { index: 2 },
            doc.as_document_mut().expect("the document must be mutable"),
        )
        .expect("delete");

    let reopened = save_and_reopen(&pdfium, &mut doc);
    assert_eq!(vec![200, 250, 350, 400], page_widths(&reopened));
}

#[test]
fn a_reorder_survives_a_save() {
    let Some(pdfium) = skip_without_pdfium() else {
        return;
    };
    let _serial = harness::serial();
    let mut doc = open_fixture(&pdfium, "pages-ladder.pdf");

    let mut history = CommandHistory::default();
    history
        .execute(
            // A rotation, not a swap: a swap is its own inverse and would pass
            // even if the permutation were applied backwards.
            Command::ReorderPages {
                order: vec![1, 2, 3, 4, 0],
            },
            doc.as_document_mut().expect("mutable"),
        )
        .expect("reorder");

    let reopened = save_and_reopen(&pdfium, &mut doc);
    assert_eq!(vec![400, 200, 250, 300, 350], page_widths(&reopened));
}

#[test]
fn an_inserted_blank_page_survives_a_save() {
    let Some(pdfium) = skip_without_pdfium() else {
        return;
    };
    let _serial = harness::serial();
    let mut doc = open_fixture(&pdfium, "pages-ladder.pdf");

    let mut history = CommandHistory::default();
    history
        .execute(
            Command::InsertBlankPage {
                at: 1,
                width_pt: 999.0,
                height_pt: 400.0,
            },
            doc.as_document_mut().expect("mutable"),
        )
        .expect("insert");

    let reopened = save_and_reopen(&pdfium, &mut doc);
    assert_eq!(vec![200, 999, 250, 300, 350, 400], page_widths(&reopened));
}

/// Undo has to be reversible *through* a save, not merely in memory.
#[test]
fn a_deletion_undone_before_saving_leaves_the_document_untouched() {
    let Some(pdfium) = skip_without_pdfium() else {
        return;
    };
    let _serial = harness::serial();
    let mut doc = open_fixture(&pdfium, "pages-ladder.pdf");

    let mut history = CommandHistory::default();
    history
        .execute(
            Command::DeletePage { index: 0 },
            doc.as_document_mut().expect("mutable"),
        )
        .expect("delete");
    history
        .undo(doc.as_document_mut().expect("mutable"))
        .expect("undo")
        .expect("something to undo");

    let reopened = save_and_reopen(&pdfium, &mut doc);
    assert_eq!(vec![200, 250, 300, 350, 400], page_widths(&reopened));
}

/// The §4.1 property, and the one the easy implementation quietly breaks.
///
/// `pdfium-render`'s `save_to_writer` hardcodes its flags to zero, which is a full
/// rewrite — measured in `examples/save_probe.rs`. A save built on it would pass
/// every other test in this file while relocating every object in the file, which
/// is exactly what destroys a signature's byte range.
#[test]
fn an_incremental_save_appends_and_leaves_the_original_bytes_alone() {
    let Some(pdfium) = skip_without_pdfium() else {
        return;
    };
    let _serial = harness::serial();
    let original = harness::fixture_bytes("pages-ladder.pdf");
    let mut doc = open_fixture(&pdfium, "pages-ladder.pdf");

    let mut history = CommandHistory::default();
    history
        .execute(
            Command::SetPageRotation {
                index: 0,
                quarter_turns: 1,
            },
            doc.as_document_mut().expect("mutable"),
        )
        .expect("rotate");

    let mut saved = Vec::new();
    doc.as_document_mut()
        .expect("mutable")
        .save_incremental(&mut saved)
        .expect("save");

    assert!(
        saved.len() > original.len(),
        "an incremental save appends; it cannot come out smaller",
    );
    assert_eq!(
        original[..],
        saved[..original.len()],
        "the original bytes must survive verbatim, or every signature over them breaks",
    );
}

// ------------------------------------------------------- what works today --
//
// The operations below reach PDFium through the binding's own safe API, so they
// are live rather than ignored. They go through the same command stack as
// everything above — the point is that the plumbing is proven end to end while
// the three blocked operations wait on one line upstream.

#[test]
fn an_inserted_page_survives_a_full_copy_save() {
    let Some(pdfium) = skip_without_pdfium() else {
        return;
    };
    let _serial = harness::serial();
    let mut doc = open_fixture(&pdfium, "pages-ladder.pdf");

    let mut history = CommandHistory::default();
    history
        .execute(
            Command::InsertBlankPage {
                at: 1,
                width_pt: 999.0,
                height_pt: 400.0,
            },
            doc.as_document_mut().expect("mutable"),
        )
        .expect("insert");

    let reopened = harness::save_full_copy_and_reopen(&mut doc);
    assert_eq!(vec![200, 999, 250, 300, 350, 400], page_widths(&reopened));
}

/// Rotation is the operation whose undo record was wrong until the engine could
/// report the prior value: without it, undo could only ever restore zero, which
/// looks correct exactly when the page started unrotated.
#[test]
fn a_rotation_survives_a_save_and_undoes_to_what_was_there_before() {
    let Some(pdfium) = skip_without_pdfium() else {
        return;
    };
    let _serial = harness::serial();
    let mut doc = open_fixture(&pdfium, "pages-ladder.pdf");

    let mut history = CommandHistory::default();
    // Two turns in a row, so the second one's undo has something other than zero
    // to restore. A single rotation from an unrotated page cannot tell the two
    // implementations apart.
    for turns in [1u8, 3u8] {
        history
            .execute(
                Command::SetPageRotation {
                    index: 2,
                    quarter_turns: turns,
                },
                doc.as_document_mut().expect("mutable"),
            )
            .expect("rotate");
    }

    let mut saved = harness::save_full_copy_and_reopen(&mut doc);
    assert_eq!(
        3,
        saved
            .as_document_mut()
            .expect("mutable")
            .page_rotation(2)
            .expect("rotation"),
        "the rotation has to be written, not merely applied in memory",
    );

    history
        .undo(doc.as_document_mut().expect("mutable"))
        .expect("undo")
        .expect("something to undo");
    assert_eq!(
        1,
        doc.as_document_mut()
            .expect("mutable")
            .page_rotation(2)
            .expect("rotation"),
        "undo must restore the previous rotation, not zero",
    );
}

#[test]
fn extracting_pages_produces_a_document_of_just_those_pages() {
    let Some(pdfium) = skip_without_pdfium() else {
        return;
    };
    let _serial = harness::serial();
    let mut doc = open_fixture(&pdfium, "pages-ladder.pdf");

    let extracted = doc
        .as_document_mut()
        .expect("mutable")
        .extract_pages(&[3, 1])
        .expect("extract");

    assert_eq!(
        vec![350, 250],
        page_widths(&extracted),
        "extraction keeps the order it was asked for, not the source order",
    );
}

#[test]
fn a_document_is_clean_until_something_changes_it() {
    let Some(pdfium) = skip_without_pdfium() else {
        return;
    };
    let _serial = harness::serial();
    let mut doc = open_fixture(&pdfium, "single-page.pdf");
    assert!(!doc.as_document_mut().expect("mutable").is_dirty());

    doc.as_document_mut()
        .expect("mutable")
        .set_page_rotation(0, 1)
        .expect("rotate");
    assert!(doc.as_document_mut().expect("mutable").is_dirty());
}

/// The three operations the vendored patch unblocked, all in one place: they
/// used to return an error naming `PdfDocument::handle()`, and the point of the
/// patch was that one line reaches all three.
#[test]
fn the_operations_the_patch_unblocked_all_work() {
    let Some(pdfium) = skip_without_pdfium() else {
        return;
    };
    let _serial = harness::serial();
    let mut doc = open_fixture(&pdfium, "pages-ladder.pdf");
    let mutable = doc.as_document_mut().expect("mutable");

    let removed = mutable.delete_page(0).expect("delete");
    assert_eq!(200.0, removed.size.width_pt.round());

    mutable.reorder_pages(&[1, 0, 2, 3]).expect("reorder");
    mutable
        .save_incremental(&mut Vec::new())
        .expect("incremental save");
}

/// Opening documents from several threads at once must not corrupt them.
///
/// Deliberately goes through `registry`, not `PdfiumDocument`, because the
/// registry is where the fix lives: `insert_with` holds the registry lock across
/// the PDFium open, so no document is ever constructed while another is being
/// built or torn down at an address PDFium has recycled.
///
/// Measured without that, by `examples/handle_race_probe.rs`: 771 of 800 opens
/// failed outright and 3 of the 29 reads that got through returned another
/// document's page sizes — one of them reproducing `[200, 612, 612, 612]`, the
/// exact failure that first appeared in this suite.
///
/// This cannot prove the absence of a race and does not claim to; it fails loudly
/// against the unserialised version, which is what makes it worth having.
///
/// `harness::serial()` is still taken. It excludes the *other* tests, which drive
/// `PdfiumDocument` directly and so are not covered by the registry lock — the two
/// spawned threads below are unaffected by it, and they are the concurrency under
/// test.
#[test]
fn opening_documents_concurrently_gives_each_one_its_own_pages() {
    let Some(_pdfium) = skip_without_pdfium() else {
        return;
    };
    let _serial = harness::serial();

    let ladder = harness::fixture_path("pages-ladder.pdf");
    let mixed = harness::fixture_path("mixed-sizes.pdf");

    // Two different files, so a mix-up is visible as the wrong page sizes rather
    // than hiding behind identical ones.
    let expectations = [
        (ladder, vec![200, 250, 300, 350, 400]),
        (mixed, vec![595, 420, 612, 842]),
    ];

    let threads: Vec<_> = expectations
        .into_iter()
        .map(|(path, expected)| {
            std::thread::spawn(move || {
                for round in 0..40 {
                    let path = path.to_str().expect("fixture path").to_owned();
                    let handle = registry::insert_with(move || {
                        Ok(Box::new(PdfiumDocument::open_path(&path, None)?) as Box<dyn Document>)
                    })
                    .unwrap_or_else(|e| panic!("round {round}: open failed: {e}"));

                    let widths = registry::with_session(handle, |session| {
                        Ok((0..session.document.page_count())
                            .map(|i| {
                                session
                                    .document
                                    .page_size(i)
                                    .map(|s| s.width_pt.round() as i32)
                                    .unwrap_or(-1)
                            })
                            .collect::<Vec<_>>())
                    })
                    .expect("read page sizes");

                    assert_eq!(
                        expected, widths,
                        "round {round} read another document's page geometry",
                    );
                    registry::remove(handle);
                }
            })
        })
        .collect();

    for thread in threads {
        thread.join().expect("a thread saw the wrong document");
    }
}

/// A mark survives a save and a reopen, which is the whole point of the feature.
///
/// Asserting against `annotation_count` rather than reading the mark back through
/// our own model on purpose: the count comes from PDFium's view of the page, so a
/// pass means the annotation is really in the file, not merely that we can parse
/// what we just wrote.
#[test]
fn a_highlight_is_still_there_after_a_save_and_reopen() {
    let Some(pdfium) = skip_without_pdfium() else {
        return;
    };
    let _serial = harness::serial();

    let mut doc = open_fixture(&pdfium, "pages-ladder.pdf");
    assert_eq!(
        0,
        doc.annotation_count(1).expect("count"),
        "the fixture must start clean or this test proves nothing",
    );

    let mut history = CommandHistory::default();
    history
        .execute(
            Command::AddAnnotation {
                page_index: 1,
                annotation: Annotation::Highlight {
                    rects: vec![Rect {
                        left: 20.0,
                        top: 30.0,
                        right: 180.0,
                        bottom: 44.0,
                    }],
                    color: Color {
                        r: 255,
                        g: 224,
                        b: 102,
                        a: 128,
                    },
                },
            },
            doc.as_document_mut().expect("mutable"),
        )
        .expect("add the highlight");

    assert_eq!(1, doc.annotation_count(1).expect("count"));

    let reopened = save_and_reopen(&pdfium, &mut doc);
    assert_eq!(
        1,
        reopened.annotation_count(1).expect("count"),
        "the mark did not survive the save",
    );
    // And it landed on the page it was put on, not on every page.
    assert_eq!(0, reopened.annotation_count(0).expect("count"));
}

/// Ink is a different write path — strokes rather than quad points — so it gets
/// its own round trip rather than riding on the highlight's.
#[test]
fn a_drawn_stroke_survives_a_save() {
    let Some(pdfium) = skip_without_pdfium() else {
        return;
    };
    let _serial = harness::serial();

    let mut doc = open_fixture(&pdfium, "single-page.pdf");
    let stroke: Vec<Point> = (0..12)
        .map(|i| Point {
            x: 20.0 + i as f32 * 8.0,
            y: 50.0 + (i % 3) as f32 * 4.0,
        })
        .collect();

    doc.as_document_mut()
        .expect("mutable")
        .add_annotation(
            0,
            &Annotation::Ink {
                strokes: vec![stroke],
                color: Color {
                    r: 0,
                    g: 0,
                    b: 255,
                    a: 255,
                },
                width: 3.0,
            },
        )
        .expect("add ink");

    let reopened = save_and_reopen(&pdfium, &mut doc);
    assert_eq!(1, reopened.annotation_count(0).expect("count"));
}

/// Undoing a mark before saving leaves the file with nothing in it.
///
/// The counterpart to the page-tree version of this test: an edit that was undone
/// must not reach the document at all, or undo is only a UI illusion.
#[test]
fn a_mark_undone_before_saving_never_reaches_the_file() {
    let Some(pdfium) = skip_without_pdfium() else {
        return;
    };
    let _serial = harness::serial();

    let mut doc = open_fixture(&pdfium, "single-page.pdf");
    let mut history = CommandHistory::default();

    history
        .execute(
            Command::AddAnnotation {
                page_index: 0,
                annotation: Annotation::Note {
                    rect: Rect {
                        left: 10.0,
                        top: 10.0,
                        right: 30.0,
                        bottom: 30.0,
                    },
                    contents: "temporary".into(),
                    color: Color {
                        r: 255,
                        g: 255,
                        b: 0,
                        a: 255,
                    },
                },
            },
            doc.as_document_mut().expect("mutable"),
        )
        .expect("add the note");
    assert_eq!(1, doc.annotation_count(0).expect("count"));

    history
        .undo(doc.as_document_mut().expect("mutable"))
        .expect("undo")
        .expect("something to undo");

    assert_eq!(0, doc.annotation_count(0).expect("count"));

    let reopened = save_and_reopen(&pdfium, &mut doc);
    assert_eq!(
        0,
        reopened.annotation_count(0).expect("count"),
        "an undone mark was written to the file anyway",
    );
}

/// A saved mark comes back with its geometry, which is what makes it erasable.
///
/// Reading is not merely the inverse of writing here: the mark goes out through
/// quad points in PDF space and comes back through them, so an error in either
/// direction — or a disagreement between the two about the CropBox — shows up as
/// a rect in the wrong place rather than as a failure.
#[test]
fn a_saved_highlight_can_be_read_back_where_it_was_put() {
    let Some(pdfium) = skip_without_pdfium() else {
        return;
    };
    let _serial = harness::serial();

    let mut doc = open_fixture(&pdfium, "pages-ladder.pdf");
    let placed = Rect {
        left: 20.0,
        top: 30.0,
        right: 180.0,
        bottom: 44.0,
    };

    doc.as_document_mut()
        .expect("mutable")
        .add_annotation(
            1,
            &Annotation::Highlight {
                rects: vec![placed],
                color: Color {
                    r: 255,
                    g: 224,
                    b: 102,
                    a: 128,
                },
            },
        )
        .expect("add");

    let reopened = save_and_reopen(&pdfium, &mut doc);
    let marks = reopened.annotations(1).expect("read back");
    assert_eq!(1, marks.len(), "exactly the one mark that was written");

    match &marks[0].annotation {
        Annotation::Highlight { rects, .. } => {
            assert_eq!(1, rects.len());
            let r = rects[0];
            // Within a point: the round trip goes through f32 PDF coordinates.
            assert!((r.left - placed.left).abs() < 1.0, "left was {}", r.left);
            assert!((r.top - placed.top).abs() < 1.0, "top was {}", r.top);
            assert!(
                (r.right - placed.right).abs() < 1.0,
                "right was {}",
                r.right
            );
            assert!(
                (r.bottom - placed.bottom).abs() < 1.0,
                "bottom was {}",
                r.bottom,
            );
        }
        other => panic!("read back as {other:?}, not a highlight"),
    }
}

/// Erasing a saved mark works, and undoing the erase puts it back.
///
/// This is the whole reason reading exists: the undo record needs the mark
/// itself, so an erase without a read is an erase that cannot be undone.
#[test]
fn a_saved_mark_can_be_erased_and_the_erase_undone() {
    let Some(pdfium) = skip_without_pdfium() else {
        return;
    };
    let _serial = harness::serial();

    let mut doc = open_fixture(&pdfium, "single-page.pdf");
    doc.as_document_mut()
        .expect("mutable")
        .add_annotation(
            0,
            &Annotation::Ink {
                strokes: vec![vec![
                    Point { x: 10.0, y: 20.0 },
                    Point { x: 40.0, y: 25.0 },
                    Point { x: 70.0, y: 20.0 },
                ]],
                color: Color {
                    r: 0,
                    g: 0,
                    b: 255,
                    a: 255,
                },
                width: 3.0,
            },
        )
        .expect("add");

    let mut history = CommandHistory::default();
    history
        .execute(
            Command::RemoveAnnotation {
                page_index: 0,
                index: 0,
            },
            doc.as_document_mut().expect("mutable"),
        )
        .expect("erase");
    assert_eq!(0, doc.annotation_count(0).expect("count"));

    history
        .undo(doc.as_document_mut().expect("mutable"))
        .expect("undo")
        .expect("something to undo");

    let marks = doc.annotations(0).expect("read back");
    assert_eq!(1, marks.len(), "the erased mark did not come back");
    match &marks[0].annotation {
        Annotation::Ink { strokes, .. } => {
            assert_eq!(1, strokes.len());
            assert_eq!(
                3,
                strokes[0].len(),
                "the stroke lost points on the way back"
            );
            assert!((strokes[0][0].x - 10.0).abs() < 1.0);
            assert!((strokes[0][0].y - 20.0).abs() < 1.0);
        }
        other => panic!("came back as {other:?}, not ink"),
    }
}

/// What this engine writes is a valid PDF, according to a reader that did not
/// write it.
///
/// The gap this closes: every other test here saves with PDFium and reopens with
/// PDFium. That proves the command's effect was written and says nothing about
/// whether the file is well-formed, because PDFium reconstructs a broken
/// cross-reference table without complaint.
///
/// `xref-stream.pdf` is the fixture that matters. The others use classic `xref`
/// tables and saved cleanly throughout, while `FPDF_INCREMENTAL` on a document
/// with a cross-reference *stream* wrote a file qpdf called damaged — with no edit
/// at all. Modern PDFs use streams, so that was most real documents.
#[test]
fn every_save_path_produces_a_file_an_external_reader_accepts() {
    let Some(pdfium) = skip_without_pdfium() else {
        return;
    };
    let _serial = harness::serial();

    for fixture in [
        "xref-stream.pdf",
        "pages-ladder.pdf",
        "mixed-sizes.pdf",
        "single-page.pdf",
        // Carries content streams, which the others do not. The repair that
        // closes a trailing xref stream keys on `stream`, so a fixture set with
        // no streams in it cannot exercise that branch at all — the same shape of
        // gap that let the xref-stream defect through in the first place.
        "text-lines.pdf",
        "quadrants.pdf",
    ] {
        // Untouched first. An incremental save of a document nobody has edited
        // must still be a valid file, and that is the case that was broken.
        let mut doc = open_fixture(&pdfium, fixture);
        harness::check_both_saves(&mut doc, &format!("{fixture}-untouched"));

        // Then with each kind of edit, since they append different objects.
        let mut edited = open_fixture(&pdfium, fixture);
        edited
            .as_document_mut()
            .expect("mutable")
            .set_page_rotation(0, 1)
            .expect("rotate");
        edited
            .as_document_mut()
            .expect("mutable")
            .add_annotation(
                0,
                &Annotation::Highlight {
                    rects: vec![Rect {
                        left: 10.0,
                        top: 10.0,
                        right: 60.0,
                        bottom: 24.0,
                    }],
                    color: Color {
                        r: 255,
                        g: 224,
                        b: 102,
                        a: 128,
                    },
                },
            )
            .expect("annotate");
        harness::check_both_saves(&mut edited, &format!("{fixture}-edited"));
    }
}

/// A mark's colour has to come back the colour it went in.
///
/// Measured on a phone: a red arrow drawn, saved and reopened came back yellow.
/// PDFium refuses to report `/C` for any annotation carrying an appearance
/// stream — which every mark this engine writes has, because that is what makes
/// other viewers draw it — so the colour is recorded under a key of our own and
/// read from there first.
#[test]
fn a_marks_colour_survives_a_save() {
    let Some(pdfium) = skip_without_pdfium() else {
        return;
    };
    let _serial = harness::serial();

    let mut doc = open_fixture(&pdfium, "single-page.pdf");
    let crimson = Color {
        r: 224,
        g: 49,
        b: 49,
        a: 255,
    };

    let mut history = CommandHistory::default();
    history
        .execute(
            Command::AddAnnotation {
                page_index: 0,
                annotation: Annotation::Ink {
                    strokes: vec![vec![
                        Point { x: 100.0, y: 100.0 },
                        Point { x: 200.0, y: 150.0 },
                    ]],
                    color: crimson,
                    width: 2.0,
                },
            },
            doc.as_document_mut().expect("mutable"),
        )
        .expect("add the mark");

    // Rendered before saving, which is what the app does and what makes this
    // reproduce: PDFium generates and stores an appearance stream when it draws
    // an annotation, and from then on it refuses to report `/C`.
    doc.page(0)
        .expect("page")
        .render_region(&pdf_core::document::RegionRequest {
            crop: Rect {
                left: 0.0,
                top: 0.0,
                right: 300.0,
                bottom: 300.0,
            },
            scale: 1.0,
            render_annotations: true,
            render_form_data: true,
        })
        .expect("render the page");

    let reopened = save_and_reopen(&pdfium, &mut doc);
    let marks = reopened.annotations(0).expect("read them back");

    let colour = match marks.first().map(|m| &m.annotation) {
        Some(Annotation::Ink { color, .. }) => *color,
        other => panic!("expected one ink mark, got {other:?}"),
    };
    assert_eq!(
        crimson, colour,
        "the mark changed colour on the way through"
    );
}
