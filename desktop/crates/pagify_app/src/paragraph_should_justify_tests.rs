use super::paragraph_should_justify;
use pdf_core::document::Rect;

fn line(objects: &[usize], left: f32, right: f32) -> (Vec<usize>, Rect) {
    (objects.to_vec(), Rect { left, right, top: 0.0, bottom: 10.0 })
}

/// A single line has nothing to stretch to begin with.
#[test]
fn one_line_is_never_justified() {
    assert!(!paragraph_should_justify(&[line(&[0], 10.0, 80.0)]));
}

/// Two lines sharing the same left margin — an ordinary wrapped
/// paragraph — are the case this exists to keep justifying.
#[test]
fn two_lines_at_the_same_margin_are_justified() {
    let lines = [line(&[0], 10.0, 90.0), line(&[1], 10.0, 60.0)];
    assert!(paragraph_should_justify(&lines));
}

/// **The bug, pinned down**: a label's value line sits indented well to
/// the right of the label above it — "Current Input: ..." over an
/// indented "1050mA" — not flush against the same margin, so this must
/// not be treated as a real wrapped paragraph's second line.
#[test]
fn an_indented_second_line_is_not_justified() {
    let lines = [line(&[0], 10.0, 200.0), line(&[1], 40.0, 90.0)];
    assert!(!paragraph_should_justify(&lines));
}

/// A sub-point difference is measurement noise between runs extracted
/// from the same left-aligned block, not a real indent.
#[test]
fn a_tiny_margin_difference_is_tolerated() {
    let lines = [line(&[0], 10.0, 200.0), line(&[1], 10.6, 90.0)];
    assert!(paragraph_should_justify(&lines));
}

/// Three lines: the first two share a margin but the third is indented —
/// one mismatch anywhere in the paragraph is enough to call the whole
/// thing not a real wrap, not just the one mismatched pair.
#[test]
fn one_indented_line_among_several_still_blocks_justification() {
    let lines = [line(&[0], 10.0, 200.0), line(&[1], 10.0, 180.0), line(&[2], 40.0, 90.0)];
    assert!(!paragraph_should_justify(&lines));
}
