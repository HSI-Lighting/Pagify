//! Phase 8's acceptance test: does the markup survive a save?
//!
//! The build plan is explicit about how to check. "Reopen, rebuild, keep
//! editing — this is the acceptance test, and it should be automated", and
//! "follow the house rule: read the file back off disk with none of your own
//! code in the way, and assert on what is actually there."
//!
//! So these write real files to a temporary directory, drop every handle, and
//! open them again from scratch. Nothing is asserted about an in-memory object
//! that never went anywhere.

use std::path::PathBuf;

use cad_kernel::{Arc, Circle, Geom, Line, Vec2};
use pagify_shell::commit;
use pagify_shell::markup::{Layer, HIT_TOLERANCE_PT};
use pagify_shell::page_space::AppPoint;
use pagify_shell::{tools, Session};
use pdf_core::document::Color;

fn fixture(name: &str) -> String {
    format!(
        "{}/../../../workspace/Pagify/rust/pdf_core/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    )
}

/// A directory that cleans up after itself, named per test so two running at
/// once cannot collide.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("pagify-round-trip-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        Scratch(dir)
    }
    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const INK: Color = Color { r: 220, g: 30, b: 60, a: 255 };

fn drawn_on(height: f64) -> Layer {
    let mut layer = Layer::new(height);
    layer.add(Geom::Line(Line { a: Vec2::new(50.0, 60.0), b: Vec2::new(250.0, 60.0) }));
    layer.add(Geom::Circle(Circle { center: Vec2::new(150.0, 150.0), radius: 40.0 }));
    layer.add(Geom::Arc(Arc {
        center: Vec2::new(100.0, 250.0),
        radius: 25.0,
        start_angle: 0.25,
        sweep_angle: 1.75,
    }));
    layer
}

#[test]
fn markup_survives_a_save_and_is_still_live_geometry() {
    let scratch = Scratch::new("live");
    let saved = scratch.file("marked.pdf");

    let height = {
        let session = Session::open(fixture("single-page.pdf")).expect("open");
        let height = session.page_size(0).expect("size").height_pt as f64;

        let layer = drawn_on(height);
        let committed = session.commit_markup(0, &layer, INK, 1.5).expect("commit");
        assert_eq!(committed.objects_stored, 3);
        assert!(committed.skipped.is_empty());
        assert!(committed.strokes_written >= 3);

        session.save_to(&saved, true).expect("save");
        height
    };
    // Every handle is dropped here. What follows reads the file, nothing else.

    assert!(saved.is_file(), "nothing was written");

    let reopened = Session::open(&saved).expect("reopen the saved file");
    let restored = reopened
        .restore_markup(0)
        .expect("read markup")
        .expect("the page should carry markup");

    assert_eq!(restored.len(), 3, "not everything came back");
    assert_eq!(restored.space().height_pt(), height);

    // The point of the whole phase: a circle came back as a *circle*, with its
    // exact centre and radius — not as sixty-odd line segments that could never
    // be filleted against again.
    let circle = restored
        .objects()
        .iter()
        .find_map(|o| match &o.geom {
            Geom::Circle(c) => Some(*c),
            _ => None,
        })
        .expect("the circle is not a circle any more");
    assert_eq!(circle.center, Vec2::new(150.0, 150.0));
    assert_eq!(circle.radius, 40.0);

    let arc = restored
        .objects()
        .iter()
        .find_map(|o| match &o.geom {
            Geom::Arc(a) => Some(*a),
            _ => None,
        })
        .expect("the arc is not an arc any more");
    assert_eq!(arc.radius, 25.0);
    assert_eq!(arc.sweep_angle, 1.75);
}

#[test]
fn a_restored_mark_can_still_be_picked_and_edited() {
    // "Reopen, rebuild, **keep editing**." Geometry that comes back but cannot
    // be selected or modified has not really survived.
    let scratch = Scratch::new("keep-editing");
    let saved = scratch.file("marked.pdf");

    {
        let session = Session::open(fixture("single-page.pdf")).expect("open");
        let height = session.page_size(0).expect("size").height_pt as f64;
        session
            .commit_markup(0, &drawn_on(height), INK, 1.5)
            .expect("commit");
        session.save_to(&saved, true).expect("save");
    }

    let reopened = Session::open(&saved).expect("reopen");
    let mut restored = reopened.restore_markup(0).expect("read").expect("markup");
    let height = restored.space().height_pt();

    // Pick the horizontal line by clicking on it, in app space.
    let on_the_line = AppPoint::new(150.0, height - 60.0);
    let picked = restored
        .select_at(on_the_line, HIT_TOLERANCE_PT, false)
        .expect("the restored line cannot be picked");

    // And modify it — the operation that is impossible against flattened ink.
    assert_eq!(tools::move_selection(&mut restored, Vec2::new(0.0, 100.0)), 1);
    match &restored.objects()[picked].geom {
        Geom::Line(l) => assert_eq!(l.a.y, 160.0),
        other => panic!("the line became {other:?}"),
    }

    // A fillet needs two live objects with identity. That it runs at all is the
    // proof that §5.2's "live, not flattened" held across the save.
    let a = restored.add(Geom::Line(Line { a: Vec2::new(400.0, 400.0), b: Vec2::new(500.0, 400.0) }));
    let b = restored.add(Geom::Line(Line { a: Vec2::new(500.0, 400.0), b: Vec2::new(500.0, 500.0) }));
    tools::fillet_pair(&mut restored, a, Vec2::new(450.0, 400.0), b, Vec2::new(500.0, 450.0), 10.0)
        .expect("a restored layer must still be filletable");
}

#[test]
fn committing_twice_leaves_one_copy_not_two() {
    // Every save re-commits. Without the carrier id to find the previous write
    // by, a document saved five times carries five copies of every mark — each
    // one printing on top of the last.
    let scratch = Scratch::new("idempotent");
    let saved = scratch.file("twice.pdf");

    {
        let session = Session::open(fixture("single-page.pdf")).expect("open");
        let height = session.page_size(0).expect("size").height_pt as f64;
        let layer = drawn_on(height);

        session.commit_markup(0, &layer, INK, 1.5).expect("first commit");
        session.commit_markup(0, &layer, INK, 1.5).expect("second commit");
        session.save_to(&saved, true).expect("save");
    }

    let reopened = Session::open(&saved).expect("reopen");
    let restored = reopened.restore_markup(0).expect("read").expect("markup");
    assert_eq!(restored.len(), 3, "the geometry was stored more than once");

    let ink = reopened
        .with_engine(|s| s.document.annotations(0))
        .expect("annotations")
        .iter()
        .filter(|a| matches!(a.annotation, pdf_core::document::Annotation::Ink { .. }))
        .count();
    assert_eq!(ink, 3, "a second commit left a duplicate set of ink");
}

#[test]
fn a_document_with_no_markup_reports_none_rather_than_failing() {
    let session = Session::open(fixture("pages-ladder.pdf")).expect("open");
    assert!(session.restore_markup(0).expect("read").is_none());
}

#[test]
fn markup_on_one_page_does_not_appear_on_another() {
    let scratch = Scratch::new("per-page");
    let saved = scratch.file("multi.pdf");

    {
        let session = Session::open(fixture("pages-ladder.pdf")).expect("open");
        let height = session.page_size(2).expect("size").height_pt as f64;
        session.commit_markup(2, &drawn_on(height), INK, 1.0).expect("commit");
        session.save_to(&saved, true).expect("save");
    }

    let reopened = Session::open(&saved).expect("reopen");
    assert!(reopened.restore_markup(0).expect("read").is_none());
    assert!(reopened.restore_markup(1).expect("read").is_none());
    assert_eq!(reopened.restore_markup(2).expect("read").expect("page 3").len(), 3);
    assert!(reopened.restore_markup(3).expect("read").is_none());
}

#[test]
fn an_incremental_save_keeps_the_original_bytes_in_front() {
    // What incremental save is *for*: a signature covers a byte range, and a
    // full rewrite relocates every object in the file. Asserted on the bytes
    // rather than on the API having been called with `true`.
    let scratch = Scratch::new("incremental");
    let saved = scratch.file("appended.pdf");

    let original = std::fs::read(fixture("single-page.pdf")).expect("read fixture");

    {
        let session = Session::open(fixture("single-page.pdf")).expect("open");
        let height = session.page_size(0).expect("size").height_pt as f64;
        session.commit_markup(0, &drawn_on(height), INK, 1.0).expect("commit");
        session.save_to(&saved, true).expect("save");
    }

    let written = std::fs::read(&saved).expect("read what was saved");
    assert!(written.len() > original.len(), "an incremental save only ever appends");
    assert_eq!(
        &written[..original.len()],
        &original[..],
        "the original bytes were rewritten — no signature over them could survive"
    );
}

#[test]
fn a_file_whose_name_has_spaces_opens_through_the_command_box() {
    // The reported bug, end to end: dispatch a typed line, take the path it
    // produced, and actually open the document with it. Testing the parser
    // alone would not have caught a path that parsed correctly and then failed
    // to open.
    use pagify_shell::command::{dispatch, Dispatch};
    use pagify_shell::verbs::Verb;

    let scratch = Scratch::new("spaced-name");
    let spaced = scratch.file("HSI CATALOG 2026.pdf");
    std::fs::copy(fixture("single-page.pdf"), &spaced).expect("stage the file");

    for line in [
        format!("open {}", spaced.display()),
        format!("open \"{}\"", spaced.display()),
        format!("open {}", spaced.display().to_string().replace(' ', "\\ ")),
    ] {
        let path = match dispatch(&line) {
            Some(Dispatch::Pagify(Verb::Open(path))) => path,
            other => panic!("`{line}` gave {other:?}"),
        };
        assert_eq!(path, spaced, "for `{line}`");

        let session = Session::open(&path).unwrap_or_else(|e| panic!("`{line}` did not open: {e}"));
        assert_eq!(session.page_count().expect("pages"), 1);
    }
}

#[test]
fn a_recent_entry_round_trips_back_into_an_open_command() {
    // The Recent Documents panel emits `open "<path>"`. If that quoting did not
    // survive the parser, every row in the panel would be dead for any file
    // with a space in its path — which is most of them.
    use pagify_shell::command::{dispatch, Dispatch};
    use pagify_shell::recent::Recent;
    use pagify_shell::verbs::Verb;

    let scratch = Scratch::new("recent-round-trip");
    let spaced = scratch.file("Site Plan Rev B.pdf");
    std::fs::copy(fixture("single-page.pdf"), &spaced).expect("stage the file");

    let mut recent = Recent::default();
    recent.record(&spaced, 1, 1_788_186_180);
    let entry = recent.present().first().copied().expect("one entry").clone();

    let line = format!("open \"{}\"", entry.path.to_string_lossy());
    match dispatch(&line) {
        Some(Dispatch::Pagify(Verb::Open(path))) => {
            assert!(Session::open(&path).is_ok(), "the panel's own command did not open the file");
        }
        other => panic!("the panel emitted `{line}`, which gave {other:?}"),
    }
}

/// The page operations wired to the Organize tab, driven the way the app drives
/// them — through a `Session`.
///
/// `pages-ladder.pdf` has five pages of **distinct widths** — 200, 250, 300,
/// 350, 400 — which is the whole reason these can be checked at all: the
/// sequence of widths says which page landed where, with no text to extract and
/// no pixels to compare.
mod page_operations {
    use pagify_shell::Session;
    use pdf_core::command::Command;

    fn session(name: &str) -> Session {
        let src = format!(
            "{}/../../../workspace/Pagify/rust/pdf_core/fixtures/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        // A copy: these edit, and an edit that reached a fixture would break
        // every other test in the workspace.
        let dir = std::env::temp_dir().join("pagify-page-ops");
        std::fs::create_dir_all(&dir).expect("scratch dir");
        let to = dir.join(format!("{:?}-{name}", std::thread::current().id()).replace(['(', ')', ' '], ""));
        std::fs::copy(&src, &to).expect("copy the fixture");
        Session::open(&to).expect("open")
    }

    fn widths(session: &Session) -> Vec<i32> {
        session
            .page_sizes()
            .expect("sizes")
            .iter()
            .map(|s| s.width_pt.round() as i32)
            .collect()
    }

    #[test]
    fn reversing_turns_the_document_back_to_front() {
        if std::env::var("PAGIFY_PDFIUM_LIB").is_err() {
            return;
        }
        let session = session("pages-ladder.pdf");
        assert_eq!(widths(&session), vec![200, 250, 300, 350, 400]);

        let order = pagify_shell::organize::order_for_reverse(5);
        session.execute(Command::ReorderPages { order }).expect("reorder");

        assert_eq!(widths(&session), vec![400, 350, 300, 250, 200]);
    }

    #[test]
    fn swapping_exchanges_two_pages_and_leaves_the_others() {
        if std::env::var("PAGIFY_PDFIUM_LIB").is_err() {
            return;
        }
        let session = session("pages-ladder.pdf");
        let order = pagify_shell::organize::order_for_swap(5, 2, 5).expect("order");
        session.execute(Command::ReorderPages { order }).expect("reorder");

        assert_eq!(widths(&session), vec![200, 400, 300, 350, 250]);
    }

    /// Duplicating puts the copies straight after the block, keeping it
    /// together — which is what somebody duplicating a section means.
    #[test]
    fn duplicating_a_run_keeps_the_block_together() {
        if std::env::var("PAGIFY_PDFIUM_LIB").is_err() {
            return;
        }
        let session = session("pages-ladder.pdf");
        assert_eq!(widths(&session), vec![200, 250, 300, 350, 400]);

        session.duplicate_pages(&[0, 1, 2], 3).expect("duplicate");

        assert_eq!(
            widths(&session),
            vec![200, 250, 300, 200, 250, 300, 350, 400],
            "the copies did not land as a block after the originals"
        );
    }

    #[test]
    fn duplicating_one_page_puts_the_copy_next_to_it() {
        if std::env::var("PAGIFY_PDFIUM_LIB").is_err() {
            return;
        }
        let session = session("pages-ladder.pdf");
        session.duplicate_pages(&[1], 2).expect("duplicate");
        assert_eq!(widths(&session), vec![200, 250, 250, 300, 350, 400]);
    }

    /// It changes the document, so it has to be reversible.
    #[test]
    fn a_duplicate_can_be_undone() {
        if std::env::var("PAGIFY_PDFIUM_LIB").is_err() {
            return;
        }
        let session = session("pages-ladder.pdf");
        session.duplicate_pages(&[0], 1).expect("duplicate");
        assert_eq!(widths(&session).len(), 6);

        session.undo().expect("undo");
        assert_eq!(
            widths(&session),
            vec![200, 250, 300, 350, 400],
            "undo did not take the copy back off"
        );
    }

    /// The copy must be a copy, not a reference — editing one page must not
    /// change the other. Checked through rotation, which is a property of the
    /// page itself.
    #[test]
    fn a_duplicated_page_is_independent_of_its_original() {
        if std::env::var("PAGIFY_PDFIUM_LIB").is_err() {
            return;
        }
        let session = session("pages-ladder.pdf");
        session.duplicate_pages(&[0], 1).expect("duplicate");

        session
            .execute(Command::SetPageRotation { index: 1, quarter_turns: 1 })
            .expect("rotate the copy");

        assert_eq!(session.page_rotation(0).expect("read"), 0, "the original turned as well");
        assert_eq!(session.page_rotation(1).expect("read"), 1, "the copy did not turn");
    }

    /// Cropping changes what a page *shows*. This crate reports the crop box as
    /// the page size, so the trim is visible in `page_sizes` — which is also how
    /// a reader would notice it.
    #[test]
    fn cropping_trims_what_the_page_shows() {
        if std::env::var("PAGIFY_PDFIUM_LIB").is_err() {
            return;
        }
        let session = session("pages-ladder.pdf");
        let before = session.page_crop(0).expect("read the crop");
        let width = (before.right - before.left).abs();

        let crop = pdf_core::document::Rect {
            left: before.left + 20.0,
            top: before.top + 20.0,
            right: before.right - 20.0,
            bottom: before.bottom - 20.0,
        };
        session.execute(Command::SetPageCrop { index: 0, crop }).expect("crop");

        let after = session.page_crop(0).expect("read");
        let now = (after.right - after.left).abs();
        assert!(
            (width - now - 40.0).abs() < 1.0,
            "the page went from {width:.0}pt wide to {now:.0}pt, expected {:.0}",
            width - 40.0
        );
    }

    /// The crop box is a window, not a knife — and undo has to restore the
    /// rectangle that was there, not some default.
    #[test]
    fn a_crop_can_be_undone_back_to_the_rectangle_it_replaced() {
        if std::env::var("PAGIFY_PDFIUM_LIB").is_err() {
            return;
        }
        let session = session("pages-ladder.pdf");
        let before = session.page_crop(0).expect("read");

        let crop = pdf_core::document::Rect {
            left: before.left + 30.0,
            top: before.top + 30.0,
            right: before.right - 30.0,
            bottom: before.bottom - 30.0,
        };
        session.execute(Command::SetPageCrop { index: 0, crop }).expect("crop");
        session.undo().expect("undo");

        let after = session.page_crop(0).expect("read");
        for (name, a, b) in [
            ("left", before.left, after.left),
            ("top", before.top, after.top),
            ("right", before.right, after.right),
            ("bottom", before.bottom, after.bottom),
        ] {
            assert!((a - b).abs() < 1.0, "{name} came back as {b}, was {a}");
        }
    }

    /// A crop with no area is not a page, and a page that renders as nothing
    /// looks exactly like a bug in the renderer.
    #[test]
    fn a_crop_with_no_area_is_refused() {
        if std::env::var("PAGIFY_PDFIUM_LIB").is_err() {
            return;
        }
        let session = session("pages-ladder.pdf");
        let flat = pdf_core::document::Rect { left: 100.0, top: 100.0, right: 100.0, bottom: 300.0 };
        assert!(session.execute(Command::SetPageCrop { index: 0, crop: flat }).is_err());
    }

    #[test]
    fn resizing_changes_the_sheet() {
        if std::env::var("PAGIFY_PDFIUM_LIB").is_err() {
            return;
        }
        let session = session("pages-ladder.pdf");
        session
            .execute(Command::SetPageSize { index: 0, width_pt: 595.28, height_pt: 841.89 })
            .expect("resize");

        let after = session.page_crop(0).expect("read");
        assert!(
            ((after.right - after.left).abs() - 595.28).abs() < 1.0,
            "the sheet is {:.0}pt wide, expected 595",
            (after.right - after.left).abs()
        );
    }

    /// **The reason the undo record stores a matrix rather than a size.**
    ///
    /// Re-deriving a scale from the old dimensions would centre the content on
    /// the way back, so a page whose content was not centred to begin with
    /// would not return to where it started. An inverse is exact — and the way
    /// to see it is the page's own text, which must land back where it was.
    #[test]
    fn undoing_a_resize_puts_the_content_back_where_it_was() {
        if std::env::var("PAGIFY_PDFIUM_LIB").is_err() {
            return;
        }
        let session = session("text-lines.pdf");
        let before = session.characters(0).expect("characters");
        let first = before.line_rects(0..1).into_iter().next().expect("a character box");

        session
            .execute(Command::SetPageSize { index: 0, width_pt: 841.89, height_pt: 1190.55 })
            .expect("resize");
        session.undo().expect("undo");

        let after = session.characters(0).expect("characters");
        let back = after.line_rects(0..1).into_iter().next().expect("a character box");

        for (name, a, b) in [
            ("left", first.left, back.left),
            ("top", first.top, back.top),
        ] {
            assert!(
                (a - b).abs() < 1.5,
                "{name} came back at {b:.1}, was {a:.1} — the content did not return"
            );
        }
    }

    #[test]
    fn a_paper_size_can_be_named_or_given_in_points() {
        assert_eq!(pagify_shell::verbs::paper_size("a4"), Some((595.28, 841.89)));
        assert_eq!(pagify_shell::verbs::paper_size("LETTER"), Some((612.0, 792.0)));
        assert_eq!(pagify_shell::verbs::paper_size("300x400"), Some((300.0, 400.0)));
        assert_eq!(pagify_shell::verbs::paper_size("nonsense"), None);
        // A sheet with no area is not a sheet.
        assert_eq!(pagify_shell::verbs::paper_size("0x400"), None);
    }

    /// Rotation is relative and the engine's command is absolute. A document can
    /// arrive with pages already at different angles — a landscape drawing among
    /// portrait sheets — and setting them all to one value straightens some and
    /// turns others sideways.
    #[test]
    fn rotating_turns_each_page_from_where_it_already_was() {
        if std::env::var("PAGIFY_PDFIUM_LIB").is_err() {
            return;
        }
        let session = session("pages-ladder.pdf");
        session
            .execute(Command::SetPageRotation { index: 1, quarter_turns: 1 })
            .expect("set one page sideways");

        for page in 0..2usize {
            let from = session.page_rotation(page).expect("read") as i32;
            let to = (((from + 1) % 4) + 4) % 4;
            session
                .execute(Command::SetPageRotation { index: page, quarter_turns: to as u8 })
                .expect("rotate");
        }

        assert_eq!(session.page_rotation(0).expect("read"), 1, "an upright page did not turn");
        assert_eq!(
            session.page_rotation(1).expect("read"),
            2,
            "a page already sideways was straightened instead of turned further"
        );
    }
}
