//! Sizes and sectioning for the run editor, as plain arithmetic.
//!
//! Moved here from `pagify_app` (design review Phase 4a): every function is a
//! function of numbers and text alone — no document, no PDFium, no egui — so
//! the decisions can be reasoned about and tested without a window. The app
//! draws with the answers; it does not compute them.

/// The size the run editor draws its words at — the size they are *drawn*,
/// not the number in the file. Plenty of producers write `1 Tf` and put the
/// real size in the text matrix — `pdf_core` already knows that, and says so
/// where `TJ` displacements are scaled — so the box a run occupies, not its
/// nominal size, is the fallback: taking the nominal size put the editor at
/// one point and the words came out as a whisper. Ink is most of an em.
///
/// **One line's worth of that box, not the whole thing.** `box_height` is
/// the *union* of every line for a paragraph — see `EditingRun::lines` —
/// and a font the height of eight stacked lines is not "the size the words
/// are drawn", it is eight of them stacked and then some. Reported from
/// use: opening a paragraph filled the screen with enormous type, wrapping
/// mid-word because nothing that large could fit the box's own width
/// either.
///
/// **`em_ratio`, when it is known, replaces the fixed `0.92` guess.** That
/// constant fits no particular face especially well — a font with deep
/// descenders and one with almost none do not turn a box height into a
/// point size by the same fraction. `em_ratio` is a specific font's own
/// `(ascent - descent) / 1000`, read once when its face is loaded, so the
/// fallback divides by what this face actually is rather than an average of
/// every face. `None` keeps the old constant — the program's own substitute
/// font, whose metrics were never asked for.
pub fn run_editor_font_size(
    box_height: f32,
    line_count: usize,
    requested_size: f32,
    view_scale: f32,
    em_ratio: Option<f32>,
) -> f32 {
    // A bare epsilon, not a readable pixel size — see `draw_run_editor`'s own
    // `base_screen_height` doc for why a floor here has to stay far below
    // anything a real zoom level would reach: a bigger one would make the
    // choice between `nominal` and this fallback flip at some zoom purely
    // because the floor stopped `per_line` shrinking while `nominal` kept
    // shrinking, not because either genuinely changed size.
    let per_line = (box_height.max(0.5)) / line_count.max(1) as f32;
    let nominal = requested_size * view_scale;
    if nominal >= per_line * 0.5 {
        return nominal;
    }
    match em_ratio {
        Some(ratio) if ratio > 0.05 => per_line / ratio,
        _ => per_line * 0.92,
    }
}

/// How much bigger — or smaller — than the run's own opening size the run
/// editor's box should draw itself right now, given the screen size that
/// size drew at (`base_on_screen`) and what the *current* `Size` control
/// draws at (`current_on_screen`). `1.0` when nothing has changed.
///
/// **Reported from use: "it locked in a text box, so i cant see the actual
/// scale it will be once i increase the font size."** `draw_run_editor`
/// already tracked the Size slider live for the font drawn *inside* the
/// box; the box itself stayed pinned to whatever rectangle the run measured
/// when the editor opened, so a bigger size only ever crowded or overflowed
/// that fixed frame. This is the ratio `draw_run_editor` now scales both of
/// the box's own screen dimensions by, so growing the size is something a
/// person watches happen rather than discovers after applying it.
///
/// Clamped well short of where a screen coordinate would misbehave — a size
/// of literally zero, or a division by a `base_on_screen` rounded to
/// nothing, must shrink or grow the box, never collapse or explode it.
pub fn run_editor_box_grow(base_on_screen: f32, current_on_screen: f32) -> f32 {
    (current_on_screen / base_on_screen.max(1.0)).clamp(0.1, 20.0)
}

/// The screen-pixel size the run editor actually draws its glyphs at, given
/// what [`run_editor_font_size`] computed for the current zoom.
///
/// **Deliberately not clamped to a fixed pixel range.** `on_screen` is
/// already exactly proportional to `view.scale` — the same zoom that shrinks
/// and grows everything else on the page — and a floor or a ceiling on top
/// of that breaks exactly that proportionality the moment either end of it
/// is reached: the rest of the page keeps scaling with the zoom and this
/// stops, so the editor visibly grows or shrinks *relative* to the page
/// instead of staying the one size it always was next to it. Reported from
/// use as the preview's own scale changing as the page was zoomed in and
/// out. A page's own text has no such floor either — at extreme zoom it
/// gets exactly as small or as large as the arithmetic says, and this now
/// matches it. Only a hard floor far below anything a zoom level would
/// plausibly reach, so a literal zero can never reach `FontId`.
pub fn run_editor_glyph_size(on_screen: f32) -> f32 {
    on_screen.max(0.5)
}

/// Splits `text` into the stretches the document's own face can draw and the
/// stretches it cannot, as `(byte range, drawable)` pairs that tile the whole
/// string in order. `covered` says whether the face has ink for a character.
///
/// **Whitespace is always drawable.** Every face maps a space to an empty
/// glyph, so "has ink" would send each space to the fallback face and chop a
/// line into a section per word; a space belongs to whichever section it
/// sits in the middle of, and to the document's own face between two
/// uncovered letters.
///
/// **Why this exists — reported from use, with a screenshot: a paragraph
/// opened in the document's own face came up with letters missing, and the
/// heading it began with was bold besides.** The face installed for the
/// editor is the document's embedded *subset*, which keeps its whole `cmap`
/// and every advance width but has no outline for a letter the page never
/// drew in that face — on one real datasheet a heading's subset has none for
/// b, f, j, k, q or z. egui finds such a letter in the document's face, draws
/// the empty glyph, and never reaches the fallback face behind it in the
/// family. So those letters are laid out in a separate section, in the
/// fallback face, and the rest stays in the document's.
pub fn editor_sections(
    text: &str,
    covered: &dyn Fn(char) -> bool,
) -> Vec<(std::ops::Range<usize>, bool)> {
    let mut sections: Vec<(std::ops::Range<usize>, bool)> = Vec::new();
    for (at, c) in text.char_indices() {
        let drawable = c.is_whitespace() || covered(c);
        let end = at + c.len_utf8();
        match sections.last_mut() {
            Some((range, was)) if *was == drawable => range.end = end,
            _ => sections.push((at..end, drawable)),
        }
    }
    sections
}

/// Round to the nearest multiple of 8, not always down — used to group
/// "near-identical shades of one background" together while sampling. A
/// plain `& 0xF8` sends 255 (true white, the single most common page
/// background there is) to 248, so a run editor opened over a plain white
/// page sat on a visibly darker patch than the real page around it —
/// reported from use as the editor still looking boxed after its frame was
/// already fixed.
pub fn quantize_to_nearest_8(c: u8) -> u8 {
    (((c as u16 + 4) / 8 * 8).min(255)) as u8
}
