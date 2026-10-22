//! A page that carries its own `/Rotate`: what the app reads and writes has to be
//! where the page *shows* it.
//!
//! **Checked against the picture, not against the code under test.** The
//! conversion lives in `pdf_core::document::frame`; a test that asked the same
//! frame where a word should be would agree with it whatever it did. These render
//! the turned page (PDFium turns it) and look for ink where the engine's answer
//! says the words are, and for the effect of a write in the picture.

use pagify_shell::Session;
use pdf_core::command::Command;
use pdf_core::document::{Annotation, Color, Glyph, Point, Rect};

fn fixture(name: &str) -> String {
    format!("{}/../../../rust/pdf_core/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))
}

/// The fixture's first page, turned `quarters` clockwise by the engine's own command.
fn turned(quarters: u8) -> Session {
    let session = Session::open(fixture("two-column.pdf")).expect("open");
    if quarters != 0 {
        session
            .execute(Command::SetPageRotation { index: 0, quarter_turns: quarters })
            .expect("turn the page");
    }
    session
}

fn dark(p: &[u8]) -> bool {
    // RGBA; a pixel is ink when it is clearly darker than paper.
    (p[0] as u32 + p[1] as u32 + p[2] as u32) / 3 < 140
}

/// The share of a rectangle's pixels that are ink, in a picture taken at 1 pt a pixel.
fn ink_share(session: &Session, rect: Rect) -> f32 {
    let raster = session.render_page(0, 1.0).expect("render");
    let (px, w, h) = raster.crop(
        rect.left.floor().max(0.0) as u32,
        rect.top.floor().max(0.0) as u32,
        rect.right.ceil().max(0.0) as u32,
        rect.bottom.ceil().max(0.0) as u32,
    );
    if w == 0 || h == 0 {
        return 0.0;
    }
    px.chunks_exact(4).filter(|p| dark(p)).count() as f32 / (w * h) as f32
}

fn words(session: &Session) -> Vec<pdf_core::document::TextRun> {
    session
        .text_runs(0)
        .expect("runs")
        .into_iter()
        .filter(|r| r.text.trim().chars().count() >= 4 && (r.rect.right - r.rect.left).max(r.rect.bottom - r.rect.top) > 20.0)
        .collect()
}

#[test]
fn every_turn_reads_the_words_where_the_picture_has_them() {
    for quarters in 0..4u8 {
        let session = turned(quarters);
        let size = session.page_size(0).expect("size");
        let runs = words(&session);
        assert!(runs.len() >= 10, "turn {quarters}: only {} words", runs.len());
        let mut inked = 0;
        for run in &runs {
            let r = run.rect;
            assert!(
                r.left >= -1.0 && r.top >= -1.0 && r.right <= size.width_pt + 1.0 && r.bottom <= size.height_pt + 1.0,
                "turn {quarters}: {:?} is off the {:?} page it is drawn on",
                run.text,
                (size.width_pt, size.height_pt)
            );
            if ink_share(&session, r) > 0.04 {
                inked += 1;
            }
        }
        assert!(
            inked * 10 >= runs.len() * 9,
            "turn {quarters}: only {inked} of {} words have ink under their box",
            runs.len()
        );
    }
}

#[test]
fn a_turned_pages_words_run_the_way_the_picture_reads_them() {
    // Upright text on a sheet turned a quarter clockwise reads top to bottom,
    // turned a half it reads right to left, and so on: the direction a run is
    // given must be the one it has in the picture, and its size must be real.
    for (quarters, expected) in [(0u8, (1.0f32, 0.0f32)), (1, (0.0, 1.0)), (2, (-1.0, 0.0)), (3, (0.0, -1.0))] {
        let session = turned(quarters);
        let styles = session.run_styles(0).expect("styles");
        for run in &words(&session) {
            let axis = styles[&run.object].axis;
            assert!(
                (axis.0 - expected.0).abs() < 1e-3 && (axis.1 - expected.1).abs() < 1e-3,
                "turn {quarters}: {:?} runs along {axis:?}, expected {expected:?}",
                run.text
            );
            assert!(run.size > 1.0, "turn {quarters}: {:?} has size {}", run.text, run.size);
            let (w, h) = (run.rect.right - run.rect.left, run.rect.bottom - run.rect.top);
            assert!(
                if expected.0 != 0.0 { w > h } else { h > w },
                "turn {quarters}: {:?} is {w} x {h}, which is not the shape of a line running {expected:?}",
                run.text
            );
        }
    }
}

#[test]
fn characters_and_runs_agree_on_a_turned_page() {
    // Selection and Find use the character boxes, Edit Text uses the run boxes:
    // both must be in the one frame.
    for quarters in 1..4u8 {
        let session = turned(quarters);
        let chars = session.characters(0).expect("characters");
        let run = words(&session).into_iter().next().expect("a word");
        let mid = (run.rect.top + run.rect.bottom) / 2.0;
        let inside = (1..9)
            .filter(|i| {
                let x = run.rect.left + (run.rect.right - run.rect.left) * (*i as f32) / 9.0;
                chars.hit(x, mid).is_some()
            })
            .count();
        assert!(ink_share(&session, run.rect) > 0.04, "turn {quarters}: no ink under {:?}'s box", run.text);
        assert!(inside >= 6, "turn {quarters}: only {inside} of 8 points along {:?} hit a character", run.text);
    }
}

#[test]
fn a_move_to_the_right_moves_the_word_to_the_right_in_the_picture() {
    for quarters in 0..4u8 {
        let session = turned(quarters);
        let run = words(&session).into_iter().next().expect("a word");
        let by = Point { x: 40.0, y: 0.0 };
        session
            .execute(Command::MoveObjects { page_index: 0, objects: vec![run.object], by })
            .expect("move");
        let now = session.text_run_at(0, run.object).expect("run").expect("still there");
        assert!(
            (now.rect.left - run.rect.left - 40.0).abs() < 1.0 && (now.rect.top - run.rect.top).abs() < 1.0,
            "turn {quarters}: moved from {:?} to {:?}",
            run.rect,
            now.rect
        );
        assert!(ink_share(&session, now.rect) > 0.04, "turn {quarters}: no ink where the word now is");
        let left_behind = Rect { right: (run.rect.left + 40.0).min(run.rect.right), ..run.rect };
        assert!(
            ink_share(&session, left_behind) < 0.02,
            "turn {quarters}: the word is still where it was"
        );
    }
}

#[test]
fn a_redact_drawn_over_a_word_on_screen_takes_that_word() {
    for quarters in 1..4u8 {
        let session = turned(quarters);
        let all = words(&session);
        let target = all[all.len() / 2].clone();
        session
            .execute(Command::Redact {
                page_index: 0,
                area: target.rect,
                fill: Some(Color { r: 0, g: 0, b: 0, a: 255 }),
                allow_incomplete: true,
                outlined_fonts: vec![],
            })
            .expect("redact");
        let after = session.text_runs(0).expect("runs");
        assert!(
            !after.iter().any(|r| r.object == target.object && r.text.trim() == target.text.trim()),
            "turn {quarters}: {:?} is still on the page",
            target.text
        );
        assert!(
            after.iter().filter(|r| r.text.trim().len() >= 4).count() + 3 >= all.len(),
            "turn {quarters}: far more than one word went"
        );
        let inner = Rect {
            left: target.rect.left + 1.5,
            top: target.rect.top + 1.5,
            right: target.rect.right - 1.5,
            bottom: target.rect.bottom - 1.5,
        };
        let share = ink_share(&session, inner);
        assert!(share > 0.9, "turn {quarters}: the redaction is not over the word (ink {share}, box {:?})", target.rect);
    }
}

#[test]
fn a_picture_placed_on_screen_reads_the_right_way_round() {
    // Left half red, right half blue, on a rectangle placed on the page as shown.
    let (w, h) = (8u32, 4u32);
    let mut rgba = Vec::new();
    for _y in 0..h {
        for x in 0..w {
            rgba.extend_from_slice(if x < w / 2 { &[255, 0, 0, 255] } else { &[0, 0, 255, 255] });
        }
    }
    for quarters in 0..4u8 {
        let session = turned(quarters);
        let spot = Rect { left: 200.0, top: 300.0, right: 280.0, bottom: 340.0 };
        session
            .execute(Command::AddAnnotation {
                page_index: 0,
                annotation: Annotation::Image { rect: spot, rgba: rgba.clone(), width: w, height: h },
            })
            .expect("place");
        let raster = session.render_page(0, 1.0).expect("render");
        let at = |x: f32, y: f32| {
            let (px, _, _) = raster.crop(x as u32, y as u32, x as u32, y as u32);
            (px[0], px[2])
        };
        let (lr, lb) = at(220.0, 320.0);
        let (rr, rb) = at(260.0, 320.0);
        assert!(lr > 200 && lb < 80, "turn {quarters}: the left of the picture is not red: ({lr}, {lb})");
        assert!(rb > 200 && rr < 80, "turn {quarters}: the right of the picture is not blue: ({rr}, {rb})");
    }
}

#[test]
fn new_text_placed_on_screen_runs_level_in_the_picture() {
    for quarters in 0..4u8 {
        let session = turned(quarters);
        let glyphs: Vec<Glyph> = "WWWWWWWW"
            .chars()
            .enumerate()
            .map(|(i, c)| Glyph { ch: c.to_string(), id: 0, x: 150.0 + i as f32 * 14.0, y: 420.0, radians: 0.0 })
            .collect();
        session
            .execute(Command::AddAnnotation {
                page_index: 0,
                annotation: Annotation::Text {
                    text: "WWWWWWWW".into(),
                    font: "Helvetica".into(),
                    font_asset: None,
                    size: 14.0,
                    color: Color { r: 0, g: 0, b: 0, a: 255 },
                    glyphs,
                    id: 7,
                    restore: String::new(),
                    frame: vec![],
                    frame_width: 0.0,
                    scale_x: 1.0,
                    more: vec![],
                    rules: vec![],
                },
            })
            .expect("write");
        let wide = Rect { left: 140.0, top: 400.0, right: 270.0, bottom: 430.0 };
        let tall = Rect { left: 140.0, top: 400.0, right: 170.0, bottom: 530.0 };
        let here = ink_share(&session, wide);
        assert!(here > 0.05, "turn {quarters}: no level line of text where it was placed ({here})");
        // A line written sideways would stand in the tall box instead.
        assert!(ink_share(&session, tall) < here, "turn {quarters}: the line runs the wrong way");
    }
}
