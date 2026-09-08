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
    let model_space = source
        .block_records
        .iter()
        .find(|b| b.name.eq_ignore_ascii_case("*Model_Space"))
        .map(|b| b.handle);

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
            if common_of(entity).map(|c| c.owner_handle) != Some(record.handle) {
                continue;
            }
            match part_of(entity, &mut drawing) {
                Some(part) => held.push(part),
                None => {}
            }
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
        if let Some(owner) = model_space {
            // A drawing with no model-space record is not one this can filter,
            // and an empty document reported as a successful open is the worst
            // outcome available — so the filter only applies where there is
            // something to filter by.
            match common_of(entity) {
                Some(common) if common.owner_handle == owner => {}
                _ => continue,
            }
        }
        match part_of(entity, &mut drawing) {
            Some(Part::Shape(shape)) => drawing.entities.push(shape),
            Some(Part::Inside { name, put }) => {
                expand(&name, put, &mut drawing, &blocks, 0);
            }
            None => {}
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
/// **`Insert` must be in this list.** The model-space filter judges by owner,
/// and an entity this cannot answer for is treated as not-model-space — so
/// leaving block references out drops every one of them before anything looks
/// at it, and the drawing arrives with all its symbols missing.
fn common_of(entity: &acadrust::EntityType) -> Option<&acadrust::entities::EntityCommon> {
    use acadrust::EntityType as E;
    Some(match entity {
        E::Line(x) => &x.common,
        E::Circle(x) => &x.common,
        E::Arc(x) => &x.common,
        E::Ellipse(x) => &x.common,
        E::LwPolyline(x) => &x.common,
        E::Polyline2D(x) => &x.common,
        E::Point(x) => &x.common,
        E::Text(x) => &x.common,
        E::MText(x) => &x.common,
        E::Hatch(x) => &x.common,
        E::Spline(x) => &x.common,
        E::Insert(x) => &x.common,
        _ => return None,
    })
}

/// One entity, as something to draw or a reference to follow.
fn part_of(entity: &acadrust::EntityType, drawing: &mut Drawing) -> Option<Part> {
    use acadrust::EntityType as E;

    let layer = drawing.layer_for(common_of(entity).map(|c| c.layer.trim()).unwrap_or("0"));

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
        E::Hatch(_) => {
            drawing.note("hatching");
            return None;
        }
        E::Spline(_) => {
            drawing.note("curves this cannot draw yet");
            return None;
        }
        _ => {
            drawing.note("shapes this cannot draw yet");
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
    }
}
