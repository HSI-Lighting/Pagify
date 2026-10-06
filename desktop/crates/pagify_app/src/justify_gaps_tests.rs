use super::justify_gaps;

/// Three words, a natural width of 60 and a target of 90: the 30-point
/// shortfall splits across the two gaps, 15 each, and the first word
/// gets none — nothing comes before it to stretch.
#[test]
fn the_shortfall_splits_evenly_across_every_gap() {
    let gaps = justify_gaps(&[20.0, 20.0, 20.0], 90.0);
    assert_eq!(gaps, vec![0.0, 15.0, 15.0]);
}

/// One word: nothing to stretch, so nothing is added, however wide the
/// target is.
#[test]
fn a_single_word_is_left_alone() {
    assert_eq!(justify_gaps(&[40.0], 200.0), vec![0.0]);
}

/// Zero words: same answer, and no division by zero.
#[test]
fn no_words_divides_by_nothing() {
    assert_eq!(justify_gaps(&[], 200.0), Vec::<f32>::new());
}

/// A line that already reaches (or overflows) the target only ever
/// gains space, never loses it by compressing a word into the gap.
#[test]
fn a_line_already_at_or_past_the_target_is_not_compressed() {
    let gaps = justify_gaps(&[60.0, 60.0], 90.0);
    assert_eq!(gaps, vec![0.0, 0.0], "a line already past its target should not shrink");
}
