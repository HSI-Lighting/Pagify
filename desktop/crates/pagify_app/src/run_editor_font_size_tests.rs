use super::run_editor_font_size;

/// **The bug, pinned down**: an eight-line paragraph's box is roughly
/// eight lines tall. Before this divided by the line count, the
/// fallback size was that whole box's height — a font eight lines tall,
/// filling the screen with type big enough to wrap "lighting" into
/// "light" and "ing" on separate lines, which is exactly what was
/// reported.
#[test]
fn a_paragraphs_fallback_size_is_one_lines_share_of_the_box_not_the_whole_box() {
    let one_line = run_editor_font_size(14.0, 1, 0.0, 1.0, None);
    let eight_lines = run_editor_font_size(14.0 * 8.0, 8, 0.0, 1.0, None);
    assert!(
        (one_line - eight_lines).abs() < 0.5,
        "a paragraph of evenly-spaced lines should fall back to about the \
         same size as a single line of the same height: {one_line} vs {eight_lines}"
    );
}

/// A real, sane font size for the run always wins over the fallback —
/// the box is only ever a guess for when there is nothing better.
#[test]
fn a_real_requested_size_is_not_overridden_by_the_fallback() {
    // A realistic single-line box for 12pt text — not the exaggerated
    // box that would make even a real size lose to the fallback, which
    // is exactly the bug this whole function exists to avoid the other
    // way around.
    let size = run_editor_font_size(14.0, 1, 12.0, 1.0, None);
    assert!(
        (size - 12.0).abs() < 0.5,
        "a real 12pt run should render at about 12pt, not fall back to the box: {size}"
    );
}

/// Zero lines cannot mean dividing by zero.
#[test]
fn it_never_divides_by_a_zero_line_count() {
    let size = run_editor_font_size(20.0, 0, 0.0, 1.0, None);
    assert!(size.is_finite() && size > 0.0, "got {size}");
}

/// **`em_ratio` sizes the fallback from the face's own metrics, not the
/// fixed `0.92` guess.** A face with unusually deep descenders — an
/// `(ascent - descent)` of 1.4 em, well past the roughly-1-em most
/// fonts use — needs a *smaller* point size to fill the same box height
/// than the flat guess would give it; this is the whole reason the
/// ratio is asked for instead of assumed.
#[test]
fn a_known_em_ratio_replaces_the_flat_guess() {
    let flat_guess = run_editor_font_size(14.0, 1, 0.0, 1.0, None);
    let from_metrics = run_editor_font_size(14.0, 1, 0.0, 1.0, Some(1.4));
    assert!(
        from_metrics < flat_guess,
        "a face with deep descenders should size smaller than the flat guess: \
         {from_metrics} vs {flat_guess}"
    );
    assert!(
        (from_metrics - 14.0 / 1.4).abs() < 0.01,
        "expected exactly box_height / em_ratio: got {from_metrics}"
    );
}

/// A nonsense ratio (a face with no real ascent/descent spread) falls
/// back to the flat guess rather than dividing by something tiny and
/// producing an absurd size.
#[test]
fn a_degenerate_em_ratio_is_not_trusted() {
    let flat_guess = run_editor_font_size(14.0, 1, 0.0, 1.0, None);
    let degenerate = run_editor_font_size(14.0, 1, 0.0, 1.0, Some(0.0));
    assert_eq!(degenerate, flat_guess, "a zero ratio should fall back to the flat guess");
}

/// **The bug, pinned down: an ordinary "zoomed out to see the page"
/// level, not an extreme one, used to prop the fallback size back up
/// instead of letting it shrink with the rest of the page.** `box_height`
/// here stands in for `draw_run_editor`'s own `base_screen_height` — a
/// fixed page-space line height (6pt) carried through two different
/// zoom levels — and an untrustworthy nominal size (`0.0`) forces the
/// fallback branch at both, isolating it from `nominal`'s own, already
/// correct scaling. Dividing each result back out by its own zoom has to
/// land on the same page-space number; a hard floor anywhere in the
/// chain breaks that at whichever end of the range reaches it first.
#[test]
fn the_fallback_stays_proportional_across_a_wide_zoom_range() {
    let page_space_height = 6.0_f32;
    let low_zoom = 0.3_f32;
    let high_zoom = 2.0_f32;
    let size_at_low = run_editor_font_size(page_space_height * low_zoom, 1, 0.0, low_zoom, None);
    let size_at_high = run_editor_font_size(page_space_height * high_zoom, 1, 0.0, high_zoom, None);
    assert!(
        ((size_at_low / low_zoom) - (size_at_high / high_zoom)).abs() < 0.01,
        "the fallback size is not proportional to zoom: {size_at_low} at {low_zoom}x vs \
         {size_at_high} at {high_zoom}x"
    );
}
