// Adapted from PdfCraft (https://github.com/storytold/pdfcraft, crates/ui-egui/src/edit_text_ui.rs @0dc77fe).
// Copyright (c) 2026 ArtCraft Team and the PdfCraft contributors. MIT License; see THIRD_PARTY_NOTICES.md.

//! Editing a page's own text **where it is**, not in a box laid over it.
//!
//! The Edit Text box used to cover the paragraph with a sheet of paper and set its own
//! text on that, so the thing being edited stopped looking like the thing on the page.
//! The idea, from PdfCraft, is that the editor paints nothing of its own until the
//! words change: the page's picture is what shows, and the editor's text is laid out
//! (so the caret, the selection and a click all land on real letters) but drawn in no
//! colour. These are the pieces of that which need no window and no document.

use std::sync::Arc;

use egui::{Color32, Rect};

/// `galley` drawn in no colour: its rows still place the caret and the selection.
pub(crate) fn unseen(mut galley: Arc<egui::Galley>) -> Arc<egui::Galley> {
    for placed in &mut Arc::make_mut(&mut galley).rows {
        for vertex in &mut Arc::make_mut(&mut placed.row).visuals.mesh.vertices {
            vertex.color = Color32::TRANSPARENT;
        }
    }
    galley
}

/// `galley` with the rows listed in `rows` (`true` = blank it) drawn in no colour, the others as
/// they are. **For the lines the box does not own**: a line the page draws as shapes stays on the
/// page under the box, but its words are still in the box's text, and once the rest is visible
/// they would be painted over those shapes.
pub(crate) fn unseen_rows(mut galley: Arc<egui::Galley>, rows: &[bool]) -> Arc<egui::Galley> {
    if !rows.iter().any(|blank| *blank) {
        return galley;
    }
    for (index, placed) in Arc::make_mut(&mut galley).rows.iter_mut().enumerate() {
        if rows.get(index).copied().unwrap_or(false) {
            for vertex in &mut Arc::make_mut(&mut placed.row).visuals.mesh.vertices {
                vertex.color = Color32::TRANSPARENT;
            }
        }
    }
    galley
}

/// Where one row of the editor's text must sit: its left edge, its baseline, and how far
/// its tallest letter rises above it and its lowest falls below it. Screen pixels,
/// from the galley's own origin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RowTarget {
    pub left: f32,
    pub baseline: f32,
    pub ascent: f32,
    pub descent: f32,
}

/// `galley` with each row moved onto its page line: row *i* takes `targets[i]`.
///
/// **Glyphs, meshes, caret and selection take the same move**, because egui lays rows
/// out on its own line height and the page's lines are somewhere else. A row past the
/// last target sits `leading` below the one before it (a line typed beyond the page's
/// own), keeping the last target's left edge, ascent and descent. With no targets the
/// galley is as it was.
pub(crate) fn snap_rows(mut galley: Arc<egui::Galley>, targets: &[RowTarget], leading: f32) -> Arc<egui::Galley> {
    let Some(last) = targets.last().copied() else { return galley };
    let g = Arc::make_mut(&mut galley);
    let mut bounds = Rect::NOTHING;
    let mut meshes = Rect::NOTHING;
    for (index, placed) in g.rows.iter_mut().enumerate() {
        let target = targets.get(index).copied().unwrap_or(RowTarget {
            baseline: last.baseline + index.saturating_add(1).saturating_sub(targets.len()) as f32 * leading,
            ..last
        });
        let row = Arc::make_mut(&mut placed.row);
        placed.pos.y = target.baseline - target.ascent;
        placed.pos.x = target.left;
        for glyph in &mut row.glyphs {
            let dy = target.ascent - glyph.pos.y;
            glyph.pos.y = target.ascent;
            glyph.font_ascent = target.ascent;
            glyph.font_height = target.ascent + target.descent;
            glyph.line_height = target.ascent + target.descent;
            if !glyph.uv_rect.is_nothing() {
                for vertex in row.visuals.mesh.vertices.iter_mut().skip(glyph.first_vertex as usize).take(4) {
                    vertex.pos.y += dy;
                }
            }
        }
        row.size.y = (target.ascent + target.descent).max(1.0);
        row.visuals.mesh_bounds = row.visuals.mesh.vertices.iter().fold(Rect::NOTHING, |r, v| r.union(Rect::from_min_max(v.pos, v.pos)));
        bounds = bounds.union(Rect::from_min_size(placed.pos, row.size));
        meshes = meshes.union(row.visuals.mesh_bounds.translate(placed.pos.to_vec2()));
    }
    g.rect = Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(bounds.right().max(0.0), bounds.bottom().max(0.0)));
    g.mesh_bounds = meshes;
    galley
}

/// The extra space after each letter that makes a line of `chars` letters, laid out
/// `natural` px wide, `target` px wide — spread over the gaps between letters. A line of
/// one letter has no gap, so it takes the whole difference.
pub(crate) fn fit_spacing(natural: f32, target: f32, chars: usize) -> f32 {
    (target - natural) / chars.saturating_sub(1).max(1) as f32
}

/// `galley` with each row that has a width to fit spread so that **its last letter** (not its last
/// space) ends exactly `widths[i]` px from the row's start; `None` leaves a row as laid out. The
/// first letter stays; each later one takes its share of the difference ([`fit_spacing`]), glyph and
/// drawn mesh alike, so a line set in egui's font ends where the page's letters do and the caret and
/// a click land on the letters the page shows. **The spaces after the last letter keep their own
/// width**: a page's text box can run well past its letters (a table cell's padding), and only the
/// letters are what the person points at.
pub(crate) fn fit_rows(mut galley: Arc<egui::Galley>, widths: &[Option<f32>]) -> Arc<egui::Galley> {
    let g = Arc::make_mut(&mut galley);
    let mut bounds = Rect::NOTHING;
    let mut meshes = Rect::NOTHING;
    for (index, placed) in g.rows.iter_mut().enumerate() {
        if let Some(target) = widths.get(index).copied().flatten().filter(|w| w.is_finite() && *w > 0.0) {
            let row = Arc::make_mut(&mut placed.row);
            // A row of nothing but spaces has no letters to fit.
            if let Some(last_letter) = row.glyphs.iter().rposition(|g| !g.chr.is_whitespace()) {
                // From the last letter's right edge: the row's own width is rounded up to the pixel.
                let natural = row.glyphs[last_letter].pos.x + row.glyphs[last_letter].advance_width;
                let spacing = fit_spacing(natural, target, last_letter + 1);
                for (letter, glyph) in row.glyphs.iter_mut().enumerate() {
                    let dx = letter.min(last_letter) as f32 * spacing;
                    glyph.pos.x += dx;
                    if !glyph.uv_rect.is_nothing() {
                        for vertex in row.visuals.mesh.vertices.iter_mut().skip(glyph.first_vertex as usize).take(4) {
                            vertex.pos.x += dx;
                        }
                    }
                }
                row.size.x = row.glyphs.last().map_or(target, |g| g.pos.x + g.advance_width);
            }
            row.visuals.mesh_bounds = row.visuals.mesh.vertices.iter().fold(Rect::NOTHING, |r, v| r.union(Rect::from_min_max(v.pos, v.pos)));
        }
        bounds = bounds.union(Rect::from_min_size(placed.pos, placed.row.size));
        meshes = meshes.union(placed.row.visuals.mesh_bounds.translate(placed.pos.to_vec2()));
    }
    g.rect = Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(bounds.right().max(0.0), bounds.bottom().max(0.0)));
    g.mesh_bounds = meshes;
    galley
}

/// Whether a box holding `buffer` asks for nothing over what `original` said: the same
/// words (their edges aside), and no look changed (`look_dirty`). Typed away entirely is a
/// change, unless there was nothing to delete. **The one rule** for "the box is still the
/// page's own text", read by the editor's invisible text, the paper under it and the
/// picture of the page without it.
pub(crate) fn unchanged(buffer: &str, original: &str, look_dirty: bool) -> bool {
    let typed = buffer.trim();
    if typed.is_empty() {
        return original.trim().is_empty();
    }
    typed == original.trim() && !look_dirty
}

/// The look of an edited paragraph, held after it is applied: **the page's own picture of
/// the edited page does not exist until it is rendered again**, so for that moment the page
/// without the old words (`picture`) and the typed words (`galley`) stay up, and nothing
/// flashes the old text or the bare paper. Good for one zoom and turn of the view.
#[derive(Clone)]
pub(crate) struct Settling {
    pub(crate) page: usize,
    /// The document's `render_epoch` while the box was open; an applied edit moves it.
    pub(crate) epoch: u64,
    pub(crate) zoom: f32,
    pub(crate) rotation: u8,
    pub(crate) picture: egui::TextureHandle,
    pub(crate) galley: Arc<egui::Galley>,
    /// Where the galley's corner is, in page points from the page's corner.
    pub(crate) at_pt: (f32, f32),
    pub(crate) ink: Color32,
    /// When it was last refreshed, `ctx.input(|i| i.time)`: it is let go of after [`SETTLE_SECS`].
    pub(crate) since: f64,
}

/// The longest a settling look is held. A page renders again in well under this; past it
/// something is wrong with the render and the page's own state is the honest thing to show.
pub(crate) const SETTLE_SECS: f64 = 3.0;

/// A caret colour that shows on the page behind `ink`: with nothing drawn behind the
/// editor, dark ink means a light page, so black; pale ink means a dark band, so white.
pub(crate) fn caret_on(ink: Color32) -> Color32 {
    let luma = 299 * u32::from(ink.r()) + 587 * u32::from(ink.g()) + 114 * u32::from(ink.b());
    if luma > 127_500 { Color32::WHITE } else { Color32::BLACK }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `text` laid out in egui's own font, 20 px, as the editor would.
    fn laid_out(text: &str) -> Arc<egui::Galley> {
        let out = std::rc::Rc::new(std::cell::RefCell::new(None));
        let sink = out.clone();
        let text = text.to_string();
        let mut harness = egui_kittest::Harness::new_ui(move |ui| {
            let mut job = egui::text::LayoutJob::default();
            job.append(
                &text,
                0.0,
                egui::TextFormat { font_id: egui::FontId::proportional(20.0), color: Color32::BLACK, ..Default::default() },
            );
            *sink.borrow_mut() = Some(ui.fonts_mut(|f| f.layout_job(job)));
        });
        harness.run();
        let galley = out.borrow_mut().take().expect("the text was laid out");
        galley
    }

    #[test]
    fn unseen_draws_nothing_but_keeps_the_rows() {
        let before = laid_out("one\ntwo");
        let after = unseen(before.clone());
        assert_eq!(after.rows.len(), before.rows.len());
        for (a, b) in after.rows.iter().zip(&before.rows) {
            assert_eq!(a.pos, b.pos);
            assert_eq!(a.row.size, b.row.size);
            assert!(!a.row.visuals.mesh.vertices.is_empty(), "no letters were laid out, so this proves nothing");
            assert!(a.row.visuals.mesh.vertices.iter().all(|v| v.color == Color32::TRANSPARENT));
        }
        assert!(before.rows[0].row.visuals.mesh.vertices.iter().any(|v| v.color != Color32::TRANSPARENT));
    }

    #[test]
    fn each_row_lands_on_its_baseline() {
        let targets = [
            RowTarget { left: 5.0, baseline: 10.0, ascent: 8.0, descent: 3.0 },
            RowTarget { left: 17.0, baseline: 40.0, ascent: 8.0, descent: 3.0 },
        ];
        let galley = snap_rows(laid_out("one\ntwo"), &targets, 30.0);
        for (row, target) in galley.rows.iter().zip(&targets) {
            let glyph = row.row.glyphs.first().expect("a letter");
            assert!((row.pos.y + glyph.pos.y - target.baseline).abs() < 0.01, "the letters' baseline");
            assert!((row.pos.y + target.ascent - target.baseline).abs() < 0.01, "the row's top");
            assert!((row.pos.x - target.left).abs() < 0.01, "the row's left edge");
        }
        // The galley's own extent follows, or the caret and the box would clip the rows.
        assert!(galley.rect.bottom() >= 40.0 + 3.0 - 0.01);
        // The drawn letters moved with their rows, not only the numbers about them.
        let drawn = galley.mesh_bounds;
        assert!(drawn.bottom() > 40.0 - 8.0 && drawn.top() < 40.0, "second row's letters at {drawn:?}");
    }

    #[test]
    fn a_row_past_the_targets_follows_the_leading() {
        let targets = [
            RowTarget { left: 0.0, baseline: 10.0, ascent: 8.0, descent: 3.0 },
            RowTarget { left: 0.0, baseline: 40.0, ascent: 8.0, descent: 3.0 },
        ];
        let galley = snap_rows(laid_out("a\nb\nc"), &targets, 30.0);
        assert_eq!(galley.rows.len(), 3);
        assert!((galley.rows[2].pos.y + 8.0 - 70.0).abs() < 0.01, "third row at {}", galley.rows[2].pos.y);
    }

    #[test]
    fn with_no_targets_the_galley_is_as_it_was() {
        let before = laid_out("one\ntwo");
        let after = snap_rows(before.clone(), &[], 30.0);
        assert_eq!(after.rows[1].pos, before.rows[1].pos);
    }

    #[test]
    fn fit_spacing_spreads_the_difference_over_the_gaps() {
        assert_eq!(fit_spacing(100.0, 110.0, 11), 1.0);
        assert_eq!(fit_spacing(10.0, 12.0, 1), 2.0);
        assert_eq!(fit_spacing(10.0, 12.0, 0), 2.0);
        assert_eq!(fit_spacing(100.0, 90.0, 11), -1.0);
    }

    #[test]
    fn a_row_fitted_to_a_width_ends_there_and_keeps_its_start() {
        let before = laid_out("hello world");
        let natural = before.rows[0].row.glyphs.last().map(|g| g.pos.x + g.advance_width).expect("letters");
        let after = fit_rows(before.clone(), &[Some(natural + 30.0)]);
        let (first, last) = (after.rows[0].row.glyphs.first().expect("a letter"), after.rows[0].row.glyphs.last().expect("a letter"));
        assert_eq!(first.pos.x, before.rows[0].row.glyphs[0].pos.x, "the first letter does not move");
        assert!((last.pos.x + last.advance_width - (natural + 30.0)).abs() < 0.01, "the last letter ends at {}", last.pos.x + last.advance_width);
        assert!((after.rows[0].row.size.x - (natural + 30.0)).abs() < 0.01, "the row is as wide as asked");
        // The drawn letters moved with their numbers.
        assert!(after.mesh_bounds.right() > before.mesh_bounds.right() + 25.0, "{:?} {:?}", before.mesh_bounds, after.mesh_bounds);
    }

    #[test]
    fn a_row_with_trailing_spaces_is_fitted_by_its_last_letter() {
        let before = laid_out("hello   ");
        let right_of = |galley: &egui::Galley, last_letter: bool| {
            let glyphs = &galley.rows[0].row.glyphs;
            let glyph = if last_letter { glyphs.iter().rev().find(|g| !g.chr.is_whitespace()) } else { glyphs.last() };
            let glyph = glyph.expect("a glyph");
            glyph.pos.x + glyph.advance_width
        };
        let letters = right_of(&before, true);
        let after = fit_rows(before.clone(), &[Some(letters + 20.0)]);
        assert!((right_of(&after, true) - (letters + 20.0)).abs() < 0.01, "the last letter ends at {}", right_of(&after, true));
        // The spaces after it keep their own width: they are not stretched with the letters.
        let trailing = |galley: &egui::Galley| right_of(galley, false) - right_of(galley, true);
        assert!((trailing(&after) - trailing(&before)).abs() < 0.01, "the spaces went from {} to {}", trailing(&before), trailing(&after));
    }

    #[test]
    fn a_row_with_no_width_to_fit_is_left_alone() {
        let before = laid_out("one
two");
        let after = fit_rows(before.clone(), &[None, Some(before.rows[1].row.size.x + 10.0)]);
        assert_eq!(after.rows[0].row.glyphs[2].pos, before.rows[0].row.glyphs[2].pos);
        assert!(after.rows[1].row.glyphs[2].pos.x > before.rows[1].row.glyphs[2].pos.x);
    }

    #[test]
    fn words_are_unchanged_when_only_the_edges_differ() {
        assert!(unchanged("alpha beta", "alpha beta", false));
        assert!(unchanged("alpha beta 
", "alpha beta", false));
        assert!(!unchanged("alpha betax", "alpha beta", false));
        assert!(!unchanged("alpha beta", "alpha beta", true), "a look changed");
        assert!(!unchanged("", "alpha beta", false), "typed away is a change");
        assert!(unchanged("", "  ", true), "nothing to delete");
    }

    #[test]
    fn unseen_rows_blanks_only_the_rows_it_is_told_to() {
        let galley = unseen_rows(laid_out("one\ntwo\nthree"), &[false, true]);
        let visible = |row: usize| galley.rows[row].row.visuals.mesh.vertices.iter().any(|v| v.color.a() > 0);
        assert!(visible(0), "row 0 was not asked for");
        assert!(!visible(1), "row 1 was");
        assert!(visible(2), "row 2 is past the list, so it is left alone");
        assert_eq!(unseen_rows(laid_out("one"), &[]).rows.len(), 1, "an empty list changes nothing");
    }

    #[test]
    fn the_caret_is_white_over_pale_ink() {
        assert_eq!(caret_on(Color32::WHITE), Color32::WHITE);
        assert_eq!(caret_on(Color32::BLACK), Color32::BLACK);
        assert_eq!(caret_on(Color32::from_rgb(0x14, 0x73, 0xE6)), Color32::BLACK);
        assert_eq!(caret_on(Color32::from_rgb(0xF2, 0xF2, 0xA0)), Color32::WHITE);
    }
}
