//! Writing the markup into the PDF, and getting it back — build plan phase 8.
//!
//! "The phase that decides whether any of the above survives a save."
//!
//! Two things go into the file, and they are not the same thing:
//!
//! 1. **The appearance**, as real PDF content — ink annotations that any reader
//!    will draw and any printer will print. This is what the markup *is* to
//!    everyone who does not have Pagify.
//! 2. **The live geometry**, as a versioned blob, so the marks are still a line
//!    and an arc when the file is reopened rather than a set of frozen strokes.
//!    Without this, trim and fillet stop working the moment you save — which is
//!    the whole reason §5.2 insists the layer stays live.
//!
//! ## Why the blob is not a serialisation of `cad_kernel`'s types
//!
//! It would be less code, and it would be a trap. The blob is a **file format**:
//! once a document has been written with it, its shape is a compatibility
//! obligation forever. Deriving it from the kernel's structs would hand SIMLUX
//! the power to change Pagify's file format by refactoring a struct it has every
//! right to refactor. So the format is declared here, converted at the boundary,
//! and versioned — §5.2's "define the blob format once, in Rust, with a version
//! field", taken literally.
//!
//! ## What the carrier is
//!
//! `pdf_core` gives exactly one place to put an opaque blob that survives a save
//! and can be found again: the `restore` string on an `Annotation::Text`, which
//! is tagged onto the page's content with the mark's own id. That mechanism is
//! already proven — it is what keeps a caption editable across any number of
//! saves. The carrier here is a single fully transparent space, which writes the
//! tag and the blob and draws nothing.

use serde::{Deserialize, Serialize};

use cad_kernel::{Arc, Circle, DObject, Geom, Line, Point, PolyVertex, Polyline, Vec2};
use pdf_core::document::{Annotation, Color, Glyph, Point as PdfPoint};
use pdf_core::registry::DocumentSession;
use pdf_core::Result;

use crate::markup::Layer;
use crate::page_space::PageSpace;

/// Bump when the meaning of an existing field changes. Adding an optional field
/// does not need it; changing what one *means* does, because an old reader must
/// be able to tell it cannot understand the file rather than misread it.
pub const FORMAT_VERSION: u32 = 1;

/// Marks a blob as ours. Other tools write `restore` blobs too; a blob that is
/// not ours must be left alone rather than parsed and thrown away.
pub const MAGIC: &str = "pagify.markup";

/// The id every markup carrier is written under, so `remove_text` can find the
/// previous one on re-commit. Distinct from the ids the caption tool uses.
pub const CARRIER_ID: i32 = 0x7061_67_01;

/// How finely curves are flattened for the *appearance*, in page points.
/// Only the appearance — the live geometry keeps its exact centre and radius.
const FLATTEN_TOLERANCE_PT: f64 = 0.25;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StoredLayer {
    pub magic: String,
    pub version: u32,
    /// The page height the geometry was authored against. Carried because the
    /// flip depends on it, and a page whose size changed between sessions would
    /// otherwise restore every mark to the wrong half.
    pub page_height_pt: f64,
    pub objects: Vec<StoredGeom>,
}

/// The geometry this format can carry.
///
/// Deliberately a closed set, and deliberately not every `Geom` variant. What
/// cannot be stored is *reported* rather than dropped quietly — see
/// [`Unstorable`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "k", rename_all = "lowercase")]
pub enum StoredGeom {
    Line { a: [f64; 2], b: [f64; 2] },
    Circle { c: [f64; 2], r: f64 },
    Arc { c: [f64; 2], r: f64, start: f64, sweep: f64 },
    Polyline { v: Vec<[f64; 3]>, closed: bool },
    Point { p: [f64; 2] },
}

impl StoredGeom {
    pub fn of(geom: &Geom) -> Option<StoredGeom> {
        Some(match geom {
            Geom::Line(l) => StoredGeom::Line { a: [l.a.x, l.a.y], b: [l.b.x, l.b.y] },
            Geom::Circle(c) => StoredGeom::Circle { c: [c.center.x, c.center.y], r: c.radius },
            Geom::Arc(a) => StoredGeom::Arc {
                c: [a.center.x, a.center.y],
                r: a.radius,
                start: a.start_angle,
                sweep: a.sweep_angle,
            },
            Geom::Polyline(p) => StoredGeom::Polyline {
                v: p.vertices.iter().map(|v| [v.pos.x, v.pos.y, v.bulge]).collect(),
                closed: p.closed,
            },
            Geom::Point(p) => StoredGeom::Point { p: [p.location.x, p.location.y] },
            _ => return None,
        })
    }

    pub fn to_geom(&self) -> Geom {
        match self {
            StoredGeom::Line { a, b } => Geom::Line(Line {
                a: Vec2::new(a[0], a[1]),
                b: Vec2::new(b[0], b[1]),
            }),
            StoredGeom::Circle { c, r } => Geom::Circle(Circle {
                center: Vec2::new(c[0], c[1]),
                radius: *r,
            }),
            StoredGeom::Arc { c, r, start, sweep } => Geom::Arc(Arc {
                center: Vec2::new(c[0], c[1]),
                radius: *r,
                start_angle: *start,
                sweep_angle: *sweep,
            }),
            StoredGeom::Polyline { v, closed } => Geom::Polyline(Polyline {
                vertices: v
                    .iter()
                    .map(|p| PolyVertex { pos: Vec2::new(p[0], p[1]), bulge: p[2] })
                    .collect(),
                closed: *closed,
                widths: Vec::new(),
            }),
            StoredGeom::Point { p } => Geom::Point(Point {
                location: Vec2::new(p[0], p[1]),
                style: 0,
                size: 0.0,
            }),
        }
    }
}

/// Geometry the format cannot carry, named so the caller can say so.
///
/// Silence here would mean a save that quietly loses work, which is the one
/// outcome worse than refusing to save at all.
#[derive(Debug, Clone, PartialEq)]
pub struct Unstorable {
    pub index: usize,
    pub what: &'static str,
}

fn describe(geom: &Geom) -> &'static str {
    match geom {
        Geom::Ellipse(_) => "an ellipse",
        Geom::EllipseArc(_) => "an elliptical arc",
        Geom::Spline(_) => "a spline",
        Geom::Hatch(_) => "a hatch",
        Geom::Text(_) => "text",
        Geom::Dimension(_) => "a dimension",
        Geom::BlockRef(_) => "a stamp",
        Geom::Wall(_) => "a wall",
        _ => "geometry of an unknown kind",
    }
}

/// Turn a layer into the blob that will be written.
pub fn to_blob(layer: &Layer) -> (String, Vec<Unstorable>) {
    let mut objects = Vec::new();
    let mut skipped = Vec::new();

    for (index, object) in layer.objects().iter().enumerate() {
        match StoredGeom::of(&object.geom) {
            Some(stored) => objects.push(stored),
            None => skipped.push(Unstorable { index, what: describe(&object.geom) }),
        }
    }

    let stored = StoredLayer {
        magic: MAGIC.to_string(),
        version: FORMAT_VERSION,
        page_height_pt: layer.space().height_pt(),
        objects,
    };

    // `expect` rather than `?`: these types have no serialisation that can
    // fail, and threading a Result through would suggest otherwise.
    (serde_json::to_string(&stored).expect("StoredLayer always serialises"), skipped)
}

/// Read a blob back, if it is ours and we understand its version.
///
/// Returns `Ok(None)` for a blob that is not ours — other tools write `restore`
/// blobs, and one that belongs to something else must be left alone.
pub fn from_blob(blob: &str) -> std::result::Result<Option<StoredLayer>, String> {
    let stored: StoredLayer = match serde_json::from_str(blob) {
        Ok(stored) => stored,
        Err(_) => return Ok(None),
    };

    if stored.magic != MAGIC {
        return Ok(None);
    }
    if stored.version > FORMAT_VERSION {
        return Err(format!(
            "this page's markup was written by a newer Pagify (format {}, this build reads {}). \
             Opening it read-only rather than rewriting it — saving would discard what this \
             build cannot represent.",
            stored.version, FORMAT_VERSION
        ));
    }
    Ok(Some(stored))
}

/// Rebuild a layer from a blob.
pub fn to_layer(stored: &StoredLayer) -> Layer {
    let mut layer = Layer::new(stored.page_height_pt);
    for geom in &stored.objects {
        layer.add_object(DObject::new(geom.to_geom()));
    }
    layer
}

// ---------------------------------------------------------------------------
// Appearance
// ---------------------------------------------------------------------------

/// Flatten geometry to polylines in **app space**, ready to be written as ink.
///
/// The appearance is flattened; the stored geometry is not. That split is the
/// point: a reader without Pagify sees a smooth-enough curve, and Pagify itself
/// still has the exact circle to fillet against.
pub fn flatten(geom: &Geom, space: PageSpace) -> Vec<Vec<PdfPoint>> {
    let to_app = |v: Vec2| {
        let p = space.from_kernel(v);
        PdfPoint { x: p.x as f32, y: p.y as f32 }
    };

    let arc_points = |centre: Vec2, radius: f64, start: f64, sweep: f64| {
        // Segment count from the sagitta: enough that the chord never departs
        // from the true curve by more than the tolerance.
        let steps = if radius <= FLATTEN_TOLERANCE_PT {
            4
        } else {
            let max_step = 2.0 * (1.0 - FLATTEN_TOLERANCE_PT / radius).clamp(-1.0, 1.0).acos();
            ((sweep.abs() / max_step.max(1e-6)).ceil() as usize).clamp(4, 720)
        };
        (0..=steps)
            .map(|i| {
                let t = start + sweep * (i as f64 / steps as f64);
                to_app(Vec2::new(centre.x + radius * t.cos(), centre.y + radius * t.sin()))
            })
            .collect::<Vec<_>>()
    };

    match geom {
        Geom::Line(l) => vec![vec![to_app(l.a), to_app(l.b)]],
        Geom::Circle(c) => {
            vec![arc_points(c.center, c.radius, 0.0, std::f64::consts::TAU)]
        }
        Geom::Arc(a) => vec![arc_points(a.center, a.radius, a.start_angle, a.sweep_angle)],
        Geom::Polyline(p) => {
            let mut points: Vec<PdfPoint> = p.vertices.iter().map(|v| to_app(v.pos)).collect();
            if p.closed {
                if let Some(first) = points.first().copied() {
                    points.push(first);
                }
            }
            vec![points]
        }
        Geom::Point(p) => {
            // A point has no extent. Drawn as a small cross, or it is invisible
            // in every reader that is not Pagify.
            let size = 2.0;
            let c = p.location;
            vec![
                vec![to_app(Vec2::new(c.x - size, c.y)), to_app(Vec2::new(c.x + size, c.y))],
                vec![to_app(Vec2::new(c.x, c.y - size)), to_app(Vec2::new(c.x, c.y + size))],
            ]
        }
        _ => Vec::new(),
    }
}

/// What a commit did.
#[derive(Debug, Clone, PartialEq)]
pub struct Committed {
    pub strokes_written: usize,
    pub objects_stored: usize,
    pub skipped: Vec<Unstorable>,
}

/// Write a page's markup into the document.
///
/// Idempotent: committing twice leaves one copy, because the carrier is removed
/// by id first and the ink it describes goes with it.
pub fn commit_page(
    session: &mut DocumentSession,
    page: usize,
    layer: &Layer,
    colour: Color,
    width: f32,
) -> Result<Committed> {
    // Reading and writing are split because `annotations` lives on `Document`
    // and the mutators on `DocumentMut`, and the two borrows cannot overlap.
    // Everything that has to be *read* about the previous commit is gathered
    // first, then the document is taken mutably once.
    let previous_blob = {
        let doc = session
            .document
            .as_document_mut()
            .ok_or(pdf_core::PdfError::Unsupported("editing this document"))?;
        doc.text_mark_restore(page, CARRIER_ID).ok()
    };

    let doomed = previous_ink_indices(&*session.document, page, layer.space(), previous_blob)?;
    let (blob, skipped) = to_blob(layer);

    let doc = session
        .document
        .as_document_mut()
        .ok_or(pdf_core::PdfError::Unsupported("editing this document"))?;

    // Clear what a previous commit left. `remove_text` is keyed on the carrier
    // id, so it finds the blob however the page has been edited since — §5.2's
    // "tag every written object with the layer's id".
    let _ = doc.remove_text(page, CARRIER_ID);
    // From the back: removing by index renumbers everything after it.
    for index in doomed.into_iter().rev() {
        let _ = doc.remove_annotation(page, index);
    }

    let mut strokes_written = 0;
    for object in layer.objects() {
        let strokes = flatten(&object.geom, layer.space());
        if strokes.is_empty() {
            continue;
        }
        strokes_written += strokes.len();
        doc.add_annotation(
            page,
            &Annotation::Ink { strokes, color: colour, width },
        )?;
    }

    // The carrier: one fully transparent space, which writes the marked-content
    // tag and the blob and draws nothing at all.
    doc.add_annotation(
        page,
        &Annotation::Text {
            text: " ".into(),
            font: "Helvetica".into(),
            font_asset: None,
            size: 1.0,
            color: Color { r: 0, g: 0, b: 0, a: 0 },
            glyphs: vec![Glyph { ch: " ".into(), id: 0, x: 0.0, y: 0.0, radians: 0.0 }],
            id: CARRIER_ID,
            restore: blob,
            frame: Vec::new(),
            frame_width: 0.0,
        },
    )?;

    Ok(Committed {
        strokes_written,
        objects_stored: layer.objects().len() - skipped.len(),
        skipped,
    })
}

/// Which annotations on this page are ink a previous commit wrote.
///
/// Identified by geometry rather than by index: indices shift as a document is
/// edited, and removing by a remembered index deletes whatever has since moved
/// into that slot — someone else's highlight. Anything that is not an exact
/// match for a stroke the previous blob describes is left alone, because
/// leaving a duplicate is recoverable and deleting a reviewer's mark is not.
fn previous_ink_indices(
    doc: &dyn pdf_core::document::Document,
    page: usize,
    space: PageSpace,
    previous_blob: Option<String>,
) -> Result<Vec<usize>> {
    let Some(previous) = previous_blob else { return Ok(Vec::new()) };
    let Ok(Some(stored)) = from_blob(&previous) else {
        return Ok(Vec::new());
    };

    let mut ours: Vec<Vec<PdfPoint>> = Vec::new();
    for geom in &stored.objects {
        ours.extend(flatten(&geom.to_geom(), space));
    }

    let mut doomed = Vec::new();
    for indexed in doc.annotations(page)? {
        if let Annotation::Ink { strokes, .. } = &indexed.annotation {
            if !strokes.is_empty()
                && strokes.iter().all(|s| ours.iter().any(|o| same_stroke(s, o)))
            {
                doomed.push(indexed.index);
            }
        }
    }
    doomed.sort_unstable();
    Ok(doomed)
}

fn same_stroke(a: &[PdfPoint], b: &[PdfPoint]) -> bool {
    // A tolerance, not equality: the points go out through PDFium and come back
    // through its own coordinate handling, and demanding bit equality of an f32
    // that has been round-tripped is how this silently stops matching.
    const NEAR: f32 = 0.01;
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(p, q)| (p.x - q.x).abs() < NEAR && (p.y - q.y).abs() < NEAR)
}

/// Read a page's markup back out of a document.
pub fn restore_page(
    doc: &dyn pdf_core::document::Document,
    page: usize,
) -> std::result::Result<Option<StoredLayer>, String> {
    let blobs = doc.text_marks(page).map_err(|e| e.to_string())?;
    for blob in blobs {
        if let Some(stored) = from_blob(&blob)? {
            return Ok(Some(stored));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layer_with_shapes() -> Layer {
        let mut layer = Layer::new(800.0);
        layer.add(Geom::Line(Line { a: Vec2::new(10.0, 20.0), b: Vec2::new(300.0, 400.0) }));
        layer.add(Geom::Circle(Circle { center: Vec2::new(200.0, 200.0), radius: 50.0 }));
        layer.add(Geom::Arc(Arc {
            center: Vec2::new(100.0, 100.0),
            radius: 30.0,
            start_angle: 0.5,
            sweep_angle: 1.2,
        }));
        layer.add(Geom::Polyline(Polyline {
            vertices: vec![
                PolyVertex { pos: Vec2::new(0.0, 0.0), bulge: 0.0 },
                PolyVertex { pos: Vec2::new(50.0, 0.0), bulge: 0.4 },
                PolyVertex { pos: Vec2::new(50.0, 50.0), bulge: 0.0 },
            ],
            closed: false,
            widths: Vec::new(),
        }));
        layer
    }

    #[test]
    fn geometry_survives_the_blob_exactly() {
        let layer = layer_with_shapes();
        let (blob, skipped) = to_blob(&layer);
        assert!(skipped.is_empty());

        let stored = from_blob(&blob).unwrap().expect("ours");
        let rebuilt = to_layer(&stored);

        assert_eq!(rebuilt.len(), layer.len());
        for (before, after) in layer.objects().iter().zip(rebuilt.objects()) {
            assert_eq!(
                StoredGeom::of(&before.geom),
                StoredGeom::of(&after.geom),
                "geometry changed through the blob"
            );
        }
    }

    #[test]
    fn a_curve_keeps_its_exact_centre_and_radius_rather_than_being_flattened() {
        // The distinction the whole phase rests on: the *appearance* is
        // flattened, the geometry is not. A circle that came back as 64 line
        // segments could never be filleted against again.
        let mut layer = Layer::new(800.0);
        layer.add(Geom::Circle(Circle { center: Vec2::new(123.5, 77.25), radius: 41.125 }));

        let (blob, _) = to_blob(&layer);
        let rebuilt = to_layer(&from_blob(&blob).unwrap().unwrap());

        match &rebuilt.objects()[0].geom {
            Geom::Circle(c) => {
                assert_eq!(c.center, Vec2::new(123.5, 77.25));
                assert_eq!(c.radius, 41.125);
            }
            other => panic!("a circle came back as {other:?}"),
        }
    }

    #[test]
    fn the_page_height_travels_with_the_geometry() {
        let layer = Layer::new(1234.5);
        let (blob, _) = to_blob(&layer);
        assert_eq!(from_blob(&blob).unwrap().unwrap().page_height_pt, 1234.5);
    }

    #[test]
    fn a_blob_that_is_not_ours_is_left_alone_rather_than_parsed() {
        assert_eq!(from_blob("{\"some\":\"other tool\"}").unwrap(), None);
        assert_eq!(from_blob("not json at all").unwrap(), None);
        assert_eq!(from_blob("").unwrap(), None);
    }

    #[test]
    fn a_newer_format_refuses_rather_than_silently_dropping_what_it_cannot_read() {
        let blob = serde_json::to_string(&StoredLayer {
            magic: MAGIC.into(),
            version: FORMAT_VERSION + 1,
            page_height_pt: 800.0,
            objects: Vec::new(),
        })
        .unwrap();

        let refused = from_blob(&blob).expect_err("must refuse");
        assert!(refused.contains("newer"), "unhelpful message: {refused}");
    }

    #[test]
    fn geometry_the_format_cannot_carry_is_reported_not_dropped_silently() {
        use cad_kernel::Text as KText;
        let mut layer = Layer::new(800.0);
        layer.add(Geom::Line(Line { a: Vec2::ZERO, b: Vec2::new(1.0, 1.0) }));
        layer.add(Geom::Text(KText::empty()));

        let (_, skipped) = to_blob(&layer);
        assert_eq!(skipped.len(), 1);
        assert_eq!(skipped[0].index, 1);
        assert_eq!(skipped[0].what, "text");
    }

    #[test]
    fn flattening_a_circle_closes_it_and_respects_the_tolerance() {
        let space = PageSpace::new(800.0);
        let circle = Geom::Circle(Circle { center: Vec2::new(100.0, 100.0), radius: 50.0 });
        let strokes = flatten(&circle, space);

        assert_eq!(strokes.len(), 1);
        let points = &strokes[0];
        assert!(points.len() > 16, "too coarse: {} points", points.len());

        let first = points.first().unwrap();
        let last = points.last().unwrap();
        assert!((first.x - last.x).abs() < 0.01 && (first.y - last.y).abs() < 0.01,
            "the ring does not close");

        // Every flattened point is on the circle, in app space.
        for p in points {
            let dx = p.x as f64 - 100.0;
            let dy = p.y as f64 - (800.0 - 100.0);
            assert!(((dx * dx + dy * dy).sqrt() - 50.0).abs() < 0.05, "point off the circle");
        }
    }

    #[test]
    fn flattening_puts_the_ink_in_app_space_not_kernel_space() {
        let space = PageSpace::new(800.0);
        let line = Geom::Line(Line { a: Vec2::new(10.0, 100.0), b: Vec2::new(20.0, 100.0) });
        let strokes = flatten(&line, space);

        assert_eq!(strokes[0][0].y, 700.0, "ink written in kernel space would be mirrored");
    }

    #[test]
    fn a_point_is_drawn_as_a_cross_so_it_is_visible_outside_pagify() {
        let strokes = flatten(
            &Geom::Point(Point { location: Vec2::new(50.0, 50.0), style: 0, size: 0.0 }),
            PageSpace::new(800.0),
        );
        assert_eq!(strokes.len(), 2, "a bare point would be invisible in any other reader");
    }
}
