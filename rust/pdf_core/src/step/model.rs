//! The B-Rep this crate tessellates, in our own vocabulary.
//!
//! **Deliberately not step-io's types.** The parser is pinned at 0.2.4 and says
//! of itself: "expect breaking API changes at any time." Typing the tessellator
//! against its structures would make every one of those changes a geometry
//! rewrite. Everything the parser produces is adapted into the types here, so a
//! breaking release — or swapping the parser out entirely — is one file.
//!
//! The set is small on purpose. It is what the [audit](super::audit) found in
//! real supplier files, and nothing else: analytic surfaces, four curve kinds,
//! and the three orientation flags that decide which way a face points.

/// A point or a direction in space. Plain data, no linear-algebra dependency.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Point3 {
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    pub fn minus(self, other: Self) -> Self {
        Self::new(self.x - other.x, self.y - other.y, self.z - other.z)
    }

    pub fn plus(self, other: Self) -> Self {
        Self::new(self.x + other.x, self.y + other.y, self.z + other.z)
    }

    pub fn scaled(self, by: f64) -> Self {
        Self::new(self.x * by, self.y * by, self.z * by)
    }

    pub fn cross(self, other: Self) -> Self {
        Self::new(
            self.y * other.z - self.z * other.y,
            self.z * other.x - self.x * other.z,
            self.x * other.y - self.y * other.x,
        )
    }

    pub fn dot(self, other: Self) -> f64 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    pub fn length(self) -> f64 {
        self.dot(self).sqrt()
    }

    /// A unit vector, or `None` for something too short to have a direction.
    ///
    /// Returning `None` rather than dividing by a near-zero length: a normal of
    /// infinity renders as a face that is lit from nowhere, which looks like a
    /// shading bug rather than the degenerate triangle it actually is.
    pub fn normalised(self) -> Option<Self> {
        let length = self.length();
        if length < 1e-12 {
            None
        } else {
            Some(self.scaled(1.0 / length))
        }
    }
}

/// A point in a surface's own two-dimensional parameter space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point2 {
    pub u: f64,
    pub v: f64,
}

impl Point2 {
    pub const fn new(u: f64, v: f64) -> Self {
        Self { u, v }
    }
}

/// A right-handed frame: where a surface sits and how it is turned.
///
/// STEP gives these as an `AXIS2_PLACEMENT_3D` — an origin, an axis and a
/// reference direction — and every analytic surface is defined relative to one.
#[derive(Debug, Clone, Copy)]
pub struct Frame {
    pub origin: Point3,
    /// The local z, which for a plane is its normal and for a cylinder its axis.
    pub axis: Point3,
    /// The local x. STEP calls it the reference direction.
    pub reference: Point3,
}

impl Frame {
    /// The local y, completing the right-handed set.
    pub fn second(&self) -> Point3 {
        self.axis.cross(self.reference)
    }

    /// A local (x, y, z) turned into world coordinates.
    pub fn to_world(&self, x: f64, y: f64, z: f64) -> Point3 {
        self.origin
            .plus(self.reference.scaled(x))
            .plus(self.second().scaled(y))
            .plus(self.axis.scaled(z))
    }
}

/// The surfaces a face can be built on.
///
/// Analytic only. Freeform surfaces are counted and reported rather than
/// tessellated — see [`super::audit`] for how often that happens and what is
/// done about it.
#[derive(Debug, Clone)]
pub enum Surface {
    Plane { frame: Frame },
    Cylinder { frame: Frame, radius: f64 },
    Cone { frame: Frame, radius: f64, half_angle: f64 },
    Sphere { frame: Frame, radius: f64 },
    Torus { frame: Frame, major: f64, minor: f64 },
    /// A freeform patch. Boxed because it carries its control net and a
    /// sampling of itself, and a `Face` is cloned about freely.
    Spline(std::sync::Arc<super::spline::Spline>),
    /// Present in the file and not tessellated. Kept rather than dropped so it
    /// can be counted and named; never silently discarded.
    Unsupported { what: &'static str },
}

/// The curves an edge can follow.
///
/// `Spline` earns its place from the audit: freeform *edges* appear in 68% of
/// real files, including files whose surfaces are entirely analytic. Leaving it
/// out would have lost those faces to a curve type, not a surface type — the
/// failure the original plan did not have a gate for.
#[derive(Debug, Clone)]
pub enum Curve {
    Line { from: Point3, direction: Point3 },
    Circle { frame: Frame, radius: f64 },
    Ellipse { frame: Frame, major: f64, minor: f64 },
    /// Points already in order; used for anything sampled elsewhere.
    Polyline { points: Vec<Point3> },
    /// A B-spline, evaluated by de Boor. Degree, knots and control points as the
    /// file gives them; weights only for the rational form.
    Spline {
        degree: usize,
        control: Vec<Point3>,
        knots: Vec<f64>,
        weights: Option<Vec<f64>>,
    },
}

/// Identifies an edge so two faces sharing one can share its discretisation.
///
/// The first of the four traps: if the two faces meeting at an edge each
/// discretise it themselves, rounding sends them to slightly different points
/// and the mesh cracks along every seam. One id, one polyline, both faces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EdgeId(pub u64);

#[derive(Debug, Clone)]
pub struct Edge {
    pub id: EdgeId,
    pub curve: Curve,
    pub start: Point3,
    pub end: Point3,
    /// Whether the curve's own direction runs start → end.
    ///
    /// STEP's `EDGE_CURVE.same_sense`. Distinct from the loop's traversal
    /// direction below, and composing the two wrongly is trap two.
    pub same_sense: bool,
}

/// A closed circuit of edges, each with the direction it is walked in.
#[derive(Debug, Clone)]
pub struct Loop {
    /// Edge, and its `ORIENTED_EDGE.orientation`.
    pub edges: Vec<(EdgeId, bool)>,
    /// The `FACE_BOUND.orientation` of the bound this loop came from.
    pub bound_forward: bool,
}

#[derive(Debug, Clone)]
pub struct Face {
    pub surface: Surface,
    pub outer: Loop,
    pub inners: Vec<Loop>,
    /// `ADVANCED_FACE.same_sense`: whether the face agrees with its surface.
    pub same_sense: bool,
    /// Which placed copy of which component this face belongs to.
    ///
    /// Faces are grouped by it so a solid can be checked for being inside out
    /// on its own: a file can be right about one part and wrong about the next,
    /// and "encloses a volume" only means something per solid.
    pub component: u64,
}

/// One solid, and everything about it that could not be represented.
#[derive(Debug, Clone, Default)]
pub struct Solid {
    pub faces: Vec<Face>,
    pub edges: Vec<Edge>,
    /// Faces the file had that this model could not hold, by reason.
    ///
    /// **Never a silent drop.** A part shown with holes in it and no
    /// explanation is worse than one refused outright, because it looks like
    /// the part.
    pub skipped: Vec<Skipped>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Skipped {
    pub what: String,
    pub count: usize,
}

impl Solid {
    pub fn edge(&self, id: EdgeId) -> Option<&Edge> {
        self.edges.iter().find(|edge| edge.id == id)
    }
}
