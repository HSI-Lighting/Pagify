use super::majority_look;

/// A fragment: its object, its look — `(face name, size bits)`, as the
/// call site builds it — and how much text it holds.
fn frag(object: usize, face: &str, size: f32, weight: usize) -> (usize, (Option<String>, u32), usize) {
    (object, (Some(face.to_string()), size.to_bits()), weight)
}

// **Changed from the first versions of these four tests**: each entry
// grew a third field, the weight, because `majority_look` no longer
// counts one vote per fragment. The weights below are all 1, so the
// expectations are the originals unchanged — what they pinned (a minority
// heading loses, a uniform paragraph keeps its first line, a tie goes to
// the first look, nothing gives nothing) still holds.

#[test]
fn a_minority_heading_does_not_outvote_the_body_beneath_it() {
    let lines = [
        frag(0, "Bold", 14.0, 1),
        frag(1, "Regular", 10.0, 1),
        frag(2, "Regular", 10.0, 1),
        frag(3, "Regular", 10.0, 1),
    ];
    assert_eq!(majority_look(&lines), Some(1), "the body's look should win, not the heading's");
}

#[test]
fn a_uniform_paragraph_keeps_its_first_line() {
    let lines = [frag(5, "Regular", 10.0, 1), frag(6, "Regular", 10.0, 1)];
    assert_eq!(majority_look(&lines), Some(5));
}

#[test]
fn a_tie_resolves_to_whichever_look_appeared_first() {
    let lines = [frag(0, "A", 10.0, 1), frag(1, "B", 12.0, 1)];
    assert_eq!(majority_look(&lines), Some(0));
}

#[test]
fn an_empty_paragraph_has_no_look_to_take() {
    assert_eq!(majority_look::<(Option<String>, u32)>(&[]), None);
}

/// **A three-letter minority in another weight does not win.** "HSI " —
/// one scrap of Medium inside a paragraph of Light — is a fragment like any
/// other; it must not take the paragraph's look, wherever it sits.
#[test]
fn a_three_letter_minority_in_another_style_does_not_win() {
    let in_the_middle = [frag(0, "Light", 8.0, 40), frag(1, "Medium", 8.0, 3), frag(2, "Light", 8.0, 35)];
    assert_eq!(majority_look(&in_the_middle), Some(0));
    let first = [frag(0, "Medium", 8.0, 3), frag(1, "Light", 8.0, 40)];
    assert_eq!(majority_look(&first), Some(1), "first in the paragraph is not entitled to win");
}

/// **A heading cut into five short fragments (18 characters in all) loses
/// to six body fragments (90), even though it comes first.** The body
/// wins on count here too; this is the shape the report described.
#[test]
fn a_heading_in_five_scraps_loses_to_the_body_even_coming_first() {
    let mut lines = Vec::new();
    for (i, chars) in [4, 3, 4, 3, 4].into_iter().enumerate() {
        lines.push(frag(i, "ExtraBold", 12.0, chars));
    }
    for i in 0..6 {
        lines.push(frag(10 + i, "Light", 8.0, 15));
    }
    assert_eq!(majority_look(&lines), Some(10), "the first body fragment, not the first heading scrap");
}

/// **Weight, not fragment count, decides.** A heading chopped into seven
/// scraps (18 characters) against a body of just three long ones (90):
/// counting fragments gives the heading seven votes to three. This is the
/// case a per-fragment tally gets wrong.
#[test]
fn more_fragments_do_not_beat_more_text() {
    let mut lines = Vec::new();
    for i in 0..7 {
        lines.push(frag(i, "ExtraBold", 12.0, if i < 4 { 3 } else { 2 }));
    }
    for i in 0..3 {
        lines.push(frag(10 + i, "Light", 8.0, 30));
    }
    assert_eq!(majority_look(&lines), Some(10));
}

/// Equal total weight is still a tie, and a tie goes to the look that
/// appeared first — however its weight is spread over fragments.
#[test]
fn equal_weight_in_different_pieces_is_a_tie_and_the_first_look_wins() {
    let lines = [frag(0, "A", 8.0, 10), frag(1, "B", 8.0, 5), frag(2, "B", 8.0, 5)];
    assert_eq!(majority_look(&lines), Some(0));
}

/// Fragments with nothing in them (a lone space) cast no weight, so they
/// cannot swing the look — and a paragraph that is *all* of them still
/// answers, with the first.
#[test]
fn empty_fragments_carry_no_weight() {
    let lines = [frag(0, "Ghost", 8.0, 0), frag(1, "Real", 8.0, 1)];
    assert_eq!(majority_look(&lines), Some(1));
    let all_empty = [frag(4, "A", 8.0, 0), frag(5, "B", 8.0, 0)];
    assert_eq!(majority_look(&all_empty), Some(4));
}

/// The look is whatever the caller says it is: a font identity works as
/// well as a name and a size, which is what the next caller will pass.
#[test]
fn the_look_can_be_any_comparable_key() {
    let lines = [(0usize, 7u8, 3usize), (1, 2, 40), (2, 2, 5)];
    assert_eq!(majority_look(&lines), Some(1));
}
