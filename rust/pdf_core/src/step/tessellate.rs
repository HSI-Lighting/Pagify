//! Faces into triangles.
//!
//! Four steps per face, in this order:
//!
//! 1. take each bounding edge's points from the shared [cache](super::curve),
//!    so the two faces meeting at an edge cannot disagree about where it is
//! 2. [project](super::project) them into the surface's own `(u, v)`
//! 3. cut the resulting polygon-with-holes into triangles
//! 4. map the triangles back into space and give each a normal
//!
//! # Nothing disappears quietly
//!
//! A face that cannot be tessellated is **counted and named**, never dropped.
//! A part drawn with a wall missing looks like the part — there is nothing on
//! screen to say a face was lost — so the count is the only thing standing
//! between a silent geometry bug and a user who believes what they are looking
//! at.

use super::curve::EdgeCache;
use super::model::{Face, Point2, Point3, Skipped, Solid, Surface};
use super::orient::{self, walk};
use super::project::{self, unwrap_seam};

/// A triangle in space, with the normal it should be shaded by.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Triangle {
    pub a: Point3,
    pub b: Point3,
    pub c: Point3,
    pub normal: Point3,
}

/// What came of tessellating a solid.
#[derive(Debug, Clone, Default)]
pub struct Mesh {
    pub triangles: Vec<Triangle>,
    /// Faces that produced nothing, by reason. Reported to the user.
    pub skipped: Vec<Skipped>,
}

impl Mesh {
    /// The corners of the box the mesh occupies, for fitting it to the view.
    pub fn bounds(&self) -> Option<(Point3, Point3)> {
        let mut low = Point3::new(f64::MAX, f64::MAX, f64::MAX);
        let mut high = Point3::new(f64::MIN, f64::MIN, f64::MIN);

        for triangle in &self.triangles {
            for point in [triangle.a, triangle.b, triangle.c] {
                low = Point3::new(low.x.min(point.x), low.y.min(point.y), low.z.min(point.z));
                high = Point3::new(high.x.max(point.x), high.y.max(point.y), high.z.max(point.z));
            }
        }

        if self.triangles.is_empty() {
            None
        } else {
            Some((low, high))
        }
    }

    fn skip(&mut self, reason: &str) {
        if let Some(existing) = self.skipped.iter_mut().find(|s| s.what == reason) {
            existing.count += 1;
        } else {
            self.skipped.push(Skipped { what: reason.to_string(), count: 1 });
        }
    }
}


/// The sag to tessellate a solid at, scaled to its size.
///
/// **A fixed tolerance is wrong at both ends of a real catalogue.** The audited
/// parts run from a 10 mm button to a 3 m ring light. At a flat 0.02 mm the
/// button gets 35 segments round a hole — right — and the ring light gets 604
/// round each of its cylinders, which is half a million triangles for a shape
/// that is smooth at any reasonable size on a phone screen. Neither part is
/// unusual; the constant was.
///
/// So it is a fraction of the model's own diagonal: the error is held to
/// roughly a pixel of whatever is on screen, whatever the part measures. The
/// floor stops a tiny part being tessellated to the limits of `f64`.
pub fn recommended_sag(solid: &Solid) -> f64 {
    let mut low = Point3::new(f64::MAX, f64::MAX, f64::MAX);
    let mut high = Point3::new(f64::MIN, f64::MIN, f64::MIN);
    let mut seen = false;

    for edge in &solid.edges {
        for point in [edge.start, edge.end] {
            low = Point3::new(low.x.min(point.x), low.y.min(point.y), low.z.min(point.z));
            high = Point3::new(high.x.max(point.x), high.y.max(point.y), high.z.max(point.z));
            seen = true;
        }
    }

    if !seen {
        return crate::step::curve::DEFAULT_SAG;
    }

    let diagonal = high.minus(low).length();
    // A thousandth of the part. On a screen a few hundred pixels across, that
    // is well under one pixel of error, so a finer figure buys nothing that can
    // be seen and costs triangles that must be carried.
    (diagonal / 1000.0).max(1e-4)
}

/// Tessellate a whole solid.
pub fn tessellate(solid: &Solid, sag: f64) -> Mesh {
    // Every edge discretised before any face is cut, which is what makes the
    // shared boundary shared. See [`super::curve`].
    let cache = EdgeCache::build(&solid.edges, sag);
    let mut mesh = Mesh::default();

    for skipped in &solid.skipped {
        mesh.skipped.push(skipped.clone());
    }

    for face in &solid.faces {
        match face_triangles(face, &cache, sag) {
            Ok(triangles) => mesh.triangles.extend(triangles),
            Err(reason) => mesh.skip(reason),
        }
    }

    mesh
}

/// One face's triangles, or why it produced none.
pub fn face_triangles(
    face: &Face,
    cache: &EdgeCache,
    sag: f64,
) -> Result<Vec<Triangle>, &'static str> {
    // The reason the adapter gave, not a general one: a freeform surface and
    // a surface type this has never heard of are different problems, and
    // collapsing them tells the user the wrong thing about their file.
    if let Surface::Unsupported { what } = face.surface {
        return Err(what);
    }

    let outer = boundary(face, &face.outer, cache).ok_or("an unreadable outer boundary")?;
    if outer.len() < 3 {
        return Err("a boundary with no area");
    }

    let mut inners = Vec::new();
    for hole in &face.inners {
        match boundary(face, hole, cache) {
            Some(points) if points.len() >= 3 => inners.push(points),
            // A hole that cannot be read is not a reason to lose the face: the
            // face without its hole is wrong in one small place, where the face
            // missing entirely is a gap in the solid.
            _ => continue,
        }
    }

    let flat = triangulate(&outer, &inners)?;

    // Boundary points alone leave long chords across a curved face; see
    // [`parameter_limits`]. Splitting happens here, in parameter space, so
    // every new vertex lands exactly on the surface rather than on the chord.
    let (max_u, max_v) = parameter_limits(&face.surface, sag);
    let flat: Vec<Point2> = refine(
        flat.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect(),
        max_u,
        max_v,
    )
    .into_iter()
    .flatten()
    .collect();

    // Which way the loop actually wound, decided once for the face rather than
    // per triangle: a curved face's triangles do not all agree, and asking each
    // one separately is how a cylinder ends up with a few inverted facets.
    let sample = outer[outer.len() / 2];
    let surface_normal =
        project::normal_at(&face.surface, sample).ok_or("a surface with no normal")?;
    let outward = orient::outward_normal(surface_normal, face.same_sense);

    let mut triangles = Vec::with_capacity(flat.len() / 3);
    for corner in flat.chunks_exact(3) {
        let (Some(a), Some(b), Some(c)) = (
            project::evaluate(&face.surface, corner[0]),
            project::evaluate(&face.surface, corner[1]),
            project::evaluate(&face.surface, corner[2]),
        ) else {
            continue;
        };

        // The normal comes from the surface at the triangle's middle, not from
        // its winding. On a curved face the winding gives the chord's normal,
        // which is why a coarsely tessellated cylinder looks faceted even where
        // its silhouette is smooth.
        let middle = Point2::new(
            (corner[0].u + corner[1].u + corner[2].u) / 3.0,
            (corner[0].v + corner[1].v + corner[2].v) / 3.0,
        );
        let normal = project::normal_at(&face.surface, middle)
            .map(|n| orient::outward_normal(n, face.same_sense))
            .unwrap_or(outward);

        // And the winding is made to match it, so a renderer that culls back
        // faces keeps this one. Getting this wrong does not look wrong — the
        // face simply is not there.
        let wound = b.minus(a).cross(c.minus(a));
        if wound.dot(normal) < 0.0 {
            triangles.push(Triangle { a, b: c, c: b, normal });
        } else {
            triangles.push(Triangle { a, b, c, normal });
        }
    }

    if triangles.is_empty() {
        Err("a face that produced no triangles")
    } else {
        Ok(triangles)
    }
}

/// One loop, walked in order and expressed in the surface's coordinates.
pub(crate) fn boundary(face: &Face, the_loop: &super::model::Loop, cache: &EdgeCache) -> Option<Vec<Point2>> {
    let mut points: Vec<Point3> = Vec::new();

    for (id, forwards) in walk(the_loop) {
        let chain = cache.points(id, forwards)?;
        // The first point of each edge is the last of the one before it. Kept
        // once, or every vertex of the loop is duplicated and the triangulator
        // sees a zero-width spike at each corner.
        let start = usize::from(!points.is_empty());
        points.extend(chain.into_iter().skip(start));
    }

    // A closed loop returns to its start; the repeat is not a vertex.
    if points.len() > 1 {
        let first = points[0];
        if points[points.len() - 1].minus(first).length() < 1e-9 {
            points.pop();
        }
    }

    let mut projected: Vec<Option<Point2>> = points
        .iter()
        .map(|point| project::project(&face.surface, *point))
        .collect();

    unwrap_seam(&mut projected, project::u_is_periodic(&face.surface));
    projected.into_iter().collect()
}


/// How far a triangle may reach across a surface before its chord sags too far.
///
/// **Boundary points alone are not enough on a curved face.** A cylinder wall
/// is a rectangle in parameter space, and a triangulator handed a rectangle
/// quite reasonably cuts it into long triangles spanning the whole width — in
/// space, chords straight through the solid. Every vertex still sits exactly on
/// the surface, which is what makes it easy to miss: the mesh is not wrong
/// anywhere a vertex is, only everywhere between them.
///
/// The limit is the same sag rule the edges use: the angle whose chord departs
/// from the true surface by at most `sag`. Directions the surface is straight
/// in — a cylinder's axis, a cone's rulings — are unlimited, because a chord
/// along a ruling *is* the surface.
///
/// Returns the largest allowed span in `u` and in `v`.
pub(crate) fn parameter_limits(surface: &Surface, sag: f64) -> (f64, f64) {
    let angular = |radius: f64| -> f64 {
        if radius <= sag {
            std::f64::consts::PI
        } else {
            2.0 * (1.0 - sag / radius).clamp(-1.0, 1.0).acos()
        }
    };

    match surface {
        // Flat: a triangle of any size lies exactly on it.
        Surface::Plane { .. } | Surface::Unsupported { .. } => (f64::INFINITY, f64::INFINITY),
        // Straight along the axis, curved around it.
        Surface::Cylinder { radius, .. } => (angular(*radius), f64::INFINITY),
        // A cone is ruled too, so only the sweep is limited. The radius varies
        // with height; the base radius is the conservative choice at the narrow
        // end and the tolerance is only ever exceeded in the direction of finer.
        Surface::Cone { radius, .. } => (angular(radius.max(sag)), f64::INFINITY),
        Surface::Sphere { radius, .. } => (angular(*radius), angular(*radius)),
        Surface::Torus { major, minor, .. } => (angular(major + minor), angular(*minor)),
    }
}

/// Split triangles until none reaches further than the surface allows.
///
/// Longest-edge bisection: the offending edge is halved and the triangle
/// becomes two. The new vertex is a parameter pair, so mapping it back puts it
/// exactly on the surface rather than on the chord — which is the entire point,
/// and why this is done in parameter space rather than on the finished mesh.
fn refine(triangles: Vec<[Point2; 3]>, max_u: f64, max_v: f64) -> Vec<[Point2; 3]> {
    if !max_u.is_finite() && !max_v.is_finite() {
        return triangles;
    }

    // A ceiling on the work, so a surface with an absurd radius cannot turn one
    // face into millions of triangles and take the app down with it.
    const MOST: usize = 200_000;

    let mut pending = triangles;
    let mut done: Vec<[Point2; 3]> = Vec::new();

    while let Some(triangle) = pending.pop() {
        if done.len() + pending.len() >= MOST {
            done.push(triangle);
            continue;
        }

        // The edge that overreaches by the most, measured as a fraction of what
        // it is allowed, so u and v are compared on the same scale.
        let mut worst = 0usize;
        let mut excess = 1.0;
        for edge in 0..3 {
            let from = triangle[edge];
            let to = triangle[(edge + 1) % 3];
            let over = ((from.u - to.u).abs() / max_u).max((from.v - to.v).abs() / max_v);
            if over > excess {
                excess = over;
                worst = edge;
            }
        }

        if excess <= 1.0 {
            done.push(triangle);
            continue;
        }

        let from = triangle[worst];
        let to = triangle[(worst + 1) % 3];
        let opposite = triangle[(worst + 2) % 3];
        let middle = Point2::new((from.u + to.u) / 2.0, (from.v + to.v) / 2.0);

        pending.push([from, middle, opposite]);
        pending.push([middle, to, opposite]);
    }

    done
}
/// Cut a polygon with holes into triangles, in parameter space.
pub(crate) fn triangulate(outer: &[Point2], holes: &[Vec<Point2>]) -> Result<Vec<Point2>, &'static str> {
    let mut flat: Vec<f64> = Vec::with_capacity((outer.len() + holes.len() * 4) * 2);
    let mut all: Vec<Point2> = Vec::with_capacity(outer.len());
    let mut hole_starts = Vec::with_capacity(holes.len());

    for point in outer {
        flat.push(point.u);
        flat.push(point.v);
        all.push(*point);
    }
    for hole in holes {
        hole_starts.push(all.len());
        for point in hole {
            flat.push(point.u);
            flat.push(point.v);
            all.push(*point);
        }
    }

    let indices = earcutr::earcut(&flat, &hole_starts, 2).map_err(|_| "a boundary that would not cut")?;
    if indices.is_empty() {
        return Err("a boundary that cut into nothing");
    }

    Ok(indices.into_iter().filter_map(|index| all.get(index).copied()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::step::curve::DEFAULT_SAG;
    use crate::step::model::{Curve, Edge, EdgeId, Frame, Loop};

    fn frame() -> Frame {
        Frame {
            origin: Point3::new(0.0, 0.0, 0.0),
            axis: Point3::new(0.0, 0.0, 1.0),
            reference: Point3::new(1.0, 0.0, 0.0),
        }
    }

    fn line(id: u64, from: Point3, to: Point3) -> Edge {
        Edge {
            id: EdgeId(id),
            curve: Curve::Line { from, direction: to.minus(from) },
            start: from,
            end: to,
            same_sense: true,
        }
    }

    /// A unit square on z = 0, as four straight edges.
    fn square_solid(same_sense: bool) -> Solid {
        let corners = [
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(4.0, 0.0, 0.0),
            Point3::new(4.0, 4.0, 0.0),
            Point3::new(0.0, 4.0, 0.0),
        ];
        let edges = (0..4)
            .map(|index| line(index as u64 + 1, corners[index], corners[(index + 1) % 4]))
            .collect();

        Solid {
            faces: vec![Face {
                surface: Surface::Plane { frame: frame() },
                outer: Loop {
                    edges: (1..=4).map(|id| (EdgeId(id), true)).collect(),
                    bound_forward: true,
                },
                inners: Vec::new(),
                same_sense,
            }],
            edges,
            skipped: Vec::new(),
        }
    }

    // ---- a face at all -------------------------------------------------------

    #[test]
    fn a_square_becomes_two_triangles() {
        let mesh = tessellate(&square_solid(true), DEFAULT_SAG);
        assert_eq!(2, mesh.triangles.len());
        assert!(mesh.skipped.is_empty(), "{:?}", mesh.skipped);
    }

    /// The area is right, which a triangulation that overlaps itself would fail.
    #[test]
    fn the_triangles_cover_the_face_exactly_once() {
        let mesh = tessellate(&square_solid(true), DEFAULT_SAG);
        let area: f64 = mesh
            .triangles
            .iter()
            .map(|t| t.b.minus(t.a).cross(t.c.minus(t.a)).length() / 2.0)
            .sum();

        assert!((area - 16.0).abs() < 1e-9, "covered {area}, expected 16");
    }

    // ---- the winding, which decides whether a face is visible at all ---------

    /// Every triangle winds the way its face points.
    ///
    /// Under backface culling a triangle wound the other way is not drawn, so
    /// this failing does not look like a shading bug — the face is simply
    /// absent, and the part appears to have a hole in it.
    #[test]
    fn every_triangle_winds_with_its_normal() {
        for same_sense in [true, false] {
            let mesh = tessellate(&square_solid(same_sense), DEFAULT_SAG);
            for triangle in &mesh.triangles {
                let wound = triangle.b.minus(triangle.a).cross(triangle.c.minus(triangle.a));
                assert!(
                    wound.dot(triangle.normal) > 0.0,
                    "same_sense={same_sense}: {triangle:?} winds against its normal",
                );
            }
        }
    }

    /// And `same_sense = false` really does turn the face over.
    #[test]
    fn same_sense_false_points_the_face_the_other_way() {
        let facing = tessellate(&square_solid(true), DEFAULT_SAG);
        let flipped = tessellate(&square_solid(false), DEFAULT_SAG);

        assert!(facing.triangles[0].normal.z > 0.9);
        assert!(flipped.triangles[0].normal.z < -0.9);
    }

    // ---- holes ---------------------------------------------------------------

    #[test]
    fn a_face_with_a_hole_has_the_hole_taken_out_of_it() {
        let mut solid = square_solid(true);

        // A square hole in the middle, wound the other way as a hole is.
        let hole = [
            Point3::new(1.0, 1.0, 0.0),
            Point3::new(1.0, 3.0, 0.0),
            Point3::new(3.0, 3.0, 0.0),
            Point3::new(3.0, 1.0, 0.0),
        ];
        for index in 0..4 {
            solid.edges.push(line(10 + index as u64, hole[index], hole[(index + 1) % 4]));
        }
        solid.faces[0].inners.push(Loop {
            edges: (10..14).map(|id| (EdgeId(id), true)).collect(),
            bound_forward: true,
        });

        let mesh = tessellate(&solid, DEFAULT_SAG);
        let area: f64 = mesh
            .triangles
            .iter()
            .map(|t| t.b.minus(t.a).cross(t.c.minus(t.a)).length() / 2.0)
            .sum();

        // Sixteen less the four of the hole.
        assert!((area - 12.0).abs() < 1e-6, "covered {area}, expected 12");
    }

    // ---- curved faces --------------------------------------------------------

    /// A cylinder wall tessellates, and its normals point outwards everywhere.
    #[test]
    fn a_cylinder_wall_is_round_and_faces_outwards() {
        let radius = 5.0;
        let bottom = Point3::new(radius, 0.0, 0.0);
        let top = Point3::new(radius, 0.0, 10.0);

        let rim = |id: u64, height: f64| Edge {
            id: EdgeId(id),
            curve: Curve::Circle {
                frame: Frame {
                    origin: Point3::new(0.0, 0.0, height),
                    ..frame()
                },
                radius,
            },
            start: Point3::new(radius, 0.0, height),
            end: Point3::new(radius, 0.0, height),
            same_sense: true,
        };

        let solid = Solid {
            faces: vec![Face {
                surface: Surface::Cylinder { frame: frame(), radius },
                outer: Loop {
                    edges: vec![
                        (EdgeId(1), true),  // bottom rim
                        (EdgeId(2), true),  // up the seam
                        (EdgeId(3), false), // top rim, the other way
                        (EdgeId(4), false), // back down
                    ],
                    bound_forward: true,
                },
                inners: Vec::new(),
                same_sense: true,
            }],
            edges: vec![
                rim(1, 0.0),
                line(2, bottom, top),
                rim(3, 10.0),
                line(4, bottom, top),
            ],
            skipped: Vec::new(),
        };

        let mesh = tessellate(&solid, DEFAULT_SAG);
        assert!(!mesh.triangles.is_empty(), "skipped: {:?}", mesh.skipped);

        // **The property that matters: no chord cuts through the solid.**
        // Every vertex sitting exactly on the cylinder proves nothing on its
        // own — a triangle spanning half the barrel has all three vertices on
        // the surface and passes straight through the middle of the part. What
        // has to hold is that the surface between the vertices is never far
        // from the triangle, which is the same sag the edges are held to.
        for triangle in &mesh.triangles {
            for pair in [(triangle.a, triangle.b), (triangle.b, triangle.c), (triangle.c, triangle.a)] {
                let middle = pair.0.plus(pair.1).scaled(0.5);
                let sag = radius - middle.x.hypot(middle.y);
                assert!(
                    sag <= DEFAULT_SAG * 1.05,
                    "a chord sags {sag} into the solid, allowed {DEFAULT_SAG}",
                );
            }
            // Radial, and never inward-facing.
            assert!(triangle.normal.z.abs() < 1e-6, "not radial: {:?}", triangle.normal);
            let middle = triangle.a.plus(triangle.b).plus(triangle.c).scaled(1.0 / 3.0);
            let outwards = Point3::new(middle.x, middle.y, 0.0);
            assert!(outwards.dot(triangle.normal) > 0.0, "inwards at {middle:?}");
        }
    }

    // ---- nothing vanishes ----------------------------------------------------

    /// A freeform face is counted, by name, rather than dropped.
    #[test]
    fn an_unsupported_face_is_counted_and_named() {
        let mut solid = square_solid(true);
        solid.faces.push(Face {
            surface: Surface::Unsupported { what: "freeform surfaces" },
            outer: Loop { edges: vec![(EdgeId(1), true)], bound_forward: true },
            inners: Vec::new(),
            same_sense: true,
        });

        let mesh = tessellate(&solid, DEFAULT_SAG);

        assert_eq!(2, mesh.triangles.len(), "the good face still came through");
        assert_eq!(1, mesh.skipped.len());
        assert_eq!("freeform surfaces", mesh.skipped[0].what);
        assert_eq!(1, mesh.skipped[0].count);
    }

    /// Several of the same kind are counted together, not listed one by one.
    #[test]
    fn faces_skipped_for_the_same_reason_are_counted_together() {
        let mut solid = square_solid(true);
        for _ in 0..3 {
            solid.faces.push(Face {
                surface: Surface::Unsupported { what: "freeform surfaces" },
                outer: Loop { edges: vec![(EdgeId(1), true)], bound_forward: true },
                inners: Vec::new(),
                same_sense: true,
            });
        }

        let mesh = tessellate(&solid, DEFAULT_SAG);
        assert_eq!(1, mesh.skipped.len());
        assert_eq!(3, mesh.skipped[0].count);
    }

    /// A face whose edges are missing is reported, not silently absent.
    #[test]
    fn a_face_with_an_unreadable_boundary_is_reported() {
        let mut solid = square_solid(true);
        solid.edges.remove(0);

        let mesh = tessellate(&solid, DEFAULT_SAG);
        assert!(mesh.triangles.is_empty());
        assert_eq!(1, mesh.skipped.len(), "{:?}", mesh.skipped);
    }

    /// A hole that cannot be read costs the hole, not the whole face.
    ///
    /// The face without its hole is wrong in one small place; the face missing
    /// altogether is a gap straight through the solid.
    #[test]
    fn an_unreadable_hole_does_not_take_the_face_with_it() {
        let mut solid = square_solid(true);
        solid.faces[0].inners.push(Loop {
            edges: vec![(EdgeId(999), true)], // never cached
            bound_forward: true,
        });

        let mesh = tessellate(&solid, DEFAULT_SAG);
        assert_eq!(2, mesh.triangles.len(), "the face itself survived");
    }

    // ---- fitting to view -----------------------------------------------------

    // ---- how far a triangle may reach ---------------------------------------

    /// A plane is flat, so a triangle of any size lies exactly on it.
    #[test]
    fn a_plane_is_never_refined() {
        let (u, v) = parameter_limits(&Surface::Plane { frame: frame() }, DEFAULT_SAG);
        assert!(u.is_infinite() && v.is_infinite());
    }

    /// A cylinder is straight along its axis: only the sweep is limited.
    ///
    /// Limiting the axis as well would multiply the triangles of every hole
    /// and boss in a part for no gain at all -- a chord along a ruling *is*
    /// the surface.
    #[test]
    fn a_cylinder_is_limited_around_but_not_along() {
        let (u, v) = parameter_limits(&Surface::Cylinder { frame: frame(), radius: 5.0 }, DEFAULT_SAG);
        assert!(u.is_finite() && u > 0.0, "no sweep limit: {u}");
        assert!(v.is_infinite(), "the axis should not be subdivided");
    }

    /// A sphere curves both ways, so both are limited.
    #[test]
    fn a_sphere_is_limited_in_both_directions() {
        let (u, v) = parameter_limits(&Surface::Sphere { frame: frame(), radius: 8.0 }, DEFAULT_SAG);
        assert!(u.is_finite() && v.is_finite(), "{u}, {v}");
    }

    /// A tighter tolerance means smaller steps, which is the whole idea.
    #[test]
    fn a_finer_tolerance_allows_less_reach() {
        let coarse = parameter_limits(&Surface::Cylinder { frame: frame(), radius: 5.0 }, 0.5).0;
        let fine = parameter_limits(&Surface::Cylinder { frame: frame(), radius: 5.0 }, 0.001).0;
        assert!(fine < coarse, "fine {fine} was not tighter than coarse {coarse}");
    }

    /// Splitting stops once every triangle is inside the limit.
    #[test]
    fn refining_stops_when_the_triangles_are_small_enough() {
        let big = vec![[
            Point2::new(0.0, 0.0),
            Point2::new(6.0, 0.0),
            Point2::new(0.0, 6.0),
        ]];

        let refined = refine(big, 1.0, 1.0);

        assert!(refined.len() > 1, "nothing was split");
        for triangle in &refined {
            for edge in 0..3 {
                let from = triangle[edge];
                let to = triangle[(edge + 1) % 3];
                assert!(
                    (from.u - to.u).abs() <= 1.0 + 1e-9 && (from.v - to.v).abs() <= 1.0 + 1e-9,
                    "an edge still spans too far: {from:?} to {to:?}",
                );
            }
        }
    }

    /// Refining keeps the area it started with: no gaps, no overlaps.
    #[test]
    fn refining_neither_loses_nor_duplicates_area() {
        let area = |t: &[Point2; 3]| {
            ((t[1].u - t[0].u) * (t[2].v - t[0].v) - (t[2].u - t[0].u) * (t[1].v - t[0].v)).abs() / 2.0
        };
        let original = [Point2::new(0.0, 0.0), Point2::new(6.0, 0.0), Point2::new(0.0, 6.0)];
        let before = area(&original);

        let after: f64 = refine(vec![original], 0.7, 0.7).iter().map(area).sum();

        assert!((before - after).abs() < 1e-9, "{before} became {after}");
    }

    #[test]
    fn the_bounds_of_a_square_are_the_square() {
        let mesh = tessellate(&square_solid(true), DEFAULT_SAG);
        let (low, high) = mesh.bounds().expect("a mesh has bounds");

        assert_eq!(Point3::new(0.0, 0.0, 0.0), low);
        assert_eq!(Point3::new(4.0, 4.0, 0.0), high);
    }

    #[test]
    fn an_empty_mesh_has_no_bounds_rather_than_a_point_at_the_origin() {
        assert!(Mesh::default().bounds().is_none());
    }
}
