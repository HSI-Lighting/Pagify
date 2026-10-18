//! Text-entry fields that cannot take a double-click they would crash on.
//!
//! **egui 0.36's word selection underflows on text that epaint lays out with
//! more glyphs than characters.** Any right-to-left run (Persian, Arabic,
//! Hebrew) is one way in: harfrust returns the run in visual order, the
//! cluster bookkeeping pads the wrong ranges, and a galley row ends up with
//! more glyphs than the string has characters (upstream issue
//! <https://github.com/emilk/egui/issues/8576>). Double-clicking then asks
//! for the word around a cursor that `cursor_from_pos` put *past* the last
//! character, and `ccursor_previous_word`'s `CharIndex(chars) - cursor` —
//! an unsigned subtraction — panics with "attempt to subtract with
//! overflow" inside epaint. A bare `\r` does the same thing a different
//! way: `find_line_start` only knows `\n` while `ccursor_next_line` treats
//! `\r` as a line break too, so the cursor is again beyond the "line" the
//! word code was handed.
//!
//! Reported from use: double-clicking a Persian replacement in Search &
//! Replace closed the program.
//!
//! **The same arithmetic runs for word-wise keyboard moves and deletes**
//! (`Alt`/`Ctrl` + `←`/`→`, `Alt`/`Ctrl` + Backspace/Delete, `Ctrl+W`), so
//! those are defused on risky text too.
//!
//! The real fix belongs upstream (the issue above), and moving off egui
//! 0.36 is not a thing to do inside a bug fix. So the field that is about
//! to run the offending arithmetic is drawn in a disabled scope **for that
//! one frame**: `Response::double_clicked()` is then false, the selection
//! code never runs, and nothing is lost but the word operation the user
//! asked for. Single clicks, typing, dragging a selection, copy/paste,
//! character-by-character moves and every other frame are untouched, and a
//! field whose text is ordinary left-to-right behaves exactly as before.

use egui::{Response, Ui};

/// `text` can trip egui's word-selection arithmetic if it is double-clicked.
///
/// Right-to-left scripts and bidi controls are the shaping cases the egui
/// issue measures; `\r` is the line-boundary mismatch. Erring on the side of
/// defusing a click costs a failed word selection, while a miss crashes.
pub(crate) fn risky(text: &str) -> bool {
    text.chars().any(|c| {
        matches!(
            c,
            '\r' | '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
        ) || matches!(
            u32::from(c),
            0x0590..=0x05FF // Hebrew
            | 0x0600..=0x06FF // Arabic
            | 0x0700..=0x074F // Syriac
            | 0x0750..=0x077F // Arabic Supplement
            | 0x0780..=0x07BF // Thaana
            | 0x07C0..=0x07FF // NKo
            | 0x0800..=0x083F // Samaritan
            | 0x0840..=0x085F // Mandaic
            | 0x0860..=0x086F // Syriac Supplement
            | 0x08A0..=0x08FF // Arabic Extended-A
            | 0xFB1D..=0xFB4F // Hebrew presentation forms
            | 0xFB50..=0xFDFF // Arabic presentation forms A
            | 0xFE70..=0xFEFF // Arabic presentation forms B
            | 0x1E800..=0x1EFFF // Mende Kikakui and other historic RTL scripts
        )
    })
}

/// Add a text edit built by `build`, defusing the input that would panic or
/// misbehave when `risky` (see [`risky`]).
///
/// `risky` is passed in rather than derived from a `&str` here so callers can
/// read their buffer first and hand the builder a mutable borrow afterwards —
/// the two could not be held at once.
pub(crate) fn add(ui: &mut Ui, risky: bool, build: impl FnOnce(&mut Ui) -> Response) -> Response {
    // `button_double_clicked` is already true on the press frame that makes
    // the widget's own `response.double_clicked()` true, so checking here is
    // in time. Triple-click selects a line through the same arithmetic, and
    // the word-wise keys below move/delete through it as well; each is
    // defused for the same reason.
    let dangerous = risky
        && ui.input(|i| {
            i.pointer.button_double_clicked(egui::PointerButton::Primary)
                || i.pointer.button_triple_clicked(egui::PointerButton::Primary)
                || (word_modifier(&i.modifiers)
                    && (i.key_pressed(egui::Key::ArrowLeft)
                        || i.key_pressed(egui::Key::ArrowRight)
                        || i.key_pressed(egui::Key::Backspace)
                        || i.key_pressed(egui::Key::Delete)
                        || i.key_pressed(egui::Key::W)))
        });
    if !dangerous {
        return build(ui);
    }
    ui.scope(|ui| {
        ui.disable();
        build(ui)
    })
    .inner
}

/// The modifiers egui's word-wise text operations read: `Alt` on macOS,
/// `Ctrl` (or `Cmd`, which equals `command` on both) everywhere they are
/// bound. See `move_single_cursor` and the Backspace/Delete arms in egui's
/// own text-edit code.
fn word_modifier(modifiers: &egui::Modifiers) -> bool {
    modifiers.alt || modifiers.ctrl || modifiers.command || modifiers.mac_cmd
}

#[cfg(test)]
mod tests {
    use super::risky;

    #[test]
    fn right_to_left_and_carriage_returns_are_risky() {
        assert!(risky("\u{633}\u{644}\u{627}\u{645}"), "Persian is not risky");
        assert!(risky("\u{5E9}\u{5DC}\u{5D5}\u{5DD}"), "Hebrew is not risky");
        assert!(risky("ab\rcd"), "a carriage return is not risky");
        assert!(risky("\u{200F}"), "a bidi mark is not risky");
    }

    #[test]
    fn ordinary_left_to_right_text_is_not_risky() {
        assert!(!risky(""));
        assert!(!risky("Search & Replace"));
        assert!(!risky("cafe\u{0301}"), "a composed Latin accent must still double-click");
        assert!(!risky("\u{65E5}\u{672C}\u{8A9E}"), "CJK is left-to-right here");
    }
}
