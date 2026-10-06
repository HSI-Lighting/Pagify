use super::{fix_extracted_text, hyphens_before_drawn_lines, join_paragraph_lines};

/// The buffer for lines the way the editor builds it: harden, join, fix.
fn buffer(lines: &[&str], drawn: &[bool]) -> String {
    let mut texts: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    hyphens_before_drawn_lines(&mut texts, drawn);
    fix_extracted_text(&join_paragraph_lines(&texts, &[]))
}

/// **The reported line**: "driv" + the page's hyphen code, then a line the
/// page draws (the placeholder). The hyphen stays.
#[test]
fn a_hyphen_code_before_a_placeholder_line_is_a_hyphen() {
    let lines = ["including COB, driv\u{2}", "[drawn text]", "work in harmony"];
    assert_eq!(
        buffer(&lines, &[false, true, false]),
        "including COB, driv-\n[drawn text]\nwork in harmony"
    );
    // And what it was: without knowing the next line is drawn, the code is
    // dropped, which is what took the hyphen off the page when the line was retyped.
    assert_eq!(
        buffer(&lines, &[false, false, false]),
        "including COB, driv\n[drawn text]\nwork in harmony"
    );
}

/// A line with a drawn word at its start has text, so a letter may follow
/// — or not: it is drawn, so the next character is not the evidence.
#[test]
fn a_hyphen_code_before_a_line_with_a_drawn_word_is_a_hyphen_too() {
    assert_eq!(buffer(&["that are var\u{2}", "  for different"], &[false, true]), "that are var-\n  for different");
    assert_eq!(buffer(&["that are var\u{2}", ". for different"], &[false, true]), "that are var-\n. for different");
}

/// Before a line that is written the old rule stands: a letter on the far
/// side makes it a hyphen, anything else drops it.
#[test]
fn before_a_written_line_the_old_rule_stands() {
    assert_eq!(buffer(&["a well\u{2}", "known word"], &[false, false]), "a well-\nknown word");
    assert_eq!(buffer(&["ISO 9001\u{2}", "2015 certified"], &[false, false]), "ISO 9001\n2015 certified");
    assert_eq!(buffer(&["a well\u{2}", "(known) word"], &[false, false]), "a well\n(known) word");
}

/// Only a code that follows a letter is read as a hyphen; after a digit, a
/// space or nothing it is dropped, drawn line or not.
#[test]
fn a_code_that_does_not_follow_a_letter_is_dropped_even_before_a_drawn_line() {
    assert_eq!(buffer(&["page 12\u{2}", "[drawn text]"], &[false, true]), "page 12\n[drawn text]");
    assert_eq!(buffer(&["a \u{2}", "[drawn text]"], &[false, true]), "a \n[drawn text]");
    assert_eq!(buffer(&["\u{2}", "[drawn text]"], &[false, true]), "\n[drawn text]");
}

/// A hyphen already there is not doubled, and nothing is touched when
/// nothing is drawn.
#[test]
fn a_real_hyphen_is_not_doubled_and_plain_text_is_left_alone() {
    assert_eq!(buffer(&["well-", "[drawn text]"], &[false, true]), "well-\n[drawn text]");
    assert_eq!(buffer(&["a", "b", "c"], &[false, false, false]), "a\nb\nc");
    // No flags at all means nothing is drawn: the old rule stands (a digit on
    // the far side, so the code is dropped).
    assert_eq!(buffer(&["a\u{2}", "1"], &[]), "a\n1");
}

/// Several lines, several drawn ones, and the first line being drawn: each
/// code is judged against the line after it; nothing indexes out of range.
#[test]
fn each_code_is_judged_by_the_line_after_it_and_nothing_goes_out_of_range() {
    let lines = ["one\u{2}", "[drawn text]", "two\u{2}", "three", "four\u{2}", "[drawn text]"];
    let drawn = [false, true, false, false, false, true];
    assert_eq!(
        buffer(&lines, &drawn),
        "one-\n[drawn text]\ntwo-\nthree\nfour-\n[drawn text]",
        "two- comes from the letter after it, the other two from the drawn lines"
    );
    assert_eq!(buffer(&["[drawn text]", "x\u{2}"], &[true, false]), "[drawn text]\nx");
    let mut empty: Vec<String> = Vec::new();
    hyphens_before_drawn_lines(&mut empty, &[true]);
    let mut one = vec!["a\u{2}".to_string()];
    hyphens_before_drawn_lines(&mut one, &[true]);
    assert_eq!(one, ["a\u{2}"], "a first line has no line above it to harden");
}

/// Characters that are more than one byte are not cut in half.
#[test]
fn a_multi_byte_letter_before_the_code_is_a_letter() {
    assert_eq!(buffer(&["café\u{2}", "[drawn text]"], &[false, true]), "café-\n[drawn text]");
    assert_eq!(buffer(&["日本\u{2}", "[drawn text]"], &[false, true]), "日本-\n[drawn text]");
}
