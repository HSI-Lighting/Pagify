use super::looks_rotated;
use pdf_core::document::Rect;

/// **The exact box a real page's rotated "54mm" dimension label
/// reported**: narrower than it is tall by nearly 4 to 1.
#[test]
fn a_narrow_tall_multi_character_box_looks_rotated() {
    let rect = Rect { left: 429.6, top: 74.6, right: 434.2, bottom: 91.6 };
    assert!(looks_rotated(&rect, "54mm".chars().count()));
}

/// An ordinary line of body text — wide, short — never looks rotated,
/// however many characters it holds.
#[test]
fn an_ordinary_line_of_text_does_not_look_rotated() {
    let rect = Rect { left: 194.6, top: 127.8, right: 366.6, bottom: 135.4 };
    assert!(!looks_rotated(&rect, "The sleek and modern design of Camino ".chars().count()));
}

/// A short run of narrow letters ("Ill") can be legitimately taller
/// than wide at a large size without being rotated at all — the
/// character-count gate exists precisely to leave these alone.
#[test]
fn a_short_run_of_narrow_letters_is_not_flagged() {
    let rect = Rect { left: 0.0, top: 0.0, right: 8.0, bottom: 20.0 };
    assert!(!looks_rotated(&rect, "Ill".chars().count()));
}

#[test]
fn an_empty_run_is_never_flagged() {
    let rect = Rect { left: 0.0, top: 0.0, right: 1.0, bottom: 50.0 };
    assert!(!looks_rotated(&rect, 0));
}
