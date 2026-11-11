//! One line of text in the order it is seen, for a window that lays text out left
//! to right.
//!
//! egui 0.36 shapes each stretch of one font with harfrust, which joins Arabic
//! letters and turns a right-to-left stretch around itself — but it lays the
//! stretches out left to right in the order they were typed, and harfrust takes a
//! stretch's direction from its script alone. A Persian file name came out with
//! every word right and the words backwards. The Unicode bidi algorithm decides
//! the order here; harfrust is left only what it does right, one stretch of
//! right-to-left letters.

use unicode_bidi::BidiInfo;

/// `text` as the pieces to draw left to right, one egui section each: a piece of
/// right-to-left letters in typed order (harfrust turns it around), anything else
/// as it is to be seen. A piece that has to be seen left to right, digits in a
/// Persian name, is handed over turned around, because harfrust will turn it again.
pub fn visual_pieces(text: &str) -> Vec<String> {
    let bidi = BidiInfo::new(text, None);
    let mut out = Vec::new();
    for paragraph in &bidi.paragraphs {
        let (levels, runs) = bidi.visual_runs(paragraph, paragraph.range.clone());
        for run in runs {
            let rtl = levels[run.start].is_rtl();
            // Stretches of right-to-left script, and of everything else. A joiner
            // (ZWNJ, ZWJ) stays with the letter before it, as egui keeps it.
            let mut pieces: Vec<(bool, String)> = Vec::new();
            for c in text[run].chars() {
                let joiner = matches!(c, '\u{200C}' | '\u{200D}');
                match pieces.last_mut() {
                    Some(last) if joiner || last.0 == rtl_script(c) => last.1.push(c),
                    _ => pieces.push((rtl_script(c), c.to_string())),
                }
            }
            if rtl {
                pieces.reverse();
            }
            for (letters, piece) in pieces {
                out.push(match (rtl, letters) {
                    (true, true) | (false, false) => piece,
                    (true, false) => piece.chars().rev().map(mirrored).collect(),
                    (false, true) => piece.chars().rev().collect(),
                });
            }
        }
    }
    out
}

/// Whether `text` holds any right-to-left letter (see [`rtl_script`]).
pub fn has_right_to_left(text: &str) -> bool {
    text.chars().any(rtl_script)
}

/// A character harfrust shapes right to left: Hebrew, Arabic, Syriac, Thaana, N'Ko
/// and the rest of those blocks, and their presentation forms.
///
/// `ponytail:` by block, not by the Unicode Script property — the Arabic comma, which is
/// Common, counts as Arabic. It only decides where a piece ends.
fn rtl_script(c: char) -> bool {
    matches!(c as u32, 0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFC | 0x10800..=0x10FFF | 0x1E800..=0x1EFFF)
}

/// A bracket seen in right-to-left text faces the other way.
fn mirrored(c: char) -> char {
    match c {
        '(' => ')',
        ')' => '(',
        '[' => ']',
        ']' => '[',
        '{' => '}',
        '}' => '{',
        '<' => '>',
        '>' => '<',
        '«' => '»',
        '»' => '«',
        _ => c,
    }
}

#[cfg(test)]
mod tests {
    use super::visual_pieces;

    #[test]
    fn a_persian_name_reads_from_the_right_with_its_extension_on_the_left() {
        assert_eq!(
            visual_pieces("چالش_های موجود و  راهکارها.pdf"),
            ["pdf", ".", "راهکارها", "  ", "و", " ", "موجود", " ", "های", "_", "چالش"]
        );
    }

    #[test]
    fn a_latin_name_is_one_piece_as_typed() {
        assert_eq!(visual_pieces("AA125 -LIGHTING LAYOUT (rev 2).pdf"), ["AA125 -LIGHTING LAYOUT (rev 2).pdf"]);
        assert!(visual_pieces("").is_empty());
    }

    #[test]
    fn a_persian_word_in_a_latin_name_stays_where_it_was_typed() {
        assert_eq!(visual_pieces("Report گزارش.pdf"), ["Report ", "گزارش", ".pdf"]);
    }

    #[test]
    fn persian_digits_are_seen_left_to_right_so_harfrust_is_handed_them_turned() {
        assert_eq!(visual_pieces("گزارش ۱۴۰۳.pdf"), ["pdf", ".", "۳۰۴۱", " ", "گزارش"]);
    }

    #[test]
    fn brackets_in_a_persian_name_face_the_way_it_reads() {
        assert_eq!(visual_pieces("گزارش (نهایی).pdf"), ["pdf", ".(", "نهایی", ") ", "گزارش"]);
    }

    #[test]
    fn a_joiner_stays_in_its_word() {
        assert_eq!(visual_pieces("چالش\u{200C}ها"), ["چالش\u{200C}ها"]);
    }
}
