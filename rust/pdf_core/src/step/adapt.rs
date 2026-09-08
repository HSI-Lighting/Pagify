//! From the parser's world into ours.
//!
//! **This is the only file that knows `step-io` exists.** The crate is version
//! 0.2.4 and says of itself "expect breaking API changes at any time", so it is
//! pinned exactly and kept behind this wall. When it breaks — or is replaced —
//! the cost is this file, not the tessellator, the projection or the
//! orientation rules. Same reasoning as keeping annotation operations off the
//! mutation trait: what changes should not be what everything is typed against.
//!
//! Everything unsupported becomes [`Surface::Unsupported`] rather than being
//! dropped, so the count reaches the user instead of the part quietly losing a
//! wall.

use std::collections::HashMap;

use step_io::generated::model as raw;

use super::assembly::{self, Rigid};
use super::model::{Curve, Edge, EdgeId, Face, Frame, Loop, Point3, Skipped, Solid, Surface};

/// Read a STEP file into a solid this crate can tessellate.
pub fn read(bytes: &[u8]) -> Result<Solid, String> {
    let (model, _report) = step_io::read(bytes).map_err(|error| format!("{error:?}"))?;
    Ok(convert(&model))
}

/// Everything in a parsed model, turned into one solid.
///
/// **Every face in the file, whichever shell or part it belongs to.** An
/// assembly is drawn as the sum of its solids rather than refused: the audit
/// found 39% of real files are assemblies, and to a tessellator the difference
/// is more faces, not different geometry. What is *not* done is applying each
/// component's placement transform, so an assembly whose parts are positioned
/// by transform rather than by coordinates will show them stacked at the
/// origin. That is recorded here as a known limit rather than hidden.
pub fn convert(model: &raw::StepModel) -> Solid {
    let mut solid = Solid::default();
    let mut edges: HashMap<u64, Edge> = HashMap::new();
    let mut unreadable: HashMap<&'static str, usize> = HashMap::new();

    // **Where each part sits, and how many copies of it there are.**
    // Without this every component is drawn about its own origin, inside every
    // other one -- which does not read as a fault, it reads as a different
    // object.
    // Every face, with where its copy sits and an edge namespace of its own.
    let mut placed: Vec<(usize, Rigid, u64)> = Vec::new();
    let faces_of = faces_of(model);
    for (copy, instance) in instances(model, &faces_of).iter().enumerate() {
        for face in faces_of.get(&instance.geometry).into_iter().flatten() {
            placed.push((*face, instance.put, (copy as u64 + 1) << 40));
        }
    }

    // **A file that files none of its faces under a solid still draws.** Faces
    // outside a closed shell are legal, and a part with no assembly structure
    // at all is the ordinary case; either way there is nothing to place, so
    // everything is taken as one component where it lies.
    if placed.is_empty() {
        placed = (0..model.advanced_face_arena.items.len())
            .map(|face| (face, Rigid::identity(), 0))
            .collect();
    }

    for (face_index, put, edge_base) in &placed {
        let (put, edge_base) = (*put, *edge_base);
        let face = &model.advanced_face_arena.items[*face_index];
        // **Not counted here.** An unsupported surface still becomes a
        // face, and the tessellator counts every face it cannot draw. Adding
        // it to the tally as well reported each of them twice — 916 faces
        // missing from a part that was only missing 462, which is a lie in
        // the same direction as hiding them would have been.
        let surface = match surface_of(model, &face.face_geometry, &put) {
            Ok(surface) => surface,
            Err(what) => Surface::Unsupported { what },
        };

        let mut outer = None;
        let mut inners = Vec::new();

        for bound in &face.bounds {
            let (loop_ref, orientation, is_outer) = match bound {
                raw::FaceBoundRef::FaceOuterBound(id) => {
                    let bound = model.face_outer_bound_arena.get(id.0);
                    (&bound.bound, bound.orientation, true)
                }
                raw::FaceBoundRef::FaceBound(id) => {
                    let bound = model.face_bound_arena.get(id.0);
                    (&bound.bound, bound.orientation, false)
                }
                _ => continue,
            };

            let Some(walked) = edge_loop(model, loop_ref, orientation, &put, edge_base, &mut edges) else {
                continue;
            };

            // **The first bound is the outer one when nothing says otherwise.**
            // Plenty of exporters write every bound as a plain `FACE_BOUND`,
            // including the outer one; treating them all as holes leaves the
            // face with no boundary at all and loses it silently.
            if is_outer || outer.is_none() {
                if let Some(previous) = outer.replace(walked) {
                    inners.push(previous);
                }
            } else {
                inners.push(walked);
            }
        }

        // A face with no boundary this could read is a real loss, and the
        // only one the tessellator will never see. Counted here or nowhere.
        let Some(outer) = outer else {
            *unreadable.entry("a face with no usable boundary").or_default() += 1;
            continue;
        };

        solid.faces.push(Face {
            surface,
            outer,
            inners,
            same_sense: face.same_sense,
        });
    }

    solid.edges = edges.into_values().collect();
    solid.edges.sort_by_key(|edge| edge.id);

    for (what, count) in unreadable {
        solid.skipped.push(Skipped { what: what.to_string(), count });
    }

    solid
}


/// One placement of one component's geometry.
///
/// **A list, not a map.** The same geometry is commonly *instanced*: a turbine
/// with two identical brackets stores the bracket once and places it twice, and
/// this file does exactly that — five mapped items pointing at one
/// representation. A map from face to transform can only hold the last of
/// those, so one bracket appears and the other silently does not.
struct Instance {
    /// Which `ADVANCED_BREP_SHAPE_REPRESENTATION` holds the geometry.
    geometry: u64,
    /// Where this copy of it sits.
    put: Rigid,
}

/// Every placed copy of every component.
///
/// The tree is built from two kinds of link, because assemblies use both:
///
/// - `REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION` between nodes, written
///   as a complex instance wearing three names at once.
/// - `MAPPED_ITEM`, which is how a node points at the geometry it draws, and
///   carries a transform of its own.
///
/// A file using only the first works with only the first read, which is how a
/// half-finished version of this looked correct on some files and left every
/// part at the origin on this one.
fn instances(model: &raw::StepModel, faces: &HashMap<u64, Vec<usize>>) -> Vec<Instance> {
    // How each node sits in its parent.
    let mut parent_of: HashMap<u64, (u64, Rigid)> = HashMap::new();
    for complex in &model.complex_unit_arena.items {
        let mut pair = None;
        let mut operator = None;
        for part in &complex.parts {
            match part {
                raw::UnitPart::RepresentationRelationship { rep_1, rep_2, .. } => {
                    pair = Some((rep_1, rep_2));
                }
                raw::UnitPart::RepresentationRelationshipWithTransformation {
                    transformation_operator,
                } => operator = Some(transformation_operator),
                _ => {}
            }
        }

        let (Some((rep_1, rep_2)), Some(operator)) = (pair, operator) else {
            continue;
        };
        // **rep_1 is the component and rep_2 the assembly.** Backwards, every
        // part is keyed on its parent and they overwrite one another —
        // fifteen placements collapsing into three.
        let (Some(child), Some(parent)) = (rep_key(rep_1), rep_key(rep_2)) else {
            continue;
        };
        if child == parent {
            continue;
        }

        let raw::TransformationRef::ItemDefinedTransformation(id) = operator else {
            continue;
        };
        let transform = model.item_defined_transformation_arena.get(id.0);
        // item_1 is the component's own origin, item_2 where it lands.
        let (Some(own), Some(into_parent)) = (
            placement_item(model, &transform.transform_item_1),
            placement_item(model, &transform.transform_item_2),
        ) else {
            continue;
        };

        parent_of.insert(child, (parent, assembly::between(&into_parent, &own)));
    }

    // Every node that holds geometry, placed where the tree puts it. A node
    // may also reach its geometry through a mapped item, which carries a
    // transform of its own on top of the node's.
    // **A node the tree names, or geometry something maps in -- never both.**
    // The same solid is often listed under a SHAPE_REPRESENTATION the assembly
    // places *and* under an ADVANCED_BREP_SHAPE_REPRESENTATION beside it.
    // Instancing each would draw every face twice, once in the right place and
    // once at the origin, which reads as a ghost of the part inside itself.
    let mut instances = Vec::new();
    for key in faces.keys() {
        if *key >= SHAPE_REPRESENTATION_BASE {
            instances.push(Instance {
                geometry: *key,
                put: assembly::to_root(*key, &parent_of),
            });
        }
    }

    for (index, node) in model.shape_representation_arena.items.iter().enumerate() {
        let here = assembly::to_root(SHAPE_REPRESENTATION_BASE + index as u64, &parent_of);

        for item in &node.items {
            let raw::RepresentationItemRef::MappedItem(mapped) = item else {
                continue;
            };
            let mapped = model.mapped_item_arena.get(mapped.0);
            let raw::RepresentationMapRef::RepresentationMap(source) = &mapped.mapping_source
            else {
                continue;
            };
            let source = model.representation_map_arena.get(source.0);
            let raw::RepresentationRef::AdvancedBrepShapeRepresentation(geometry) =
                &source.mapped_representation
            else {
                continue;
            };
            let (Some(own), Some(into_node)) = (
                placement_item(model, &source.mapping_origin),
                placement_item(model, &mapped.mapping_target),
            ) else {
                continue;
            };

            instances.push(Instance {
                geometry: geometry.0 as u64,
                put: assembly::between(&into_node, &own).then(&here),
            });
        }
    }

    instances
}

/// The faces each node holds, by the path the file links them.
///
/// **Both places a solid can live.** It may sit in its own
/// `ADVANCED_BREP_SHAPE_REPRESENTATION`, reached from the assembly through a
/// mapped item — or directly among the items of the `SHAPE_REPRESENTATION`
/// that the assembly tree already names. Reading only the first found nothing
/// on a real assembly and quietly fell back to drawing everything at the
/// origin, which is the failure this whole module exists to prevent.
fn faces_of(model: &raw::StepModel) -> HashMap<u64, Vec<usize>> {
    let mut faces: HashMap<u64, Vec<usize>> = HashMap::new();

    let mut collect = |key: u64, items: &[raw::RepresentationItemRef]| {
        for item in items {
            let raw::RepresentationItemRef::ManifoldSolidBrep(solid) = item else {
                continue;
            };
            let raw::ClosedShellRef::ClosedShell(shell) =
                &model.manifold_solid_brep_arena.get(solid.0).outer
            else {
                continue;
            };
            for face in &model.closed_shell_arena.get(shell.0).cfs_faces {
                if let raw::FaceRef::AdvancedFace(id) = face {
                    faces.entry(key).or_default().push(id.0);
                }
            }
        }
    };

    for (index, representation) in model
        .advanced_brep_shape_representation_arena
        .items
        .iter()
        .enumerate()
    {
        collect(index as u64, &representation.items);
    }
    for (index, node) in model.shape_representation_arena.items.iter().enumerate() {
        collect(SHAPE_REPRESENTATION_BASE + index as u64, &node.items);
    }

    faces
}



/// Representations of two different kinds share one numbering.
///
/// The assembly tree links `SHAPE_REPRESENTATION` nodes while the geometry sits
/// in `ADVANCED_BREP_SHAPE_REPRESENTATION`, so a single map has to hold both.
/// Offsetting one kind keeps their indices apart without a second lookup.
const SHAPE_REPRESENTATION_BASE: u64 = 1 << 32;

/// A representation of either kind, as one number.
fn rep_key(reference: &raw::RepresentationOrRepresentationReferenceRef) -> Option<u64> {
    match reference {
        raw::RepresentationOrRepresentationReferenceRef::AdvancedBrepShapeRepresentation(id) => {
            Some(id.0 as u64)
        }
        raw::RepresentationOrRepresentationReferenceRef::ShapeRepresentation(id) => {
            Some(SHAPE_REPRESENTATION_BASE + id.0 as u64)
        }
        _ => None,
    }
}

/// The index of a shape representation, if the reference names one this reads.
fn brep_representation(reference: &raw::RepresentationOrRepresentationReferenceRef) -> Option<usize> {
    match reference {
        raw::RepresentationOrRepresentationReferenceRef::AdvancedBrepShapeRepresentation(id) => {
            Some(id.0)
        }
        _ => None,
    }
}

/// A transformation's placement, as a frame.
fn placement_item(model: &raw::StepModel, item: &raw::RepresentationItemRef) -> Option<Frame> {
    match item {
        raw::RepresentationItemRef::Axis2Placement3d(id) => frame_at(model, id.0),
        _ => None,
    }
}

fn edge_loop(
    model: &raw::StepModel,
    loop_ref: &raw::LoopRef,
    orientation: bool,
    put: &Rigid,
    edge_base: u64,
    edges: &mut HashMap<u64, Edge>,
) -> Option<Loop> {
    let raw::LoopRef::EdgeLoop(id) = loop_ref else {
        return None;
    };

    let mut walked = Vec::new();
    for oriented_ref in &model.edge_loop_arena.get(id.0).edge_list {
        let Some(oriented_id) = oriented_edge_id(oriented_ref) else { continue };
        let oriented = model.oriented_edge_arena.get(oriented_id);
        let raw::EdgeRef::EdgeCurve(curve_id) = &oriented.edge_element else {
            continue;
        };

        // The arena index doubles as the shared identity: two faces reaching the
        // same `EDGE_CURVE` reach the same index, which is exactly what the
        // discretisation cache keys on.
        let id = EdgeId(edge_base + curve_id.0 as u64);
        if !edges.contains_key(&id.0) {
            if let Some(edge) = edge_of(model, curve_id, id, put) {
                edges.insert(id.0, edge);
            } else {
                continue;
            }
        }
        walked.push((id, oriented.orientation));
    }

    if walked.is_empty() {
        None
    } else {
        Some(Loop { edges: walked, bound_forward: orientation })
    }
}

fn edge_of(
    model: &raw::StepModel,
    curve_id: &raw::EdgeCurveId,
    id: EdgeId,
    put: &Rigid,
) -> Option<Edge> {
    let edge = model.edge_curve_arena.get(curve_id.0);
    let start = put.apply(vertex(model, &edge.edge_start)?);
    let end = put.apply(vertex(model, &edge.edge_end)?);
    let curve = curve_of(model, &edge.edge_geometry, start, end, put);

    Some(Edge { id, curve, start, end, same_sense: edge.same_sense })
}

fn vertex(model: &raw::StepModel, vertex: &raw::VertexRef) -> Option<Point3> {
    let raw::VertexRef::VertexPoint(id) = vertex else {
        return None;
    };
    let raw::PointRef::CartesianPoint(point) = &model.vertex_point_arena.get(id.0).vertex_geometry
    else {
        return None;
    };
    Some(cartesian(model.cartesian_point_arena.get(point.0)))
}

/// A curve, or the straight line between the vertices as the fallback.
///
/// **Never nothing.** A missing curve leaves the loop open, and an open loop
/// triangulates to nothing at all — the face vanishes with no error raised.
/// A chord is the wrong shape but the right topology, and the face survives to
/// be looked at.
fn curve_of(
    model: &raw::StepModel,
    curve: &raw::CurveRef,
    start: Point3,
    end: Point3,
    put: &Rigid,
) -> Curve {
    match curve {
        raw::CurveRef::Line(id) => {
            let line = model.line_arena.get(id.0);
            let Some(vector_at) = vector_id(&line.dir) else {
                return Curve::Polyline { points: vec![start, end] };
            };
            let vector = model.vector_arena.get(vector_at);
            Curve::Line {
                from: put.apply(
                    point_id(&line.pnt)
                        .map(|at| cartesian(model.cartesian_point_arena.get(at)))
                        .unwrap_or(start),
                ),
                direction: put
                    .direction(direction(model, &vector.orientation))
                    .scaled(vector.magnitude),
            }
        }

        raw::CurveRef::Circle(id) => {
            let circle = model.circle_arena.get(id.0);
            match placement_any(&circle.position).and_then(|at| frame_at(model, at)) {
                Some(frame) => Curve::Circle { frame: put.moved(&frame), radius: circle.radius },
                None => Curve::Polyline { points: vec![start, end] },
            }
        }

        raw::CurveRef::Ellipse(id) => {
            let ellipse = model.ellipse_arena.get(id.0);
            match placement_any(&ellipse.position).and_then(|at| frame_at(model, at)) {
                Some(frame) => Curve::Ellipse {
                    frame: put.moved(&frame),
                    major: ellipse.semi_axis_1,
                    minor: ellipse.semi_axis_2,
                },
                None => Curve::Polyline { points: vec![start, end] },
            }
        }

        // The curve type the audit found on 68% of real files, on edges of
        // faces that are otherwise entirely supported.
        raw::CurveRef::BSplineCurveWithKnots(id) => {
            let spline = model.b_spline_curve_with_knots_arena.get(id.0);
            Curve::Spline {
                degree: spline.degree.max(0) as usize,
                control: spline
                    .control_points_list
                    .iter()
                    .filter_map(|point| {
                        point_id(point)
                            .map(|at| put.apply(cartesian(model.cartesian_point_arena.get(at))))
                    })
                    .collect(),
                knots: expand_knots(&spline.knots, &spline.knot_multiplicities),
                weights: None,
            }
        }

        _ => Curve::Polyline { points: vec![start, end] },
    }
}

/// STEP stores distinct knots and how many times each repeats.
fn expand_knots(distinct: &[f64], multiplicities: &[i64]) -> Vec<f64> {
    let mut knots = Vec::new();
    for (value, times) in distinct.iter().zip(multiplicities.iter()) {
        for _ in 0..(*times).max(0) {
            knots.push(*value);
        }
    }
    knots
}

fn surface_of(
    model: &raw::StepModel,
    surface: &raw::SurfaceRef,
    put: &Rigid,
) -> Result<Surface, &'static str> {
    match surface {
        raw::SurfaceRef::Plane(id) => frame_at(model, placement3d(&model.plane_arena.get(id.0).position))
            .map(|frame| Surface::Plane { frame: put.moved(&frame) })
            .ok_or("a surface with no placement"),

        raw::SurfaceRef::CylindricalSurface(id) => {
            let cylinder = model.cylindrical_surface_arena.get(id.0);
            frame_at(model, placement3d(&cylinder.position))
                .map(|frame| Surface::Cylinder { frame: put.moved(&frame), radius: cylinder.radius })
                .ok_or("a surface with no placement")
        }

        raw::SurfaceRef::ConicalSurface(id) => {
            let cone = model.conical_surface_arena.get(id.0);
            frame_at(model, placement3d(&cone.position))
                .map(|frame| Surface::Cone {
                    frame: put.moved(&frame),
                    radius: cone.radius,
                    half_angle: cone.semi_angle,
                })
                .ok_or("a surface with no placement")
        }

        raw::SurfaceRef::SphericalSurface(id) => {
            let sphere = model.spherical_surface_arena.get(id.0);
            frame_at(model, placement3d(&sphere.position))
                .map(|frame| Surface::Sphere { frame: put.moved(&frame), radius: sphere.radius })
                .ok_or("a surface with no placement")
        }

        raw::SurfaceRef::ToroidalSurface(id) => {
            let torus = model.toroidal_surface_arena.get(id.0);
            frame_at(model, placement3d(&torus.position))
                .map(|frame| Surface::Torus {
                    frame: put.moved(&frame),
                    major: torus.major_radius,
                    minor: torus.minor_radius,
                })
                .ok_or("a surface with no placement")
        }

        // **Tessellated, not refused.** Freeform faces are a minority on a
        // machined part and the *whole shape* of a turbine blade; there is no
        // threshold that makes the second case viewable, so they are
        // evaluated like any other surface.
        raw::SurfaceRef::BSplineSurfaceWithKnots(id) => {
            let surface = model.b_spline_surface_with_knots_arena.get(id.0);
            let control = control_net(model, &surface.control_points_list, put);
            super::spline::Spline::new(
                surface.u_degree.max(0) as usize,
                surface.v_degree.max(0) as usize,
                control,
                expand_knots(&surface.u_knots, &surface.u_multiplicities),
                expand_knots(&surface.v_knots, &surface.v_multiplicities),
                None,
            )
            .map(|spline| Surface::Spline(std::sync::Arc::new(spline)))
            .ok_or("a freeform surface that could not be read")
        }

        raw::SurfaceRef::RationalBSplineSurface(id) => {
            // The rational form carries weights but *not* its own knots:
            // in a complex instance those live on the WITH_KNOTS half, and
            // step-io hands the two out separately. Without them there is
            // nothing to evaluate against, so this is reported rather than
            // guessed at with a uniform vector that would bend the surface.
            let _ = model.rational_b_spline_surface_arena.get(id.0);
            Err("a rational freeform surface")
        }

        raw::SurfaceRef::BSplineSurface(_) | raw::SurfaceRef::BezierSurface(_) => {
            Err("freeform surfaces")
        }

        _ => Err("a surface Pagify 3D does not know"),
    }
}

/// An `AXIS2_PLACEMENT_3D` as a frame.
///
/// Both directions are optional in the schema. The defaults are the ones STEP
/// itself specifies — z up, x along the first axis — rather than a guess.
fn frame_at(model: &raw::StepModel, at: usize) -> Option<Frame> {
    let placement = model.axis2_placement3d_arena.get(at);
    let origin = cartesian(model.cartesian_point_arena.get(point_id(&placement.location)?));

    let axis = placement
        .axis
        .as_ref()
        .map(|d| direction(model, d))
        .unwrap_or(Point3::new(0.0, 0.0, 1.0))
        .normalised()?;

    let stated = placement
        .ref_direction
        .as_ref()
        .map(|d| direction(model, d))
        .unwrap_or(Point3::new(1.0, 0.0, 0.0));

    // The reference direction is only *approximately* perpendicular in many
    // real files. Taking it as given leaves a frame that is not orthogonal, and
    // every point projected through it is skewed by however far out it was.
    let reference = stated
        .minus(axis.scaled(stated.dot(axis)))
        .normalised()
        .or_else(|| {
            // Parallel to the axis, so it says nothing about the first axis.
            // Any perpendicular will do, and one is always available.
            let fallback = if axis.x.abs() < 0.9 {
                Point3::new(1.0, 0.0, 0.0)
            } else {
                Point3::new(0.0, 1.0, 0.0)
            };
            fallback.minus(axis.scaled(fallback.dot(axis))).normalised()
        })?;

    Some(Frame { origin, axis, reference })
}

fn direction(model: &raw::StepModel, direction: &raw::DirectionRef) -> Point3 {
    let Some(at) = direction_id(direction) else {
        return Point3::new(0.0, 0.0, 0.0);
    };
    let ratios = &model.direction_arena.get(at).direction_ratios;
    Point3::new(
        ratios.first().copied().unwrap_or(0.0),
        ratios.get(1).copied().unwrap_or(0.0),
        ratios.get(2).copied().unwrap_or(0.0),
    )
}

fn cartesian(point: &raw::CartesianPoint) -> Point3 {
    Point3::new(
        point.coordinates.first().copied().unwrap_or(0.0),
        point.coordinates.get(1).copied().unwrap_or(0.0),
        point.coordinates.get(2).copied().unwrap_or(0.0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The smallest STEP file that still contains a face.
    ///
    /// Written out rather than taken from a real part, because a real part
    /// cannot be committed — the geometry is the confidential thing. This is a
    /// single square plane with four straight edges, which is enough to prove
    /// the whole chain from bytes to a `Solid`.
    const A_SQUARE: &str = r#"ISO-10303-21;
HEADER;
FILE_DESCRIPTION((''),'1');
FILE_NAME('square','2026-01-01T00:00:00',(''),(''),'','','');
FILE_SCHEMA(('AUTOMOTIVE_DESIGN'));
ENDSEC;
DATA;
#1=CARTESIAN_POINT('',(0.,0.,0.));
#2=CARTESIAN_POINT('',(10.,0.,0.));
#3=CARTESIAN_POINT('',(10.,10.,0.));
#4=CARTESIAN_POINT('',(0.,10.,0.));
#5=DIRECTION('',(0.,0.,1.));
#6=DIRECTION('',(1.,0.,0.));
#7=AXIS2_PLACEMENT_3D('',#1,#5,#6);
#8=PLANE('',#7);
#11=VERTEX_POINT('',#1);
#12=VERTEX_POINT('',#2);
#13=VERTEX_POINT('',#3);
#14=VERTEX_POINT('',#4);
#20=DIRECTION('',(1.,0.,0.));
#21=VECTOR('',#20,1.);
#22=LINE('',#1,#21);
#23=DIRECTION('',(0.,1.,0.));
#24=VECTOR('',#23,1.);
#25=LINE('',#2,#24);
#26=DIRECTION('',(-1.,0.,0.));
#27=VECTOR('',#26,1.);
#28=LINE('',#3,#27);
#29=DIRECTION('',(0.,-1.,0.));
#30=VECTOR('',#29,1.);
#31=LINE('',#4,#30);
#41=EDGE_CURVE('',#11,#12,#22,.T.);
#42=EDGE_CURVE('',#12,#13,#25,.T.);
#43=EDGE_CURVE('',#13,#14,#28,.T.);
#44=EDGE_CURVE('',#14,#11,#31,.T.);
#51=ORIENTED_EDGE('',*,*,#41,.T.);
#52=ORIENTED_EDGE('',*,*,#42,.T.);
#53=ORIENTED_EDGE('',*,*,#43,.T.);
#54=ORIENTED_EDGE('',*,*,#44,.T.);
#60=EDGE_LOOP('',(#51,#52,#53,#54));
#61=FACE_OUTER_BOUND('',#60,.T.);
#62=ADVANCED_FACE('',(#61),#8,.T.);
ENDSEC;
END-ISO-10303-21;
"#;

    #[test]
    fn a_square_comes_through_as_one_face_and_four_edges() {
        let solid = read(A_SQUARE.as_bytes()).expect("the file parses");

        assert_eq!(1, solid.faces.len(), "skipped: {:?}", solid.skipped);
        assert_eq!(4, solid.edges.len());
        assert!(matches!(solid.faces[0].surface, Surface::Plane { .. }));
        assert!(solid.faces[0].same_sense);
        assert_eq!(4, solid.faces[0].outer.edges.len());
        assert!(solid.faces[0].inners.is_empty());
    }

    /// The whole chain: bytes in, triangles out.
    #[test]
    fn a_square_from_a_file_tessellates() {
        let solid = read(A_SQUARE.as_bytes()).expect("the file parses");
        let mesh = super::super::tessellate::tessellate(&solid, super::super::curve::DEFAULT_SAG);

        assert_eq!(2, mesh.triangles.len(), "skipped: {:?}", mesh.skipped);
        let area: f64 = mesh
            .triangles
            .iter()
            .map(|t| t.b.minus(t.a).cross(t.c.minus(t.a)).length() / 2.0)
            .sum();
        assert!((area - 100.0).abs() < 1e-6, "covered {area}, expected 100");
    }

    /// Two faces sharing an edge get the *same* edge, not one each.
    ///
    /// This is what makes the shared-edge cache work at all: if the adapter
    /// handed out two edges for one `EDGE_CURVE`, both would be discretised
    /// separately and the mesh would crack down the join.
    #[test]
    fn an_edge_used_by_two_faces_is_one_edge() {
        let solid = read(A_SQUARE.as_bytes()).expect("the file parses");
        let mut ids: Vec<_> = solid.edges.iter().map(|edge| edge.id).collect();
        ids.sort();
        ids.dedup();

        assert_eq!(solid.edges.len(), ids.len(), "an edge was duplicated");
    }

    /// A reference direction that is not square to the axis is made square.
    ///
    /// Real files carry these; taken as given, the frame is not orthogonal and
    /// every point projected through it comes out skewed — a face slightly the
    /// wrong shape, which reads as a tessellation fault.
    #[test]
    fn a_frame_is_orthogonal_even_when_the_file_is_sloppy() {
        let sloppy = A_SQUARE.replace("#6=DIRECTION('',(1.,0.,0.));", "#6=DIRECTION('',(1.,0.,0.3));");
        let solid = read(sloppy.as_bytes()).expect("the file parses");

        let Surface::Plane { frame } = &solid.faces[0].surface else {
            panic!("expected a plane");
        };
        assert!(
            frame.reference.dot(frame.axis).abs() < 1e-9,
            "reference is not square to the axis: {:?}",
            frame.reference,
        );
        assert!((frame.reference.length() - 1.0).abs() < 1e-9, "not a unit vector");
    }

    #[test]
    fn something_that_is_not_step_is_an_error_rather_than_an_empty_solid() {
        assert!(read(b"this is not a STEP file").is_err());
    }
}

// ---- resolving references ---------------------------------------------------
//
// Every reference in this parser is an enum, because the schema allows several
// concrete types wherever one is named. Only one variant of each is geometry
// this can use; the rest are legitimately different things (a 2D placement, a
// point on a surface) and resolve to `None` rather than being forced.

fn point_id(reference: &raw::CartesianPointRef) -> Option<usize> {
    match reference {
        raw::CartesianPointRef::CartesianPoint(id) => Some(id.0),
        _ => None,
    }
}

fn direction_id(reference: &raw::DirectionRef) -> Option<usize> {
    match reference {
        raw::DirectionRef::Direction(id) => Some(id.0),
        _ => None,
    }
}

fn vector_id(reference: &raw::VectorRef) -> Option<usize> {
    match reference {
        raw::VectorRef::Vector(id) => Some(id.0),
        _ => None,
    }
}

fn oriented_edge_id(reference: &raw::OrientedEdgeRef) -> Option<usize> {
    match reference {
        raw::OrientedEdgeRef::OrientedEdge(id) => Some(id.0),
        _ => None,
    }
}

/// A surface's placement, which is always three-dimensional.
fn placement3d(reference: &raw::Axis2Placement3dRef) -> usize {
    match reference {
        raw::Axis2Placement3dRef::Axis2Placement3d(id) => id.0,
    }
}

/// A curve's placement, which the schema allows to be two-dimensional.
///
/// A 2D placement belongs to a curve in parameter space, not in the model, so
/// there is no frame to build from it.
fn placement_any(reference: &raw::Axis2PlacementRef) -> Option<usize> {
    match reference {
        raw::Axis2PlacementRef::Axis2Placement3d(id) => Some(id.0),
        raw::Axis2PlacementRef::Axis2Placement2d(_) => None,
    }
}

/// Running the whole pipeline over a real file.
///
/// **Ignored, because real CAD cannot be committed.** Supplier and customer
/// geometry *is* the confidential part, so unlike a business card there is no
/// redacted form to keep in the repository. Point it at a file:
///
/// ```text
/// PAGIFY_STEP_FILE="/path/to/part.STEP" \
///   cargo test --lib step::adapt::real -- --ignored --nocapture
/// ```
#[cfg(test)]
mod real {
    use crate::step::{audit, curve::DEFAULT_SAG, tessellate};

    #[test]
    #[ignore = "needs a real CAD file; set PAGIFY_STEP_FILE"]
    fn a_real_part_goes_all_the_way_to_triangles() {
        let path = std::env::var("PAGIFY_STEP_FILE").expect("set PAGIFY_STEP_FILE");
        let bytes = std::fs::read(&path).expect("the file can be read");
        let text = String::from_utf8_lossy(&bytes);

        let census = audit::census(&text);
        println!("--- {path}");
        println!(
            "census: {} faces, {} planes, {} cylinders, {} cones, {} freeform surfaces, {} freeform curves",
            census.faces,
            census.planes,
            census.cylinders,
            census.cones,
            census.freeform_surfaces,
            census.freeform_curves,
        );
        println!("verdict: {:?}", audit::verdict(&census));

        let started = std::time::Instant::now();
        let solid = super::read(&bytes).expect("the file parses");
        let parsed = started.elapsed();

        println!(
            "adapted: {} faces, {} edges, skipped {:?} ({:?})",
            solid.faces.len(),
            solid.edges.len(),
            solid.skipped,
            parsed,
        );

        let started = std::time::Instant::now();
        let sag = tessellate::recommended_sag(&solid);
        let mesh = tessellate::tessellate(&solid, sag);
        let tessellated = started.elapsed();

        println!(
            "mesh: {} triangles, skipped {:?} ({:?})",
            mesh.triangles.len(),
            mesh.skipped,
            tessellated,
        );
        // The parameters the screen will show, printed here so a real file
        // proves them rather than a fixture.
        if let Ok(session) = crate::step::session::open(&path, &bytes) {
            println!("summary: {}", session.summary_json());
        }

        if let Some((low, high)) = mesh.bounds() {
            println!(
                "bounds: {:.2} x {:.2} x {:.2}",
                high.x - low.x,
                high.y - low.y,
                high.z - low.z,
            );
        }

        // **The divergence theorem as an independent check.** Summing
        // a·(b×c)/6 over a closed, outward-wound mesh gives the volume it
        // encloses. It is worth more than any count: a face missing leaves a
        // hole and the sum goes wrong, a face inverted subtracts instead of
        // adding, and a cracked seam leaks -- three separate failures that
        // all show up here, in a number that can be compared against the
        // bounding box without knowing anything about the part.
        let volume: f64 = mesh
            .triangles
            .iter()
            .map(|t| t.a.dot(t.b.cross(t.c)) / 6.0)
            .sum();
        if let Some((low, high)) = mesh.bounds() {
            let box_volume =
                (high.x - low.x) * (high.y - low.y) * (high.z - low.z);
            println!("volume: {volume:.2} of a {box_volume:.2} box");
            assert!(
                volume > 0.0,
                "the solid encloses no volume, or is inside out: {volume}",
            );
            assert!(
                volume <= box_volume * 1.01,
                "encloses more than its own bounding box: {volume} of {box_volume}",
            );
            for extent in [high.x - low.x, high.y - low.y, high.z - low.z] {
                assert!(extent > 1e-6, "flat in one direction: {extent}");
            }
        }

        // Every face in the file is either drawn or accounted for. A part shown
        // with a wall missing looks like the part, so an unexplained loss is the
        // one outcome that must not be possible.
        let accounted: usize = mesh.skipped.iter().map(|s| s.count).sum();
        assert!(
            solid.faces.len() <= census.faces,
            "more faces adapted than the file contains",
        );
        assert!(
            !mesh.triangles.is_empty() || accounted > 0,
            "no triangles and nothing said about why",
        );
    }
}

/// The whole pipeline over a directory of real files.
///
/// Ignored for the same reason as [`real`]: the files cannot be committed. This
/// is the sweep that catches what invented fixtures never do — an exporter's
/// habits, a degenerate face, a loop that closes the wrong way — across every
/// real part available rather than the one that was being worked on.
#[cfg(test)]
mod sweep {
    use crate::step::{audit, curve::DEFAULT_SAG, tessellate};
    use std::path::PathBuf;

    fn step_files(root: &std::path::Path, into: &mut Vec<PathBuf>, depth: usize) {
        if depth == 0 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(root) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                step_files(&path, into, depth - 1);
            } else if path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("stp") || e.eq_ignore_ascii_case("step"))
            {
                into.push(path);
            }
        }
    }

    #[test]
    #[ignore = "needs real CAD files; set PAGIFY_STEP_DIR"]
    fn every_accepted_file_produces_a_mesh() {
        let root = std::env::var("PAGIFY_STEP_DIR").expect("set PAGIFY_STEP_DIR");
        let mut files = Vec::new();
        step_files(std::path::Path::new(&root), &mut files, 8);
        files.sort();

        let mut seen = std::collections::HashSet::new();
        let (mut accepted, mut meshed, mut refused) = (0, 0, 0);

        for path in files {
            let Ok(bytes) = std::fs::read(&path) else { continue };
            let census = audit::census(&String::from_utf8_lossy(&bytes));
            if census.faces == 0 || !seen.insert(bytes.len()) {
                continue;
            }

            let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
            if let Err(refusal) = audit::verdict(&census) {
                refused += 1;
                println!("{name:<44.44} REFUSED  {}", refusal.message());
                continue;
            }
            accepted += 1;

            let Ok(solid) = super::read(&bytes) else {
                println!("{name:<44.44} PARSE FAILED");
                continue;
            };
            let sag = tessellate::recommended_sag(&solid);
            let mesh = tessellate::tessellate(&solid, sag);
            let lost: usize = mesh.skipped.iter().map(|s| s.count).sum();
            let volume: f64 = mesh
                .triangles
                .iter()
                .map(|t| t.a.dot(t.b.cross(t.c)) / 6.0)
                .sum();

            // **Every accepted part encloses a positive volume.**
            //
            // This is the guard for what the sweep found: at a fixed 0.02 mm
            // tolerance a 1.5 m ring light asked for three million triangles,
            // hit the refinement ceiling, and came out as a truncated mesh
            // whose volume was *negative* -- an inside-out shell. The triangle
            // count alone would not have caught it, and neither would looking
            // at the small parts, which were all fine.
            assert!(
                volume > 0.0,
                "{name} encloses {volume}: the mesh is not closed, or is inverted",
            );

            if !mesh.triangles.is_empty() {
                meshed += 1;
            }
            println!(
                "{name:<44.44} {:>4} faces {:>7} tris {:>5} skipped  vol {:>12.1}",
                solid.faces.len(),
                mesh.triangles.len(),
                lost,
                volume,
            );
        }

        println!("\naccepted {accepted}, of which {meshed} produced a mesh; {refused} refused");
        assert!(accepted > 0, "no file was accepted at all");
        assert_eq!(accepted, meshed, "an accepted file produced no triangles");
    }
}

/// Rendering a real part to a picture, to be looked at.
///
/// **Because the numbers cannot say whether it looks right.** A mesh can have
/// the right triangle count, a plausible volume and correct winding and still
/// be visibly wrong — a face in the wrong place, a hole that did not come out,
/// a seam. The only test for that is a person looking at it.
///
/// ```text
/// PAGIFY_STEP_FILE="/path/part.STEP" PAGIFY_STEP_PNG="/path/out.png" \
///   cargo test --lib step::adapt::picture -- --ignored --nocapture
/// ```
#[cfg(test)]
mod picture {
    use crate::step::{camera::Camera, model::Point3, raster, tessellate};

    #[test]
    #[ignore = "needs a real CAD file; set PAGIFY_STEP_FILE and PAGIFY_STEP_PNG"]
    fn a_real_part_can_be_looked_at() {
        let path = std::env::var("PAGIFY_STEP_FILE").expect("set PAGIFY_STEP_FILE");
        let out = std::env::var("PAGIFY_STEP_PNG").expect("set PAGIFY_STEP_PNG");
        let bytes = std::fs::read(&path).expect("readable");

        let solid = super::read(&bytes).expect("parses");
        let sag = tessellate::recommended_sag(&solid);
        let mesh = tessellate::tessellate(&solid, sag);
        let (low, high) = mesh.bounds().expect("a mesh has bounds");

        // Four views, so a face that only looks right from one angle cannot
        // pass. Side by side in one picture rather than four files.
        const SIZE: u32 = 320;
        let style = raster::Style::default();
        let mut sheet = raster::Canvas::new(SIZE * 4, SIZE);
        sheet.fill(style.background);

        for (index, yaw) in [0.6_f64, 2.2, 3.8, 5.4].into_iter().enumerate() {
            let mut tile = raster::Canvas::new(SIZE, SIZE);
            let camera = Camera {
                yaw,
                ..Camera::fit(low, high, 45.0_f64.to_radians())
            };
            let started = std::time::Instant::now();
            raster::draw(&mesh, &camera, &style, &mut tile);
            println!(
                "view {index}: {} pixels covered in {:?}",
                tile.covered(),
                started.elapsed(),
            );
            assert!(tile.covered() > 500, "view {index} drew almost nothing");

            for y in 0..SIZE {
                for x in 0..SIZE {
                    let from = ((y * SIZE + x) * 4) as usize;
                    let to = ((y * SIZE * 4 + index as u32 * SIZE + x) * 4) as usize;
                    sheet.pixels[to..to + 4].copy_from_slice(&tile.pixels[from..from + 4]);
                }
            }
        }

        image::save_buffer(
            &out,
            &sheet.pixels,
            SIZE * 4,
            SIZE,
            image::ExtendedColorType::Rgba8,
        )
        .expect("the picture is written");
        println!("wrote {out}");

        // **Where the drawn pixels sit, not whether the centre is filled.**
        // The first version asked for something at the middle of each tile
        // and failed on a lamp bezel -- a ring, whose middle is a hole and
        // correctly empty. What is actually being checked is that the part is
        // in frame rather than off in a corner, and the centre of what was
        // drawn answers that without assuming the part is solid.
        for index in 0..4u32 {
            let (mut sum_x, mut sum_y, mut seen) = (0.0f64, 0.0f64, 0usize);
            for y in 0..SIZE {
                for x in 0..SIZE {
                    if sheet.colour_at(index * SIZE + x, y) != style.background {
                        sum_x += x as f64;
                        sum_y += y as f64;
                        seen += 1;
                    }
                }
            }
            assert!(seen > 500, "view {index} drew almost nothing");
            let middle = SIZE as f64 / 2.0;
            let off_x = (sum_x / seen as f64 - middle).abs();
            let off_y = (sum_y / seen as f64 - middle).abs();
            assert!(
                off_x < middle * 0.5 && off_y < middle * 0.5,
                "view {index} is off centre by ({off_x:.0}, {off_y:.0}) pixels",
            );
        }

        let _ = Point3::new(0.0, 0.0, 0.0);
    }
}

/// A grid of control points, as rows of resolved coordinates.
fn control_net(
    model: &raw::StepModel,
    rows: &[Vec<raw::CartesianPointRef>],
    put: &Rigid,
) -> Vec<Vec<Point3>> {
    rows.iter()
        .map(|row| {
            row.iter()
                .filter_map(|point| {
                    point_id(point)
                        .map(|at| put.apply(cartesian(model.cartesian_point_arena.get(at))))
                })
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod where_the_triangles_go {
    use crate::step::{model::Surface, tessellate};

    #[test]
    #[ignore = "diagnostic"]
    fn per_face_counts() {
        let path = std::env::var("PAGIFY_STEP_FILE").expect("set PAGIFY_STEP_FILE");
        let bytes = std::fs::read(&path).expect("readable");
        let solid = super::read(&bytes).expect("parses");
        let sag = tessellate::recommended_sag(&solid);
        println!("sag {sag:.4}");
        let cache = crate::step::curve::EdgeCache::build(&solid.edges, sag);

        let mut counts: Vec<(usize, &'static str, (f64, f64))> = Vec::new();
        for face in &solid.faces {
            let kind = match &face.surface {
                Surface::Plane { .. } => "plane",
                Surface::Cylinder { .. } => "cylinder",
                Surface::Cone { .. } => "cone",
                Surface::Sphere { .. } => "sphere",
                Surface::Torus { .. } => "torus",
                Surface::Spline(_) => "spline",
                Surface::Unsupported { .. } => "unsupported",
            };
            let limits = tessellate::parameter_limits(&face.surface, sag);
            let count = tessellate::face_triangles(face, &cache, sag, 4096)
                .map(|t| t.len())
                .unwrap_or(0);
            counts.push((count, kind, limits));
        }
        counts.sort_by(|a, b| b.0.cmp(&a.0));

        let total: usize = counts.iter().map(|c| c.0).sum();
        println!("total {total}");
        for (count, kind, limits) in counts.iter().take(6) {
            println!("  {count:>7} {kind:<10} limits u={:.5} v={:.5}", limits.0, limits.1);
        }
    }
}

