//! Putting an assembly's parts where they belong.
//!
//! # What goes wrong without this
//!
//! A STEP assembly does not hold one model. It holds a *component* for each
//! part — each modelled about its own origin, as it was drawn — and separately
//! a transform saying where that component sits in the whole. Read the geometry
//! and ignore the transforms and every part is drawn at the origin, inside every
//! other part.
//!
//! It does not look like an error. It looks like a different object: a turbine
//! whose gear wheel and mounting brackets are somewhere inside its blades reads
//! as a strange sculpture rather than as a missing step, which is exactly what
//! a real assembly showed.
//!
//! # How it is written down
//!
//! `REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION` names two representations
//! and an `ITEM_DEFINED_TRANSFORMATION` between them. The transformation is two
//! placements: where the component's own origin lands in the parent, and what
//! the component calls its origin. Mapping one frame onto the other is the
//! transform, and a nested assembly composes them up to the root.

use super::model::{Frame, Point3};

/// A rotation and a translation. No scaling: STEP assemblies do not scale
/// components, and silently supporting it would hide a file that meant
/// something else.
#[derive(Debug, Clone, Copy)]
pub struct Rigid {
    /// The images of x, y and z.
    pub x: Point3,
    pub y: Point3,
    pub z: Point3,
    pub origin: Point3,
}

impl Rigid {
    pub const fn identity() -> Self {
        Self {
            x: Point3::new(1.0, 0.0, 0.0),
            y: Point3::new(0.0, 1.0, 0.0),
            z: Point3::new(0.0, 0.0, 1.0),
            origin: Point3::new(0.0, 0.0, 0.0),
        }
    }

    /// The transform that takes the world onto a frame.
    pub fn of(frame: &Frame) -> Self {
        Self {
            x: frame.reference,
            y: frame.second(),
            z: frame.axis,
            origin: frame.origin,
        }
    }

    /// A point moved.
    pub fn apply(&self, point: Point3) -> Point3 {
        self.origin
            .plus(self.x.scaled(point.x))
            .plus(self.y.scaled(point.y))
            .plus(self.z.scaled(point.z))
    }

    /// A direction moved: rotated, never shifted.
    ///
    /// Separate from [`apply`](Self::apply) because an axis is not a position.
    /// Translating a surface's normal is the mistake that puts a cylinder's
    /// axis somewhere in space instead of pointing along it.
    pub fn direction(&self, along: Point3) -> Point3 {
        self.x
            .scaled(along.x)
            .plus(self.y.scaled(along.y))
            .plus(self.z.scaled(along.z))
    }

    /// The inverse. A rotation's inverse is its transpose, which is why this
    /// needs no determinant and cannot fail.
    pub fn inverse(&self) -> Self {
        let x = Point3::new(self.x.x, self.y.x, self.z.x);
        let y = Point3::new(self.x.y, self.y.y, self.z.y);
        let z = Point3::new(self.x.z, self.y.z, self.z.z);
        let back = x
            .scaled(self.origin.x)
            .plus(y.scaled(self.origin.y))
            .plus(z.scaled(self.origin.z));
        Self {
            x,
            y,
            z,
            origin: back.scaled(-1.0),
        }
    }

    /// This transform, then `outer`.
    pub fn then(&self, outer: &Self) -> Self {
        Self {
            x: outer.direction(self.x),
            y: outer.direction(self.y),
            z: outer.direction(self.z),
            origin: outer.apply(self.origin),
        }
    }

    /// A whole frame moved.
    pub fn moved(&self, frame: &Frame) -> Frame {
        Frame {
            origin: self.apply(frame.origin),
            axis: self.direction(frame.axis),
            reference: self.direction(frame.reference),
        }
    }

    pub fn is_identity(&self) -> bool {
        let identity = Self::identity();
        self.origin.minus(identity.origin).length() < 1e-12
            && self.x.minus(identity.x).length() < 1e-12
            && self.y.minus(identity.y).length() < 1e-12
            && self.z.minus(identity.z).length() < 1e-12
    }
}

/// Where a component sits, from the two placements of the transformation.
///
/// `into_parent` is where the component's origin lands; `own_origin` is what
/// the component calls its origin. The map from the second onto the first is
/// the placement, and the second is very often the identity — which is why
/// taking only the first works on most files and fails silently on the rest.
pub fn between(into_parent: &Frame, own_origin: &Frame) -> Rigid {
    Rigid::of(own_origin).inverse().then(&Rigid::of(into_parent))
}

/// Compose a component's transform up to the root of the assembly.
///
/// `parent_of` gives each node its parent and the transform into it. Nested
/// assemblies are ordinary — a gearbox inside a machine inside a rig — and
/// applying only the immediate transform leaves a subassembly correct within
/// itself and in the wrong place as a whole.
pub fn to_root<Node: Copy + Eq + std::hash::Hash>(
    node: Node,
    parent_of: &std::collections::HashMap<Node, (Node, Rigid)>,
) -> Rigid {
    let mut here = node;
    let mut composed = Rigid::identity();

    // A file whose relationships form a cycle would otherwise hang. The bound
    // is generous for any real assembly and finite for a malformed one.
    for _ in 0..64 {
        match parent_of.get(&here) {
            Some((parent, step)) => {
                composed = composed.then(step);
                here = *parent;
            }
            None => break,
        }
    }

    composed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(origin: Point3, axis: Point3, reference: Point3) -> Frame {
        Frame { origin, axis, reference }
    }

    fn identity_frame() -> Frame {
        frame(
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(0.0, 0.0, 1.0),
            Point3::new(1.0, 0.0, 0.0),
        )
    }

    /// A quarter turn about z, then ten along x.
    fn quarter_turn() -> Rigid {
        Rigid {
            x: Point3::new(0.0, 1.0, 0.0),
            y: Point3::new(-1.0, 0.0, 0.0),
            z: Point3::new(0.0, 0.0, 1.0),
            origin: Point3::new(10.0, 0.0, 0.0),
        }
    }

    #[test]
    fn the_identity_moves_nothing() {
        let point = Point3::new(3.0, -4.0, 5.0);
        let moved = Rigid::identity().apply(point);
        assert!(moved.minus(point).length() < 1e-12, "{moved:?}");
    }

    #[test]
    fn a_transform_turns_and_shifts() {
        let moved = quarter_turn().apply(Point3::new(1.0, 0.0, 0.0));
        assert!(moved.minus(Point3::new(10.0, 1.0, 0.0)).length() < 1e-12, "{moved:?}");
    }

    /// A direction is rotated but never shifted.
    ///
    /// Treating an axis as a position is the mistake that leaves a cylinder
    /// pointing at the origin rather than along itself — and it renders as a
    /// part that is *almost* right, which is the hardest kind to notice.
    #[test]
    fn a_direction_is_rotated_but_not_moved() {
        let along = quarter_turn().direction(Point3::new(1.0, 0.0, 0.0));
        assert!(
            along.minus(Point3::new(0.0, 1.0, 0.0)).length() < 1e-12,
            "{along:?}",
        );
    }

    #[test]
    fn a_transform_and_its_inverse_cancel() {
        let there_and_back = quarter_turn().then(&quarter_turn().inverse());
        assert!(there_and_back.is_identity(), "{there_and_back:?}");

        let point = Point3::new(2.0, 7.0, -3.0);
        let round_trip = quarter_turn().inverse().apply(quarter_turn().apply(point));
        assert!(round_trip.minus(point).length() < 1e-9, "{round_trip:?}");
    }

    /// Composing is applying one and then the other, in that order.
    #[test]
    fn composing_is_the_same_as_applying_in_turn() {
        let shift = Rigid {
            origin: Point3::new(0.0, 0.0, 5.0),
            ..Rigid::identity()
        };
        let point = Point3::new(1.0, 0.0, 0.0);

        let composed = quarter_turn().then(&shift).apply(point);
        let by_hand = shift.apply(quarter_turn().apply(point));

        assert!(composed.minus(by_hand).length() < 1e-12, "{composed:?} vs {by_hand:?}");
    }

    /// A whole frame moves as a unit, staying square to itself.
    #[test]
    fn a_moved_frame_is_still_a_frame() {
        let moved = quarter_turn().moved(&identity_frame());

        assert!(moved.axis.dot(moved.reference).abs() < 1e-12, "no longer square");
        assert!((moved.axis.length() - 1.0).abs() < 1e-12, "no longer unit");
        assert!(moved.origin.minus(Point3::new(10.0, 0.0, 0.0)).length() < 1e-12);
    }

    // ---- what the file actually says -----------------------------------------

    /// A component whose own origin is the identity lands where the parent says.
    ///
    /// The common case, and the reason taking only the first placement appears
    /// to work — until a file states a non-trivial second one.
    #[test]
    fn a_component_at_its_own_origin_goes_where_it_is_put() {
        let into_parent = frame(
            Point3::new(0.0, 0.0, 25.0),
            Point3::new(0.0, 0.0, 1.0),
            Point3::new(1.0, 0.0, 0.0),
        );

        let placed = between(&into_parent, &identity_frame());
        let moved = placed.apply(Point3::new(1.0, 2.0, 3.0));

        assert!(moved.minus(Point3::new(1.0, 2.0, 28.0)).length() < 1e-12, "{moved:?}");
    }

    /// And when it is not the identity, the second placement matters.
    ///
    /// Ignoring it puts the component in the parent's place but rotated by
    /// whatever the component thought its own orientation was.
    #[test]
    fn the_components_own_placement_is_not_ignored() {
        let into_parent = identity_frame();
        // The component considers its origin to be turned a quarter turn.
        let own = frame(
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(0.0, 0.0, 1.0),
            Point3::new(0.0, 1.0, 0.0),
        );

        let placed = between(&into_parent, &own);
        let moved = placed.apply(Point3::new(1.0, 0.0, 0.0));

        // Its x was pointing along the world's y, so it comes back to x.
        assert!(
            moved.minus(Point3::new(0.0, -1.0, 0.0)).length() < 1e-9,
            "{moved:?}",
        );
        assert!(!placed.is_identity(), "the second placement was ignored");
    }

    // ---- nesting -------------------------------------------------------------

    /// A part inside a subassembly inside the whole ends up in the right place.
    #[test]
    fn a_nested_component_composes_all_the_way_up() {
        let mut parents = std::collections::HashMap::new();
        let up_ten = Rigid {
            origin: Point3::new(0.0, 0.0, 10.0),
            ..Rigid::identity()
        };
        let along_five = Rigid {
            origin: Point3::new(5.0, 0.0, 0.0),
            ..Rigid::identity()
        };

        parents.insert(2u32, (1u32, up_ten)); // subassembly into the whole
        parents.insert(3u32, (2u32, along_five)); // part into the subassembly

        let placed = to_root(3u32, &parents);
        let moved = placed.apply(Point3::new(0.0, 0.0, 0.0));

        assert!(
            moved.minus(Point3::new(5.0, 0.0, 10.0)).length() < 1e-12,
            "{moved:?}",
        );
    }

    #[test]
    fn a_root_component_is_left_where_it_is() {
        let parents: std::collections::HashMap<u32, (u32, Rigid)> =
            std::collections::HashMap::new();
        assert!(to_root(1u32, &parents).is_identity());
    }

    /// A file whose relationships loop does not hang.
    #[test]
    fn a_cycle_terminates_rather_than_spinning() {
        let mut parents = std::collections::HashMap::new();
        let step = Rigid {
            origin: Point3::new(1.0, 0.0, 0.0),
            ..Rigid::identity()
        };
        parents.insert(1u32, (2u32, step));
        parents.insert(2u32, (1u32, step));

        // Finishes, and produces something finite rather than looping.
        let placed = to_root(1u32, &parents);
        assert!(placed.origin.x.is_finite());
    }
}
