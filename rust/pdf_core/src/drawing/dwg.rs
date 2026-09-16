//! DWG, read natively — no AutoCAD, no converter, no extra files.
//!
//! **This is the only file that knows `acadrust` exists**, the same wall
//! [`super::dxf`] puts around the DXF format and [`crate::step::adapt`] puts
//! around `step-io`. What comes out is the same [`Drawing`] the DXF reader
//! produces, so nothing downstream can tell which door a drawing came in
//! through — and if the reader is ever replaced, the cost is this file.
//!
//! DWG is closed, versioned and undocumented, so every package that opens one
//! either licenses Autodesk's or the ODA's library, or uses a clean-room
//! reader. `acadrust` is the third, under MPL-2.0 — file-level copyleft, which
//! links into a proprietary binary with the obligation staying on its own
//! files. LibreDWG, the obvious alternative, is GPL-3.0 and would take the
//! whole application with it.
//!
//! **Ported from `cad_io::dwg`,** which was measured against real drawings
//! before it was trusted, and carries the answers to the things that are
//! invisible until they are wrong.

use std::collections::HashMap;

use super::model::{aci_colour, metres_per_unit, Drawing, Entity, Point, Shape, Units, Vertex};

/// How deep a block may reference another before this stops following.
const DEEPEST: usize = 8;

/// The most shapes one file may expand to.
const MOST: usize = 400_000;

/// Read a `.dwg` into a drawing.
pub fn read(path: &std::path::Path) -> Result<Drawing, String> {
    let mut reader = acadrust::DwgReader::from_file(path).map_err(|e| format!("{e}"))?;
    let source = reader.read().map_err(|e| format!("{e}"))?;
    Ok(convert(&source))
}

/// The mapping itself, apart from the file, so it can be exercised against a
/// document built in memory rather than only against drawings on a disk.
pub fn convert(source: &acadrust::CadDocument) -> Drawing {
    let mut drawing = Drawing::default();

    // **Taken from the file, not assumed.** A drawing in metres read as
    // millimetres puts every fitting at a thousandth of its position, and the
    // declaration was in the file the whole time.
    if let Some(metres) = metres_per_unit(source.header.insertion_units as i32) {
        drawing.units = Units { metres_per_unit: metres, declared: true };
    }

    // Layers before the entities that name them.
    for layer in source.layers.iter() {
        let name = layer.name.trim();
        if name.is_empty() {
            continue;
        }
        let id = drawing.layer_for(name);
        let entry = &mut drawing.layers[id as usize];
        entry.colour = aci_colour(layer.color.index().unwrap_or(7) as i32);
        entry.visible = !layer.is_off() && !layer.is_frozen();
    }

    // **Model space only.** `entities()` yields every entity in the file — its
    // own documentation says it hides only the BLOCK and ENDBLK markers — so
    // the *contents* of every block definition arrive as loose geometry at
    // their definition coordinates. On a real file that is the difference
    // between a drawing and a mess: three copies of a plan strewn around it
    // and every symbol definition drawn at the origin. Each entity carries the
    // handle of the block record that owns it, so the filter is exact rather
    // than a guess at coordinates.
    //
    // **Both spaces, not only the model.** A drawing has a model — the
    // building, at full size — and layouts, the sheets it is printed on, which
    // carry the title block, the revision table and the view callouts. They
    // are different coordinate systems: in this reader's test drawing the
    // model runs 1,440 units wide and the sheet is 34 by 22 inches, thirty
    // times smaller and somewhere else entirely.
    //
    // Reading only the model was losing every word on the sheet — the project
    // name, the client, the date, the sheet number, thirty-one of them in one
    // file — and reporting nothing missing, because a drawing without its
    // title block still looks like a drawing. Both are read, which is what the
    // DXF side has always done and what AutoCAD's own DXF conversion of this
    // file produces — the two now agree shape for shape.
    //
    // It does mean a sheet drawn beside the model it describes rather than
    // around it. Showing one space at a time is the honest answer and wants a
    // way to choose between them, which is a thing to build, not a line to
    // change here.
    let is_space = |name: &str| {
        let bare = name.trim().trim_start_matches(['*', '$']).to_ascii_uppercase();
        bare == "MODEL_SPACE" || bare.starts_with("PAPER_SPACE")
    };
    let spaces: Vec<_> = source
        .block_records
        .iter()
        .filter(|b| is_space(&b.name))
        .map(|b| b.handle)
        .collect();

    // Block definitions, so the references in model space have something to
    // point at. Keyed by name because that is what an INSERT names.
    let mut blocks: HashMap<String, Vec<Part>> = HashMap::new();
    let mut empty_blocks = 0usize;
    for record in source.block_records.iter() {
        // **Only model and paper space are skipped, not everything starred.**
        // Anonymous blocks — `*X##` for a dynamic block's own version of
        // itself, `*D##` for a dimension's drawn picture, `*U##` for a group
        // — hold real geometry that nested references reach for. Skipping
        // every name beginning with a star throws all of that away: on a real
        // architectural plan it left 1,457 shapes of the 27,261 the same
        // drawing gives as DXF, and reported nothing missing, because from
        // the outside a drawing missing its symbols is just a simpler drawing.
        let name = record.name.trim();
        let space = name.trim_start_matches(['*', '$']);
        if name.is_empty()
            || space.eq_ignore_ascii_case("Model_Space")
            || space.eq_ignore_ascii_case("Paper_Space")
            || space.to_ascii_uppercase().starts_with("PAPER_SPACE")
        {
            continue;
        }
        let mut held = Vec::new();
        for entity in source.entities() {
            if common_of(entity).owner_handle != record.handle {
                continue;
            }
            held.extend(parts_of(entity, &mut drawing));
        }
        if held.is_empty() {
            // **A block this reader could name but not read.** The DWG
            // decoder lists every block record and then yields entities for
            // only some of them; on a real architectural plan it named 43 and
            // produced contents for 8, so the drawing arrived with a
            // twentieth of its geometry and nothing at all was reported. The
            // same file as DXF read 168 blocks and 27,000 shapes.
            //
            // Counted, because a drawing that quietly lost most of itself
            // still looks like a drawing — which is the exact failure this
            // whole feature was told to avoid.
            empty_blocks += 1;
            continue;
        }
        blocks.insert(name.to_ascii_uppercase(), held);
    }
    if empty_blocks > 0 {
        for _ in 0..empty_blocks {
            drawing.note("symbols this reader could not open");
        }
    }

    for entity in source.entities() {
        // A drawing with no space record at all is not one this can filter,
        // and an empty document reported as a successful open is the worst
        // outcome available — so the filter only applies where there is
        // something to filter by.
        if !spaces.is_empty() && !spaces.contains(&common_of(entity).owner_handle) {
            continue;
        }
        for part in parts_of(entity, &mut drawing) {
            match part {
                Part::Shape(shape) => drawing.entities.push(shape),
                Part::Inside { name, put } => expand(&name, put, &mut drawing, &blocks, 0),
            }
        }
    }


    drawing
}

/// One thing inside a block, or in model space.
enum Part {
    Shape(Entity),
    Inside { name: String, put: Affine },
}

/// The fields every entity carries: which layer it is on, and which block
/// record owns it.
///
/// **Every variant, with no catch-all, on purpose.** This used to answer
/// `None` for anything it had not been taught, and that answer is not "count
/// it as unsupported" — both filters here judge an entity by its owner, so one
/// with no owner to give is dropped *before* the code that counts what it
/// cannot draw ever sees it. The entity then does not appear on the sheet and
/// does not appear in what the sheet says is missing, which is the one outcome
/// this whole reader exists to avoid.
///
/// It cost the same bug three times — first every block reference, then every
/// dimension, then fifty pieces of text — because each time the fix was to add
/// the one variant that had gone missing rather than to close the hole. Being
/// exhaustive closes it: a variant nobody has thought about is now a compile
/// error here instead of silence on the drawing, and deciding what to *do*
/// with an entity stays where it belongs, in `part_of`, which counts.
fn common_of(entity: &acadrust::EntityType) -> &acadrust::entities::EntityCommon {
    use acadrust::EntityType as E;
    match entity {
        E::Point(x) => &x.common,
        E::Line(x) => &x.common,
        E::Circle(x) => &x.common,
        E::Arc(x) => &x.common,
        E::Ellipse(x) => &x.common,
        E::Polyline(x) => &x.common,
        E::Polyline2D(x) => &x.common,
        E::Polyline3D(x) => &x.common,
        E::LwPolyline(x) => &x.common,
        E::Text(x) => &x.common,
        E::MText(x) => &x.common,
        E::Spline(x) => &x.common,
        E::Helix(x) => &x.common,
        // A dimension keeps its own picture in a block, and reaches its common
        // fields through the base every dimension kind shares.
        E::Dimension(x) => &x.base().common,
        E::Hatch(x) => &x.common,
        E::Solid(x) => &x.common,
        E::Face3D(x) => &x.common,
        E::Insert(x) => &x.common,
        E::Block(x) => &x.common,
        E::BlockEnd(x) => &x.common,
        E::Ray(x) => &x.common,
        E::XLine(x) => &x.common,
        E::Viewport(x) => &x.common,
        E::AttributeDefinition(x) => &x.common,
        E::AttributeEntity(x) => &x.common,
        E::Leader(x) => &x.common,
        E::MultiLeader(x) => &x.common,
        E::MLine(x) => &x.common,
        E::Mesh(x) => &x.common,
        E::RasterImage(x) => &x.common,
        E::Solid3D(x) => &x.common,
        E::Region(x) => &x.common,
        E::Body(x) => &x.common,
        E::Surface(x) => &x.common,
        E::Table(x) => &x.common,
        E::Tolerance(x) => &x.common,
        E::PolyfaceMesh(x) => &x.common,
        E::Wipeout(x) => &x.common,
        E::Shape(x) => &x.common,
        E::Underlay(x) => &x.common,
        E::Seqend(x) => &x.common,
        E::Ole2Frame(x) => &x.common,
        E::PolygonMesh(x) => &x.common,
        E::Unknown(x) => &x.common,
    }
}

/// Everything one entity puts on the sheet.
///
/// **A block reference brings its own text with it.** The words filled into a
/// block — a room number, a door mark, a fixture tag — are `ATTRIB` entities,
/// and this crate does not hand them over loose: they are held *inside* the
/// `Insert` that owns them. So no filter over the file's entities could ever
/// have found them, whatever it filtered by, and every one of them was missing
/// from the DWG side of this app while the DXF side drew them.
///
/// Which is why this returns a list rather than one part. An entity is not
/// always one thing.
fn parts_of(entity: &acadrust::EntityType, drawing: &mut Drawing) -> Vec<Part> {
    let mut out = Vec::new();

    // A hatch is a recipe, and running it gives however many strokes it gives.
    if let acadrust::EntityType::Hatch(hatch) = entity {
        let named = hatch.common.layer.trim();
        let layer = drawing.layer_for(if named.is_empty() { "0" } else { named });
        return match super::hatch::shape_of(&hatch_of(hatch), layer) {
            Some(made) => vec![Part::Shape(made)],
            // A hatch with no boundary this reader could make sense of. Counted
            // rather than passed over: a section drawing missing its materials
            // still looks like a section drawing.
            None => {
                drawing.note("hatching");
                Vec::new()
            }
        };
    }

    if let acadrust::EntityType::Insert(insert) = entity {
        // **Already in world coordinates**, so the insert's own transform must
        // not be applied to them. An attribute is placed where it was dragged
        // to, not where the block's definition would put it — applying the
        // transform a second time is how labels end up somewhere else on the
        // sheet, at a plausible-looking angle.
        let fallback = insert.common.layer.trim();
        for attribute in &insert.attributes {
            let named = attribute.common.layer.trim();
            let named = if named.is_empty() { fallback } else { named };
            let layer = drawing.layer_for(if named.is_empty() { "0" } else { named });
            if let Some(shape) = attribute_text(attribute) {
                out.push(Part::Shape(Entity { layer, shape }));
            }
        }
    }

    out.extend(part_of(entity, drawing));
    out
}

/// This crate's hatch, as the one [`super::hatch`] knows how to run.
///
/// A translation and nothing more: the pattern lines are already in radians
/// and already at their final scale, and the boundary edges only need opening
/// out into points. Everything that could be got subtly wrong about a hatch
/// lives in the module this hands to, where it is tested.
fn hatch_of(hatch: &acadrust::entities::Hatch) -> super::hatch::Hatch {
    use acadrust::entities::hatch::BoundaryEdge;

    let mut loops = Vec::new();
    for path in &hatch.paths {
        let mut ring: Vec<Point> = Vec::new();
        let edges = path.edges.len();
        for (index, edge) in path.edges.iter().enumerate() {
            match edge {
                BoundaryEdge::Polyline(p) => {
                    let vertices: Vec<Vertex> = p
                        .vertices
                        .iter()
                        .map(|v| Vertex { at: Point::new(v.x, v.y), bulge: v.z })
                        .collect();
                    ring.extend(super::hatch::flattened(&vertices, true));
                }
                // Only the start of each edge: the next one begins where this
                // ended, and the loop closes. The last edge gives both, or the
                // ring would stop one corner short of where it began.
                BoundaryEdge::Line(l) => {
                    ring.push(Point::new(l.start.x, l.start.y));
                    if index + 1 == edges {
                        ring.push(Point::new(l.end.x, l.end.y));
                    }
                }
                BoundaryEdge::CircularArc(a) => super::hatch::arc_points(
                    &mut ring,
                    Point::new(a.center.x, a.center.y),
                    a.radius,
                    1.0,
                    0.0,
                    a.start_angle,
                    a.end_angle,
                    a.counter_clockwise,
                ),
                BoundaryEdge::EllipticArc(a) => {
                    let (major_x, major_y) = (a.major_axis_endpoint.x, a.major_axis_endpoint.y);
                    let radius = major_x.hypot(major_y);
                    if radius < 1e-12 {
                        continue;
                    }
                    super::hatch::arc_points(
                        &mut ring,
                        Point::new(a.center.x, a.center.y),
                        radius,
                        a.minor_axis_ratio,
                        major_y.atan2(major_x),
                        a.start_angle,
                        a.end_angle,
                        a.counter_clockwise,
                    );
                }
                // The fit points are on the curve; the control points are not,
                // and joining those would pull the boundary inside the region
                // it is supposed to bound, all the way round.
                BoundaryEdge::Spline(s) => {
                    ring.extend(s.fit_points.iter().map(|p| Point::new(p.x, p.y)));
                }
            }
        }
        if ring.len() >= 3 {
            loops.push(ring);
        }
    }

    super::hatch::Hatch {
        loops,
        solid: hatch.is_solid,
        lines: hatch
            .pattern
            .lines
            .iter()
            .map(|line| super::hatch::PatternLine {
                angle: line.angle,
                base: Point::new(line.base_point.x, line.base_point.y),
                offset: Point::new(line.offset.x, line.offset.y),
                dashes: line.dash_lengths.clone(),
            })
            .collect(),
    }
}

/// The words in a block attribute, where they sit on the sheet.
///
/// Shared by the two ways one can arrive: held inside its `Insert`, which is
/// how this crate normally gives them, and loose in the entity list, which
/// some files still do.
fn attribute_text(attribute: &acadrust::entities::AttributeEntity) -> Option<Shape> {
    let content = super::dxf::readable(&attribute.value);
    if content.is_empty() {
        return None;
    }
    // The second alignment point is where justified text really sits; the
    // insertion point is left at the origin of its own box.
    let point = attribute.alignment_point;
    let at = if point.x != 0.0 || point.y != 0.0 { point } else { attribute.insertion_point };
    Some(Shape::Text {
        at: Point::new(at.x, at.y),
        height: if attribute.height > 0.0 { attribute.height } else { 2.5 },
        rotation: attribute.rotation,
        content,
    })
}

/// One entity, as something to draw or a reference to follow.
fn part_of(entity: &acadrust::EntityType, drawing: &mut Drawing) -> Option<Part> {
    use acadrust::EntityType as E;

    let named = common_of(entity).layer.trim();
    // An entity with no layer named belongs to layer 0, which every drawing has.
    let layer = drawing.layer_for(if named.is_empty() { "0" } else { named });

    let shape = match entity {
        E::Line(l) => Shape::Line {
            a: Point::new(l.start.x, l.start.y),
            b: Point::new(l.end.x, l.end.y),
        },

        E::Circle(c) => Shape::Arc {
            centre: Point::new(c.center.x, c.center.y),
            radius: c.radius,
            start: 0.0,
            sweep: std::f64::consts::TAU,
        },

        E::Arc(a) => {
            // **DWG stores these in radians, where DXF uses degrees** — the
            // single easiest thing to get wrong here, and it produces arcs
            // that look plausible and are not.
            let sweep = (a.end_angle - a.start_angle).rem_euclid(std::f64::consts::TAU);
            Shape::Arc {
                centre: Point::new(a.center.x, a.center.y),
                radius: a.radius,
                start: a.start_angle.rem_euclid(std::f64::consts::TAU),
                // A sweep of exactly zero is a whole circle, not an empty arc.
                sweep: if sweep < 1e-9 { std::f64::consts::TAU } else { sweep },
            }
        }

        // The major axis is a vector from the centre and the minor is a ratio
        // of it, which is exactly how this crate's ellipse is shaped — so this
        // is a copy rather than a conversion.
        E::Ellipse(e) => Shape::Ellipse {
            centre: Point::new(e.center.x, e.center.y),
            major: Point::new(e.major_axis.x, e.major_axis.y),
            ratio: e.minor_axis_ratio,
            start: 0.0,
            sweep: std::f64::consts::TAU,
        },

        E::LwPolyline(p) => Shape::Polyline {
            vertices: p
                .vertices
                .iter()
                .map(|v| Vertex { at: Point::new(v.location.x, v.location.y), bulge: v.bulge })
                .collect(),
            closed: p.is_closed,
        },

        E::Polyline2D(p) => Shape::Polyline {
            vertices: p
                .vertices
                .iter()
                .map(|v| Vertex { at: Point::new(v.location.x, v.location.y), bulge: v.bulge })
                .collect(),
            closed: p.is_closed(),
        },

        // **A dimension keeps its drawn picture in its own block** — the
        // extension lines, the arrows and, most of all, the measured figure.
        // The DXF side already expands these; not doing it here is where forty
        // of this drawing's sixty-seven pieces of text were going. Its
        // contents are in world coordinates, so it goes in where it lies.
        E::Dimension(d) => {
            let name = d.base().block_name.trim();
            if name.is_empty() {
                drawing.note("dimensions");
                return None;
            }
            return Some(Part::Inside {
                name: name.to_ascii_uppercase(),
                put: Affine::identity(),
            });
        }

        E::Insert(insert) => {
            let (x_scale, y_scale) = (insert.x_scale(), insert.y_scale());
            let (sin, cos) = insert.rotation.sin_cos();
            return Some(Part::Inside {
                name: insert.block_name.trim().to_ascii_uppercase(),
                put: Affine {
                    a: x_scale * cos,
                    b: x_scale * sin,
                    c: -y_scale * sin,
                    d: y_scale * cos,
                    e: insert.insert_point.x,
                    f: insert.insert_point.y,
                },
            });
        }

        // Counted rather than passed over, and named, because "134 not shown"
        // and "134 pieces of text not shown" are different things to be told.
        E::Point(p) => Shape::Marker { at: Point::new(p.location.x, p.location.y) },

        E::Text(t) => {
            let content = super::dxf::readable(&t.value);
            if content.is_empty() {
                return None;
            }
            // The second alignment point is where justified text really sits;
            // the insertion point is left at the origin of its own box.
            let at = t.alignment_point.filter(|p| p.x != 0.0 || p.y != 0.0).unwrap_or(t.insertion_point);
            Shape::Text {
                at: Point::new(at.x, at.y),
                height: if t.height > 0.0 { t.height } else { 2.5 },
                // Radians here, where DXF writes degrees — the same trap the
                // arcs set, in the same file format.
                rotation: t.rotation,
                content,
            }
        }

        E::MText(t) => {
            let content = super::dxf::readable(&t.value);
            if content.is_empty() {
                return None;
            }
            Shape::Text {
                at: Point::new(t.insertion_point.x, t.insertion_point.y),
                height: if t.height > 0.0 { t.height } else { 2.5 },
                rotation: t.rotation,
                content,
            }
        }
        // **The filled-in value of a block's attribute, and the reason a room
        // number shows on a plan.** A block defines a slot (`ATTDEF`) and each
        // placement of that block carries the words somebody typed into it
        // (`ATTRIB`). The DXF side has always drawn these; the DWG side never
        // saw one, because `common_of` could not answer for them and the model
        // -space filter dropped them first. Thirty-one of this drawing's
        // labels were going that way, unreported.
        E::AttributeEntity(a) => attribute_text(a)?,

        // **The slot, not the value.** An `ATTDEF` is a block's prompt — "ROOM
        // NAME" — and AutoCAD draws it only while the block is being defined,
        // never where the block is placed. Drawing it would write the word
        // "ROOM NAME" across every room on the plan. The DXF side skips these
        // for the same reason, so both formats show the same sheet.
        E::AttributeDefinition(_) => return None,

        // **Punctuation, not content.** A `SEQEND` closes a run of attributes,
        // `BLOCK`/`ENDBLK` bracket a definition, a `VIEWPORT` is a window on a
        // layout sheet. None of them is anything to draw, and now that nothing
        // is dropped before it is looked at, counting them would put a number
        // in front of somebody that stands for nothing missing at all.
        E::Seqend(_) | E::Block(_) | E::BlockEnd(_) | E::Viewport(_) => return None,

        E::Hatch(_) => {
            drawing.note("hatching");
            return None;
        }
        E::Spline(_) => {
            drawing.note("curves this cannot draw yet");
            return None;
        }
        // Named, as the DXF side names them. "84 shapes this cannot draw yet"
        // says a number and nothing about what to do next; "84 attribute
        // definitions" says which one thing to write.
        other => {
            drawing.note(&format!("{} entities this cannot draw yet", kind_of(other)));
            return None;
        }
    };

    Some(Part::Shape(Entity { layer, shape }))
}

/// Where a block's contents land. See `dxf::Affine` for why this is a matrix.
#[derive(Clone, Copy)]
struct Affine {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    e: f64,
    f: f64,
}

impl Affine {
    const fn identity() -> Self {
        Self { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 }
    }

    fn point(&self, p: Point) -> Point {
        Point::new(self.a * p.x + self.c * p.y + self.e, self.b * p.x + self.d * p.y + self.f)
    }

    fn direction(&self, p: Point) -> Point {
        Point::new(self.a * p.x + self.c * p.y, self.b * p.x + self.d * p.y)
    }

    fn then(&self, outer: &Self) -> Self {
        Self {
            a: outer.a * self.a + outer.c * self.b,
            b: outer.b * self.a + outer.d * self.b,
            c: outer.a * self.c + outer.c * self.d,
            d: outer.b * self.c + outer.d * self.d,
            e: outer.a * self.e + outer.c * self.f + outer.e,
            f: outer.b * self.e + outer.d * self.f + outer.f,
        }
    }

    fn determinant(&self) -> f64 {
        self.a * self.d - self.b * self.c
    }

    fn reverses(&self) -> bool {
        self.determinant() < 0.0
    }

    fn scale(&self) -> f64 {
        self.determinant().abs().sqrt()
    }

    fn turn(&self) -> f64 {
        self.b.atan2(self.a)
    }
}

fn expand(
    name: &str,
    put: Affine,
    drawing: &mut Drawing,
    blocks: &HashMap<String, Vec<Part>>,
    depth: usize,
) {
    if depth >= DEEPEST {
        drawing.note("blocks nested deeper than this follows");
        return;
    }
    let Some(parts) = blocks.get(name) else {
        drawing.note("references to blocks the file does not define");
        return;
    };

    for part in parts {
        if drawing.entities.len() >= MOST {
            drawing.note("shapes beyond what one drawing may hold");
            return;
        }
        match part {
            Part::Shape(entity) => drawing
                .entities
                .push(Entity { layer: entity.layer, shape: moved(&entity.shape, &put) }),
            Part::Inside { name, put: inner } => {
                expand(name, inner.then(&put), drawing, blocks, depth + 1)
            }
        }
    }
}

fn moved(shape: &Shape, put: &Affine) -> Shape {
    match shape {
        Shape::Line { a, b } => Shape::Line { a: put.point(*a), b: put.point(*b) },

        Shape::Marker { at } => Shape::Marker { at: put.point(*at) },

        Shape::Arc { centre, radius, start, sweep } => {
            let start =
                if put.reverses() { put.turn() - start - sweep } else { put.turn() + start };
            Shape::Arc {
                centre: put.point(*centre),
                radius: radius * put.scale(),
                start: start.rem_euclid(std::f64::consts::TAU),
                sweep: *sweep,
            }
        }

        Shape::Ellipse { centre, major, ratio, start, sweep } => Shape::Ellipse {
            centre: put.point(*centre),
            major: put.direction(*major),
            ratio: *ratio,
            start: *start,
            sweep: *sweep,
        },

        // Text turns and grows with the block it is in, but is never
        // mirrored into unreadability: a plan with a block placed the other
        // way round still has its labels the right way round on paper, which
        // is what CAD itself does with them.
        Shape::Text { at, height, rotation, content } => Shape::Text {
            at: put.point(*at),
            height: height * put.scale(),
            rotation: rotation + put.turn(),
            content: content.clone(),
        },

        Shape::Polyline { vertices, closed } => Shape::Polyline {
            vertices: vertices
                .iter()
                .map(|v| Vertex {
                    at: put.point(v.at),
                    bulge: if put.reverses() { -v.bulge } else { v.bulge },
                })
                .collect(),
            closed: *closed,
        },
        // Every loop moves with the block that holds it. A fill has no
        // bulges to re-sign and no direction of its own: even-odd asks how
        // many loops enclose a point, which a mirror does not change.
        Shape::Hatch { loops, lines } => Shape::Hatch {
            loops: loops.iter().map(|ring| ring.iter().map(|at| put.point(*at)).collect()).collect(),
            lines: lines.clone(),
        },
        Shape::Fill { loops } => Shape::Fill {
            loops: loops.iter().map(|ring| ring.iter().map(|at| put.point(*at)).collect()).collect(),
        },
    }
}

/// What an entity is called, for the line that says it was not drawn.
fn kind_of(entity: &acadrust::EntityType) -> &'static str {
    use acadrust::EntityType as E;
    match entity {
        E::Point(_) => "point marker",
        E::Polyline(_) | E::Polyline3D(_) => "3D polyline",
        E::Spline(_) => "spline",
        E::Helix(_) => "helix",
        E::Dimension(_) => "dimension",
        E::Solid(_) => "solid fill",
        E::Face3D(_) => "3D face",
        // The same words the DXF side uses, so one drawing read both ways
        // reports the same losses in the same language.
        E::Leader(_) | E::MultiLeader(_) => "leader",
        E::Table(_) => "table",
        E::MLine(_) => "multi-line",
        E::Wipeout(_) => "wipeout",
        E::RasterImage(_) => "image",
        _ => "other",
    }
}
