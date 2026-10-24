//! A new text box's preview: its `LayoutJob`, one section per run, built from the
//! same model the box is written from (`pagify_shell::rich_text`).

use std::collections::BTreeSet;

use eframe::egui;
use pagify_shell::rich_text::{CharLook, RichText, Script};
use pagify_shell::text_layout::SCRIPT_SCALE;

/// The egui family a face is drawn in while a box uses it — see `install_fonts`.
pub(crate) fn face_family(face: &str) -> String {
    format!("pagify-face:{face}")
}

/// The box's words as egui lays them out. `text` is what its text field holds this
/// frame — the model is brought up to it on a copy first — `px_per_pt` the view's
/// scale, and `ready` the faces installed on an earlier frame: any other is drawn in
/// the program's own face until it is, since a family egui has not bound panics.
pub(crate) fn box_job(
    rich: &RichText,
    text: &str,
    px_per_pt: f32,
    ready: &BTreeSet<String>,
    wrap_width: f32,
    align: egui::Align,
) -> egui::text::LayoutJob {
    let mut rich = rich.clone();
    rich.edit(text);
    let chars: Vec<char> = text.chars().collect();
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = wrap_width;
    // Lined up as Add to Page will put the rows: the preview showed every row at the left,
    // whatever the box was aligned to, until it was applied.
    job.halign = align;

    let spans = rich.spans(0..chars.len());
    if spans.is_empty() {
        // Nothing typed yet: the caret is as tall as the typing look.
        job.append("", 0.0, format(rich.look_at(0), px_per_pt, ready));
        return job;
    }
    for (range, look) in spans {
        let words: String = chars[range].iter().collect();
        let base = format(look, px_per_pt, ready);
        if (look.width_pct - 100.0).abs() < 0.01 {
            job.append(&words, 0.0, base);
            continue;
        }
        // A width other than the letters' own: each letter spaced as wide as it will
        // be on the page, its shape left as it is — the approximation the spec names.
        let size_px = base.font_id.size;
        for (ch, advance_em) in words.chars().zip(letter_advances(&look.face, &words)) {
            let mut each = base.clone();
            each.extra_letter_spacing += (look.width_pct / 100.0 - 1.0) * advance_em * size_px;
            let mut buf = [0u8; 4];
            job.append(ch.encode_utf8(&mut buf), 0.0, each);
        }
    }
    job
}

/// Each letter's advance in ems as `face` sets all of `text` — shaped once, since the
/// layouter runs on every frame the box is drawn: the glyphs that start at a letter
/// count for it, so a letter inside a joined cluster has none. Half an em each where
/// the face cannot set it.
pub(crate) fn letter_advances(face: &str, text: &str) -> Vec<f32> {
    let Ok(shaped) = pdf_core::text::shape(face, text) else { return vec![0.5; text.chars().count()] };
    let mut by_byte = vec![0.0; text.len() + 1];
    for glyph in &shaped.glyphs {
        if let Some(advance) = by_byte.get_mut(glyph.cluster as usize) {
            *advance += glyph.advance;
        }
    }
    text.char_indices().map(|(at, _)| by_byte[at]).collect()
}

/// `base` — a format set for a box whose own size is `base_size` points — as `look`
/// asks for it: its size (a script's reduction in) in the same proportion, its colour,
/// a line under or through it, a superscript raised, its spacing after each letter, and
/// a face typed in (not one the page already had) once it is ready. The edit-text box's
/// counterpart of [`format`], which builds a whole format from a look.
pub(crate) fn look_over(
    base: &egui::TextFormat,
    look: &CharLook,
    base_size: f32,
    ready: &BTreeSet<String>,
) -> egui::TextFormat {
    let px_per_pt = base.font_id.size / base_size.max(0.01);
    let scale = if look.script == Script::Normal { 1.0 } else { SCRIPT_SCALE };
    let size_px = (look.size * px_per_pt * scale).max(1.0);
    let family = if look.page_font.is_none() && ready.contains(&look.face) {
        egui::FontFamily::Name(face_family(&look.face).into())
    } else {
        base.font_id.family.clone()
    };
    let [r, g, b, a] = look.color;
    let color = egui::Color32::from_rgba_unmultiplied(r, g, b, a);
    let line = |on: bool| if on { egui::Stroke::new((0.05 * size_px).max(1.0), color) } else { egui::Stroke::NONE };
    egui::TextFormat {
        font_id: egui::FontId::new(size_px, family),
        color,
        extra_letter_spacing: look.letter_spacing_pt * px_per_pt,
        underline: line(look.underline),
        strikethrough: line(look.strike),
        valign: if look.script == Script::Super { egui::Align::TOP } else { base.valign },
        ..base.clone()
    }
}

fn format(look: &CharLook, px_per_pt: f32, ready: &BTreeSet<String>) -> egui::TextFormat {
    let scale = if look.script == Script::Normal { 1.0 } else { SCRIPT_SCALE };
    let size_px = (look.size * px_per_pt * scale).max(1.0);
    let family = if ready.contains(&look.face) {
        egui::FontFamily::Name(face_family(&look.face).into())
    } else {
        egui::FontFamily::Proportional
    };
    let [r, g, b, a] = look.color;
    let color = egui::Color32::from_rgba_unmultiplied(r, g, b, a);
    let line = |on: bool| if on { egui::Stroke::new((0.05 * size_px).max(1.0), color) } else { egui::Stroke::NONE };
    egui::TextFormat {
        font_id: egui::FontId::new(size_px, family),
        color,
        extra_letter_spacing: look.letter_spacing_pt * px_per_pt,
        underline: line(look.underline),
        strikethrough: line(look.strike),
        // Raised to the top of the row; a subscript, like everything else, sits on its
        // bottom — a smaller size there reads as lowered.
        valign: if look.script == Script::Super { egui::Align::TOP } else { egui::Align::BOTTOM },
        ..Default::default()
    }
}
