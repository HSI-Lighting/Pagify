use super::*;
use eframe::App as _;
use egui_kittest::Harness;

// -- splitting a buffer by what the face can draw ----------------------

fn all_but_b(c: char) -> bool {
    c != 'b'
}

#[test]
fn a_letter_the_face_cannot_draw_gets_a_section_of_its_own() {
    assert_eq!(
        editor_sections("a b a", &all_but_b),
        vec![(0..2, true), (2..3, false), (3..5, true)],
        "the space after `a` stays with the document's face; `b` is on its own"
    );
}

#[test]
fn neighbouring_uncovered_letters_share_a_section() {
    assert_eq!(editor_sections("abba", &all_but_b), vec![(0..1, true), (1..3, false), (3..4, true)]);
}

/// Whitespace is drawable, so a space between two uncovered letters is the
/// document's, not the fallback's — a line is not cut into a section per
/// word just because every word has a `b`.
#[test]
fn whitespace_counts_as_covered_even_between_uncovered_letters() {
    assert_eq!(editor_sections("b b", &all_but_b), vec![(0..1, false), (1..2, true), (2..3, false)]);
    assert_eq!(editor_sections("  \t", &|_| false), vec![(0..3, true)], "nothing but whitespace is all drawable");
}

#[test]
fn a_line_with_nothing_to_move_is_one_section() {
    assert_eq!(editor_sections("abc def", &|_| true), vec![(0..7, true)]);
    assert_eq!(editor_sections("bbb", &all_but_b), vec![(0..3, false)]);
    assert_eq!(editor_sections("", &all_but_b), vec![]);
}

/// The ranges are byte ranges and must fall on character boundaries.
#[test]
fn multi_byte_characters_are_not_cut_in_half() {
    // "é" is two bytes, "ü" two.
    assert_eq!(editor_sections("é b ü", &all_but_b), vec![(0..3, true), (3..4, false), (4..7, true)]);
    for text in ["é b ü", "日本 b 語", "b\nb", "a b a", ""] {
        let sections = editor_sections(text, &all_but_b);
        let tiled: String = sections.iter().map(|(range, _)| &text[range.clone()]).collect();
        assert_eq!(tiled, text, "the sections must tile {text:?} exactly");
    }
}

// -- the job the editor lays out ---------------------------------------

fn doc_format() -> egui::TextFormat {
    egui::TextFormat {
        font_id: egui::FontId::new(12.0, egui::FontFamily::Name(RUN_FAMILY.into())),
        color: egui::Color32::BLACK,
        ..Default::default()
    }
}

fn fallback_format() -> egui::TextFormat {
    egui::TextFormat {
        font_id: egui::FontId::proportional(12.0),
        color: egui::Color32::BLACK,
        ..Default::default()
    }
}

/// The slice of a job's text a section covers.
fn text_of<'a>(job: &'a egui::text::LayoutJob, section: &egui::text::LayoutSection) -> &'a str {
    &job.text[section.byte_range.start.0..section.byte_range.end.0]
}

/// Each section of a job as `(its text, whether it is the document's face)`.
fn sections_of(job: &egui::text::LayoutJob) -> Vec<(String, bool)> {
    job.sections
        .iter()
        .map(|s| {
            let is_doc = s.format.font_id.family == egui::FontFamily::Name(RUN_FAMILY.into());
            (text_of(job, s).to_string(), is_doc)
        })
        .collect()
}

/// **The job's text is the buffer, exactly, whatever the split** — the
/// cursor and the selection are positions in the buffer — and the letters
/// the face cannot draw are in the other face. Checked for a justified
/// paragraph and for plain text, and with newlines, doubled and trailing
/// spaces, and a line that is only an uncovered letter.
#[test]
fn the_job_text_is_the_buffer_and_the_uncovered_letters_use_the_other_face() {
    let (doc, fallback) = (doc_format(), fallback_format());
    for text in ["abc bab\nb\nlast b line", "bb  b \n\n b", "abc"] {
        for justify in [false, true] {
            let job = editor_layout_job(text, justify, 200.0, &doc, &fallback, &all_but_b, &mut |w| {
                w.chars().count() as f32 * 6.0
            });
            assert_eq!(job.text, text, "the job's text drifted from the buffer (justify {justify})");
            let sections = sections_of(&job);
            assert_eq!(
                sections.iter().map(|(t, _)| t.as_str()).collect::<String>(),
                text,
                "the sections must tile the text"
            );
            for (piece, is_doc) in &sections {
                // A section in the other face holds uncovered letters only;
                // one in the document's face holds none.
                assert_eq!(
                    piece.contains('b'),
                    !is_doc,
                    "{piece:?} is in the wrong face (justify {justify}, text {text:?})"
                );
            }
        }
    }
    // And there really are two fonts in a buffer that has the letter.
    let job = editor_layout_job("abc bab", false, 200.0, &doc, &fallback, &all_but_b, &mut |_| 0.0);
    let families: std::collections::HashSet<_> =
        job.sections.iter().map(|s| s.format.font_id.family.clone()).collect();
    assert_eq!(families.len(), 2, "one face for the letters it can draw, one for the rest");
    // Same size and colour in both.
    assert!(job.sections.iter().all(|s| s.format.font_id.size == 12.0 && s.format.color == egui::Color32::BLACK));
}

/// What a job comes to, section by section — text, extra space before it,
/// font and colour — for comparing two jobs. (`LayoutJob::append` merges
/// neighbouring sections that look alike, so the number of sections is
/// not something to assert; what they say is.)
fn shape_of(job: &egui::text::LayoutJob) -> (String, Vec<(String, f32, egui::FontId, egui::Color32)>) {
    (
        job.text.clone(),
        job.sections
            .iter()
            .map(|s| (text_of(job, s).to_string(), s.leading_space, s.format.font_id.clone(), s.format.color))
            .collect(),
    )
}

/// The layouter's job exactly as it was built before the document's face
/// was split by coverage — the old loop, copied here to prove that the
/// common path (nothing uncovered) comes out the same.
fn the_job_as_it_was(
    text: &str,
    justify: bool,
    wrap_width: f32,
    format: &egui::TextFormat,
    mut measure: impl FnMut(&str) -> f32,
) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let lines: Vec<&str> = text.split('\n').collect();
    let last = lines.len().saturating_sub(1);
    for (li, line) in lines.iter().enumerate() {
        if li > 0 {
            job.append("\n", 0.0, format.clone());
        }
        let tokens: Vec<&str> = line.split_inclusive(' ').collect();
        if !justify || li == last || tokens.len() < 2 {
            job.append(line, 0.0, format.clone());
            continue;
        }
        let widths: Vec<f32> = tokens.iter().map(|t| measure(t.trim_end())).collect();
        let gaps = justify_gaps(&widths, wrap_width);
        for (token, extra) in tokens.iter().zip(gaps) {
            job.append(token, extra, format.clone());
        }
    }
    job
}

/// **With nothing uncovered the job is what it always was**, justified or
/// not: the same text, and the same sections, in the same face, with the
/// same stretch before each word.
#[test]
fn a_buffer_with_nothing_to_move_is_laid_out_as_it_always_was() {
    let (doc, fallback) = (doc_format(), fallback_format());
    let measure = |w: &str| w.chars().count() as f32 * 6.5;
    for text in [
        "abc def\nxyz",
        "one two three\nfour five six\nseven",
        "",
        "a",
        "  lead and  double  \n\ntrail ",
    ] {
        for justify in [false, true] {
            let now = editor_layout_job(text, justify, 180.0, &doc, &fallback, &|_| true, &mut { measure });
            let then = the_job_as_it_was(text, justify, 180.0, &doc, measure);
            assert_eq!(shape_of(&now), shape_of(&then), "changed for {text:?} (justify {justify})");
        }
    }
    // And a justified line really was stretched in what is being compared.
    let stretched = the_job_as_it_was("one two three\nlast", true, 180.0, &doc, measure);
    assert!(stretched.sections.iter().any(|s| s.leading_space > 0.0), "setup: nothing was stretched");
}

/// **Justification is untouched**: the stretch a word gets is still put
/// before it, and when that word is split by an uncovered letter the
/// stretch goes before the *first* piece only.
#[test]
fn the_stretch_of_a_justified_word_goes_before_its_first_piece_only() {
    let (doc, fallback) = (doc_format(), fallback_format());
    // "abc" measures 24 and "bab" 18: 42 of 100 used, so 58 is the one gap's.
    let job = editor_layout_job("abc bab\nz", true, 100.0, &doc, &fallback, &all_but_b, &mut |w| {
        w.chars().count() as f32 * if w == "abc" { 8.0 } else { 6.0 }
    });
    let stretched: Vec<(String, f32)> = job
        .sections
        .iter()
        .filter(|s| s.leading_space != 0.0)
        .map(|s| (text_of(&job, s).to_string(), s.leading_space))
        .collect();
    assert_eq!(
        stretched,
        vec![("b".to_string(), 58.0)],
        "exactly one section is stretched: the first piece of the second word"
    );
    // The same stretch as the old, unsplit layout puts before that word.
    let then = the_job_as_it_was("abc bab\nz", true, 100.0, &doc, |w| {
        w.chars().count() as f32 * if w == "abc" { 8.0 } else { 6.0 }
    });
    let before_then: Vec<f32> = then.sections.iter().map(|s| s.leading_space).filter(|l| *l != 0.0).collect();
    assert_eq!(before_then, vec![58.0]);
}

// -- a real face with a real hole in it --------------------------------

/// The bundled Montserrat Regular with the outline of `c` emptied — the
/// way a subsetter leaves a glyph nothing drew: its `loca` entry made equal
/// to the next one. `cmap` and every advance width are untouched, so the
/// face still maps `c` and still gives it a width.
///
/// This crate has no font parser, so which glyph that is comes from asking
/// `outlined_chars` (tested in `pdf_core` on its own): the glyph whose
/// emptying uncovers `c`.
fn montserrat_without_ink_for(c: char) -> Vec<u8> {
    let font = BUNDLED_OUTLINED_FONTS[0];
    let u16_at = |at: usize| u16::from_be_bytes([font[at], font[at + 1]]) as usize;
    let u32_at = |at: usize| u32::from_be_bytes([font[at], font[at + 1], font[at + 2], font[at + 3]]) as usize;
    let table = |tag: &[u8; 4]| -> usize {
        (0..u16_at(4))
            .map(|i| 12 + 16 * i)
            .find(|&at| &font[at..at + 4] == tag)
            .map(|at| u32_at(at + 8))
            .unwrap_or_else(|| panic!("no {} table", String::from_utf8_lossy(tag)))
    };
    let (head, loca, maxp) = (table(b"head"), table(b"loca"), table(b"maxp"));
    let glyphs = u16_at(maxp + 4);
    let width = if u16_at(head + 50) != 0 { 4 } else { 2 };
    for glyph in 0..glyphs {
        let mut candidate = font.to_vec();
        let (from, to) = (loca + width * (glyph + 1), loca + width * glyph);
        let next = candidate[from..from + width].to_vec();
        candidate[to..to + width].copy_from_slice(&next);
        let coverage = pdf_core::pdf::embed::outlined_chars(&candidate).expect("still a face");
        if !coverage.has(c) {
            return candidate;
        }
    }
    panic!("no glyph of Montserrat gives {c:?} its ink");
}

/// **The premise behind reading symbol subtables in `outlined_chars`.** egui
/// draws a face whose letters sit in a Windows symbol `cmap` — its shaper
/// prefers that subtable over all others — so a coverage that called such a
/// face blank would move text egui draws perfectly well into the fallback
/// face. Montserrat with every `cmap` record rewritten as platform 3,
/// encoding 0: the letters are still found by egui, with ink, and the
/// coverage agrees.
#[test]
fn egui_draws_a_symbol_encoded_face_and_the_coverage_agrees() {
    let font = BUNDLED_OUTLINED_FONTS[0];
    let u16_at = |at: usize| u16::from_be_bytes([font[at], font[at + 1]]) as usize;
    let u32_at = |at: usize| u32::from_be_bytes([font[at], font[at + 1], font[at + 2], font[at + 3]]) as usize;
    let cmap = (0..u16_at(4))
        .map(|i| 12 + 16 * i)
        .find(|&at| &font[at..at + 4] == b"cmap")
        .map(|at| u32_at(at + 8))
        .expect("a cmap table");
    let mut symbol = font.to_vec();
    for i in 0..u16_at(cmap + 2) {
        let at = cmap + 4 + 8 * i;
        symbol[at..at + 2].copy_from_slice(&3u16.to_be_bytes());
        symbol[at + 2..at + 4].copy_from_slice(&0u16.to_be_bytes());
    }

    let mut job = egui::text::LayoutJob::default();
    job.append("abc", 0.0, doc_format());
    let galley = laid_out_in(symbol.clone(), job);
    assert_eq!(
        ink_by_char(&galley),
        [('a', true), ('b', true), ('c', true)],
        "egui should draw letters from a symbol-encoded face"
    );
    let coverage = pdf_core::pdf::embed::outlined_chars(&symbol).expect("a face");
    assert!(coverage.has('a') && coverage.has('b') && coverage.has('c'));
}

#[test]
fn the_test_face_is_what_it_claims_to_be() {
    let gutted = montserrat_without_ink_for('b');
    assert!(pdf_core::pdf::embed::can_spell(&gutted, "abc"), "the face still maps every letter");
    let coverage = pdf_core::pdf::embed::outlined_chars(&gutted).expect("a face");
    assert!(!coverage.has('b'));
    assert!(coverage.has('a') && coverage.has('c'));
}

/// Runs `layout` in a frame of a fresh context whose run face is `face`.
///
/// The harness settles a few frames as it is built, before the face can be
/// installed, and a family that is not bound yet panics rather than falls
/// back — see `install_fonts` — so nothing is laid out until the face has
/// been handed over and a frame has begun with it.
fn in_a_frame_with_run_face<R>(face: Vec<u8>, mut layout: impl FnMut(&mut egui::Ui) -> R) -> R {
    let installed = std::cell::Cell::new(false);
    let mut result = None;
    let mut h = Harness::new_ui(|ui: &mut egui::Ui| {
        if installed.get() {
            result = Some(layout(ui));
        }
    });
    install_fonts(&h.ctx, Some(face));
    installed.set(true);
    h.run_steps(3);
    drop(h);
    result.expect("a frame ran with the face installed")
}

/// Lays `job` out in a fresh context whose run face is `face`.
fn laid_out_in(face: Vec<u8>, job: egui::text::LayoutJob) -> std::sync::Arc<egui::Galley> {
    in_a_frame_with_run_face(face, |ui| ui.fonts_mut(|f| f.layout_job(job.clone())))
}

/// Which of a galley's glyphs have something to draw, by character.
fn ink_by_char(galley: &egui::Galley) -> Vec<(char, bool)> {
    galley
        .rows
        .iter()
        .flat_map(|row| row.glyphs.iter())
        .filter(|g| !g.chr.is_whitespace())
        .map(|g| (g.chr, !g.uv_rect.is_nothing()))
        .collect()
}

/// **The bug, in egui's own terms.** In the document's face alone a letter
/// the subset has no outline for is *found* — and drawn as nothing; the
/// two-face job draws it. Without the first half this would prove only that
/// the second half runs.
#[test]
fn egui_draws_a_blank_for_the_missing_letter_and_the_two_face_job_draws_it() {
    let gutted = montserrat_without_ink_for('b');
    let (doc, fallback) = (doc_format(), fallback_format());

    // As the editor used to lay it out: everything in the document's face.
    let mut old = egui::text::LayoutJob::default();
    old.append("abc bab", 0.0, doc.clone());
    let before = ink_by_char(&laid_out_in(gutted.clone(), old));
    assert_eq!(
        before,
        [('a', true), ('b', false), ('c', true), ('b', false), ('a', true), ('b', false)],
        "the document's face alone should draw nothing for b"
    );

    // As it lays it out now.
    let coverage = pdf_core::pdf::embed::outlined_chars(&gutted).expect("a face");
    let covered = |c: char| coverage.has(c);
    let job = editor_layout_job("abc bab", false, 200.0, &doc, &fallback, &covered, &mut |_| 0.0);
    let galley = laid_out_in(gutted, job);
    let after = ink_by_char(&galley);
    assert_eq!(
        after,
        [('a', true), ('b', true), ('c', true), ('b', true), ('a', true), ('b', true)],
        "every letter should have ink now, the b among them"
    );
    assert_eq!(galley.job.text, "abc bab", "the galley is of exactly the buffer");
    // Both faces share one baseline: the fallback `b` sits on the same line
    // as its neighbours rather than riding above or below them.
    let baselines: std::collections::BTreeSet<i32> =
        galley.rows.iter().flat_map(|row| row.glyphs.iter()).map(|g| (g.pos.y * 100.0).round() as i32).collect();
    assert_eq!(baselines.len(), 1, "the two faces are on different baselines: {baselines:?}");
}

/// A job with nothing uncovered measures the same as the plain call it
/// replaced — the justification inputs are unchanged for every word that
/// does not have a missing letter.
#[test]
fn a_word_measures_the_same_through_the_job_as_through_layout_no_wrap() {
    let (doc, fallback) = (doc_format(), fallback_format());
    let (plain, sectioned) = in_a_frame_with_run_face(BUNDLED_OUTLINED_FONTS[0].to_vec(), |ui| {
        let plain = ui
            .fonts_mut(|f| f.layout_no_wrap("accept".to_string(), doc.font_id.clone(), egui::Color32::BLACK))
            .size()
            .x;
        let mut job = egui::text::LayoutJob::default();
        append_sectioned(&mut job, "accept", 0.0, &doc, &fallback, &|_| true);
        (plain, ui.fonts_mut(|f| f.layout_job(job)).size().x)
    });
    assert!(plain > 10.0, "setup: the word has a real width ({plain})");
    assert_eq!(plain, sectioned);
}

// -- the whole editor ---------------------------------------------------

/// A one-page PDF drawing `words` in an embedded copy of `font` — written
/// the way the editing path embeds a whole face, so the page's own font is
/// the gutted one, as a producer's subset would be.
fn pdf_embedding(font: &[u8], words: &str) -> Vec<u8> {
    let embedded = pdf_core::pdf::embed::truetype(font, 5).expect("the face embeds");
    let content = format!("BT /F3 12 Tf 100 700 Td ({words}) Tj ET\n");
    let mut objects: Vec<(u32, Vec<u8>)> = vec![
        (1, b"<< /Type /Catalog /Pages 2 0 R >>".to_vec()),
        (2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec()),
        (
            3,
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R \
                 /Resources << /Font << /F3 {} 0 R >> >> >>",
                embedded.font
            )
            .into_bytes(),
        ),
        (4, format!("<< /Length {} >>\nstream\n{content}endstream", content.len()).into_bytes()),
    ];
    objects.extend(embedded.objects.iter().cloned());
    objects.sort_by_key(|(number, _)| *number);
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = std::collections::BTreeMap::new();
    for (number, body) in &objects {
        offsets.insert(*number, out.len());
        out.extend_from_slice(format!("{number} 0 obj\n").as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let size = objects.last().map(|(number, _)| number + 1).expect("objects");
    let xref_at = out.len();
    out.extend_from_slice(format!("xref\n0 {size}\n0000000000 65535 f \n").as_bytes());
    for number in 1..size {
        out.extend_from_slice(format!("{:010} 00000 n \n", offsets[&number]).as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {size} /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF").as_bytes(),
    );
    out
}

fn galleys_in(shape: &egui::Shape, found: &mut Vec<std::sync::Arc<egui::Galley>>) {
    match shape {
        egui::Shape::Text(text) => found.push(text.galley.clone()),
        egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| galleys_in(s, found)),
        _ => {}
    }
}

/// The editor open on a one-line page whose embedded face has no ink for
/// `b`, drawn for a few frames — the face installed, then in use.
fn editor_on_a_gutted_page(name: &str) -> Harness<'static, PagifyApp> {
    let dir = std::env::temp_dir().join("pagify-app-tests");
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let path = dir.join(format!("blank-glyph-{name}.pdf"));
    std::fs::write(&path, pdf_embedding(&montserrat_without_ink_for('b'), "abc bab")).expect("write the page");
    let app = PagifyApp::new(Some(&path.to_string_lossy()));
    assert!(app.tab().doc.is_some(), "the synthetic page did not open");
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 1000.0)).build_ui_state(
        |ui, app: &mut PagifyApp| {
            let mut frame = eframe::Frame::_new_kittest();
            app.ui(ui, &mut frame);
        },
        app,
    );
    h.run_steps(4);
    let run = h
        .state_mut()
        .tab()
        .doc
        .as_ref()
        .expect("open")
        .session
        .text_runs(0)
        .expect("runs")
        .into_iter()
        .find(|r| r.text.contains("abc"))
        .expect("the line");
    let at = AppPoint {
        x: ((run.rect.left + run.rect.right) / 2.0) as f64,
        y: ((run.rect.top + run.rect.bottom) / 2.0) as f64,
    };
    h.state_mut().pick_text_run(0, at).expect("a run was here");
    // One frame installs the face, the next marks it usable, then it draws.
    h.run_steps(5);
    h
}

/// **The reported case, on the real file: a heading's embedded subset has
/// no ink for b, f, j, k, q or z, and the editor opened on it drew those
/// letters as nothing.** Picks a heading near the top of the Marina mall
/// datasheet whose own face lacks them, types all six into the editor, and
/// reads what egui painted. Measured before this change on this very face:
/// `b f j k q z` all without ink.
///
/// Skipped, with a note, where the file is not on this machine.
#[test]
fn on_the_real_datasheet_the_letters_a_heading_face_never_drew_come_out() {
    let path = r"C:\Users\hsili\Desktop\Datasheets - Editors market - Marina mall.pdf";
    let app = PagifyApp::new(Some(path));
    if app.tab().doc.is_none() {
        eprintln!("skipping: Marina mall datasheet not present on this machine");
        return;
    }
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 1000.0)).build_ui_state(
        |ui, app: &mut PagifyApp| {
            let mut frame = eframe::Frame::_new_kittest();
            app.ui(ui, &mut frame);
        },
        app,
    );
    h.run_steps(4);

    // A run near the top of the page (so the editor is on screen) set in a
    // face that has no b, no f and no z but does have a and e.
    let heading = {
        let session = &h.state().tab().doc.as_ref().expect("open").session;
        session
            .text_runs(0)
            .expect("runs")
            .into_iter()
            .filter(|r| r.text.trim().chars().count() >= 6 && r.rect.top < 250.0)
            .find(|r| {
                session
                    .run_font_data(0, r.object)
                    .ok()
                    .flatten()
                    .and_then(|bytes| pdf_core::pdf::embed::outlined_chars(&bytes))
                    .is_some_and(|c| !c.has('b') && !c.has('f') && !c.has('z') && c.has('a') && c.has('e'))
            })
            .expect("a run in a face that lacks b and f — has the file changed?")
    };
    let at = AppPoint {
        x: ((heading.rect.left + heading.rect.right) / 2.0) as f64,
        y: ((heading.rect.top + heading.rect.bottom) / 2.0) as f64,
    };
    h.state_mut().pick_text_run(0, at).expect("picked the heading");
    // The paragraph around it may be set mostly in another face; the case
    // being tested is the heading's own face in the editor.
    h.state_mut().want_document_face(0, heading.object);
    let words = "bfjkqz abc bfjkqz";
    h.state_mut().tab_mut().edit.editing_run.as_mut().expect("editing").buffer = words.to_string();
    h.run_steps(6);
    assert!(h.state().editor_face_ready, "setup: the heading's face should be in use");

    let mut galleys = Vec::new();
    for clipped in &h.output().shapes {
        galleys_in(&clipped.shape, &mut galleys);
    }
    let galley = galleys.iter().find(|g| g.job.text == words).expect("the editor's painted text");
    assert!(
        ink_by_char(galley).iter().all(|(c, ink)| *ink || !c.is_alphabetic()),
        "a letter the heading's face never drew was painted as nothing: {:?}",
        ink_by_char(galley)
    );
}

/// **Picking a run reads which letters its face can draw — once.** The
/// face is the page's, so what it lacks is the page's gap, not the
/// program's.
#[test]
fn picking_a_run_reads_the_coverage_of_its_face_once() {
    let mut h = editor_on_a_gutted_page("coverage");
    let first = h.state().editor_face_coverage.clone().expect("the face's coverage should have been read");
    assert!(!first.has('b'), "the page's face has no ink for b");
    assert!(first.has('a') && first.has('c'));

    // Picking again — the same face — must not read it all over again.
    let run = h.state().tab().edit.editing_run.as_ref().expect("editing").clone();
    let at = AppPoint {
        x: ((run.rect.left + run.rect.right) / 2.0) as f64,
        y: ((run.rect.top + run.rect.bottom) / 2.0) as f64,
    };
    h.state_mut().tab_mut().edit.editing_run = None;
    h.state_mut().pick_text_run(0, at).expect("picked again");
    let second = h.state().editor_face_coverage.clone().expect("still there");
    assert!(std::sync::Arc::ptr_eq(&first, &second), "the same face was read a second time");
}

/// **The editor, opened on that page, draws the letter its face lacks.**
/// The galley egui painted for the box is of exactly the buffer, every
/// letter in it has ink — the `b`s too — and it is made of two fonts.
#[test]
fn the_open_editor_draws_ink_for_a_letter_its_face_never_drew() {
    let h = editor_on_a_gutted_page("editor");
    let buffer = h.state().tab().edit.editing_run.as_ref().expect("editing").buffer.clone();
    assert_eq!(buffer, "abc bab", "setup: the page's own words");
    assert!(h.state().editor_face_ready, "setup: the document's face should be installed and in use");

    let mut galleys = Vec::new();
    for clipped in &h.output().shapes {
        galleys_in(&clipped.shape, &mut galleys);
    }
    let galley = galleys
        .iter()
        .find(|g| g.job.text == buffer)
        .unwrap_or_else(|| panic!("no painted text is the buffer {buffer:?}; saw {:?}", galleys.iter().map(|g| g.job.text.clone()).collect::<Vec<_>>()));

    assert_eq!(
        ink_by_char(galley),
        [('a', true), ('b', true), ('c', true), ('b', true), ('a', true), ('b', true)],
        "the editor drew nothing for a letter the page's face never drew"
    );
    let families: std::collections::HashSet<_> =
        galley.job.sections.iter().map(|s| s.format.font_id.family.clone()).collect();
    assert!(families.contains(&egui::FontFamily::Name(RUN_FAMILY.into())), "the page's face was not used at all");
    assert!(families.contains(&egui::FontFamily::Proportional), "nothing was laid out in the other face");
}
