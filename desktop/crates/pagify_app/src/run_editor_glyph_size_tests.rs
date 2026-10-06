use super::run_editor_glyph_size;

/// **The bug, pinned down: zooming past a fixed pixel range used to stop
/// the editor scaling with the rest of the page.** Neither end of that
/// range exists any more — an extreme zoom, in either direction, must
/// still come straight out the other end proportionally.
#[test]
fn there_is_no_ceiling_at_a_heavy_zoom_in() {
    assert_eq!(run_editor_glyph_size(500.0), 500.0);
}

#[test]
fn there_is_no_floor_at_a_heavy_zoom_out() {
    let tiny = run_editor_glyph_size(2.0);
    assert!(
        tiny < 6.0,
        "a small on-screen size should stay small, not get propped back up to a fixed minimum: {tiny}"
    );
    assert_eq!(tiny, 2.0);
}

#[test]
fn only_a_literal_zero_is_floored() {
    assert_eq!(run_editor_glyph_size(0.0), 0.5);
}
