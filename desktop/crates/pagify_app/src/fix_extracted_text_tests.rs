use super::fix_extracted_text;

/// **The bug, pinned down**: a `\u{2}` (STX) sitting where a hyphen
/// should be, exactly as one real page's own broken `ToUnicode` mapping
/// produced — put back as `-`, since a letter sits on both sides.
#[test]
fn a_control_character_between_letters_becomes_a_hyphen() {
    assert_eq!(fix_extracted_text("elitee\u{2}plus"), "elitee-plus");
}

/// **Reported a second time, with a screenshot: the first fix dropped
/// this same character outright, turning "elitee-plus" into
/// "eliteeplus" wherever the mapping bug's hyphen happened to fall
/// exactly on a line wrap.** The letter after it is on the far side of
/// the `\n` that joins two lines together, not immediately next to it —
/// this is the case that requires looking past the newline rather than
/// only at the one character right next door.
#[test]
fn a_control_character_is_still_a_hyphen_across_a_line_wrap() {
    assert_eq!(fix_extracted_text("Camino elitee\u{2}\nplus 3.0"), "Camino elitee-\nplus 3.0");
}

/// With no letter on one side — the very start of the text, here — a
/// control character has no signal to be read as a hyphen and is
/// simply dropped.
#[test]
fn a_control_character_with_no_letter_beside_it_is_dropped() {
    assert_eq!(fix_extracted_text("\u{2}plus"), "plus");
}

#[test]
fn ordinary_text_is_untouched() {
    assert_eq!(fix_extracted_text("Camino elitee-plus 3.0"), "Camino elitee-plus 3.0");
}

/// Real structure, not just visible ink, survives — a paragraph's own
/// line breaks and a run's own tab are not the kind of "control
/// character" this exists to remove.
#[test]
fn newlines_and_tabs_survive() {
    assert_eq!(fix_extracted_text("one\ntwo\tthree"), "one\ntwo\tthree");
}
