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

// Used by `draw_run_editor` (Task 4 of the in-place plan); until then only the tests are.
#![allow(dead_code)]

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
    fn the_caret_is_white_over_pale_ink() {
        assert_eq!(caret_on(Color32::WHITE), Color32::WHITE);
        assert_eq!(caret_on(Color32::BLACK), Color32::BLACK);
        assert_eq!(caret_on(Color32::from_rgb(0x14, 0x73, 0xE6)), Color32::BLACK);
        assert_eq!(caret_on(Color32::from_rgb(0xF2, 0xF2, 0xA0)), Color32::WHITE);
    }
}
