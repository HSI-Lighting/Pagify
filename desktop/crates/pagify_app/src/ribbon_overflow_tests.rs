use super::ribbon_overflow_at;

/// Plenty of room: nothing overflows.
#[test]
fn everything_fits_when_there_is_room() {
    assert_eq!(ribbon_overflow_at(1000.0, &[66.0, 66.0, 66.0], 30.0), 3);
}

/// Exactly enough for two buttons plus the dropdown's own reserve, and
/// not a pixel more — the third does not fit.
#[test]
fn the_first_button_that_does_not_fit_starts_the_overflow() {
    // Two buttons (132) + the reserve (30) = 162, leaving nothing for a
    // third 66-wide button.
    assert_eq!(ribbon_overflow_at(162.0, &[66.0, 66.0, 66.0], 30.0), 2);
}

/// A divider folded into one slot's own width (as the ribbon's own call
/// site does) is just more width to fit — no special casing needed here.
#[test]
fn a_wider_slot_for_a_divider_is_still_just_a_width() {
    // Room for the first (wider, 78) slot plus the reserve (78+30=108),
    // but not for the second (66) on top of that too.
    assert_eq!(ribbon_overflow_at(120.0, &[78.0, 66.0], 30.0), 1);
}

/// No room at all: even the first button overflows.
#[test]
fn too_narrow_for_even_one_button_overflows_everything() {
    assert_eq!(ribbon_overflow_at(20.0, &[66.0, 66.0], 30.0), 0);
}

/// No buttons: nothing to overflow, regardless of width.
#[test]
fn no_buttons_is_never_an_overflow() {
    assert_eq!(ribbon_overflow_at(0.0, &[], 30.0), 0);
}
