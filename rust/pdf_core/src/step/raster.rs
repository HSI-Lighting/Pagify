//! Triangles into pixels.
//!
//! A software rasteriser with a depth buffer. No GPU, no new platform surface:
//! it writes into the same kind of buffer the PDF renderer already fills, so a
//! 3D view is another producer of pixels for a bitmap that Kotlin and Swift
//! already know how to display.
//!
//! # A depth buffer, not a depth sort
//!
//! Sorting triangles back to front and painting in order is the obvious cheap
//! answer and it is wrong on exactly the geometry a CAD part is made of: two
//! faces that pass through each other have no correct order, and neither does a
//! cycle of three. A depth buffer decides per pixel, has no such failure, and
//! costs one comparison and one float per pixel.
//!
//! # Flat shading
//!
//! One normal per triangle, which for machined geometry is what the surface
//! actually does: a chamfer *is* flat, and smoothing across it invents a
//! roundness the part does not have. Curved faces get their normals from the
//! surface rather than the triangle — see [`super::tessellate`] — so a cylinder
//! still shades smoothly without any per-vertex interpolation here.

use super::camera::Camera;
use super::model::Point3;
use super::tessellate::{Mesh, Triangle};

/// The picture, as bytes.
///
/// RGBA, one byte each, rows top to bottom with no padding. Converting to
/// whatever the platform wants happens where every other raster in this crate
/// is converted, rather than being a second thing this module has to know.
pub struct Canvas {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    depth: Vec<f32>,
}

impl Canvas {
    pub fn new(width: u32, height: u32) -> Self {
        let count = (width as usize) * (height as usize);
        Self {
            width,
            height,
            pixels: vec![0; count * 4],
            // Positive depth is distance from the eye, so everything starts
            // infinitely far away and the first triangle to arrive wins.
            depth: vec![f32::INFINITY; count],
        }
    }

    pub fn fill(&mut self, colour: [u8; 4]) {
        for pixel in self.pixels.chunks_exact_mut(4) {
            pixel.copy_from_slice(&colour);
        }
    }

    fn put(&mut self, x: u32, y: u32, depth: f32, colour: [u8; 4]) {
        let at = (y as usize) * (self.width as usize) + (x as usize);
        if depth >= self.depth[at] {
            return;
        }
        self.depth[at] = depth;
        self.pixels[at * 4..at * 4 + 4].copy_from_slice(&colour);
    }

    /// How far the nearest thing at a pixel is, for tests to look at.
    pub fn depth_at(&self, x: u32, y: u32) -> f32 {
        self.depth[(y as usize) * (self.width as usize) + (x as usize)]
    }

    pub fn colour_at(&self, x: u32, y: u32) -> [u8; 4] {
        let at = ((y as usize) * (self.width as usize) + (x as usize)) * 4;
        [
            self.pixels[at],
            self.pixels[at + 1],
            self.pixels[at + 2],
            self.pixels[at + 3],
        ]
    }

    /// How many pixels were drawn on at all.
    pub fn covered(&self) -> usize {
        self.depth.iter().filter(|d| d.is_finite()).count()
    }
}

/// How the part is lit and coloured.
#[derive(Debug, Clone, Copy)]
pub struct Style {
    pub background: [u8; 4],
    pub material: [u8; 3],
    /// Where the light comes from, in view space. Kept relative to the eye so
    /// the part stays lit as it turns, rather than swinging into shadow.
    pub light: Point3,
    /// How much light a surface facing away still receives. Without it the far
    /// side of a part is pure black and reads as a hole rather than a shadow.
    pub ambient: f64,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            background: [30, 32, 36, 255],
            material: [176, 182, 190],
            // Over the viewer's left shoulder: the convention that makes a
            // shape read as solid rather than lit from nowhere.
            light: Point3::new(-0.4, 0.6, 1.0),
            ambient: 0.28,
        }
    }
}

/// Draw a mesh.
pub fn draw(mesh: &Mesh, camera: &Camera, style: &Style, canvas: &mut Canvas) {
    canvas.fill(style.background);

    let light = style
        .light
        .normalised()
        .unwrap_or(Point3::new(0.0, 0.0, 1.0));
    let (right, up, back) = camera.axes();

    // Nothing nearer than this is drawn: a triangle straddling the eye plane
    // projects to infinity and paints the whole screen.
    let near = (camera.distance * 1e-4).max(1e-6);

    for triangle in &mesh.triangles {
        let normal = Point3::new(
            triangle.normal.dot(right),
            triangle.normal.dot(up),
            triangle.normal.dot(back),
        );

        // **Facing away, so not drawn.** With a closed solid this halves the
        // work and removes the far wall from behind the near one. A part whose
        // faces are inverted disappears here rather than shading oddly, which
        // is why the winding is settled in the tessellator with a test.
        if normal.z <= 0.0 {
            continue;
        }

        let corners = [triangle.a, triangle.b, triangle.c].map(|p| camera.to_view(p));
        if corners.iter().any(|c| -c.z <= near) {
            continue;
        }

        let shade = (normal.dot(light).max(0.0) * (1.0 - style.ambient)) + style.ambient;
        let colour = [
            (style.material[0] as f64 * shade).round().clamp(0.0, 255.0) as u8,
            (style.material[1] as f64 * shade).round().clamp(0.0, 255.0) as u8,
            (style.material[2] as f64 * shade).round().clamp(0.0, 255.0) as u8,
            255,
        ];

        let screen: Vec<(f64, f64, f32)> = corners
            .iter()
            .map(|corner| {
                let (x, y) = project(*corner, camera, canvas.width, canvas.height);
                (x, y, (-corner.z) as f32)
            })
            .collect();

        fill_triangle(canvas, &screen, colour);
    }
}

/// A view-space point on the screen, in pixels.
fn project(view: Point3, camera: &Camera, width: u32, height: u32) -> (f64, f64) {
    let depth = -view.z;
    let half_height = (camera.fov / 2.0).tan();
    let scale = (height as f64 / 2.0) / (half_height * depth);

    (
        width as f64 / 2.0 + view.x * scale,
        // Screen rows run down, the world's up runs up.
        height as f64 / 2.0 - view.y * scale,
    )
}

/// One triangle, by scanning the rows it covers.
fn fill_triangle(canvas: &mut Canvas, corners: &[(f64, f64, f32)], colour: [u8; 4]) {
    let (ax, ay, az) = corners[0];
    let (bx, by, bz) = corners[1];
    let (cx, cy, cz) = corners[2];

    let area = (bx - ax) * (cy - ay) - (cx - ax) * (by - ay);
    if area.abs() < 1e-12 {
        return; // edge-on, so it covers nothing
    }

    let low_x = ax.min(bx).min(cx).floor().max(0.0) as i64;
    let high_x = ax.max(bx).max(cx).ceil().min(canvas.width as f64) as i64;
    let low_y = ay.min(by).min(cy).floor().max(0.0) as i64;
    let high_y = ay.max(by).max(cy).ceil().min(canvas.height as f64) as i64;

    for y in low_y..high_y {
        for x in low_x..high_x {
            // Sampled at the middle of the pixel, so a triangle edge falling
            // exactly on a boundary belongs to one side or the other rather
            // than to both, which would leave a seam of double-drawn pixels.
            let px = x as f64 + 0.5;
            let py = y as f64 + 0.5;

            let w0 = ((bx - px) * (cy - py) - (cx - px) * (by - py)) / area;
            let w1 = ((cx - px) * (ay - py) - (ax - px) * (cy - py)) / area;
            let w2 = 1.0 - w0 - w1;

            if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                continue;
            }

            let depth = (w0 as f32) * az + (w1 as f32) * bz + (w2 as f32) * cz;
            canvas.put(x as u32, y as u32, depth, colour);
        }
    }
}

/// The whole job: fit a mesh to a canvas and draw it.
pub fn draw_fitted(mesh: &Mesh, canvas: &mut Canvas, style: &Style) -> Option<Camera> {
    let (low, high) = mesh.bounds()?;
    let camera = Camera::fit(low, high, 45.0_f64.to_radians());
    draw(mesh, &camera, style, canvas);
    Some(camera)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::step::model::Point3;

    fn triangle(a: Point3, b: Point3, c: Point3, normal: Point3) -> Triangle {
        Triangle { a, b, c, normal }
    }

    /// A square facing the camera, one unit each way, at a given depth.
    fn facing_square(z: f64) -> Mesh {
        let towards = Point3::new(0.0, 0.0, 1.0);
        Mesh {
            triangles: vec![
                triangle(
                    Point3::new(-1.0, -1.0, z),
                    Point3::new(1.0, -1.0, z),
                    Point3::new(1.0, 1.0, z),
                    towards,
                ),
                triangle(
                    Point3::new(-1.0, -1.0, z),
                    Point3::new(1.0, 1.0, z),
                    Point3::new(-1.0, 1.0, z),
                    towards,
                ),
            ],
            skipped: Vec::new(),
        }
    }

    /// Straight on from +z, so a square facing +z faces the eye.
    fn head_on(distance: f64) -> Camera {
        Camera {
            target: Point3::new(0.0, 0.0, 0.0),
            distance,
            yaw: 0.0,
            pitch: std::f64::consts::FRAC_PI_2 - 1e-9,
            fov: 45.0_f64.to_radians(),
        }
    }

    // ---- something is drawn --------------------------------------------------

    #[test]
    fn a_facing_square_covers_the_middle_of_the_canvas() {
        let mut canvas = Canvas::new(64, 64);
        draw(&facing_square(0.0), &head_on(10.0), &Style::default(), &mut canvas);

        assert!(canvas.depth_at(32, 32).is_finite(), "nothing in the middle");
        assert!(canvas.covered() > 100, "only {} pixels drawn", canvas.covered());
    }

    #[test]
    fn an_empty_mesh_leaves_the_background() {
        let style = Style::default();
        let mut canvas = Canvas::new(16, 16);
        draw(&Mesh::default(), &head_on(10.0), &style, &mut canvas);

        assert_eq!(style.background, canvas.colour_at(8, 8));
        assert_eq!(0, canvas.covered());
    }

    // ---- the depth buffer ----------------------------------------------------

    /// The nearer surface wins whichever order the triangles arrive in.
    ///
    /// This is the whole reason for a depth buffer rather than a back-to-front
    /// sort. Drawn in the wrong order, a painter's algorithm shows the far
    /// surface through the near one.
    #[test]
    fn the_nearer_surface_wins_whatever_the_order() {
        let style = Style::default();
        let near_first = {
            let mut mesh = facing_square(2.0);
            mesh.triangles.extend(facing_square(-2.0).triangles);
            mesh
        };
        let far_first = {
            let mut mesh = facing_square(-2.0);
            mesh.triangles.extend(facing_square(2.0).triangles);
            mesh
        };

        let mut one = Canvas::new(32, 32);
        let mut other = Canvas::new(32, 32);
        draw(&near_first, &head_on(10.0), &style, &mut one);
        draw(&far_first, &head_on(10.0), &style, &mut other);

        assert_eq!(
            one.depth_at(16, 16),
            other.depth_at(16, 16),
            "the result depended on the order the triangles were given in",
        );
        // And it is the near one: eight away, not twelve.
        assert!((one.depth_at(16, 16) - 8.0).abs() < 0.1, "{}", one.depth_at(16, 16));
    }

    /// Two surfaces passing through each other have no correct order at all.
    ///
    /// A sort has to pick one, and picks wrong for half the pixels. Each pixel
    /// is decided on its own here, so the crossing comes out right.
    #[test]
    fn interpenetrating_surfaces_are_resolved_per_pixel() {
        let mut mesh = facing_square(0.0);
        // A second square, tilted so it passes through the first.
        mesh.triangles.push(triangle(
            Point3::new(-1.0, -1.0, -1.0),
            Point3::new(1.0, -1.0, 1.0),
            Point3::new(1.0, 1.0, 1.0),
            Point3::new(0.0, 0.0, 1.0),
        ));

        let mut canvas = Canvas::new(64, 64);
        draw(&mesh, &head_on(10.0), &Style::default(), &mut canvas);

        let mut depths = Vec::new();
        for x in 20..44 {
            let depth = canvas.depth_at(x, 32);
            if depth.is_finite() {
                depths.push(depth);
            }
        }
        let nearest = depths.iter().cloned().fold(f32::INFINITY, f32::min);
        let furthest = depths.iter().cloned().fold(f32::NEG_INFINITY, f32::max);

        assert!(
            furthest - nearest > 0.2,
            "the crossing was flattened to one depth: {nearest} to {furthest}",
        );
    }

    // ---- facing away ---------------------------------------------------------

    /// A triangle facing away is not drawn.
    #[test]
    fn a_back_face_is_skipped() {
        let mut mesh = facing_square(0.0);
        for triangle in &mut mesh.triangles {
            triangle.normal = Point3::new(0.0, 0.0, -1.0);
        }

        let mut canvas = Canvas::new(32, 32);
        draw(&mesh, &head_on(10.0), &Style::default(), &mut canvas);

        assert_eq!(0, canvas.covered(), "a face pointing away was drawn");
    }

    // ---- shading -------------------------------------------------------------

    /// A surface square to the light is brighter than one turned away.
    #[test]
    fn shading_follows_the_normal() {
        let style = Style::default();
        let mut lit = Canvas::new(32, 32);
        draw(&facing_square(0.0), &head_on(10.0), &style, &mut lit);

        let mut tilted_mesh = facing_square(0.0);
        for triangle in &mut tilted_mesh.triangles {
            triangle.normal = Point3::new(0.8, 0.0, 0.6).normalised().expect("a normal");
        }
        let mut tilted = Canvas::new(32, 32);
        draw(&tilted_mesh, &head_on(10.0), &style, &mut tilted);

        assert_ne!(
            lit.colour_at(16, 16),
            tilted.colour_at(16, 16),
            "turning a face changed nothing about how it is lit",
        );
    }

    /// Nothing is ever pure black, so a shape is never mistaken for a hole.
    #[test]
    fn a_face_turned_from_the_light_is_still_visible() {
        let style = Style::default();
        let mut mesh = facing_square(0.0);
        for triangle in &mut mesh.triangles {
            // Facing the eye, but as far from the light as that allows.
            triangle.normal = Point3::new(0.0, 0.0, 1.0);
        }

        let mut canvas = Canvas::new(32, 32);
        draw(&mesh, &head_on(10.0), &style, &mut canvas);

        let colour = canvas.colour_at(16, 16);
        assert!(colour[0] > 0 || colour[1] > 0 || colour[2] > 0, "pure black");
        assert_ne!(style.background, colour, "indistinguishable from the background");
    }

    // ---- the eye plane -------------------------------------------------------

    /// A triangle behind the eye does not paint the whole screen.
    ///
    /// Projection divides by depth, so a corner at or behind the eye plane
    /// gives coordinates near infinity — and a bounding box covering the canvas
    /// is filled with a single triangle. It looks like a total rendering
    /// failure and is one badly placed vertex.
    #[test]
    fn geometry_behind_the_eye_is_dropped_rather_than_smeared() {
        let camera = head_on(10.0);
        let behind = camera.eye().plus(Point3::new(0.0, 0.0, 5.0));
        let mesh = Mesh {
            triangles: vec![triangle(
                behind,
                behind.plus(Point3::new(1.0, 0.0, 0.0)),
                behind.plus(Point3::new(0.0, 1.0, 0.0)),
                Point3::new(0.0, 0.0, 1.0),
            )],
            skipped: Vec::new(),
        };

        let mut canvas = Canvas::new(32, 32);
        draw(&mesh, &camera, &Style::default(), &mut canvas);

        assert!(canvas.covered() < 32 * 32, "the screen was smeared");
    }

    // ---- fitting -------------------------------------------------------------

    /// A fitted part is on screen, and not up against the edge.
    #[test]
    fn a_fitted_mesh_lands_inside_the_canvas() {
        let mut canvas = Canvas::new(64, 64);
        let drawn = draw_fitted(&facing_square(0.0), &mut canvas, &Style::default());

        assert!(drawn.is_some());
        assert!(canvas.depth_at(32, 32).is_finite(), "nothing in the middle");

        // The border is untouched, so the part is not running off the screen.
        for along in 0..64 {
            assert!(!canvas.depth_at(along, 0).is_finite(), "touching the top");
            assert!(!canvas.depth_at(0, along).is_finite(), "touching the left");
        }
    }

    #[test]
    fn an_empty_mesh_cannot_be_fitted() {
        let mut canvas = Canvas::new(16, 16);
        assert!(draw_fitted(&Mesh::default(), &mut canvas, &Style::default()).is_none());
    }

    // ---- coverage doesn't leak ----------------------------------------------

    /// Nothing is written outside the canvas, whatever the geometry.
    #[test]
    fn an_enormous_triangle_stays_inside_the_buffer() {
        let mesh = Mesh {
            triangles: vec![triangle(
                Point3::new(-1e6, -1e6, 0.0),
                Point3::new(1e6, -1e6, 0.0),
                Point3::new(0.0, 1e6, 0.0),
                Point3::new(0.0, 0.0, 1.0),
            )],
            skipped: Vec::new(),
        };

        let mut canvas = Canvas::new(16, 16);
        // Would panic on an out-of-bounds write rather than failing an assert.
        draw(&mesh, &head_on(10.0), &Style::default(), &mut canvas);
        assert_eq!(16 * 16 * 4, canvas.pixels.len());
    }
}
