use super::quantize_to_nearest_8;

/// The one value this fix is actually about: a plain page background
/// must sample back as true white, not a visibly darker near-white.
#[test]
fn true_white_stays_white() {
    assert_eq!(quantize_to_nearest_8(255), 255);
}

#[test]
fn black_stays_black() {
    assert_eq!(quantize_to_nearest_8(0), 0);
}

#[test]
fn a_value_already_on_a_multiple_of_8_is_unchanged() {
    assert_eq!(quantize_to_nearest_8(128), 128);
}

#[test]
fn nearby_shades_round_to_the_same_bucket() {
    // 250 and 251 are both on the low side of the 248/256 boundary and
    // should collapse to the same quantised value.
    assert_eq!(quantize_to_nearest_8(250), quantize_to_nearest_8(251));
}
