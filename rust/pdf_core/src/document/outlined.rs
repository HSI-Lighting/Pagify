//! Type converted to outlines — reading it, not rasterising it.
//!
//! See [`crate::document::glyphs`] for the matcher this feeds. This module is
//! the other half: turning a page's path objects into the same [`Outline`]
//! shape a font's glyphs already come in, and turning a page's worth of
//! recognised outlines back into reading order.
//!
//! Two problems, kept apart because they fail differently.
//!
//! **Reading one path object's contours** is exact. A PDF path is a sequence of
//! move/line/curve operators and this module's only job is not to lose any of
//! it — get the curve flattening right, get the contour boundaries right, and
//! the shape that comes out is the shape that went in.
//!
//! **Deciding which contours belong to which letter** is not exact, and cannot
//! be made exact from geometry alone. A producer that draws a whole line as one
//! compound path — Illustrator's default — hands this module thirty letters'
//! worth of contours with no boundary between them at all. The split used here
//! is the one the plan specifies: **connected components by bounding-box
//! overlap**. It is right for the common case — a ring inside a bowl, as in `o`
//! or `a` or `8`, always overlaps its own outer contour — and it is named wrong
//! in two opposite directions, both measured on the real fixture this module is
//! tested against rather than reasoned about in the abstract.
//!
//! **Under-merging.** A glyph like `i` or `%` whose parts do not touch splits
//! into two clusters instead of staying one. This is the failure that shows up
//! most: every `i` in `outlined.pdf` comes back as its stem alone or nothing,
//! because the dot and the stem share no bounding-box area at all. It fails
//! safely — a dot clustered on its own is a valid, tiny outline that the
//! matcher either identifies as something or, more often, refuses as
//! ambiguous — never a corruption of a neighbouring letter, because clustering
//! never *merges into* a letter it wasn't touching.
//!
//! **Over-merging**, the opposite mistake, measured as genuinely rare rather
//! than assumed absent: of 501 clusters found on that same page, 4 span more
//! than one page object, meaning a handful of tightly kerned neighbours
//! touched closely enough to be read as one shape. Rare enough not to be the
//! dominant source of missed letters — most of what a real page loses is
//! ordinary threshold misses in
//! [`crate::document::glyphs::Outline::distance_to`], the small differences in
//! curve fitting the plan itself expects and accepts — but real, and not a
//! case this function's rule can tell apart from the nested-ring case that
//! makes bounding-box overlap the right call to begin with.

use super::glyphs::{flatten_cubic, flatten_quad, Catalogue, Contour, Outline};
use super::layout::{Glyph, Rect};

// ------------------------------------------------------------ reading a path --

/// One operator from a PDF path's segment list, decoupled from PDFium's own
/// types so the state machine below can be exercised without a live document.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum SegmentKind {
    MoveTo,
    LineTo,
    /// **One point of three.** PDF's `c` operator gives two control points and
    /// an endpoint in a single call; PDFium's segment API does not — it reports
    /// each as its own `BezierTo`, in order, and expects the reader to count.
    /// Measured against a real fixture rather than assumed: three consecutive
    /// `BezierTo` segments came back for every curve emitted, and a `quad_to`
    /// raised to a cubic (the exact shape [`crate::document::glyphs`]'s own
    /// fixture generator emits) came back with its two control points equal,
    /// which is what confirmed the grouping rather than merely suggesting it.
    BezierTo,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PathSegment {
    pub kind: SegmentKind,
    pub x: f32,
    pub y: f32,
    /// PDFium reports this as a property of a segment, not as a segment of its
    /// own — closing the loop.
    pub close: bool,
}

/// Rebuild an [`Outline`] from a path object's raw segment sequence.
///
/// The state this needs is exactly what a pen needs: where it is, and where the
/// current contour started (so `close` can be more than a marker — it decides
/// whether to keep going or start fresh). `BezierTo` segments are buffered
/// three at a time because the source gives them one point per call; a stray
/// run whose length is not a multiple of three is a malformed file, not a
/// contour, and its dangling points are dropped rather than guessed at.
pub(crate) fn build_outline(segments: &[PathSegment]) -> Outline {
    const STEPS: usize = 16;

    let mut contours = Vec::new();
    let mut current = Contour::new();
    let mut pen = (0.0f32, 0.0f32);
    let mut bezier_buffer: Vec<(f32, f32)> = Vec::with_capacity(2);

    let mut finish_contour = |current: &mut Contour, contours: &mut Vec<Contour>| {
        if current.len() >= 3 {
            contours.push(std::mem::take(current));
        } else {
            current.clear();
        }
    };

    for segment in segments {
        match segment.kind {
            SegmentKind::MoveTo => {
                finish_contour(&mut current, &mut contours);
                pen = (segment.x, segment.y);
                current.push(pen);
                bezier_buffer.clear();
            }
            SegmentKind::LineTo => {
                pen = (segment.x, segment.y);
                current.push(pen);
            }
            SegmentKind::BezierTo => {
                bezier_buffer.push((segment.x, segment.y));
                if bezier_buffer.len() == 3 {
                    let (c1, c2, to) = (bezier_buffer[0], bezier_buffer[1], bezier_buffer[2]);
                    // The common case this fixture and most real curve-to-cubic
                    // elevation produce: two equal control points, which is a
                    // quadratic wearing a cubic's clothes. Flattened the same
                    // way either shape reaches this module keeps one formula
                    // rather than two that could disagree at the edges.
                    if c1 == c2 {
                        current.extend(flatten_quad(pen, c1, to, STEPS));
                    } else {
                        current.extend(flatten_cubic(pen, c1, c2, to, STEPS));
                    }
                    pen = to;
                    bezier_buffer.clear();
                }
            }
        }
        if segment.close {
            finish_contour(&mut current, &mut contours);
            bezier_buffer.clear();
        }
    }
    finish_contour(&mut current, &mut contours);

    Outline { contours }
}

// ----------------------------------------------------------------- splitting --

/// One contour, and which page object it came from.
///
/// The object index is carried all the way through clustering and matching so
/// that a caller who wants to *remove* a recognised letter — redaction, not
/// just extraction — knows exactly which objects to take off the page. A
/// cluster spanning contours from several objects (Illustrator's compound-path
/// case) removes all of them; a cluster from one object removes just that one.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SourcedContour {
    pub contour: Contour,
    pub object: usize,
}

fn bounds_of(contour: &Contour) -> Rect {
    let (mut l, mut t, mut r, mut b) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for &(x, y) in contour {
        l = l.min(x);
        r = r.max(x);
        t = t.min(y);
        b = b.max(y);
    }
    Rect { left: l, top: t, right: r, bottom: b }
}

fn overlaps(a: &Rect, b: &Rect) -> bool {
    a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom
}

/// One glyph's worth of contours, gathered from wherever they were drawn.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Cluster {
    pub outline: Outline,
    pub bounds: Rect,
    /// Every page object that contributed a contour, de-duplicated. Empty only
    /// if the cluster came from contours with no object recorded, which does
    /// not happen through the PDFium-facing entry point.
    pub objects: Vec<usize>,
}

/// Group contours into per-glyph clusters by bounding-box overlap.
///
/// **Connected components, not proximity.** Overlap is the test the plan
/// specifies, and it is the test that tells a nested ring from a neighbouring
/// letter: `o`'s inner contour is *inside* its outer one, which is the
/// strongest kind of overlap there is, while two adjacent letters in ordinarily
/// set type do not share area at all. A looser, distance-based rule would need
/// a threshold tuned to a type size this function is never told.
pub(crate) fn cluster(contours: Vec<SourcedContour>) -> Vec<Cluster> {
    let boxed: Vec<(Rect, SourcedContour)> =
        contours.into_iter().map(|sc| (bounds_of(&sc.contour), sc)).collect();
    let n = boxed.len();

    // Union-find over contour indices — the ordinary way to build connected
    // components without an explicit graph.
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(parent: &mut [usize], x: usize) -> usize {
        if parent[x] != x {
            parent[x] = find(parent, parent[x]);
        }
        parent[x]
    }
    for i in 0..n {
        for j in (i + 1)..n {
            if overlaps(&boxed[i].0, &boxed[j].0) {
                let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
                if ri != rj {
                    parent[ri] = rj;
                }
            }
        }
    }

    let mut groups: std::collections::HashMap<usize, Vec<usize>> = std::collections::HashMap::new();
    for i in 0..n {
        groups.entry(find(&mut parent, i)).or_default().push(i);
    }

    let mut clusters: Vec<Cluster> = groups
        .into_values()
        .map(|indices| {
            let mut bounds: Option<Rect> = None;
            let mut objects = Vec::new();
            let mut outline_contours = Vec::with_capacity(indices.len());
            for i in indices {
                let (b, sc) = &boxed[i];
                bounds = Some(match bounds {
                    None => *b,
                    Some(acc) => Rect {
                        left: acc.left.min(b.left),
                        top: acc.top.min(b.top),
                        right: acc.right.max(b.right),
                        bottom: acc.bottom.max(b.bottom),
                    },
                });
                if !objects.contains(&sc.object) {
                    objects.push(sc.object);
                }
                outline_contours.push(sc.contour.clone());
            }
            Cluster {
                outline: Outline { contours: outline_contours },
                bounds: bounds.expect("at least one contour per group"),
                objects,
            }
        })
        .collect();

    // Left to right. Not load-bearing for correctness — `layout::reconstruct`
    // derives order from geometry regardless of input order — but a stable,
    // readable order makes this function's own tests and any debugging output
    // sane to look at.
    clusters.sort_by(|a, b| {
        a.bounds.left.partial_cmp(&b.bounds.left).unwrap_or(std::cmp::Ordering::Equal)
    });
    clusters
}

// -------------------------------------------------------------- identifying --

/// One cluster, matched or not.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Identified {
    pub ch: char,
    pub bounds: Rect,
    pub objects: Vec<usize>,
    pub score: f32,
}

/// Match every cluster against the catalogue, keeping only what cleared the
/// tolerance.
///
/// **Silent on a miss, not an error.** A page runs headlines, body copy and a
/// logo through the same path objects; the logo will not match anything in a
/// text catalogue and that is not a failure of the page, the catalogue, or this
/// function — it is a shape that is not a letter.
pub(crate) fn identify(clusters: &[Cluster], catalogue: &Catalogue, tolerance: f32) -> Vec<Identified> {
    clusters
        .iter()
        .filter_map(|cluster| {
            catalogue.identify(&cluster.outline, tolerance).map(|(ch, score)| Identified {
                ch,
                bounds: cluster.bounds,
                objects: cluster.objects.clone(),
                score,
            })
        })
        .collect()
}

/// Identified clusters, as [`Glyph`]s ready for [`super::layout::reconstruct`].
///
/// Nothing here decides reading order — that is the point. Matching a page's
/// worth of outlines and handing the results, in whatever order they were
/// found, to the same geometry-driven reassembly every other extraction path
/// uses is the entire second half of the plan this module implements.
pub(crate) fn as_glyphs(identified: &[Identified]) -> Vec<Glyph> {
    identified.iter().map(|i| Glyph { ch: i.ch, rect: i.bounds, angle: 0.0 }).collect()
}

/// Identified clusters, grouped into words with a box and a confidence each —
/// the shape [`crate::ocr::RecognisedWord`] wants, which is what OCR already
/// produces for a scanned page. [`crate::command::Command::AddTextLayer`] can
/// take either without knowing which one recognised the words, so a caller
/// wiring this in does not need a second code path for it.
///
/// `tolerance` must be the value [`identify`] was actually called with.
/// Confidence here is not a probability estimate the way a neural recogniser's
/// is — it is how close a match sat inside the tolerance that already decided
/// it counted at all, linearly rescaled to 0–1 so a caller can rank or filter
/// these exactly as it already does for OCR's.
pub(crate) fn as_words(identified: &[Identified], tolerance: f32) -> Vec<crate::ocr::RecognisedWord> {
    let safe_tolerance = tolerance.max(1e-6);
    let confidence_of = |score: f32| (1.0 - score / safe_tolerance).clamp(0.0, 1.0);
    let glyphs = as_glyphs(identified);

    super::layout::words(&glyphs)
        .into_iter()
        .map(|word| {
            // Matched back to the identified entry by value — `(rect, ch)` is
            // not a synthetic key, it is the only thing a `Glyph` carries once
            // grouping has already happened.
            let char_confidence: Vec<f32> = word
                .glyphs
                .iter()
                .map(|g| {
                    identified
                        .iter()
                        .find(|i| i.bounds == g.rect && i.ch == g.ch)
                        .map(|i| confidence_of(i.score))
                        .unwrap_or(0.0)
                })
                .collect();
            // The weakest character, matching how a review pass already reads
            // `RecognisedWord::worst` elsewhere: a word can look fine on
            // average and still contain the one character that turned a part
            // number into a different one.
            let confidence =
                char_confidence.iter().copied().fold(f32::INFINITY, f32::min);
            crate::ocr::RecognisedWord {
                text: word.text,
                rect: word.rect.into(),
                confidence: if confidence.is_finite() { confidence } else { 0.0 },
                char_confidence,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::glyphs::Outline as GOutline;

    fn seg(kind: SegmentKind, x: f32, y: f32) -> PathSegment {
        PathSegment { kind, x, y, close: false }
    }
    fn closing(kind: SegmentKind, x: f32, y: f32) -> PathSegment {
        PathSegment { kind, x, y, close: true }
    }

    // -- build_outline -------------------------------------------------

    #[test]
    fn a_moveto_lineto_close_makes_one_triangle_contour() {
        let outline = build_outline(&[
            seg(SegmentKind::MoveTo, 0.0, 0.0),
            seg(SegmentKind::LineTo, 10.0, 0.0),
            closing(SegmentKind::LineTo, 5.0, 10.0),
        ]);
        assert_eq!(outline.contours.len(), 1);
        assert_eq!(outline.contours[0], vec![(0.0, 0.0), (10.0, 0.0), (5.0, 10.0)]);
    }

    #[test]
    fn two_movetos_make_two_contours_like_the_two_rings_of_an_o() {
        let outline = build_outline(&[
            seg(SegmentKind::MoveTo, 0.0, 0.0),
            seg(SegmentKind::LineTo, 10.0, 0.0),
            closing(SegmentKind::LineTo, 5.0, 10.0),
            seg(SegmentKind::MoveTo, 2.0, 2.0),
            seg(SegmentKind::LineTo, 8.0, 2.0),
            closing(SegmentKind::LineTo, 5.0, 8.0),
        ]);
        assert_eq!(outline.contours.len(), 2);
    }

    /// **The grouping this module exists to get right.** Three consecutive
    /// `BezierTo` segments are one cubic curve — control point, control point,
    /// endpoint — because that is what a real fixture measured as, not what the
    /// PDF spec's `c` operator shape merely suggests.
    #[test]
    fn three_beziertos_in_a_row_are_one_curve_not_three_lines() {
        let outline = build_outline(&[
            seg(SegmentKind::MoveTo, 0.0, 0.0),
            seg(SegmentKind::BezierTo, 0.0, 10.0),
            seg(SegmentKind::BezierTo, 10.0, 10.0),
            closing(SegmentKind::BezierTo, 10.0, 0.0),
        ]);
        assert_eq!(outline.contours.len(), 1);
        // Flattened into steps, not left as three raw points.
        assert!(
            outline.contours[0].len() > 4,
            "a curve collapsed into its raw control points: {:?}",
            outline.contours[0]
        );
        // And it actually curves — the midpoint of a genuine cubic bulges away
        // from the straight line between its ends, unlike three literal line
        // segments through those same three points.
        let mid = outline.contours[0][outline.contours[0].len() / 2];
        assert!(mid.0 > 0.5, "flattened points look like straight lines: {mid:?}");
    }

    /// **What actually turned up in the fixture.** Two equal control points is
    /// a quadratic raised to a cubic, and it is flattened as the quadratic it
    /// really is rather than as a degenerate cubic that happens to agree.
    #[test]
    fn equal_control_points_are_read_as_the_underlying_quadratic() {
        let outline = build_outline(&[
            seg(SegmentKind::MoveTo, 0.0, 0.0),
            seg(SegmentKind::BezierTo, 5.0, 10.0),
            seg(SegmentKind::BezierTo, 5.0, 10.0),
            closing(SegmentKind::BezierTo, 10.0, 0.0),
        ]);
        let quad = flatten_quad((0.0, 0.0), (5.0, 10.0), (10.0, 0.0), 16);
        assert_eq!(&outline.contours[0][1..], quad.as_slice());
    }

    #[test]
    fn a_contour_with_fewer_than_three_points_is_dropped() {
        let outline = build_outline(&[seg(SegmentKind::MoveTo, 0.0, 0.0), closing(SegmentKind::LineTo, 1.0, 1.0)]);
        assert!(outline.contours.is_empty(), "a two-point sliver was kept as a contour");
    }

    #[test]
    fn a_dangling_bezier_run_is_dropped_rather_than_guessed_at() {
        // One or two BezierTo segments with no third: a malformed file, not a
        // curve. The rest of the contour is still salvaged.
        let outline = build_outline(&[
            seg(SegmentKind::MoveTo, 0.0, 0.0),
            seg(SegmentKind::LineTo, 10.0, 0.0),
            seg(SegmentKind::BezierTo, 5.0, 5.0),
            closing(SegmentKind::LineTo, 5.0, 10.0),
        ]);
        assert_eq!(outline.contours[0], vec![(0.0, 0.0), (10.0, 0.0), (5.0, 10.0)]);
    }

    #[test]
    fn an_empty_segment_list_is_an_empty_outline() {
        assert!(build_outline(&[]).contours.is_empty());
    }

    // -- cluster ---------------------------------------------------------

    fn rect_contour(l: f32, t: f32, r: f32, b: f32) -> Contour {
        vec![(l, t), (r, t), (r, b), (l, b)]
    }
    fn sc(object: usize, l: f32, t: f32, r: f32, b: f32) -> SourcedContour {
        SourcedContour { contour: rect_contour(l, t, r, b), object }
    }

    #[test]
    fn one_contour_is_its_own_cluster() {
        let clusters = cluster(vec![sc(0, 0.0, 0.0, 10.0, 10.0)]);
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].objects, vec![0]);
    }

    /// **The case the whole design is for.** An `o`'s inner ring lies wholly
    /// inside its outer one — the strongest overlap there is — so both merge
    /// into a single two-contour glyph.
    #[test]
    fn a_nested_ring_merges_with_its_outer_contour() {
        let clusters = cluster(vec![sc(0, 0.0, 0.0, 10.0, 10.0), sc(0, 3.0, 3.0, 7.0, 7.0)]);
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].outline.contours.len(), 2);
    }

    /// Two ordinary letters, side by side, do not merge.
    #[test]
    fn two_separate_letters_stay_two_clusters() {
        let clusters = cluster(vec![sc(0, 0.0, 0.0, 10.0, 10.0), sc(1, 20.0, 0.0, 30.0, 10.0)]);
        assert_eq!(clusters.len(), 2);
    }

    /// Illustrator's compound-path case: many contours, one page object, none
    /// of them touching. Each stays its own cluster — this function does not
    /// invent adjacency that is not there — and the object id is still carried
    /// correctly through every one of them.
    #[test]
    fn one_object_holding_several_untouching_letters_still_splits() {
        let clusters = cluster(vec![
            sc(0, 0.0, 0.0, 10.0, 10.0),
            sc(0, 20.0, 0.0, 30.0, 10.0),
            sc(0, 40.0, 0.0, 50.0, 10.0),
        ]);
        assert_eq!(clusters.len(), 3);
        assert!(clusters.iter().all(|c| c.objects == vec![0]));
    }

    /// **The stated limitation.** A dot and its stem do not overlap, so they
    /// come back as two clusters rather than one `i`. Named as a gap in the
    /// module docs rather than patched with a distance heuristic this function
    /// was never given a type size to tune.
    #[test]
    fn a_disjoint_dot_and_stem_do_not_merge() {
        let dot = sc(0, 4.0, 0.0, 6.0, 2.0);
        let stem = sc(0, 4.0, 4.0, 6.0, 10.0);
        let clusters = cluster(vec![dot, stem]);
        assert_eq!(clusters.len(), 2, "documenting current behaviour, not asserting it is desired");
    }

    #[test]
    fn a_cluster_from_two_objects_lists_both() {
        let clusters = cluster(vec![sc(3, 0.0, 0.0, 10.0, 10.0), sc(7, 2.0, 2.0, 8.0, 8.0)]);
        assert_eq!(clusters.len(), 1);
        let mut objects = clusters[0].objects.clone();
        objects.sort_unstable();
        assert_eq!(objects, vec![3, 7]);
    }

    #[test]
    fn clustering_nothing_returns_nothing() {
        assert!(cluster(Vec::new()).is_empty());
    }

    #[test]
    fn merging_is_transitive_across_a_chain_of_overlaps() {
        // A overlaps B, B overlaps C, A does not overlap C directly — all three
        // must still land in one cluster, or a wide cursive ligature spanning
        // three touching strokes would be read as two separate letters.
        let clusters = cluster(vec![
            sc(0, 0.0, 0.0, 10.0, 10.0),
            sc(1, 9.0, 0.0, 19.0, 10.0),
            sc(2, 18.0, 0.0, 28.0, 10.0),
        ]);
        assert_eq!(clusters.len(), 1);
        let mut objects = clusters[0].objects.clone();
        objects.sort_unstable();
        assert_eq!(objects, vec![0, 1, 2]);
    }

    // -- identify + as_glyphs --------------------------------------------

    fn square_outline(size: f32) -> GOutline {
        GOutline { contours: vec![vec![(0.0, 0.0), (size, 0.0), (size, size), (0.0, size)]] }
    }

    #[test]
    fn a_cluster_that_matches_the_catalogue_is_identified() {
        let mut catalogue = Catalogue::default();
        catalogue.insert('S', square_outline(10.0));

        let clusters = vec![Cluster {
            outline: square_outline(250.0),
            bounds: Rect { left: 10.0, top: 20.0, right: 60.0, bottom: 70.0 },
            objects: vec![4],
        }];

        let found = identify(&clusters, &catalogue, 0.05);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].ch, 'S');
        assert_eq!(found[0].objects, vec![4]);
    }

    /// A shape that is not a letter — a logo, a rule, a flourish — is simply
    /// absent from the result. Not an error; nothing to say about it.
    #[test]
    fn an_unmatched_cluster_is_silently_dropped() {
        let mut catalogue = Catalogue::default();
        catalogue.insert('S', square_outline(10.0));

        let triangle = GOutline { contours: vec![vec![(0.0, 0.0), (10.0, 0.0), (5.0, 10.0)]] };
        let clusters =
            vec![Cluster { outline: triangle, bounds: Rect { left: 0.0, top: 0.0, right: 1.0, bottom: 1.0 }, objects: vec![0] }];

        assert!(identify(&clusters, &catalogue, 0.01).is_empty());
    }

    #[test]
    fn identified_clusters_become_glyphs_at_their_own_bounds() {
        let identified = vec![Identified {
            ch: 'x',
            bounds: Rect { left: 1.0, top: 2.0, right: 3.0, bottom: 4.0 },
            objects: vec![0],
            score: 0.0,
        }];
        let glyphs = as_glyphs(&identified);
        assert_eq!(glyphs.len(), 1);
        assert_eq!(glyphs[0].ch, 'x');
        assert_eq!(glyphs[0].rect, Rect { left: 1.0, top: 2.0, right: 3.0, bottom: 4.0 });
        assert_eq!(glyphs[0].angle, 0.0);
    }

    // -- end to end, still with no PDF and no real font -------------------

    /// The whole pipeline — segments to outline, outline to cluster, cluster to
    /// letter, letter to reassembled text — proved with synthetic shapes before
    /// any of it touches PDFium or a real font.
    #[test]
    fn the_full_pure_pipeline_recovers_a_word() {
        let mut catalogue = Catalogue::default();
        catalogue.insert('S', square_outline(10.0));
        catalogue.insert('O', GOutline {
            contours: vec![
                rect_contour(0.0, 0.0, 10.0, 10.0),
                rect_contour(3.0, 3.0, 7.0, 7.0),
            ],
        });

        // "OS", built as raw path segments the way PDFium would report them —
        // 'O' as two nested rectangles from one compound object, 'S' from a
        // second, separate object.
        let o_outer = build_outline(&[
            seg(SegmentKind::MoveTo, 0.0, 0.0),
            seg(SegmentKind::LineTo, 100.0, 0.0),
            seg(SegmentKind::LineTo, 100.0, 100.0),
            closing(SegmentKind::LineTo, 0.0, 100.0),
        ]);
        let o_inner = build_outline(&[
            seg(SegmentKind::MoveTo, 30.0, 30.0),
            seg(SegmentKind::LineTo, 70.0, 30.0),
            seg(SegmentKind::LineTo, 70.0, 70.0),
            closing(SegmentKind::LineTo, 30.0, 70.0),
        ]);
        // Set close enough to read as one word: `layout::reconstruct` treats a
        // gap over a quarter of the glyph's height as a word space, so the two
        // letters have to sit nearer than that to prove they come back as
        // "OS" and not "O S" — which is what an earlier, wider gap correctly
        // produced, and which was this test's own mistake, not the pipeline's.
        let s_shape = build_outline(&[
            seg(SegmentKind::MoveTo, 110.0, 0.0),
            seg(SegmentKind::LineTo, 210.0, 0.0),
            seg(SegmentKind::LineTo, 210.0, 100.0),
            closing(SegmentKind::LineTo, 110.0, 100.0),
        ]);

        let contours = vec![
            SourcedContour { contour: o_outer.contours[0].clone(), object: 10 },
            SourcedContour { contour: o_inner.contours[0].clone(), object: 10 },
            SourcedContour { contour: s_shape.contours[0].clone(), object: 11 },
        ];

        let clusters = cluster(contours);
        assert_eq!(clusters.len(), 2, "the O's two rings must merge; the S must stay apart");

        let identified = identify(&clusters, &catalogue, 0.05);
        assert_eq!(identified.len(), 2);

        let glyphs = as_glyphs(&identified);
        let text = super::super::layout::reconstruct(&glyphs).plain();
        assert_eq!(text, "OS", "the letters did not come back in reading order: {text:?}");
    }

    // -- as_words ----------------------------------------------------------

    fn oands_identified(score_o: f32, score_s: f32) -> Vec<Identified> {
        vec![
            Identified {
                ch: 'O',
                bounds: Rect { left: 0.0, top: 0.0, right: 100.0, bottom: 100.0 },
                objects: vec![10],
                score: score_o,
            },
            Identified {
                ch: 'S',
                bounds: Rect { left: 110.0, top: 0.0, right: 210.0, bottom: 100.0 },
                objects: vec![11],
                score: score_s,
            },
        ]
    }

    /// A perfect match — score 0, as far from the tolerance as a match can
    /// be — reports as full confidence, not merely "high".
    #[test]
    fn a_perfect_match_reports_full_confidence() {
        let identified = oands_identified(0.0, 0.0);
        let words = as_words(&identified, 0.05);

        assert_eq!(words.len(), 1, "one word, same as `reconstruct` finds for this layout");
        assert_eq!(words[0].text, "OS");
        assert_eq!(words[0].confidence, 1.0);
        assert_eq!(words[0].char_confidence, vec![1.0, 1.0]);
    }

    /// **What a review pass sorts on**, carried through from a geometric
    /// distance rather than only from OCR: the word's own confidence is its
    /// *worst* letter, not an average that a good `O` could hide a bad `S`
    /// behind.
    #[test]
    fn word_confidence_is_the_weakest_letter_not_the_average() {
        let identified = oands_identified(0.0, 0.04); // S at exactly half the tolerance
        let words = as_words(&identified, 0.08);

        assert_eq!(words[0].char_confidence, vec![1.0, 0.5]);
        assert_eq!(words[0].confidence, 0.5, "the average would read 0.75, and hide the weak letter");
    }

    /// A match right at the tolerance itself — the worst a caller of
    /// `identify` ever admits — reports as zero confidence rather than a
    /// small positive number: it was barely accepted, not barely rejected.
    #[test]
    fn a_match_at_the_tolerance_itself_reports_zero_confidence() {
        let identified = oands_identified(0.0, 0.08);
        let words = as_words(&identified, 0.08);
        assert_eq!(words[0].char_confidence[1], 0.0);
    }

    #[test]
    fn the_words_rect_is_the_page_space_union_of_its_letters() {
        let identified = oands_identified(0.0, 0.0);
        let words = as_words(&identified, 0.05);

        assert_eq!(words[0].rect.left, 0.0);
        assert_eq!(words[0].rect.right, 210.0);
    }

    #[test]
    fn no_identified_clusters_means_no_words() {
        assert!(as_words(&[], 0.05).is_empty());
    }
}
