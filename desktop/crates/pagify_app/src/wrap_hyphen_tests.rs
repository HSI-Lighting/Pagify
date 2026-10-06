use super::*;

/// A one-page PDF drawing each of `lines` — `(x, y, words)`, `y` up from
/// the bottom as PDF has it — as its own Helvetica 10 pt text object,
/// followed by `extra` raw content-stream operators (a path, say). `/F1`
/// is Helvetica and `/F2` Helvetica-Bold, for `extra` to use.
pub(super) fn pdf_with(lines: &[(f32, f32, &str)], extra: &str) -> Vec<u8> {
    let mut content = String::new();
    for (x, y, words) in lines {
        content.push_str(&format!("BT /F1 10 Tf {x} {y} Td ({words}) Tj ET\n"));
    }
    content.push_str(extra);
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R \
         /Resources << /Font << /F1 5 0 R /F2 6 0 R >> >> >>"
            .to_string(),
        format!("<< /Length {} >>\nstream\n{content}endstream", content.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold >>".to_string(),
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
        format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF", objects.len() + 1)
            .as_bytes(),
    );
    out
}

/// The page written to a scratch file of its own and opened.
pub(super) fn open_page(name: &str, lines: &[(f32, f32, &str)], extra: &str) -> PagifyApp {
    let dir = std::env::temp_dir().join("pagify-app-tests");
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let path = dir.join(format!("wrap-hyphen-{name}.pdf"));
    std::fs::write(&path, pdf_with(lines, extra)).expect("write the synthetic page");
    let app = PagifyApp::new(Some(&path.to_string_lossy()));
    assert!(app.tab().doc.is_some(), "{name} did not open");
    app
}

/// A three-line paragraph whose wraps cut two words in half ("gam|ma",
/// "epsi|lon"). Nothing on the page draws a hyphen anywhere.
const THREE_LINES: [(f32, f32, &str); 3] = [
    (100.0, 700.0, "alpha beta gam"),
    (100.0, 688.0, "ma delta epsi"),
    (100.0, 676.0, "lon zeta eta"),
];

fn pick_middle_line(app: &mut PagifyApp) {
    let middle = app
        .tab_mut()
        .doc
        .as_ref()
        .expect("open")
        .session
        .text_runs(0)
        .expect("runs")
        .into_iter()
        .find(|r| r.text.contains("ma delta"))
        .expect("the middle line");
    let at = AppPoint {
        x: ((middle.rect.left + middle.rect.right) / 2.0) as f64,
        y: ((middle.rect.top + middle.rect.bottom) / 2.0) as f64,
    };
    app.submit("edittext");
    app.pick_text_run(0, at).expect("a run was here");
}

/// **Does an invented hyphen reach the page? Yes — and not only on the
/// line that was edited.**
///
/// `join_paragraph_lines` used to put a "-" after every line that ended in
/// a letter and was followed by one, whether or not the page showed a
/// hyphen there. That is bad enough in the box. But applying rewrites
/// *every* line of the paragraph from the buffer (see
/// `paragraph_edit_commands`: unchanged lines are written too), so the
/// invented "-" was typed into the page as real text — on the edited line
/// and on the lines the person never touched.
///
/// Run against the code before the fix, this is the failing test: it
/// reports `"alpha beta gam-"` on the page.
#[test]
fn applying_a_paragraph_never_writes_a_hyphen_the_page_did_not_show() {
    let mut app = open_page("apply", &THREE_LINES, "");
    pick_middle_line(&mut app);

    let edit = app.tab_mut().editing_run.as_ref().expect("the paragraph should have opened");
    assert_eq!(edit.lines.len(), 3, "setup: all three lines should be one paragraph");
    let objects: Vec<usize> = edit.lines.iter().map(|(o, _)| o[0]).collect();
    let retyped = edit.buffer.replace("delta", "DELTA");
    app.tab_mut().editing_run.as_mut().expect("editing").buffer = retyped;
    app.apply_editing_page();

    let after: std::collections::HashMap<usize, String> = app
        .tab_mut()
        .doc
        .as_ref()
        .expect("open")
        .session
        .text_runs(0)
        .expect("runs")
        .into_iter()
        .map(|r| (r.object, r.text))
        .collect();
    // All three at once, so a failure shows which lines were written to.
    // PDFium reads a hyphen glyph at the end of a text object back as
    // U+0002, so a written hyphen shows up as `\u{2}` here, not as `-`.
    let on_page: Vec<String> = objects
        .iter()
        .map(|o| after.get(o).map(|s| s.trim().to_string()).unwrap_or_default())
        .collect();
    assert_eq!(
        on_page,
        ["alpha beta gam", "ma DELTA epsi", "lon zeta eta"],
        "the page should hold exactly what was typed: the untouched first line unchanged, \
         the edited middle line edited, and no hyphen the page never showed"
    );
}

/// The three lines, opened as a paragraph. With `mark`, the page also
/// draws a hyphen — a short thin filled bar a little above the baseline,
/// right after the first line's last glyph — which is how a producer that
/// converts its hyphen glyph to a path leaves one.
fn open_three_lines(name: &str, mark: bool) -> PagifyApp {
    let extra = if mark {
        let probe = open_page(&format!("{name}-probe"), &THREE_LINES, "");
        let first = probe
            .tab()
            .doc
            .as_ref()
            .expect("open")
            .session
            .text_runs(0)
            .expect("runs")
            .into_iter()
            .find(|r| r.text.contains("alpha beta gam"))
            .expect("the first line");
        let em = first.size;
        // PDF's y runs up from the bottom of the 792 pt page.
        let baseline = 792.0 - first.origin.y;
        format!(
            "{:.2} {:.2} {:.2} {:.2} re f\n",
            first.rect.right + 0.1 * em,
            baseline + 0.25 * em,
            0.3 * em,
            0.07 * em
        )
    } else {
        String::new()
    };
    let mut app = open_page(name, &THREE_LINES, &extra);
    pick_middle_line(&mut app);
    app
}

/// **Where the page draws a hyphen, the box shows it** — the case the
/// "letters on both sides" rule was first added for, kept working: a line
/// ending "...gam", a small horizontal shape right after it, and "ma ..."
/// below.
#[test]
fn opening_a_paragraph_restores_a_hyphen_the_page_draws() {
    let mut app = open_three_lines("marked", true);
    let edit = app.tab_mut().editing_run.as_ref().expect("the paragraph should have opened");
    assert_eq!(edit.lines.len(), 3, "setup: all three lines should be one paragraph");
    assert_eq!(
        edit.buffer, "alpha beta gam-\nma delta epsi\nlon zeta eta",
        "the drawn hyphen after the first line should be in the box, and only that one"
    );
}

/// **And the same three lines with no mark get no hyphen anywhere** — both
/// wraps cut a word in half, and neither shows a hyphen.
#[test]
fn opening_the_same_paragraph_without_the_mark_shows_no_hyphen() {
    let mut app = open_three_lines("unmarked", false);
    let edit = app.tab_mut().editing_run.as_ref().expect("the paragraph should have opened");
    assert_eq!(edit.lines.len(), 3, "setup: all three lines should be one paragraph");
    assert_eq!(edit.buffer, "alpha beta gam\nma delta epsi\nlon zeta eta");
}

/// **The box opens in the look most of the *text* has.** Two lines, each a
/// three-letter bold "HSI " followed by a long run of regular body text.
/// One vote per fragment makes that a tie — two bold, two regular — and a
/// tie goes to whatever came first, which is the bold scrap. By ink the
/// regular text has 49 characters against 6, and the box must open at the
/// body's size (8 pt, not the scrap's 12) in the body's face.
#[test]
fn the_box_opens_in_the_look_most_of_the_text_has() {
    let content = "BT /F2 12 Tf 100 700 Td (HSI ) Tj /F1 8 Tf (is a powerful accent light) Tj ET\n\
                   BT /F2 12 Tf 100 688 Td (HSI ) Tj /F1 8 Tf (is a strong CRI) Tj ET\n";
    let mut app = open_page("look", &[], content);
    let body = app
        .tab()
        .doc
        .as_ref()
        .expect("open")
        .session
        .text_runs(0)
        .expect("runs")
        .into_iter()
        .find(|r| r.text.contains("powerful accent"))
        .expect("the first line's body text");
    let at = AppPoint {
        x: ((body.rect.left + body.rect.right) / 2.0) as f64,
        y: ((body.rect.top + body.rect.bottom) / 2.0) as f64,
    };
    app.submit("edittext");
    app.pick_text_run(0, at).expect("a run was here");

    let edit = app.tab_mut().editing_run.as_ref().expect("the paragraph should have opened");
    assert_eq!(edit.lines.len(), 2, "setup: both lines should be one paragraph: {:?}", edit.buffer);
    assert!(
        edit.lines.iter().all(|(objects, _)| objects.len() == 2),
        "setup: each line is the bold scrap and the body text: {:?}",
        edit.lines
    );
    assert_eq!(edit.style.size, Some(8.0), "the box opened in the scrap's size, not the body's");
    assert_eq!(edit.current_face.as_deref(), Some("Helvetica"), "the box opened in the scrap's face");
}

/// **On the real datasheets, the box shows a hyphen at a wrap exactly where
/// the page's own text carries one.** The invariant that was broken:
/// Illustrator's justified columns end almost every line on a letter, and
/// each such join used to grow a "-". The Marina mall sheet carries 72
/// real wrap hyphens on its three pages (read back as U+0002), which must
/// still come through; everything else must not.
///
/// Skipped, with a note, for a file that is not on this machine.
#[test]
fn on_real_pages_the_box_shows_a_wrap_hyphen_exactly_where_the_text_carries_one() {
    // A few lines of the same column on the Marina sheet — each opens its
    // own paragraph, and each of these wraps ends on a real hyphen.
    let marina = r"C:\Users\hsili\Desktop\Datasheets - Editors market - Marina mall.pdf";
    let cases = [
        (marina, "VEGA series is powerful"),
        (marina, "lizes the latest LED"),
        (marina, "and distribution, resulting"),
        (marina, "powerful lighting, such as large"),
        (r"C:\Users\hsili\Downloads\CAMINO elitee-plus 3.0.pdf", "manufacturers"),
    ];
    let mut wraps_checked = 0;
    let mut hyphens_carried = 0;
    let mut marina_seen = false;
    for (path, seed_words) in cases {
        let mut app = PagifyApp::new(Some(path));
        let Some(doc) = &app.tab().doc else {
            eprintln!("skipping: {path} is not present on this machine");
            continue;
        };
        marina_seen |= path == marina;
        let seed = doc
            .session
            .text_runs(0)
            .expect("runs")
            .into_iter()
            .find(|r| r.text.contains(seed_words))
            .unwrap_or_else(|| panic!("{seed_words:?} not found on page 1 of {path} — has the file changed?"));
        let at = AppPoint {
            x: ((seed.rect.left + seed.rect.right) / 2.0) as f64,
            y: ((seed.rect.top + seed.rect.bottom) / 2.0) as f64,
        };
        app.submit("edittext");
        app.pick_text_run(0, at).expect("a run was here");

        let edit = app.tab_mut().editing_run.as_ref().expect("the paragraph should have opened").clone();
        let wanted: std::collections::HashSet<usize> =
            edit.lines.iter().flat_map(|(objects, _)| objects.iter().copied()).collect();
        let text_of: std::collections::HashMap<usize, String> = app
            .tab()
            .doc
            .as_ref()
            .expect("open")
            .session
            .text_runs_some(0, &wanted)
            .expect("runs")
            .into_iter()
            .map(|r| (r.object, r.text))
            .collect();
        let shown: Vec<&str> = edit.buffer.split('\n').collect();
        assert_eq!(shown.len(), edit.lines.len(), "the buffer must hold one line per line of the page");
        for i in 0..edit.lines.len().saturating_sub(1) {
            let line_text: String =
                edit.lines[i].0.iter().map(|o| text_of.get(o).map(String::as_str).unwrap_or("")).collect();
            let line_text = line_text.trim_end();
            let carried = line_text.ends_with('\u{2}') || line_text.ends_with('-');
            let shows = shown[i].ends_with('-');
            let next_starts_with_a_letter = shown[i + 1].chars().next().is_some_and(char::is_alphabetic);
            wraps_checked += 1;
            hyphens_carried += usize::from(carried);
            assert!(
                !shows || carried,
                "{path}: line {i} shows a hyphen the page's text does not carry: {:?} / {line_text:?}",
                shown[i]
            );
            assert!(
                !(carried && next_starts_with_a_letter) || shows,
                "{path}: line {i} lost a hyphen the page's text carries: {:?} / {line_text:?}",
                shown[i]
            );
        }
    }
    eprintln!("checked {wraps_checked} wraps on real pages, {hyphens_carried} carrying a hyphen");
    // Not vacuous: where the Marina sheet is present, real hyphens were met
    // — so "no hyphen invented" and "no hyphen lost" were both really tested.
    assert!(!marina_seen || hyphens_carried > 0, "no real wrap hyphen was met on the Marina sheet");
}

// -- the geometry on its own -------------------------------------------

use pdf_core::document::{DrawnKind, DrawnObject, Rect};

fn drawn(kind: DrawnKind, left: f32, top: f32, right: f32, bottom: f32) -> DrawnObject {
    DrawnObject {
        object: 0,
        kind,
        rect: Rect { left, top, right, bottom },
        label: String::new(),
        depth: 0,
        opacity: 1.0,
        movable: true,
    }
}

/// A 10 pt line whose last glyph ends at x = 200 and whose baseline is at
/// y = 100 (the page's `y` grows downward).
const END: LineEnd = LineEnd { right: 200.0, baseline: 100.0, em: 10.0 };

/// A hyphen as a producer draws one: 0.3 em wide, 0.08 em thick, its
/// middle 0.28 em above the baseline, starting 0.05 em after the glyph.
fn hyphen() -> DrawnObject {
    drawn(DrawnKind::Shape, 200.5, 96.8, 203.5, 97.6)
}

#[test]
fn a_short_thin_horizontal_shape_just_after_the_line_is_a_hyphen_mark() {
    assert_eq!(wrap_hyphen_marks(&[Some(END), Some(END)], &[hyphen()]), [true, true]);
    assert!(is_hyphen_mark(&hyphen(), END));
}

#[test]
fn with_nothing_drawn_there_no_line_has_a_mark() {
    assert_eq!(wrap_hyphen_marks(&[Some(END), Some(END)], &[]), [false, false]);
}

/// The mark belongs to the line it follows, not to every line.
#[test]
fn a_mark_is_found_only_at_the_line_it_follows() {
    let second = LineEnd { right: 190.0, baseline: 112.0, em: 10.0 };
    // A hyphen after the second line (x = 190, baseline 112).
    let after_second = drawn(DrawnKind::Shape, 190.5, 108.8, 193.5, 109.6);
    assert_eq!(
        wrap_hyphen_marks(&[Some(END), Some(second), Some(END)], &[after_second]),
        [false, true, false]
    );
}

/// **What else sits at the end of a line must not read as a hyphen.** Each
/// of these differs from [`hyphen`] in exactly one respect.
#[test]
fn what_else_ends_a_line_is_not_a_hyphen() {
    let cases = [
        ("an underline, below the baseline", drawn(DrawnKind::Shape, 200.5, 101.5, 203.5, 102.3)),
        ("a rule, three ems long", drawn(DrawnKind::Shape, 200.5, 96.8, 230.5, 97.6)),
        ("a strike-through, starting inside the last word", drawn(DrawnKind::Shape, 195.0, 96.8, 198.0, 97.6)),
        ("a full stop, as tall as it is wide", drawn(DrawnKind::Shape, 200.5, 98.5, 201.5, 99.5)),
        ("an outlined letter, 0.7 em tall", drawn(DrawnKind::Shape, 200.5, 93.0, 205.0, 100.0)),
        ("a small block, 0.3 em tall", drawn(DrawnKind::Shape, 200.5, 97.0, 205.5, 100.0)),
        ("a mark a whole word away", drawn(DrawnKind::Shape, 207.0, 96.8, 210.0, 97.6)),
        ("a bar an em above the baseline", drawn(DrawnKind::Shape, 200.5, 89.6, 203.5, 90.4)),
        ("words, not a path", drawn(DrawnKind::Words, 200.5, 96.8, 203.5, 97.6)),
        ("a picture", drawn(DrawnKind::Picture, 200.5, 96.8, 203.5, 97.6)),
    ];
    for (what, shape) in cases {
        assert!(!is_hyphen_mark(&shape, END), "{what} was taken for a hyphen mark");
        assert_eq!(wrap_hyphen_marks(&[Some(END)], &[shape]), [false], "{what}");
    }
}

/// Everything is judged in ems of the line's own size: the same bar is a
/// hyphen after a 10 pt line and a speck after a 40 pt one.
#[test]
fn the_size_of_the_line_decides_what_counts_as_short() {
    assert!(is_hyphen_mark(&hyphen(), END));
    let big = LineEnd { em: 40.0, ..END };
    assert!(!is_hyphen_mark(&hyphen(), big), "a 3 pt bar is under 0.1 em of a 40 pt line");
}

#[test]
fn a_line_whose_geometry_is_unknown_has_no_mark() {
    assert_eq!(wrap_hyphen_marks(&[None, Some(END)], &[hyphen()]), [false, true]);
    assert!(!is_hyphen_mark(&hyphen(), LineEnd { em: 0.0, ..END }), "a zero-size line has no ems to measure in");
}
