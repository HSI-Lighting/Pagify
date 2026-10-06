use super::run_editor_box_grow;

#[test]
fn an_unchanged_size_grows_by_exactly_one() {
    assert_eq!(run_editor_box_grow(20.0, 20.0), 1.0);
}

/// **The reported gap, in numbers: doubling the size must double the
/// box**, not leave it exactly where it started.
#[test]
fn doubling_the_size_doubles_the_box() {
    assert!((run_editor_box_grow(20.0, 40.0) - 2.0).abs() < 1e-6);
}

#[test]
fn halving_the_size_halves_the_box() {
    assert!((run_editor_box_grow(20.0, 10.0) - 0.5).abs() < 1e-6);
}

/// An absurd request shrinks or grows the box, never collapses or
/// explodes it off screen.
#[test]
fn extreme_ratios_stay_clamped() {
    assert_eq!(run_editor_box_grow(20.0, 100_000.0), 20.0);
    assert_eq!(run_editor_box_grow(20.0, 0.0), 0.1);
}

#[test]
fn a_vanishing_base_does_not_divide_by_zero() {
    assert!(run_editor_box_grow(0.0, 20.0).is_finite());
}
