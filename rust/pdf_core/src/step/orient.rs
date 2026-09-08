//! Which way a face points, and which way its loops run.
//!
//! # The second trap, and why this has a module to itself
//!
//! Three independent booleans decide the answer, and they compose:
//!
//! | Flag | Says |
//! |---|---|
//! | `ORIENTED_EDGE.orientation` | whether this edge is walked start → end |
//! | `FACE_BOUND.orientation` | whether the whole loop is walked as given |
//! | `ADVANCED_FACE.same_sense` | whether the face agrees with its surface |
//!
//! Get any one of them wrong and a face's normal inverts. Under backface
//! culling an inverted face does not look wrong — it **disappears**, and a part
//! with a missing wall reads as a tessellation failure or a hole in the model.
//! Under two-sided shading it reads as a lighting bug. Either way the eye is
//! sent to the wrong place, which is why this is arithmetic with tests rather
//! than a flag threaded through the geometry and hoped about.
//!
//! Written before any rendering exists, on purpose: a flipped face has to fail
//! a test here, at the point the decision is made, rather than be noticed later
//! as a shading oddity in a picture.

use super::model::{Face, Loop, Point3};

/// Whether an edge is walked from its start vertex towards its end vertex.
///
/// Reversing the loop reverses every edge in it, so the two flags cancel: an
/// edge marked backwards inside a loop that is itself marked backwards runs
/// forwards. Hence equality rather than `and`, which is the mistake this
/// function exists to make impossible to write twice.
pub fn edge_runs_forward(oriented_edge: bool, bound_forward: bool) -> bool {
    oriented_edge == bound_forward
}

/// The order the edges of a loop are visited in.
///
/// Yields `(edge, forwards)` pairs already composed, so callers never see the
/// two flags separately and cannot combine them a second time by accident.
pub fn walk(the_loop: &Loop) -> Vec<(super::model::EdgeId, bool)> {
    let forwards: Vec<_> = the_loop
        .edges
        .iter()
        .map(|(id, oriented)| (*id, edge_runs_forward(*oriented, the_loop.bound_forward)))
        .collect();

    if the_loop.bound_forward {
        forwards
    } else {
        // A reversed bound is walked in the opposite order as well as with each
        // edge reversed. Reversing only the edges leaves the circuit visiting
        // its vertices out of sequence, which produces a self-crossing polygon
        // rather than an inverted one — a subtler wrong than a flipped normal
        // and much harder to see.
        forwards.into_iter().rev().collect()
    }
}

/// The outward normal of a face, given its surface's own normal at a point.
///
/// `same_sense` is the only thing that can flip it, and it is the flag most
/// often ignored: a solid modelled with inward-facing surfaces is perfectly
/// legal STEP and renders inside out without this.
pub fn outward_normal(surface_normal: Point3, same_sense: bool) -> Point3 {
    if same_sense {
        surface_normal
    } else {
        surface_normal.scaled(-1.0)
    }
}

/// The normal of a polygon from its winding, by Newell's method.
///
/// Newell rather than the cross product of the first two edges: a real face
/// loop starts with whatever edge the file listed first, and if those two
/// happen to be collinear — which they are wherever a straight side was split
/// at a vertex — the cross product is zero and the normal is nonsense. Newell
/// uses every vertex, so no single pair can ruin it, and it is correct for
/// non-planar loops too.
pub fn winding_normal(points: &[Point3]) -> Option<Point3> {
    if points.len() < 3 {
        return None;
    }
    let mut normal = Point3::new(0.0, 0.0, 0.0);
    for (index, current) in points.iter().enumerate() {
        let next = points[(index + 1) % points.len()];
        normal = normal.plus(Point3::new(
            (current.y - next.y) * (current.z + next.z),
            (current.z - next.z) * (current.x + next.x),
            (current.x - next.x) * (current.y + next.y),
        ));
    }
    normal.normalised()
}

/// Whether a loop, walked as the flags say, winds the way the face points.
///
/// This is the check the whole module exists for. It is separate from the
/// tessellator so it can be asserted on a face directly, before there are any
/// triangles to look at.
pub fn winding_agrees(face: &Face, walked_points: &[Point3], surface_normal: Point3) -> bool {
    match winding_normal(walked_points) {
        None => false,
        Some(from_winding) => {
            from_winding.dot(outward_normal(surface_normal, face.same_sense)) > 0.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::step::model::{Curve, Edge, EdgeId, Frame, Point3, Surface};

    fn frame_z_up() -> Frame {
        Frame {
            origin: Point3::new(0.0, 0.0, 0.0),
            axis: Point3::new(0.0, 0.0, 1.0),
            reference: Point3::new(1.0, 0.0, 0.0),
        }
    }

    /// A unit square on the z = 0 plane, wound anticlockwise seen from +z.
    fn square() -> Vec<Point3> {
        vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(1.0, 1.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ]
    }

    fn face(same_sense: bool, bound_forward: bool) -> Face {
        Face {
            surface: Surface::Plane { frame: frame_z_up() },
            outer: Loop {
                edges: vec![
                    (EdgeId(1), true),
                    (EdgeId(2), true),
                    (EdgeId(3), true),
                    (EdgeId(4), true),
                ],
                bound_forward,
            },
            inners: Vec::new(),
            same_sense,
            component: 0,
        }
    }

    // ---- the flags, one at a time ------------------------------------------

    /// Two reversals cancel. The `and` that looks right here is the bug.
    #[test]
    fn a_reversed_edge_in_a_reversed_loop_runs_forwards() {
        assert!(edge_runs_forward(false, false));
        assert!(edge_runs_forward(true, true));
        assert!(!edge_runs_forward(true, false));
        assert!(!edge_runs_forward(false, true));
    }

    /// A reversed bound walks the circuit backwards as well as each edge.
    ///
    /// Reversing only the edges and keeping the order gives a loop that visits
    /// its vertices out of sequence — a self-crossing polygon, which does not
    /// look like an inverted face and is much harder to recognise.
    #[test]
    fn a_reversed_bound_reverses_the_order_too() {
        let reversed = walk(&Loop {
            edges: vec![(EdgeId(1), true), (EdgeId(2), true), (EdgeId(3), true)],
            bound_forward: false,
        });

        assert_eq!(
            vec![(EdgeId(3), false), (EdgeId(2), false), (EdgeId(1), false)],
            reversed,
        );
    }

    #[test]
    fn a_forward_bound_is_left_alone() {
        let forward = walk(&Loop {
            edges: vec![(EdgeId(1), true), (EdgeId(2), false)],
            bound_forward: true,
        });
        assert_eq!(vec![(EdgeId(1), true), (EdgeId(2), false)], forward);
    }

    /// `same_sense = false` is legal STEP and inverts the face.
    #[test]
    fn same_sense_false_turns_the_face_over() {
        let up = Point3::new(0.0, 0.0, 1.0);
        assert_eq!(1.0, outward_normal(up, true).z);
        assert_eq!(-1.0, outward_normal(up, false).z);
    }

    // ---- winding ------------------------------------------------------------

    #[test]
    fn an_anticlockwise_square_faces_the_way_it_is_wound() {
        let normal = winding_normal(&square()).expect("a square has a normal");
        assert!(normal.z > 0.9, "expected +z, got {normal:?}");
    }

    #[test]
    fn reversing_the_winding_reverses_the_normal() {
        let mut backwards = square();
        backwards.reverse();
        let normal = winding_normal(&backwards).expect("a square has a normal");
        assert!(normal.z < -0.9, "expected -z, got {normal:?}");
    }

    /// Newell's reason for existing: a side split at a vertex.
    ///
    /// The first two edges are then collinear, and the obvious
    /// first-two-edges cross product is the zero vector. Every real part has
    /// this — any face whose straight edge meets another face part-way along.
    #[test]
    fn a_split_side_still_has_a_normal() {
        let with_a_split = vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(0.5, 0.0, 0.0), // collinear with the next
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(1.0, 1.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ];

        // What the naive method would have produced, kept as the contrast.
        let naive = with_a_split[1]
            .minus(with_a_split[0])
            .cross(with_a_split[2].minus(with_a_split[1]));
        assert!(naive.length() < 1e-12, "the first two edges are collinear");

        let normal = winding_normal(&with_a_split).expect("Newell copes");
        assert!(normal.z > 0.9, "expected +z, got {normal:?}");
    }

    #[test]
    fn a_degenerate_loop_has_no_normal() {
        assert!(winding_normal(&[]).is_none());
        assert!(winding_normal(&square()[..2]).is_none());
        let all_the_same = vec![Point3::new(1.0, 1.0, 1.0); 5];
        assert!(winding_normal(&all_the_same).is_none());
    }

    // ---- the two composed ---------------------------------------------------

    /// The whole point: every combination of the flags agrees with itself.
    ///
    /// A face whose surface points up and whose loop winds anticlockwise agrees
    /// when `same_sense` is true; the same loop disagrees when it is false, and
    /// the fix is to wind the other way — not to negate the normal a second
    /// time somewhere else, which is how these errors get "fixed" into a state
    /// where two wrongs cancel on one part and not on the next.
    #[test]
    fn winding_and_normal_agree_in_every_combination() {
        let up = Point3::new(0.0, 0.0, 1.0);

        for same_sense in [true, false] {
            for bound_forward in [true, false] {
                let subject = face(same_sense, bound_forward);
                let mut points = square();
                if !same_sense {
                    points.reverse();
                }

                assert!(
                    winding_agrees(&subject, &points, up),
                    "same_sense={same_sense} bound_forward={bound_forward} disagreed",
                );
            }
        }
    }

    /// And it can fail. A test that only ever passes proves nothing.
    #[test]
    fn a_face_wound_the_wrong_way_is_caught() {
        let up = Point3::new(0.0, 0.0, 1.0);

        // Anticlockwise loop on a face declared to disagree with its surface:
        // the normal points down, the winding says up.
        assert!(!winding_agrees(&face(false, true), &square(), up));

        // And the mirror: clockwise loop on a face that agrees.
        let mut backwards = square();
        backwards.reverse();
        assert!(!winding_agrees(&face(true, true), &backwards, up));
    }

    /// A loop that encloses nothing cannot be said to agree with anything.
    #[test]
    fn a_collapsed_face_does_not_quietly_agree() {
        let up = Point3::new(0.0, 0.0, 1.0);
        let collapsed = vec![Point3::new(2.0, 2.0, 0.0); 4];
        assert!(!winding_agrees(&face(true, true), &collapsed, up));
    }

    /// The edge's own `same_sense` is a third, separate thing.
    ///
    /// It says whether the *curve* runs start → end, which decides how the
    /// curve is sampled — not which way the loop is walked. Keeping it out of
    /// [`walk`] is deliberate: folding it in here would make a reversed curve
    /// on a forward edge silently reverse the loop.
    #[test]
    fn an_edges_own_sense_is_not_the_loops_sense() {
        let edge = Edge {
            id: EdgeId(1),
            curve: Curve::Line {
                from: Point3::new(0.0, 0.0, 0.0),
                direction: Point3::new(1.0, 0.0, 0.0),
            },
            start: Point3::new(0.0, 0.0, 0.0),
            end: Point3::new(1.0, 0.0, 0.0),
            same_sense: false,
        };

        // Walking is decided by the loop's flags alone.
        let walked = walk(&Loop {
            edges: vec![(edge.id, true)],
            bound_forward: true,
        });
        assert_eq!(vec![(EdgeId(1), true)], walked);
        assert!(!edge.same_sense, "and the curve's own sense is untouched");
    }
}
