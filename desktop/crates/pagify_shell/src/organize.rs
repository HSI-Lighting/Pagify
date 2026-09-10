//! Organize and Review — build plan phase 10.
//!
//! "Most of this is engine functionality that already exists and needs a
//! desktop surface, not new engine work." That is exactly right, and it is why
//! this module is thin: every page operation is a `pdf_core::command::Command`
//! put through `engine::execute`, which means each one is undoable, redoable
//! and — because those commands were built serialisable — recordable by phase
//! 11 for free.
//!
//! What is *not* thin is the page-range parser. It is the one place a user
//! types numbers that then delete things, one-based on the way in and
//! zero-based on the way through, and an off-by-one here removes the wrong
//! page.

use std::collections::BTreeSet;

/// Parse a page range as a person writes it: `1-3,5,9-11`.
///
/// **One-based on the way in, zero-based on the way out**, because that is the
/// boundary and a boundary is the only safe place for the conversion to happen.
/// Rejects rather than clamps: `delete 1-500` on a five-page document is far
/// more likely to be a mistake than a request to delete everything, and
/// silently reinterpreting it is how someone loses four pages they meant to
/// keep.
pub fn parse_range(spec: &str, page_count: usize) -> Result<Vec<usize>, String> {
    if page_count == 0 {
        return Err("there are no pages".into());
    }

    let spec = spec.trim();
    if spec.is_empty() {
        return Err("which pages? try `1-3,5`, or `all`".into());
    }
    if spec.eq_ignore_ascii_case("all") {
        return Ok((0..page_count).collect());
    }

    // A set, so `1-3,2` is not page 2 twice — a duplicate index would delete
    // one page and then whatever slid into its place.
    let mut pages = BTreeSet::new();

    for part in spec.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }

        let (first, last) = match part.split_once('-') {
            Some((a, b)) => (one_based(a, page_count)?, one_based(b, page_count)?),
            None => {
                let single = one_based(part, page_count)?;
                (single, single)
            }
        };

        // Accept a descending range rather than rejecting it: `5-1` is
        // unambiguous about which pages it means.
        let (from, to) = if first <= last { (first, last) } else { (last, first) };
        for page in from..=to {
            pages.insert(page);
        }
    }

    if pages.is_empty() {
        return Err(format!("'{spec}' does not name any page"));
    }
    Ok(pages.into_iter().collect())
}

fn one_based(token: &str, page_count: usize) -> Result<usize, String> {
    let token = token.trim();
    let number: usize = token
        .parse()
        .map_err(|_| format!("'{token}' is not a page number"))?;

    if number == 0 {
        return Err("pages are numbered from 1".into());
    }
    if number > page_count {
        return Err(format!(
            "page {number} does not exist — there {} {page_count} page{}",
            if page_count == 1 { "is" } else { "are" },
            if page_count == 1 { "" } else { "s" }
        ));
    }
    Ok(number - 1)
}

/// The order array `ReorderPages` wants, from a thumbnail drag.
///
/// `before` is **the page, in the current numbering, that the moved pages land
/// in front of** — the drop target of a drag, not the final index of what was
/// dragged. `before == page_count` means "put them at the end".
///
/// The distinction is not pedantry and it is why this parameter is not called
/// `to`: moving page 0 to sit before page 2 leaves it at index *1*, because
/// removing it from the front shifted everything down. Both readings are
/// plausible from a name like `to`, they differ by one, and the difference is a
/// page in the wrong place. The engine's own contract is a third thing again —
/// `order[i]` is where the page currently at `i` ends up — so the conversion
/// happens once, here, and is tested rather than re-derived at each call site.
/// The order that turns a document back to front.
///
/// `order[i]` is where the page currently at `i` ends up, which is the same
/// convention `ReorderPages` uses — and the inverse of the one people reach for
/// first ("page i comes from j"). For a reversal the two happen to agree, which
/// is exactly why it is worth writing down: the next operation built on this
/// will not be so forgiving.
pub fn order_for_reverse(count: usize) -> Vec<usize> {
    (0..count).map(|i| count - 1 - i).collect()
}

/// The order that exchanges two pages, leaving every other page alone.
///
/// One-based in, zero-based out, like every other page argument the box takes.
pub fn order_for_swap(count: usize, a: usize, b: usize) -> Result<Vec<usize>, String> {
    let out_of_range = |n: usize| format!("there is no page {n} — the document has {count}.");
    if a == 0 || a > count {
        return Err(out_of_range(a));
    }
    if b == 0 || b > count {
        return Err(out_of_range(b));
    }
    if a == b {
        return Err("those are the same page.".into());
    }

    let mut order: Vec<usize> = (0..count).collect();
    order.swap(a - 1, b - 1);
    Ok(order)
}

pub fn order_for_move(
    page_count: usize,
    moving: &[usize],
    before: usize,
) -> Result<Vec<usize>, String> {
    let to = before;
    if moving.is_empty() {
        return Err("no pages to move".into());
    }
    if moving.iter().any(|p| *p >= page_count) {
        return Err("that page does not exist".into());
    }
    if to > page_count {
        return Err("cannot move pages past the end of the document".into());
    }

    let moved: BTreeSet<usize> = moving.iter().copied().collect();

    // The new sequence of *current* indices, in the order they will appear.
    let mut sequence: Vec<usize> = Vec::with_capacity(page_count);
    for page in 0..page_count {
        if page == to {
            sequence.extend(moved.iter().copied());
        }
        if !moved.contains(&page) {
            sequence.push(page);
        }
    }
    if to >= page_count {
        sequence.extend(moved.iter().copied());
    }

    // Invert: order[current] = new position.
    let mut order = vec![0usize; page_count];
    for (new_position, current) in sequence.iter().enumerate() {
        order[*current] = new_position;
    }
    Ok(order)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_range_is_one_based_going_in_and_zero_based_coming_out() {
        assert_eq!(parse_range("1", 5).unwrap(), vec![0]);
        assert_eq!(parse_range("1-3", 5).unwrap(), vec![0, 1, 2]);
        assert_eq!(parse_range("1-2,4", 5).unwrap(), vec![0, 1, 3]);
        assert_eq!(parse_range("all", 3).unwrap(), vec![0, 1, 2]);
    }

    #[test]
    fn overlapping_parts_do_not_name_a_page_twice() {
        // A duplicate index would delete one page and then whatever slid into
        // its place.
        assert_eq!(parse_range("1-3,2,2-3", 5).unwrap(), vec![0, 1, 2]);
    }

    #[test]
    fn a_descending_range_is_read_rather_than_refused() {
        assert_eq!(parse_range("3-1", 5).unwrap(), vec![0, 1, 2]);
    }

    #[test]
    fn a_range_past_the_end_is_refused_rather_than_clamped() {
        // Clamping `1-500` on a five-page document to "all of it" is how
        // someone loses four pages they meant to keep.
        let problem = parse_range("1-500", 5).unwrap_err();
        assert!(problem.contains("does not exist"), "unhelpful: {problem}");

        assert!(parse_range("0", 5).is_err(), "pages are numbered from 1");
        assert!(parse_range("banana", 5).is_err());
        assert!(parse_range("", 5).is_err());
        assert!(parse_range("1", 0).is_err());
    }

    #[test]
    fn moving_a_page_forward_produces_the_order_the_engine_wants() {
        // Five pages; drag page 0 so it lands in front of page 2.
        // The document then reads: 1, 0, 2, 3, 4.
        let order = order_for_move(5, &[0], 2).unwrap();

        // order[current] = new position. Page 0 ends at index 1, not 2 —
        // taking it out of the front shifted everything down by one. That is
        // the whole reason the parameter is called `before`.
        assert_eq!(order[0], 1);
        assert_eq!(order[1], 0);
        assert_eq!(order[2], 2);
        assert_eq!(order[3], 3);
        assert_eq!(order[4], 4);
    }

    #[test]
    fn dropping_in_front_of_a_later_page_reads_the_way_a_drag_looks() {
        // Drag page 3 up so it sits in front of page 1: 0, 3, 1, 2, 4.
        let order = order_for_move(5, &[3], 1).unwrap();
        let mut sequence = vec![0usize; 5];
        for (current, new) in order.iter().enumerate() {
            sequence[*new] = current;
        }
        assert_eq!(sequence, vec![0, 3, 1, 2, 4]);
    }

    #[test]
    fn dropping_at_the_end_puts_the_pages_last() {
        let order = order_for_move(4, &[0], 4).unwrap();
        let mut sequence = vec![0usize; 4];
        for (current, new) in order.iter().enumerate() {
            sequence[*new] = current;
        }
        assert_eq!(sequence, vec![1, 2, 3, 0]);
    }

    #[test]
    fn an_order_is_always_a_permutation() {
        // The property that matters: every page must end up somewhere, and no
        // two in the same place. A non-permutation is a corrupt page tree.
        for before in 0..=6 {
            for moving in [vec![0], vec![5], vec![1, 2], vec![0, 5], vec![2, 3, 4]] {
                let order = order_for_move(6, &moving, before).unwrap();
                let mut seen: Vec<usize> = order.clone();
                seen.sort_unstable();
                assert_eq!(
                    seen,
                    (0..6).collect::<Vec<_>>(),
                    "moving {moving:?} before {before} produced {order:?}, which is not a permutation"
                );
            }
        }
    }

    #[test]
    fn moving_to_the_end_is_allowed_and_moving_past_it_is_not() {
        assert!(order_for_move(3, &[0], 3).is_ok(), "to the end is a real place");
        assert!(order_for_move(3, &[0], 4).is_err());
        assert!(order_for_move(3, &[9], 0).is_err());
        assert!(order_for_move(3, &[], 0).is_err());
    }
}

#[cfg(test)]
mod reorder_tests {
    use super::*;

    #[test]
    fn reversing_puts_the_last_page_first() {
        assert_eq!(order_for_reverse(4), vec![3, 2, 1, 0]);
    }

    #[test]
    fn reversing_nothing_is_nothing() {
        assert!(order_for_reverse(0).is_empty());
        assert_eq!(order_for_reverse(1), vec![0]);
    }

    /// Every page must still be there afterwards. A reorder that loses or
    /// repeats an index is a document that loses or repeats a page, and the
    /// engine has no reason to notice.
    #[test]
    fn a_reversal_is_a_permutation() {
        let order = order_for_reverse(9);
        let mut seen = order.clone();
        seen.sort_unstable();
        assert_eq!(seen, (0..9).collect::<Vec<_>>());
    }

    #[test]
    fn swapping_exchanges_two_and_leaves_the_rest() {
        assert_eq!(order_for_swap(5, 2, 4).unwrap(), vec![0, 3, 2, 1, 4]);
    }

    #[test]
    fn a_swap_is_a_permutation() {
        let order = order_for_swap(6, 1, 6).unwrap();
        let mut seen = order.clone();
        seen.sort_unstable();
        assert_eq!(seen, (0..6).collect::<Vec<_>>());
    }

    #[test]
    fn swapping_a_page_that_is_not_there_is_refused() {
        assert!(order_for_swap(3, 1, 9).is_err());
        assert!(order_for_swap(3, 0, 2).is_err());
    }

    /// Refused rather than quietly doing nothing: asking to swap a page with
    /// itself is a typo, and a silent success hides it.
    #[test]
    fn swapping_a_page_with_itself_is_refused() {
        assert!(order_for_swap(3, 2, 2).is_err());
    }
}
