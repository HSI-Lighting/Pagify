use super::join_paragraph_lines;

/// **The bug, pinned down against the real file**: the run before the
/// wrap is `"...light"`, the run after is `"ing solution..."` — nothing
/// at all between them in the extracted text, not even a broken
/// character. The producer drew its wrap-hyphen as its own small mark,
/// never as a glyph this can repair; a hyphen has to be *added*, not
/// found.
///
/// **Changed from the first version of this test**, which passed no flags
/// at all because the function guessed from the shape of the break. Now
/// the caller says the page drew a mark there (`hyphen_after`), and the
/// expectation is the same string.
#[test]
fn a_word_cut_mid_line_gets_its_hyphen_back_where_the_page_draws_one() {
    let lines = ["...heat dis".to_string(), "sipation and high CRI".to_string()];
    assert_eq!(join_paragraph_lines(&lines, &[true]), "...heat dis-\nsipation and high CRI");
}

/// **The same two lines with no mark on the page get no hyphen — the
/// regression this change exists for.** Letters touching on both sides of
/// a break is what every wrap of a producer that never writes a trailing
/// space looks like; it proves nothing about a hyphen. This was
/// `...dis-\nsipation` before.
#[test]
fn the_same_lines_with_no_mark_get_no_hyphen() {
    let lines = ["...heat dis".to_string(), "sipation and high CRI".to_string()];
    assert_eq!(join_paragraph_lines(&lines, &[false]), "...heat dis\nsipation and high CRI");
    assert_eq!(
        join_paragraph_lines(&lines, &[]),
        "...heat dis\nsipation and high CRI",
        "no flags at all reads as no marks"
    );
}

/// An ordinary wrap breaks *at* a space — the line above still ends in
/// one — so nothing is inserted; two real, separate lines must not grow
/// a hyphen neither of them had. Not even if a mark is claimed there: a
/// drawn hyphen splits a word, and a line ending in a space splits none.
#[test]
fn an_ordinary_wrap_gets_no_hyphen() {
    let lines = ["prioritizes function, but ".to_string(), "also values design.".to_string()];
    assert_eq!(join_paragraph_lines(&lines, &[false]), "prioritizes function, but \nalso values design.");
    assert_eq!(join_paragraph_lines(&lines, &[true]), "prioritizes function, but \nalso values design.");
}

/// Punctuation ending a line — a sentence's own full stop, not a cut
/// word — must not be read as a wrap either, whatever the flag says.
#[test]
fn a_line_ending_in_punctuation_gets_no_hyphen() {
    let lines = ["a full sentence.".to_string(), "The next one.".to_string()];
    assert_eq!(join_paragraph_lines(&lines, &[false]), "a full sentence.\nThe next one.");
    assert_eq!(join_paragraph_lines(&lines, &[true]), "a full sentence.\nThe next one.");
}

#[test]
fn a_single_line_is_returned_as_is() {
    assert_eq!(join_paragraph_lines(&["only one line".to_string()], &[]), "only one line");
    assert_eq!(join_paragraph_lines(&["only one line".to_string()], &[true]), "only one line");
}

/// The flag belongs to one wrap: with three lines, only the join it names
/// grows a hyphen.
#[test]
fn each_flag_applies_to_its_own_wrap_only() {
    let lines = ["alpha gam".to_string(), "ma delta epsi".to_string(), "lon zeta".to_string()];
    assert_eq!(
        join_paragraph_lines(&lines, &[true, false]),
        "alpha gam-\nma delta epsi\nlon zeta"
    );
    assert_eq!(
        join_paragraph_lines(&lines, &[false, true]),
        "alpha gam\nma delta epsi-\nlon zeta"
    );
    assert_eq!(
        join_paragraph_lines(&lines, &[true, true]),
        "alpha gam-\nma delta epsi-\nlon zeta"
    );
}

/// A hyphen the text already carries — a real one, or the U+0002 PDFium
/// reads one back as, which `fix_extracted_text` turns into "-" afterwards
/// — is never doubled by a mark drawn beside it.
#[test]
fn a_hyphen_the_text_already_carries_is_not_doubled() {
    let real = ["a well-".to_string(), "known word".to_string()];
    assert_eq!(join_paragraph_lines(&real, &[true]), "a well-\nknown word");
    let read_back = ["a well\u{2}".to_string(), "known word".to_string()];
    assert_eq!(join_paragraph_lines(&read_back, &[true]), "a well\u{2}\nknown word");
}

/// A word split across a digit boundary is still a split word: a drawn
/// mark between "9001" and "2015" is a hyphen, and the old letters-only
/// guard would have dropped it.
#[test]
fn a_drawn_mark_between_digits_is_a_hyphen_too() {
    let lines = ["ISO 9001".to_string(), "2015 certified".to_string()];
    assert_eq!(join_paragraph_lines(&lines, &[true]), "ISO 9001-\n2015 certified");
}
