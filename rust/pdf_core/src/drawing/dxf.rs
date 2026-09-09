//! DXF, read.
//!
//! A DXF file is nothing but alternating lines: a group code, then its value.
//! Codes are grouped into sections — HEADER, TABLES, BLOCKS, ENTITIES — and an
//! entity is a run of codes between one `0` and the next.
//!
//! **Ported from `cad_io::dxf`, not written afresh.** That reader has been
//! through real supplier drawings and carries the answers to things that are
//! invisible until they are wrong: which entities are in object coordinates and
//! which are in world ones, that a −Z extrusion is an exact mirror, that
//! mirroring a polyline must negate every bulge or its curves bow the wrong
//! way. The writing half is left behind — this app never writes a DXF — and the
//! target is this crate's own small [`Drawing`] rather than a CAD kernel.

use std::collections::HashMap;

use super::model::{aci_colour, metres_per_unit, Drawing, Entity, Point, Shape, Units, Vertex};

/// How deep a block may reference another before this stops following.
///
/// Blocks nest — a chair inside an office inside a floor — and a file whose
/// blocks reference each other in a circle would otherwise never finish.
const DEEPEST: usize = 8;

/// The most shapes one file may expand to.
///
/// A block placed a thousand times, each holding a thousand shapes, is a
/// million shapes from a small file. The bound is what stops a drawing that
/// looks harmless from exhausting the phone before anything reaches the screen.
const MOST: usize = 400_000;

/// Read a DXF into a drawing.
///
/// Lenient by design: an entity this does not understand costs that entity and
/// is counted, never the file. Only a stream that is not code/value pairs at
/// all is an error, because nothing can be done with it.
pub fn read(text: &str) -> Result<Drawing, String> {
    let pairs = pairs_of(text)?;
    let mut drawing = Drawing::default();
    let mut blocks: HashMap<String, Vec<Part>> = HashMap::new();

    let mut at = 0;
    while at < pairs.len() {
        let (code, value) = pairs[at];
        if code == 0 && value == "SECTION" && at + 1 < pairs.len() && pairs[at + 1].0 == 2 {
            at = match pairs[at + 1].1 {
                "HEADER" => header(&pairs, at + 2, &mut drawing),
                "TABLES" => tables(&pairs, at + 2, &mut drawing),
                // BLOCKS comes before ENTITIES in the file, so a definition is
                // always in hand before the INSERT that reaches for it.
                "BLOCKS" => block_definitions(&pairs, at + 2, &mut drawing, &mut blocks),
                "ENTITIES" => entities(&pairs, at + 2, &mut drawing, &blocks),
                _ => end_of_section(&pairs, at + 2),
            };
            continue;
        }
        at += 1;
    }

    Ok(drawing)
}

/// The file as (code, value) pairs, borrowing rather than copying.
///
/// A large drawing is tens of millions of these. Allocating a `String` for each
/// value is the difference between a file opening and a phone giving up, so the
/// values are slices of the text that was read.
fn pairs_of(text: &str) -> Result<Vec<(i32, &str)>, String> {
    let mut lines = text.lines();
    let mut out: Vec<(i32, &str)> = Vec::with_capacity(text.len() / 24 + 16);
    while let Some(code_line) = lines.next() {
        let code = code_line.trim();
        if code.is_empty() {
            continue;
        }
        let value = lines.next().ok_or("a group code with no value after it")?;
        let code: i32 = code.parse().map_err(|_| format!("'{code}' is not a group code"))?;
        out.push((code, value.trim()));
    }
    Ok(out)
}

fn end_of_section(pairs: &[(i32, &str)], from: usize) -> usize {
    let mut at = from;
    while at < pairs.len() {
        if pairs[at] == (0, "ENDSEC") {
            return at + 1;
        }
        at += 1;
    }
    pairs.len()
}

/// The header, for the one thing in it that matters here: what a unit means.
fn header(pairs: &[(i32, &str)], from: usize, drawing: &mut Drawing) -> usize {
    let mut at = from;
    while at < pairs.len() {
        if pairs[at] == (0, "ENDSEC") {
            return at + 1;
        }
        if pairs[at] == (9, "$INSUNITS") {
            if let Some(&(70, value)) = pairs.get(at + 1) {
                if let Some(metres) = value.parse().ok().and_then(metres_per_unit) {
                    drawing.units = Units { metres_per_unit: metres, declared: true };
                }
            }
        }
        at += 1;
    }
    pairs.len()
}

/// The layer table, so entities have somewhere to belong before they arrive.
fn tables(pairs: &[(i32, &str)], from: usize, drawing: &mut Drawing) -> usize {
    let mut at = from;
    while at < pairs.len() {
        let (code, value) = pairs[at];
        if code == 0 && value == "ENDSEC" {
            return at + 1;
        }
        if code == 0 && value == "LAYER" {
            let fields = fields_of(pairs, at + 1);
            let name = field(&fields, 2).unwrap_or("");
            if !name.is_empty() {
                let colour = field(&fields, 62).and_then(|v| v.parse::<i32>().ok()).unwrap_or(7);
                let id = drawing.layer_for(name);
                let layer = &mut drawing.layers[id as usize];
                // **A negative colour index means the layer is off.** Not a
                // separate flag, which is why it is easy to miss and why a
                // drawing then shows construction lines nobody wanted.
                layer.visible = colour >= 0;
                layer.colour = aci_colour(colour.abs());
            }
        }
        at += 1;
    }
    pairs.len()
}

/// One thing inside a block definition.
///
/// **Blocks nest, and most of a drawing is usually inside them.** On a real
/// architectural plan, 13,243 of 14,421 lines sat inside block definitions, and
/// 131 of the 210 block references were *within other blocks* — a stair made of
/// steps, a door made of a leaf and a swing. A reader that expands only the
/// references at the top level draws about a seventh of the drawing, and every
/// line it does draw is in the right place — so the result looks like a drawing
/// that was simply emptier than expected.
enum Part {
    Shape(Entity),
    Inside { name: String, put: Affine },
}

/// Every block definition, as the parts it holds about its own origin.
fn block_definitions<'a>(
    pairs: &[(i32, &'a str)],
    from: usize,
    drawing: &mut Drawing,
    blocks: &mut HashMap<String, Vec<Part>>,
) -> usize {
    let mut at = from;
    while at < pairs.len() {
        let (code, value) = pairs[at];
        if code == 0 && value == "ENDSEC" {
            return at + 1;
        }
        if code == 0 && value == "BLOCK" {
            let name = fields_of(pairs, at + 1)
                .iter()
                .find(|&&(c, _)| c == 2)
                .map(|&(_, v)| v)
                .unwrap_or("")
                .to_string();

            let mut held = Vec::new();
            at = one_block(pairs, at + 1, drawing, &mut held);
            // **Model and paper space are not blocks to expand.** Their
            // contents are in the ENTITIES section, and importing them here
            // would draw the whole plan a second time on top of itself. Both
            // spellings, because files in the wild use `$` as well as `*`.
            let upper = name.to_ascii_uppercase();
            let space = upper.trim_start_matches(['*', '$']);
            if !name.is_empty()
                && !space.starts_with("MODEL_SPACE")
                && !space.starts_with("PAPER_SPACE")
            {
                if std::env::var("PAGIFY_DXF_TRACE").is_ok() && held.len() > 200 {
                    eprintln!("block {upper}: {} parts", held.len());
                }
                blocks.insert(upper, held);
            }
            continue;
        }
        at += 1;
    }
    pairs.len()
}

/// One block's contents, up to its ENDBLK.
fn one_block(
    pairs: &[(i32, &str)],
    from: usize,
    drawing: &mut Drawing,
    into: &mut Vec<Part>,
) -> usize {
    let mut at = from;
    while at < pairs.len() {
        let (code, value) = pairs[at];
        if code == 0 && (value == "ENDBLK" || value == "ENDSEC") {
            return if value == "ENDBLK" { at + 1 } else { at };
        }
        if code == 0 && value != "BLOCK" {
            let fields = fields_of(pairs, at + 1);
            // A reference to another block is kept as a reference. Where it
            // ends up depends on where *this* block is placed, which is not
            // known until something places it.
            if value == "INSERT" {
                if let Some(name) = field(&fields, 2) {
                    into.push(Part::Inside {
                        name: name.to_ascii_uppercase(),
                        put: insert_transform(&fields),
                    });
                }
            } else if value == "HATCH" {
                into.extend(hatch_shapes(&fields, drawing).into_iter().map(Part::Shape));
            } else if value == "POLYLINE" {
                let (entity, next) = old_polyline(pairs, at, drawing);
                if let Some(entity) = entity {
                    into.push(Part::Shape(entity));
                }
                at = next;
                continue;
            } else if let Some(entity) = build(value, &fields, drawing) {
                into.push(Part::Shape(entity));
            }
        }
        at += 1;
    }
    pairs.len()
}

/// The old kind of polyline: a header, then a VERTEX per point, then SEQEND.
///
/// Kept because real files still use it — an architectural plan that had none
/// of the modern `LWPOLYLINE` at all had seventy-three of these. Read as one
/// entity rather than as the seventy-three headers and two hundred loose
/// vertices they look like, which is what makes the difference between a
/// counted loss and a drawn shape.
///
/// Returns where to carry on from, which is past the SEQEND: the VERTEX entries
/// belong to this and must not be looked at again as entities of their own.
fn old_polyline(
    pairs: &[(i32, &str)],
    from: usize,
    drawing: &mut Drawing,
) -> (Option<Entity>, usize) {
    let header = fields_of(pairs, from + 1);
    let layer = drawing.layer_for(field(&header, 8).unwrap_or("0"));
    let flags = field(&header, 70).and_then(|v| v.parse::<i32>().ok()).unwrap_or(0);
    let closed = flags & 1 != 0;
    // Bit 3 and bit 6 mark a 3D polyline and a polygon mesh. Their vertices are
    // not a flat outline, so drawing them as one would be an invention.
    let flat = flags & 0b0100_1000 == 0;

    let mut vertices = Vec::new();
    let mut at = from + 1;
    while at < pairs.len() {
        let (code, value) = pairs[at];
        if code == 0 && value == "SEQEND" {
            at += 1;
            break;
        }
        if code == 0 && value == "VERTEX" {
            let fields = fields_of(pairs, at + 1);
            if let (Some(x), Some(y)) = (number(&fields, 10), number(&fields, 20)) {
                vertices.push(Vertex { at: Point::new(x, y), bulge: number(&fields, 42).unwrap_or(0.0) });
            }
        } else if code == 0 {
            // Anything else means the SEQEND is missing and this has run into
            // the next entity. Stop rather than swallow it.
            break;
        }
        at += 1;
    }

    if !flat {
        drawing.note("polylines that are not flat");
        return (None, at);
    }
    if vertices.len() < 2 {
        return (None, at);
    }
    (Some(Entity { layer, shape: Shape::Polyline { vertices, closed } }), at)
}

/// The drawing proper.
fn entities(
    pairs: &[(i32, &str)],
    from: usize,
    drawing: &mut Drawing,
    blocks: &HashMap<String, Vec<Part>>,
) -> usize {
    let mut at = from;
    while at < pairs.len() {
        let (code, value) = pairs[at];
        if code == 0 && value == "ENDSEC" {
            return at + 1;
        }
        if code == 0 {
            let fields = fields_of(pairs, at + 1);
            if value == "INSERT" {
                if let Some(name) = field(&fields, 2) {
                    let put = insert_transform(&fields);
                    expand(&name.to_ascii_uppercase(), put, drawing, blocks, 0);
                }
            } else if value == "DIMENSION" {
                // **A dimension keeps its own picture in a block.** AutoCAD
                // draws the extension lines, the arrows and — the part that
                // matters most — the measured figure into an anonymous block,
                // and the DIMENSION entity names it on code 2. Working the
                // geometry out from the definition points and a dimension
                // style would be inventing a second answer to a question the
                // file has already answered, and getting the figure wrong on a
                // drawing is worse than not drawing it.
                //
                // Its contents are already in world coordinates, so it goes in
                // where it lies.
                match field(&fields, 2) {
                    Some(name) => {
                        expand(&name.to_ascii_uppercase(), Affine::identity(), drawing, blocks, 0)
                    }
                    None => drawing.note("dimensions"),
                }
            } else if value == "HATCH" {
                let filled = hatch_shapes(&fields, drawing);
                drawing.entities.extend(filled);
            } else if value == "POLYLINE" {
                let (entity, next) = old_polyline(pairs, at, drawing);
                if let Some(entity) = entity {
                    drawing.entities.push(entity);
                }
                at = next;
                continue;
            } else if let Some(entity) = build(value, &fields, drawing) {
                drawing.entities.push(entity);
            }
        }
        at += 1;
    }
    pairs.len()
}

/// Every code belonging to one entity: from here to the next `0`.
fn fields_of<'a>(pairs: &[(i32, &'a str)], from: usize) -> Vec<(i32, &'a str)> {
    let mut out = Vec::new();
    let mut at = from;
    while at < pairs.len() && pairs[at].0 != 0 {
        out.push(pairs[at]);
        at += 1;
    }
    out
}

fn field<'a>(fields: &[(i32, &'a str)], code: i32) -> Option<&'a str> {
    fields.iter().find(|&&(c, _)| c == code).map(|&(_, v)| v)
}

fn number(fields: &[(i32, &str)], code: i32) -> Option<f64> {
    field(fields, code).and_then(|v| v.parse().ok())
}

/// Which way an entity's own coordinate system faces.
///
/// **Most DXF entities are written in object coordinates**, not world ones, and
/// carry a 210/220/230 extrusion vector saying which way that system points.
/// Reading the numbers as world coordinates is right for the extrusion nearly
/// every entity has and a silent mirror for the rest — and it is not rare:
/// AutoCAD writes `(0, 0, -1)` whenever a circle, arc or block reference is
/// mirrored. A plan can arrive with half its blocks flipped and its walls
/// exactly where they belong.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Facing {
    /// Extrusion +z. Object coordinates *are* world coordinates.
    World,
    /// Extrusion −z: object `(x, y)` is world `(−x, y)`. An exact mirror.
    MirroredX,
    /// Anything else — an entity standing on a tilted plane. Left alone: a flat
    /// drawing has nowhere to put it, and projecting it turns its circles into
    /// ellipses, which is a different wrong answer rather than a fix.
    Tilted,
}

fn facing_of(fields: &[(i32, &str)]) -> Facing {
    let axis = |code: i32, dflt: f64| number(fields, code).unwrap_or(dflt);
    let (nx, ny, nz) = (axis(210, 0.0), axis(220, 0.0), axis(230, 1.0));
    // 1/64 is the DXF arbitrary-axis algorithm's own threshold, so this agrees
    // with the transform it stands in for.
    if nx.abs() >= 1.0 / 64.0 || ny.abs() >= 1.0 / 64.0 {
        return Facing::Tilted;
    }
    if nz < 0.0 {
        Facing::MirroredX
    } else {
        Facing::World
    }
}

/// One entity, or `None` where there is nothing faithful to make of it.
fn build(kind: &str, fields: &[(i32, &str)], drawing: &mut Drawing) -> Option<Entity> {
    let layer = drawing.layer_for(field(fields, 8).unwrap_or("0"));

    let flip = facing_of(fields) == Facing::MirroredX;
    let placed = |p: Point| if flip { Point::new(-p.x, p.y) } else { p };

    let shape = match kind {
        // **LINE and ELLIPSE are world entities.** The DXF reference says so
        // per entity, and the two lists are not the same: mirroring a LINE
        // because it happens to carry a 210 puts it somewhere it is not.
        "LINE" => Shape::Line {
            a: Point::new(number(fields, 10)?, number(fields, 20)?),
            b: Point::new(number(fields, 11)?, number(fields, 21)?),
        },

        "CIRCLE" => Shape::Arc {
            centre: placed(Point::new(number(fields, 10)?, number(fields, 20)?)),
            radius: number(fields, 40)?,
            start: 0.0,
            sweep: std::f64::consts::TAU,
        },

        "ARC" => {
            let from = number(fields, 50)?.to_radians();
            let to = number(fields, 51)?.to_radians();
            // **A mirror reverses the sweep.** An object angle is measured from
            // the object's own x toward its y, and under a −z extrusion that
            // pair maps to world (−1, 0) and (0, 1) — so the angle arrives as
            // π − θ and runs the other way. Keeping the start and letting the
            // sweep run as it was draws the complement: everything except the
            // piece that is actually there.
            let (from, to) = if flip {
                (std::f64::consts::PI - to, std::f64::consts::PI - from)
            } else {
                (from, to)
            };
            let sweep = (to - from).rem_euclid(std::f64::consts::TAU);
            Shape::Arc {
                centre: placed(Point::new(number(fields, 10)?, number(fields, 20)?)),
                radius: number(fields, 40)?,
                start: from.rem_euclid(std::f64::consts::TAU),
                // Exactly zero means a whole circle, not an empty arc.
                sweep: if sweep < 1e-9 { std::f64::consts::TAU } else { sweep },
            }
        }

        "ELLIPSE" => {
            let start = number(fields, 41).unwrap_or(0.0);
            let end = number(fields, 42).unwrap_or(std::f64::consts::TAU);
            let sweep = (end - start).rem_euclid(std::f64::consts::TAU);
            Shape::Ellipse {
                centre: Point::new(number(fields, 10)?, number(fields, 20)?),
                major: Point::new(number(fields, 11)?, number(fields, 21)?),
                ratio: number(fields, 40)?,
                start,
                sweep: if sweep < 1e-9 { std::f64::consts::TAU } else { sweep },
            }
        }

        "LWPOLYLINE" => {
            let closed = field(fields, 70).and_then(|v| v.parse::<i32>().ok()).unwrap_or(0) & 1 != 0;
            let mut vertices: Vec<Vertex> = Vec::new();
            let mut holding: Option<Point> = None;
            let mut bulge = 0.0;

            // The coordinates are interleaved rather than grouped, so this
            // walks the codes in order and closes each vertex when the next
            // one starts.
            for &(code, value) in fields {
                match code {
                    10 => {
                        if let Some(at) = holding.take() {
                            vertices.push(Vertex { at, bulge });
                            bulge = 0.0;
                        }
                        holding = Some(Point::new(value.parse().unwrap_or(0.0), 0.0));
                    }
                    20 => {
                        if let Some(at) = holding.as_mut() {
                            at.y = value.parse().unwrap_or(0.0);
                        }
                    }
                    42 => bulge = value.parse().unwrap_or(0.0),
                    _ => {}
                }
            }
            if let Some(at) = holding.take() {
                vertices.push(Vertex { at, bulge });
            }
            if vertices.is_empty() {
                return None;
            }

            // **A mirror flips every bulge.** Bulge is signed — positive bows
            // anticlockwise — so reflecting the vertices without negating it
            // leaves every curve bowing the wrong way: a door that swings into
            // the wall. The order of the vertices is untouched.
            if flip {
                for vertex in &mut vertices {
                    vertex.at.x = -vertex.at.x;
                    vertex.bulge = -vertex.bulge;
                }
            }

            Shape::Polyline { vertices, closed }
        }

        "TEXT" | "MTEXT" | "ATTRIB" => {
            let content = readable(field(fields, 1).unwrap_or(""));
            if content.is_empty() {
                return None;
            }
            // **The second point is where aligned text actually sits.** With a
            // horizontal or vertical justification set, 10/20 is left at the
            // origin of the text's own box and 11/21 carries the real place.
            // Taking the first blindly stacks every centred label at the corner
            // of whatever it labels.
            let aligned = field(fields, 72).and_then(|v| v.parse::<i32>().ok()).unwrap_or(0) != 0
                || field(fields, 73).and_then(|v| v.parse::<i32>().ok()).unwrap_or(0) != 0;
            let at = match (aligned, number(fields, 11), number(fields, 21)) {
                (true, Some(x), Some(y)) => Point::new(x, y),
                _ => Point::new(number(fields, 10)?, number(fields, 20)?),
            };

            Shape::Text {
                at: placed(at),
                // MTEXT calls its height 40 too, so one path serves both.
                height: number(fields, 40).filter(|h| *h > 0.0).unwrap_or(2.5),
                rotation: number(fields, 50).unwrap_or(0.0).to_radians(),
                content,
            }
        }
        // DIMENSION is handled where blocks can be expanded; a leader that
        // carries no block of its own is still counted.
        "LEADER" | "MULTILEADER" => {
            drawing.note("leaders");
            return None;
        }
        // Handled where more than one shape can be returned; see
        // [`hatch_shapes`]. Reaching here would mean a caller that did not
        // check, so it is counted rather than passed over in silence.
        "HATCH" => {
            drawing.note("hatching");
            return None;
        }
        "SPLINE" => {
            drawing.note("curves this cannot draw yet");
            return None;
        }
        // A filled triangle or quadrilateral — arrowheads, and the solid
        // fills on a section. Drawn as its outline rather than filled, which
        // at the size these are is a difference nobody can see, and is one
        // fewer kind of path for the renderer to carry.
        //
        // **The corners are stored 1, 2, 4, 3.** Not a quirk this reader can
        // choose to ignore: taken in the order they are written, every solid
        // comes out as a bow tie.
        "SOLID" | "3DFACE" | "TRACE" => {
            let corner = |a: i32, b: i32| match (number(fields, a), number(fields, b)) {
                (Some(x), Some(y)) => Some(placed(Point::new(x, y))),
                _ => None,
            };
            let (first, second) = (corner(10, 20)?, corner(11, 21)?);
            let third = corner(12, 22)?;
            let fourth = corner(13, 23).unwrap_or(third);

            let mut vertices = vec![first, second, fourth, third];
            // A triangle is written as a quadrilateral with its last corner
            // repeated; leaving the repeat in gives a zero-length segment.
            vertices.dedup_by(|a, b| (a.x - b.x).abs() < 1e-12 && (a.y - b.y).abs() < 1e-12);
            if vertices.len() < 3 {
                return None;
            }

            Shape::Polyline {
                vertices: vertices.into_iter().map(|at| Vertex { at, bulge: 0.0 }).collect(),
                closed: true,
            }
        }

        "POINT" => Shape::Marker { at: placed(Point::new(number(fields, 10)?, number(fields, 20)?)) },

        // **An attribute definition is a blank, not a value.** It lives in a
        // block definition and says where a value will go and what to call it;
        // the value itself arrives as an ATTRIB on each block reference, and
        // those are drawn. Drawing the definition too would print the
        // placeholder — "ROOM_NAME" — across every room that has a name.
        "ATTDEF" => return None,
        // The window a layout looks at model space through. It has no geometry
        // of its own, and this shows model space directly.
        "VIEWPORT" => return None,
        // Bookkeeping, not geometry: a vertex belongs to the polyline that
        // owns it and a SEQEND only says where one ended. Counting them as
        // losses would report a drawing as missing things it is not.
        "VERTEX" | "SEQEND" | "ENDBLK" => return None,

        // Everything else leaves counted. A drawing that quietly lost a
        // hundred entities looks exactly like one that never had them.
        other => {
            drawing.note(&format!("{} entities this cannot draw yet", other.to_ascii_uppercase()));
            return None;
        }
    };

    Some(Entity { layer, shape })
}

/// A `HATCH`, as the shapes that fill it.
///
/// **Not one entity but many**, which is why it is here rather than in
/// [`build`]: a patterned hatch is a recipe, and what goes on the sheet is
/// however many strokes running it produces.
///
/// The pattern definition written into the file is already at its final angle
/// and scale — AutoCAD applies the pattern scale (41) and angle (52) before
/// writing, and they are left in the file for a program that wants to edit the
/// hatch rather than draw it. Applying them again here would tilt every hatch
/// by its own angle twice, which is one of those errors that still looks like
/// hatching.
fn hatch_shapes(fields: &[(i32, &str)], drawing: &mut Drawing) -> Vec<Entity> {
    let layer = drawing.layer_for(field(fields, 8).unwrap_or("0"));
    let flip = facing_of(fields) == Facing::MirroredX;
    let placed = |p: Point| if flip { Point::new(-p.x, p.y) } else { p };

    let whole = |code: i32| field(fields, code).and_then(|v| v.parse::<i32>().ok());
    let mut hatch = super::hatch::Hatch {
        solid: whole(70) == Some(1),
        ..Default::default()
    };

    // The codes come in a defined order, so this walks the list once rather
    // than searching it: a hatch repeats 10 and 20 for every vertex of every
    // loop, and `field`, which finds the first, would answer with the first
    // corner of the first loop to every question asked of it.
    let mut at = 0usize;
    let mut next = |at: &mut usize| -> Option<(i32, &str)> {
        let pair = fields.get(*at).copied();
        if pair.is_some() {
            *at += 1;
        }
        pair
    };
    let value = |text: &str| text.parse::<f64>().unwrap_or(0.0);
    let count = |text: &str| text.parse::<i64>().unwrap_or(0).clamp(0, 100_000) as usize;

    // Up to the boundary data.
    while let Some((code, _)) = next(&mut at) {
        if code == 91 {
            break;
        }
    }

    while at < fields.len() {
        let Some((code, text)) = next(&mut at) else { break };
        match code {
            // A boundary path: a polyline, or a run of edges.
            92 => {
                let flags = text.parse::<i32>().unwrap_or(0);
                let ring = if flags & 2 != 0 {
                    polyline_boundary(fields, &mut at)
                } else {
                    edge_boundary(fields, &mut at)
                };
                if ring.len() >= 3 {
                    hatch.loops.push(ring.into_iter().map(placed).collect());
                }
            }
            // A line of the pattern definition. Its own codes follow in order.
            53 => {
                let mut line = super::hatch::PatternLine {
                    angle: value(text).to_radians(),
                    base: Point::new(0.0, 0.0),
                    offset: Point::new(0.0, 0.0),
                    dashes: Vec::new(),
                };
                let mut dashes = 0usize;
                while let Some(&(code, text)) = fields.get(at) {
                    match code {
                        43 => line.base.x = value(text),
                        44 => line.base.y = value(text),
                        45 => line.offset.x = value(text),
                        46 => line.offset.y = value(text),
                        79 => dashes = count(text),
                        49 => line.dashes.push(value(text)),
                        // Anything else begins the next thing along.
                        _ => break,
                    }
                    at += 1;
                    if code == 49 && line.dashes.len() >= dashes {
                        break;
                    }
                }
                // **Mirrored patterns run the other way.** The boundary is
                // mirrored above; leaving the pattern alone would hatch a
                // mirrored region at the mirror image of its own angle.
                if flip {
                    line.angle = std::f64::consts::PI - line.angle;
                    line.base.x = -line.base.x;
                    line.offset.x = -line.offset.x;
                }
                hatch.lines.push(line);
            }
            _ => {}
        }
    }

    match super::hatch::shape_of(&hatch, layer) {
        Some(made) => vec![made],
        // No boundary this reader could make sense of. Counted rather than
        // passed over: a section drawing missing its materials still looks
        // like a section drawing.
        None => {
            drawing.note("hatching");
            Vec::new()
        }
    }
}

/// A boundary written as a polyline: a count, then its vertices.
fn polyline_boundary(fields: &[(i32, &str)], at: &mut usize) -> Vec<Point> {
    let mut has_bulge = false;
    let mut wanted = 0usize;
    let mut vertices: Vec<Vertex> = Vec::new();

    while let Some(&(code, text)) = fields.get(*at) {
        match code {
            72 => has_bulge = text.parse::<i32>().unwrap_or(0) != 0,
            // 73 is the closed flag. A hatch boundary is a region, so it is
            // closed whatever the flag says — an open one bounds nothing.
            73 => {}
            93 => wanted = text.parse::<i64>().unwrap_or(0).clamp(0, 100_000) as usize,
            10 => vertices.push(Vertex {
                at: Point::new(text.parse().unwrap_or(0.0), 0.0),
                bulge: 0.0,
            }),
            20 => {
                if let Some(last) = vertices.last_mut() {
                    last.at.y = text.parse().unwrap_or(0.0);
                }
            }
            42 if has_bulge => {
                if let Some(last) = vertices.last_mut() {
                    last.bulge = text.parse().unwrap_or(0.0);
                }
            }
            // The count of source objects: the vertices are finished.
            97 => break,
            // The next path, or the pattern.
            92 | 75 | 76 | 78 | 98 | 450 => break,
            _ => {}
        }
        *at += 1;
        if wanted > 0 && vertices.len() >= wanted && code == 20 {
            *at += 1;
            break;
        }
    }
    super::hatch::flattened(&vertices, true)
}

/// A boundary written as a run of edges: lines, arcs and ellipse arcs.
///
/// Each edge names its kind on code 72 and then its own numbers. Splines are
/// walked past rather than approximated — a boundary that is nearly right
/// bleeds the hatching out of the region it belongs in.
fn edge_boundary(fields: &[(i32, &str)], at: &mut usize) -> Vec<Point> {
    let mut out: Vec<Point> = Vec::new();
    let mut edges = 0usize;
    let mut done = 0usize;

    // The edge count comes first.
    while let Some(&(code, text)) = fields.get(*at) {
        *at += 1;
        if code == 93 {
            edges = text.parse::<i64>().unwrap_or(0).clamp(0, 100_000) as usize;
            break;
        }
        if code == 92 || code == 97 {
            return out;
        }
    }

    while done < edges {
        // Every edge begins with its kind.
        let mut kind = 0;
        let mut found = false;
        while let Some(&(code, text)) = fields.get(*at) {
            if code == 72 {
                kind = text.parse::<i32>().unwrap_or(0);
                *at += 1;
                found = true;
                break;
            }
            if code == 97 || code == 92 {
                return out;
            }
            *at += 1;
        }
        if !found {
            return out;
        }
        done += 1;

        let mut numbers: Vec<(i32, f64)> = Vec::new();
        while let Some(&(code, text)) = fields.get(*at) {
            // 72 begins the next edge; these three begin what follows the path.
            if code == 72 || code == 97 || code == 92 || code == 93 {
                break;
            }
            numbers.push((code, text.parse().unwrap_or(0.0)));
            *at += 1;
        }
        let of = |want: i32| numbers.iter().find(|(code, _)| *code == want).map(|(_, v)| *v);

        match kind {
            // A line: from its start to its end. Only the start is kept — the
            // next edge begins where this one ended, and the loop is closed.
            1 => {
                if let (Some(x), Some(y)) = (of(10), of(20)) {
                    out.push(Point::new(x, y));
                }
                if done == edges {
                    if let (Some(x), Some(y)) = (of(11), of(21)) {
                        out.push(Point::new(x, y));
                    }
                }
            }
            // A circular arc, opened out. Degrees here, as everywhere in DXF.
            2 => {
                let (Some(cx), Some(cy), Some(radius)) = (of(10), of(20), of(40)) else { continue };
                let from = of(50).unwrap_or(0.0).to_radians();
                let to = of(51).unwrap_or(360.0).to_radians();
                let anticlockwise = of(73).unwrap_or(1.0) != 0.0;
                super::hatch::arc_points(&mut out, Point::new(cx, cy), radius, 1.0, 0.0, from, to, anticlockwise);
            }
            // An elliptical arc. The major axis is a vector from the centre and
            // the minor is a fraction of it, which is how the whole crate holds
            // an ellipse — so the only work is the turn.
            3 => {
                let (Some(cx), Some(cy)) = (of(10), of(20)) else { continue };
                let (major_x, major_y) = (of(11).unwrap_or(0.0), of(21).unwrap_or(0.0));
                let radius = major_x.hypot(major_y);
                if radius < 1e-12 {
                    continue;
                }
                let from = of(50).unwrap_or(0.0).to_radians();
                let to = of(51).unwrap_or(360.0).to_radians();
                let anticlockwise = of(73).unwrap_or(1.0) != 0.0;
                super::hatch::arc_points(
                    &mut out,
                    Point::new(cx, cy),
                    radius,
                    of(40).unwrap_or(1.0),
                    major_y.atan2(major_x),
                    from,
                    to,
                    anticlockwise,
                );
            }
            // A spline boundary. Its control points are not on the curve, so
            // joining them would give a region a little smaller than the real
            // one all the way round — near enough to look right and wrong at
            // every edge. The fit points, where the file gives them, are.
            4 => {
                let mut fit: Vec<Point> = Vec::new();
                let mut pending: Option<f64> = None;
                for (code, number) in &numbers {
                    match code {
                        11 => pending = Some(*number),
                        21 => {
                            if let Some(x) = pending.take() {
                                fit.push(Point::new(x, *number));
                            }
                        }
                        _ => {}
                    }
                }
                out.extend(fit);
            }
            _ => {}
        }
    }
    out
}

/// Where a block's contents land.
///
/// **An affine transform, not an insertion point with a rotation.** Blocks
/// nest, so placements compose, and "shift, turn, scale, maybe mirror" is not
/// closed under composition — a rotated block inside a squashed one is not any
/// of those things. Six numbers compose exactly, every time, however deep.
#[derive(Clone, Copy)]
pub(crate) struct Affine {
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

    /// A direction: turned and scaled, never shifted.
    fn direction(&self, p: Point) -> Point {
        Point::new(self.a * p.x + self.c * p.y, self.b * p.x + self.d * p.y)
    }

    /// This transform, then `outer`.
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

    /// Whether this turns the sheet over. Everything measured anticlockwise —
    /// an arc's sweep, a polyline's bulge — runs the other way if it does.
    fn reverses(&self) -> bool {
        self.determinant() < 0.0
    }

    /// How much bigger things get, as one number.
    ///
    /// Exact for a transform that scales both axes alike, which is what a block
    /// placement almost always is. A block squashed on one axis turns its
    /// circles into ellipses, and this draws them as circles of the average
    /// size — wrong by however much it was squashed, and right in the ordinary
    /// case. Named here rather than hidden so it is a known approximation.
    fn scale(&self) -> f64 {
        self.determinant().abs().sqrt()
    }

    /// Which way `+x` ends up pointing, for an arc's start angle.
    fn turn(&self) -> f64 {
        self.b.atan2(self.a)
    }
}

/// The transform one INSERT describes.
fn insert_transform(fields: &[(i32, &str)]) -> Affine {
    let at = Point::new(number(fields, 10).unwrap_or(0.0), number(fields, 20).unwrap_or(0.0));
    let mut x_scale = number(fields, 41).unwrap_or(1.0);
    let y_scale = number(fields, 42).unwrap_or(1.0);
    // A −z extrusion on an INSERT is one more mirror, composed with whatever
    // the scale signs already carry.
    if facing_of(fields) == Facing::MirroredX {
        x_scale = -x_scale;
    }
    let (sin, cos) = number(fields, 50).unwrap_or(0.0).to_radians().sin_cos();

    // Scale first, then rotate, then shift — the order AutoCAD defines.
    Affine {
        a: x_scale * cos,
        b: x_scale * sin,
        c: -y_scale * sin,
        d: y_scale * cos,
        e: at.x,
        f: at.y,
    }
}

/// Put a block's contents where a reference says they go, following nesting.
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
            Part::Shape(entity) => drawing.entities.push(Entity {
                layer: entity.layer,
                shape: moved(&entity.shape, &put),
            }),
            // The inner placement first, then the outer one — which is what
            // makes a chair inside an office inside a floor land in the room it
            // belongs to rather than at the building's origin.
            Part::Inside { name, put: inner } => {
                expand(name, inner.then(&put), drawing, blocks, depth + 1)
            }
        }
    }
}

/// One shape, moved by a transform.
fn moved(shape: &Shape, put: &Affine) -> Shape {
    match shape {
        Shape::Line { a, b } => Shape::Line { a: put.point(*a), b: put.point(*b) },

        Shape::Marker { at } => Shape::Marker { at: put.point(*at) },

        Shape::Arc { centre, radius, start, sweep } => {
            // A reversing transform reflects the angles, so the arc runs from
            // the other end. Keeping the start where it was and sweeping the
            // original way draws the complement — everything except the piece
            // that is there.
            let start = if put.reverses() {
                put.turn() - start - sweep
            } else {
                put.turn() + start
            };
            Shape::Arc {
                centre: put.point(*centre),
                radius: radius * put.scale(),
                start: start.rem_euclid(std::f64::consts::TAU),
                sweep: *sweep,
            }
        }

        // An ellipse needs no special care: its major axis is a vector, so the
        // transform applies to it directly.
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
                    // Bulge is signed the same way an arc's sweep is, so a
                    // transform that turns the sheet over bows every curve the
                    // other way: a door that swings into the wall.
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

/// **Ignored, because real drawings cannot be committed.** Customer and
/// supplier CAD is the confidential part. Point it at a file:
///
/// ```text
/// PAGIFY_DXF_FILE="/path/to/plan.dxf" \
///   cargo test --lib drawing::dxf::real -- --ignored --nocapture
/// ```
#[cfg(test)]
mod real {
    #[test]
    #[ignore = "needs a real drawing; set PAGIFY_DXF_FILE"]
    fn a_real_drawing_reads() {
        let path = std::env::var("PAGIFY_DXF_FILE").expect("set PAGIFY_DXF_FILE");
        let bytes = std::fs::read(&path).expect("readable");
        let text = String::from_utf8_lossy(&bytes);

        let started = std::time::Instant::now();
        let drawing = super::read(&text).expect("it parses");
        let took = started.elapsed();

        let counts = |want: fn(&super::Shape) -> bool| {
            drawing.entities.iter().filter(|e| want(&e.shape)).count()
        };
        println!("--- {path}");
        println!(
            "{} shapes, {} layers, in {:?}",
            drawing.kept(),
            drawing.layers.len(),
            took,
        );
        println!(
            "  lines {}  arcs {}  ellipses {}  polylines {}",
            counts(|s| matches!(s, super::Shape::Line { .. })),
            counts(|s| matches!(s, super::Shape::Arc { .. })),
            counts(|s| matches!(s, super::Shape::Ellipse { .. })),
            counts(|s| matches!(s, super::Shape::Polyline { .. })),
        );
        println!("  units {:?}", drawing.units);
        println!("  not shown: {:?}", drawing.skipped);
        if let Some((low, high)) = drawing.bounds() {
            println!(
                "  extent {:.1} x {:.1}, from ({:.1}, {:.1})",
                high.x - low.x,
                high.y - low.y,
                low.x,
                low.y,
            );
        }

        assert!(drawing.kept() > 0, "nothing was read at all");
    }
}

/// The words out of a piece of CAD text, without the formatting around them.
///
/// **MTEXT is not a string, it is a tiny markup language.** A paragraph arrives
/// as something like `\pxi-3,l4,t4;{\H0.7x;NOTE}\PSecond line`, and drawing it
/// verbatim puts backslashes and font codes across the sheet where the note
/// should be. The escapes that stand for a real character are kept and the ones
/// that set something are dropped, which is the difference between a label and
/// a line of noise.
///
/// Plain TEXT goes through untouched apart from its `%%` codes, which it does
/// use: `%%d` is the degree sign in nearly every drawing with an angle on it.
///
/// Written without a single backslash literal, deliberately — the character
/// this is about is exactly the one that is hardest to get through a source
/// file unaltered.
pub(crate) fn readable(raw: &str) -> String {
    const ESCAPE: char = '\u{5c}';

    let mut out = String::with_capacity(raw.len());
    let mut characters = raw.chars().peekable();

    while let Some(character) = characters.next() {
        if character == ESCAPE {
            match characters.next() {
                // The ones that stand for a character of their own. Case
                // matters and the two are opposites: capital P is a hard
                // line break, lowercase p opens a run of paragraph settings
                // that continues to its semicolon, so treating them alike
                // writes "xi-3,l4,t4;" across the sheet.
                Some('P') | Some('~') => out.push(' '),
                Some(ESCAPE) => out.push(ESCAPE),
                Some('{') => out.push('{'),
                Some('}') => out.push('}'),
                // Everything else sets something — a font, a height, a width,
                // a stacked fraction — and runs to its semicolon.
                Some(_) => {
                    for skipped in characters.by_ref() {
                        if skipped == ';' {
                            break;
                        }
                    }
                }
                None => {}
            }
            continue;
        }

        match character {
            // Grouping, which carries no text of its own.
            '{' | '}' => {}
            '%' if characters.peek() == Some(&'%') => {
                characters.next();
                match characters.next() {
                    Some('d') | Some('D') => out.push('°'),
                    Some('c') | Some('C') => out.push('Ø'),
                    Some('p') | Some('P') => out.push('±'),
                    Some('%') => out.push('%'),
                    // A code this does not know is dropped rather than shown as
                    // a stray pair of per-cent signs.
                    Some(_) | None => {}
                }
            }
            other => out.push(other),
        }
    }

    out.trim().to_string()
}

#[cfg(test)]
mod hatching {
    use super::super::model::Shape;

    /// A DXF holding one entity, written the way a file writes it.
    fn one_entity(body: &str) -> String {
        format!("0\nSECTION\n2\nENTITIES\n{body}0\nENDSEC\n0\nEOF\n")
    }

    /// A square metre hatched with lines every tenth of a unit.
    fn a_hatched_square(solid: i32) -> String {
        one_entity(&format!(
            "0\nHATCH\n8\nWALLS\n2\nANSI31\n70\n{solid}\n71\n0\n\
             91\n1\n92\n7\n72\n0\n73\n1\n93\n4\n\
             10\n0.0\n20\n0.0\n10\n1.0\n20\n0.0\n10\n1.0\n20\n1.0\n10\n0.0\n20\n1.0\n\
             97\n0\n75\n0\n76\n1\n52\n0.0\n41\n1.0\n77\n0\n78\n1\n\
             53\n0.0\n43\n0.0\n44\n0.0\n45\n0.0\n46\n0.1\n79\n0\n",
        ))
    }

    /// **A hatch is one shape, however dense its pattern.**
    ///
    /// The strokes are worked out when the drawing is drawn, at the scale it is
    /// being drawn at — not here. Running the recipe as the file is read turned
    /// thirty-three hatches on one real drawing into eighty-eight thousand line
    /// segments, fifty times the whole rest of the drawing, and at a whole-sheet
    /// fit every one of them landed in a pixel another had already covered.
    #[test]
    fn a_hatch_is_read_as_one_shape_not_as_its_strokes() {
        let drawing = super::read(&a_hatched_square(0)).expect("it parses");

        assert_eq!(1, drawing.kept(), "{:?}", drawing.entities.len());
        assert!(
            matches!(drawing.entities[0].shape, Shape::Hatch { .. }),
            "not kept as an instruction: {:?}",
            drawing.entities[0].shape,
        );
        assert!(drawing.skipped.is_empty(), "{:?}", drawing.skipped);
    }

    /// The boundary comes through as the region it bounds.
    #[test]
    fn the_boundary_is_the_square_the_file_gave() {
        let drawing = super::read(&a_hatched_square(0)).expect("it parses");

        let Shape::Hatch { loops, lines } = &drawing.entities[0].shape else {
            panic!("not a hatch");
        };
        assert_eq!(1, loops.len());
        assert_eq!(4, loops[0].len(), "{:?}", loops[0]);
        assert_eq!(1, lines.len());
        // The spacing across the lines, which is what sets the density.
        assert!((lines[0].offset.y - 0.1).abs() < 1e-9, "{:?}", lines[0]);
    }

    /// **Solid means filled, not outlined.** A region a drawing says is solid,
    /// shown as an outline with nothing inside it, is the difference between a
    /// wall in section and a gap in a wall.
    #[test]
    fn a_solid_hatch_is_a_filled_region() {
        let drawing = super::read(&a_hatched_square(1)).expect("it parses");

        assert!(
            matches!(drawing.entities[0].shape, Shape::Fill { .. }),
            "{:?}",
            drawing.entities[0].shape,
        );
    }

    /// A hatch this reader cannot make a region out of is counted, not dropped.
    #[test]
    fn a_hatch_with_no_boundary_is_reported() {
        let drawing = super::read(&one_entity("0\nHATCH\n8\nWALLS\n2\nANSI31\n70\n0\n91\n0\n"))
            .expect("it parses");

        assert_eq!(0, drawing.kept());
        assert_eq!(1, drawing.lost(), "{:?}", drawing.skipped);
    }
}

#[cfg(test)]
mod text_from_a_drawing {
    use super::readable;

    const ESCAPE: char = '\u{5c}';

    /// A paragraph's formatting does not end up on the sheet.
    #[test]
    fn mtext_markup_is_not_drawn_as_words() {
        let raw = format!("{ESCAPE}pxi-3,l4,t4;{{{ESCAPE}H0.7x;NOTE}}{ESCAPE}PSecond line");
        assert_eq!("NOTE Second line", readable(&raw));
    }

    /// The codes that stand for a character are kept.
    ///
    /// `%%d` is the degree sign, and a drawing with angles on it is full of
    /// them — dropping it quietly turns "45°" into "45".
    #[test]
    fn the_codes_that_mean_a_character_survive() {
        assert_eq!("45°", readable("45%%d"));
        assert_eq!("Ø20", readable("%%c20"));
        assert_eq!("±0.5", readable("%%p0.5"));
    }

    /// An ordinary label is left exactly as it is.
    #[test]
    fn plain_text_passes_through() {
        assert_eq!("GROUND FLOOR", readable("GROUND FLOOR"));
    }

    /// And an escaped brace is a brace, not a group that never opened.
    #[test]
    fn an_escaped_brace_is_a_brace() {
        assert_eq!("{1}", readable(&format!("{ESCAPE}{{1{ESCAPE}}}")));
    }
}
