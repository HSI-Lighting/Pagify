//! A sheet drawn sideways and turned upright by the page's own `/Rotate` (every
//! AutoCAD plot): Edit Text and Edit Object have to find, open and rewrite what
//! the page shows level.
//!
//! The page is built here the way those plots are: text set with the matrix
//! `0 -1 1 0` on a `/Rotate 270` page, so it reads level on screen.

use super::*;
use pdf_core::document::Rect;

fn centre(r: &Rect) -> AppPoint {
    AppPoint { x: ((r.left + r.right) / 2.0) as f64, y: ((r.top + r.bottom) / 2.0) as f64 }
}

/// Two lines of one paragraph and a blue square, on a `/Rotate 270` page. In PDF
/// space the lines run downward (`0 -1 1 0`); the second is one line-pitch to the
/// left of the first, which the turn shows below it.
fn sideways_sheet() -> Vec<u8> {
    let content = "BT /F1 10 Tf 0 -1 1 0 300 700 Tm (first line of the words) Tj ET\n\
                   BT /F1 10 Tf 0 -1 1 0 288 700 Tm (second line of the words) Tj ET\n\
                   0.2 0.4 0.8 rg 100 100 100 50 re f\n";
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Rotate 270 /Contents 4 0 R \
         /Resources << /Font << /F1 5 0 R >> >> >>"
            .to_string(),
        format!("<< /Length {} >>\nstream\n{content}endstream", content.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let xref_at = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in &offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF", objects.len() + 1).as_bytes(),
    );
    out
}

fn open_sheet(name: &str) -> PagifyApp {
    let dir = std::env::temp_dir().join("pagify-app-tests");
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let path = dir.join(format!("turned-{name}.pdf"));
    std::fs::write(&path, sideways_sheet()).expect("write the sheet");
    let app = PagifyApp::new(Some(&path.to_string_lossy()));
    assert!(app.tab().doc.is_some(), "{name} did not open");
    app
}

/// The words' rectangles as the page shows them, first line first.
fn line_rects(app: &PagifyApp) -> Vec<Rect> {
    let session = &app.tab().doc.as_ref().expect("open").session;
    let mut runs: Vec<_> = session.text_runs(0).expect("runs").into_iter().filter(|r| r.text.trim().len() > 5).collect();
    runs.sort_by(|a, b| a.rect.top.total_cmp(&b.rect.top));
    runs.into_iter().map(|r| r.rect).collect()
}

#[test]
fn the_sheet_is_shown_level_with_its_first_line_above_its_second() {
    let app = open_sheet("shape");
    let size = app.tab().doc.as_ref().expect("open").session.page_size(0).expect("size");
    assert!(size.width_pt > size.height_pt, "the sheet is shown landscape");
    let lines = line_rects(&app);
    assert_eq!(lines.len(), 2, "{lines:?}");
    for l in &lines {
        assert!(l.right - l.left > 3.0 * (l.bottom - l.top), "a level line is much wider than tall: {l:?}");
    }
    assert!(lines[0].bottom <= lines[1].top + 1.0, "the first line is above the second");
}

#[test]
fn edit_text_opens_the_paragraph_the_sheet_shows_level() {
    let mut app = open_sheet("pick");
    let lines = line_rects(&app);
    app.pick_text_run(0, centre(&lines[0])).expect("a click on the first line picks it");
    let edit = app.tab().edit.editing_run.as_ref().expect("the editor opened");
    assert_eq!(edit.lines.len(), 2, "both lines are one paragraph: {:?}", edit.buffer);
    assert_eq!(edit.buffer, "first line of the words\nsecond line of the words");
}

#[test]
fn a_retype_lands_on_the_sheet_where_the_words_are() {
    let mut app = open_sheet("retype");
    let lines = line_rects(&app);
    app.pick_text_run(0, centre(&lines[0])).expect("picked");
    let retyped = app.tab().edit.editing_run.as_ref().expect("editing").buffer.replace("second", "SECOND");
    app.tab_mut().edit.editing_run.as_mut().expect("editing").buffer = retyped;
    app.apply_editing_page();

    let after = line_rects(&app);
    assert_eq!(after.len(), 2, "no line was lost or added: {after:?}");
    let texts: Vec<String> = {
        let session = &app.tab().doc.as_ref().expect("open").session;
        let mut runs: Vec<_> = session.text_runs(0).expect("runs").into_iter().filter(|r| r.text.trim().len() > 5).collect();
        runs.sort_by(|a, b| a.rect.top.total_cmp(&b.rect.top));
        runs.into_iter().map(|r| r.text.trim().to_string()).collect()
    };
    assert_eq!(texts, ["first line of the words", "SECOND line of the words"]);
    for (before, now) in lines.iter().zip(&after) {
        assert!(
            (before.left - now.left).abs() < 2.0 && (before.top - now.top).abs() < 2.0,
            "a line moved: {before:?} -> {now:?}"
        );
    }
}

#[test]
fn edit_object_picks_the_words_and_the_square_where_the_sheet_shows_them() {
    let mut app = open_sheet("object");
    let words = line_rects(&app)[0];
    let square = app
        .tab()
        .doc
        .as_ref()
        .expect("open")
        .session
        .drawn_objects(0)
        .expect("drawn")
        .into_iter()
        .find(|d| d.kind == pdf_core::document::DrawnKind::Shape)
        .expect("the square")
        .rect;
    assert!(
        (square.right - square.left - 50.0).abs() < 1.0 && (square.bottom - square.top - 100.0).abs() < 1.0,
        "the 100 x 50 square is shown turned: {square:?}"
    );
    app.submit("editobject");
    assert!(app.thing_at(0, centre(&words), true).is_some(), "the words are not found where they are shown");
    assert!(app.thing_at(0, centre(&square), true).is_some(), "the square is not found where it is shown");
    let paper = AppPoint { x: 10.0, y: 10.0 };
    assert!(app.thing_at(0, paper, true).is_none(), "test assumption: nothing is drawn at the corner");
}
