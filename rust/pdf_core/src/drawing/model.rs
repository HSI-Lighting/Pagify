//! What a drawing is, once it has been read.
//!
//! **Deliberately small.** A CAD kernel models a drawing so it can be edited:
//! intersections, trimming, snapping, dimension styles, hatch resolution. None
//! of that is needed to look at one, and pulling in seventeen thousand lines of
//! it would tie this app to a working copy of another project on one machine.
//! What a viewer needs is the shapes, which layer each belongs to, and what one
//! drawing unit means — and that is what is here.

/// A point on the sheet. Drawing units, whatever the file says those are.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// One vertex of a polyline, and how the segment leaving it bows.
///
/// `bulge` is DXF's own measure: the tangent of a quarter of the segment's
/// included angle, signed anticlockwise, zero for a straight run. Kept rather
/// than flattened here because how finely to flatten it depends on the zoom,
/// which is not known until something is drawn.
#[derive(Debug, Clone, Copy)]
pub struct Vertex {
    pub at: Point,
    pub bulge: f64,
}

/// Everything this can draw.
///
/// A circle is an [`Arc`](Shape::Arc) of a full turn rather than its own
/// variant: one fewer case in every match, and the renderer would treat them
/// identically anyway.
#[derive(Debug, Clone)]
pub enum Shape {
    Line {
        a: Point,
        b: Point,
    },
    Arc {
        centre: Point,
        radius: f64,
        /// Where it starts, in radians anticlockwise from +x.
        start: f64,
        /// How far it runs, anticlockwise. A full turn is a circle.
        sweep: f64,
    },
    /// `major` is the semi-major axis **as a vector from the centre**, which is
    /// how both DXF and DWG store it, so no conversion can get it wrong.
    Ellipse {
        centre: Point,
        major: Point,
        /// Minor axis as a fraction of the major.
        ratio: f64,
        start: f64,
        sweep: f64,
    },
    Polyline {
        vertices: Vec<Vertex>,
        closed: bool,
    },
}

/// A shape, and which layer it belongs to.
#[derive(Debug, Clone)]
pub struct Entity {
    /// Index into [`Drawing::layers`].
    pub layer: u16,
    pub shape: Shape,
}

/// A layer, as far as looking at a drawing is concerned.
#[derive(Debug, Clone)]
pub struct Layer {
    pub name: String,
    /// The AutoCAD colour index, resolved to something to draw with.
    pub colour: [u8; 3],
    /// Layers can be turned off in the file itself, and a drawing shown with
    /// its off layers on is not the drawing somebody saved.
    pub visible: bool,
}

/// Something the file holds that is not on screen, and why.
///
/// Counted rather than passed over, for the reason the STEP side learned the
/// hard way: a drawing that quietly lost its fixtures looks exactly like a
/// drawing that never had any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    pub what: String,
    pub count: usize,
}

/// What one drawing unit means in metres, and whether the file said so.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Units {
    pub metres_per_unit: f64,
    /// **Declared, or assumed.** A drawing in millimetres read as metres puts
    /// everything at a thousandth of its size. The difference between the file
    /// saying so and this guessing is worth carrying, because only one of them
    /// can be trusted.
    pub declared: bool,
}

impl Default for Units {
    fn default() -> Self {
        Self { metres_per_unit: 1.0, declared: false }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Drawing {
    pub entities: Vec<Entity>,
    pub layers: Vec<Layer>,
    pub units: Units,
    pub skipped: Vec<Skipped>,
}

impl Drawing {
    /// Find a layer by name, or make one.
    ///
    /// Drawings name a layer on every entity and do not always declare it in
    /// the table first, so this has to be able to invent one.
    pub fn layer_for(&mut self, name: &str) -> u16 {
        if let Some(at) = self.layers.iter().position(|l| l.name == name) {
            return at as u16;
        }
        self.layers.push(Layer {
            name: name.to_string(),
            colour: [220, 220, 220],
            visible: true,
        });
        (self.layers.len() - 1) as u16
    }

    pub fn note(&mut self, what: &str) {
        match self.skipped.iter_mut().find(|s| s.what == what) {
            Some(already) => already.count += 1,
            None => self.skipped.push(Skipped { what: what.to_string(), count: 1 }),
        }
    }

    /// The box everything fits in, or `None` for a drawing with nothing in it.
    pub fn bounds(&self) -> Option<(Point, Point)> {
        let mut low = Point::new(f64::MAX, f64::MAX);
        let mut high = Point::new(f64::MIN, f64::MIN);
        let mut any = false;

        let mut widen = |p: Point| {
            low.x = low.x.min(p.x);
            low.y = low.y.min(p.y);
            high.x = high.x.max(p.x);
            high.y = high.y.max(p.y);
            any = true;
        };

        for entity in &self.entities {
            match &entity.shape {
                Shape::Line { a, b } => {
                    widen(*a);
                    widen(*b);
                }
                // The whole circle, not the arc's own extent. Cheap, never too
                // small, and a fitted view that is slightly loose is a great
                // deal better than one that crops the drawing.
                Shape::Arc { centre, radius, .. } => {
                    widen(Point::new(centre.x - radius, centre.y - radius));
                    widen(Point::new(centre.x + radius, centre.y + radius));
                }
                Shape::Ellipse { centre, major, ratio, .. } => {
                    let reach = major.x.hypot(major.y).max(major.x.hypot(major.y) * ratio.abs());
                    widen(Point::new(centre.x - reach, centre.y - reach));
                    widen(Point::new(centre.x + reach, centre.y + reach));
                }
                Shape::Polyline { vertices, .. } => {
                    for vertex in vertices {
                        widen(vertex.at);
                    }
                }
            }
        }

        any.then_some((low, high))
    }

    /// How many of the file's shapes are actually drawn.
    pub fn kept(&self) -> usize {
        self.entities.len()
    }

    /// How many were not, all reasons together.
    pub fn lost(&self) -> usize {
        self.skipped.iter().map(|s| s.count).sum()
    }
}

/// Metres in one drawing unit for an AutoCAD `$INSUNITS` code.
///
/// `None` where the file declares nothing usable. **0 is "unitless"** — an
/// explicit absence of a claim rather than a claim of metres — and the exotic
/// codes (angstroms, parsecs, survey feet) are left out on purpose: a drawing
/// that really is in parsecs is not one this app can help with, and a wrong
/// guess beats no guess only if it happens to be right.
///
/// The same table serves DXF and DWG, because a drawing that means one thing in
/// one format and another in the other is its own class of bug.
pub fn metres_per_unit(code: i32) -> Option<f64> {
    Some(match code {
        1 => 0.0254,        // inches
        2 => 0.3048,        // feet
        3 => 1609.344,      // miles
        4 => 0.001,         // millimetres
        5 => 0.01,          // centimetres
        6 => 1.0,           // metres
        7 => 1000.0,        // kilometres
        9 => 0.0000254,     // mils
        10 => 0.9144,       // yards
        14 => 0.1,          // decimetres
        15 => 10.0,         // decametres
        16 => 100.0,        // hectometres
        17 => 1_000_000.0,  // gigametres — listed because AutoCAD does
        _ => return None,
    })
}

/// The colour an AutoCAD colour index draws as.
///
/// Only the seven fixed ones are spelled out; the rest of the 255 are a palette
/// this does not carry, and a mid grey for them is honest about that. Index 7
/// is "white or black, whichever contrasts with the background" — on a dark
/// sheet that is white.
pub fn aci_colour(index: i32) -> [u8; 3] {
    match index {
        1 => [255, 80, 80],
        2 => [255, 255, 100],
        3 => [110, 240, 110],
        4 => [110, 235, 235],
        5 => [120, 150, 255],
        6 => [255, 120, 255],
        7 => [230, 230, 230],
        _ => [190, 190, 190],
    }
}
