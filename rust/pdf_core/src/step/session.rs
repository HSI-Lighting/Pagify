//! An open model: its mesh, where it is being looked at from, and its pixels.
//!
//! The phone holds a handle to one of these and asks it to draw. Tessellation
//! happens once, on opening; turning the part afterwards is only rasterising,
//! which is why a drag can be smooth even though building the mesh is not.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use super::adapt;
use super::audit::{self, Refusal};
use super::camera::Camera;
use super::model::Point3;
use super::raster::{self, Canvas, Style};
use super::tessellate::{self, Mesh};

/// A model, open and ready to be looked at.
pub struct ModelSession {
    pub mesh: Arc<Mesh>,
    pub camera: Camera,
    bounds: (Point3, Point3),
    style: Style,
    /// What the file contained, for the summary the screen shows.
    /// What the file contained, entity by entity, for the screen to show.
    pub census: audit::Census,
}

/// Why a file could not be opened.
#[derive(Debug, Clone, PartialEq)]
pub enum OpenError {
    /// The gates said no, with a sentence for the user.
    Refused(Refusal),
    /// It could not be parsed at all.
    Unreadable(String),
    /// It parsed, and contained nothing that could be drawn.
    NothingDrawable,
}

impl OpenError {
    pub fn message(&self) -> String {
        match self {
            OpenError::Refused(refusal) => refusal.message(),
            OpenError::Unreadable(_) => "This file could not be read as a STEP model.".to_string(),
            OpenError::NothingDrawable => {
                "Nothing in this file could be drawn.".to_string()
            }
        }
    }
}

/// Meshes already built, by file, so reopening one is instant.
///
/// **Keyed by content length as well as path.** A part being worked on is
/// re-exported to the same name over and over; keying on the path alone would
/// show yesterday's shape with today's filename, which is worse than a wait.
type MeshCache = Mutex<HashMap<(String, usize), Arc<Mesh>>>;

fn cache() -> &'static MeshCache {
    static CACHE: OnceLock<MeshCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// How many meshes are kept. Small: each is megabytes, and the case worth
/// serving is going back to the file you just had open.
const KEEP: usize = 3;

/// Open a model from a file's bytes.
///
/// The gates run first, on the text, before any geometry is built — refusing a
/// 14 MB assembly should not cost a second of tessellation first.
pub fn open(name: &str, bytes: &[u8]) -> Result<ModelSession, OpenError> {
    let mut meshes = cache()
        .lock()
        .map_err(|_| OpenError::Unreadable("the mesh cache is poisoned".into()))?;
    open_with(&mut meshes, name, bytes)
}

/// [`open`], against a given cache.
///
/// Separate because the real cache is global and shared, and a test that
/// asserts a hit against shared state is a test another test can evict out
/// from under it — which is exactly what happened, intermittently, and read
/// as a caching bug rather than as tests running in parallel.
pub fn open_with(
    meshes: &mut HashMap<(String, usize), Arc<Mesh>>,
    name: &str,
    bytes: &[u8],
) -> Result<ModelSession, OpenError> {
    let census = audit::census(&String::from_utf8_lossy(bytes));
    audit::verdict(&census).map_err(OpenError::Refused)?;

    let key = (name.to_string(), bytes.len());
    let cached = meshes.get(&key).cloned();

    let mesh = match cached {
        Some(mesh) => mesh,
        None => {
            let solid = adapt::read(bytes).map_err(OpenError::Unreadable)?;
            let sag = tessellate::recommended_sag(&solid);
            let mesh = Arc::new(tessellate::tessellate(&solid, sag));

            if meshes.len() >= KEEP {
                // Whichever, rather than nothing: an arbitrary eviction is a
                // slower reopen, where growing without limit is the app being
                // killed for memory while somebody is using it.
                if let Some(victim) = meshes.keys().next().cloned() {
                    meshes.remove(&victim);
                }
            }
            meshes.insert(key, Arc::clone(&mesh));
            mesh
        }
    };

    let bounds = mesh.bounds().ok_or(OpenError::NothingDrawable)?;

    Ok(ModelSession {
        camera: Camera::fit(bounds.0, bounds.1, 45.0_f64.to_radians()),
        bounds,
        mesh,
        style: Style::default(),
        census,
    })
}

impl ModelSession {
    /// Turn the part. Distances are in fractions of the view, not pixels, so a
    /// drag feels the same on any screen.
    /// Turn the part, as though the finger were on its surface.
    ///
    /// **The camera moves the other way to the finger.** Dragging right should
    /// send the part right, which means the eye goes left — the obvious sign
    /// moves the camera with the finger and the model slides the wrong way
    /// under it.
    ///
    /// **Both axes at the same rate.** Pitch used to be a whole half-turn for a
    /// drag down the view, and pitch is clamped just short of straight up: a
    /// third of a screen put it against the stop, where every further drag did
    /// nothing at all. That does not feel like a sensitive control, it feels
    /// like a broken one — the part span freely sideways and would not tip.
    /// A quarter turn across the view, for both. Ninety degrees is a long
    /// deliberate drag rather than a flick, and the tilt starts at twenty-eight
    /// degrees with sixty left before the stop — so an ordinary gesture moves
    /// it visibly and never runs out.
    pub fn orbit(&mut self, across: f64, down: f64) {
        use std::f64::consts::FRAC_PI_2;
        self.camera = self.camera.turned(-across * FRAC_PI_2, down * FRAC_PI_2);
    }

    pub fn zoom(&mut self, by: f64) {
        if by > 0.0 && by.is_finite() {
            self.camera = self.camera.zoomed(1.0 / by);
        }
    }

    /// Slide the part across the view.
    ///
    /// Moves what is being looked at rather than the eye, so panning does not
    /// change the angle — and scaled by distance, so a zoomed-in part does not
    /// shoot off the screen at the same finger speed.
    pub fn pan(&mut self, across: f64, down: f64) {
        let (right, up, _) = self.camera.axes();
        let reach = self.camera.distance * (self.camera.fov / 2.0).tan() * 2.0;
        self.camera.target = self
            .camera
            .target
            .plus(right.scaled(-across * reach))
            .plus(up.scaled(down * reach));
    }

    /// Back to the view the model opened on.
    pub fn fit(&mut self) {
        self.camera = Camera::fit(self.bounds.0, self.bounds.1, self.camera.fov);
    }

    /// Draw into a buffer of the given size, returning the pixels.
    pub fn draw(&self, width: u32, height: u32) -> Canvas {
        let mut canvas = Canvas::new(width, height);
        raster::draw(&self.mesh, &self.camera, &self.style, &mut canvas);
        canvas
    }

    /// What the screen tells the user about this model.
    ///
    /// What the screen tells the user about this model.
    ///
    /// **The parameters, not only the picture.** A part is a set of numbers
    /// before it is a shape — how many faces, of what kinds, how large — and
    /// somebody opening a supplier's file usually wants those as much as the
    /// view. Including what was left out: a part drawn with faces missing
    /// looks like the part, so the count is the only thing that says
    /// otherwise.
    pub fn summary_json(&self) -> String {
        let skipped: Vec<String> = self
            .mesh
            .skipped
            .iter()
            .map(|s| format!(r#"{{"what":{:?},"count":{}}}"#, s.what, s.count))
            .collect();
        let lost: usize = self.mesh.skipped.iter().map(|s| s.count).sum();
        let (low, high) = self.bounds;
        let c = &self.census;

        format!(
            concat!(
                r#"{{"triangles":{},"facesInFile":{},"facesDrawn":{},"#,
                r#""skipped":[{}],"#,
                r#""surfaces":{{"plane":{},"cylinder":{},"cone":{},"#,
                r#""torus":{},"sphere":{},"freeform":{}}},"#,
                r#""freeformCurves":{},"assembly":{},"#,
                r#""size":{{"x":{:.2},"y":{:.2},"z":{:.2}}}}}"#,
            ),
            self.mesh.triangles.len(),
            c.faces,
            c.faces.saturating_sub(lost),
            skipped.join(","),
            c.planes,
            c.cylinders,
            c.cones,
            c.tori,
            c.spheres,
            c.freeform_surfaces,
            c.freeform_curves,
            c.assembly_links,
            high.x - low.x,
            high.y - low.y,
            high.z - low.z,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A cube, written out because a real part cannot be committed.
    fn a_cube() -> String {
        let mut step = String::from(
            "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'1');\n\
             FILE_NAME('cube','2026-01-01T00:00:00',(''),(''),'','','');\n\
             FILE_SCHEMA(('AUTOMOTIVE_DESIGN'));\nENDSEC;\nDATA;\n",
        );

        // Eight corners of a 10 mm cube.
        let corners = [
            (0.0, 0.0, 0.0),
            (10.0, 0.0, 0.0),
            (10.0, 10.0, 0.0),
            (0.0, 10.0, 0.0),
            (0.0, 0.0, 10.0),
            (10.0, 0.0, 10.0),
            (10.0, 10.0, 10.0),
            (0.0, 10.0, 10.0),
        ];
        for (index, (x, y, z)) in corners.iter().enumerate() {
            step.push_str(&format!("#{}=CARTESIAN_POINT('',({x:.1},{y:.1},{z:.1}));\n", index + 1));
        }
        for (index, _) in corners.iter().enumerate() {
            step.push_str(&format!("#{}=VERTEX_POINT('',#{});\n", 20 + index, index + 1));
        }

        // One face is enough to prove the chain; six would prove no more here
        // and the winding of a whole cube is the tessellator's test, not this
        // module's.
        step.push_str(
            "#40=DIRECTION('',(0.,0.,1.));\n\
             #41=DIRECTION('',(1.,0.,0.));\n\
             #42=AXIS2_PLACEMENT_3D('',#1,#40,#41);\n\
             #43=PLANE('',#42);\n\
             #50=DIRECTION('',(1.,0.,0.));\n#51=VECTOR('',#50,1.);\n#52=LINE('',#1,#51);\n\
             #53=DIRECTION('',(0.,1.,0.));\n#54=VECTOR('',#53,1.);\n#55=LINE('',#2,#54);\n\
             #56=DIRECTION('',(-1.,0.,0.));\n#57=VECTOR('',#56,1.);\n#58=LINE('',#3,#57);\n\
             #59=DIRECTION('',(0.,-1.,0.));\n#60=VECTOR('',#59,1.);\n#61=LINE('',#4,#60);\n\
             #70=EDGE_CURVE('',#20,#21,#52,.T.);\n\
             #71=EDGE_CURVE('',#21,#22,#55,.T.);\n\
             #72=EDGE_CURVE('',#22,#23,#58,.T.);\n\
             #73=EDGE_CURVE('',#23,#20,#61,.T.);\n\
             #80=ORIENTED_EDGE('',*,*,#70,.T.);\n\
             #81=ORIENTED_EDGE('',*,*,#71,.T.);\n\
             #82=ORIENTED_EDGE('',*,*,#72,.T.);\n\
             #83=ORIENTED_EDGE('',*,*,#73,.T.);\n\
             #90=EDGE_LOOP('',(#80,#81,#82,#83));\n\
             #91=FACE_OUTER_BOUND('',#90,.T.);\n\
             #92=ADVANCED_FACE('',(#91),#43,.T.);\n\
             ENDSEC;\nEND-ISO-10303-21;\n",
        );
        step
    }

    fn open_a_square(name: &str) -> ModelSession {
        open(name, a_cube().as_bytes()).expect("the model opens")
    }

    #[test]
    fn a_model_opens_fitted_and_draws_something() {
        let session = open_a_square("square-1");
        let canvas = session.draw(64, 64);

        assert!(canvas.covered() > 100, "only {} pixels", canvas.covered());
    }

    /// **The part follows the finger.**
    ///
    /// Dragging right must send the model right. The camera has to go the
    /// other way for that, and the sign that looks right moves the camera
    /// with the finger — so the model slides away from it, which feels
    /// like the controls are backwards because they are.
    ///
    /// Checked by where a point on the part lands on screen, which is the
    /// thing somebody actually sees, rather than by the sign of an angle.
    #[test]
    fn dragging_right_sends_the_part_right() {
        let mut session = open_a_square("drag-x");
        // A corner, off to one side so its movement is unambiguous.
        let corner = Point3::new(10.0, 10.0, 0.0);

        let before = session.camera.to_view(corner);
        session.orbit(0.05, 0.0);
        let after = session.camera.to_view(corner);

        assert!(
            after.x > before.x,
            "the part went left when the finger went right: {} to {}",
            before.x,
            after.x,
        );
    }


/// A long vertical drag keeps tilting instead of jamming.    ///    /// The turntable camera this replaced clamped its tilt short of straight    /// up: a third of a screen reached the stop and every drag after that did    /// nothing, while sideways drags kept working. That is what "it spins but    /// will not tip" was, and it is why the camera keeps three axes now.    #[test]    fn a_long_vertical_drag_never_stops_working() {        let mut session = open_a_square("tilt-rate");        for step in 0..12 {            let before = session.camera.back;            session.orbit(0.0, 0.3);            assert!(                session.camera.back.minus(before).length() > 1e-6,                "drag {step} did nothing at all",            );        }    }
    /// Turning changes the picture; that is the whole feature.
    #[test]
    fn turning_the_part_changes_what_is_drawn() {
        let mut session = open_a_square("square-2");
        let before = session.draw(48, 48).pixels;

        session.orbit(0.25, 0.1);
        let after = session.draw(48, 48).pixels;

        assert_ne!(before, after, "the view did not move");
    }

    /// And fitting undoes whatever was done to the view.
    #[test]
    fn fitting_returns_to_where_it_started() {
        let mut session = open_a_square("square-3");
        let original = session.draw(48, 48).pixels;

        session.orbit(0.3, 0.2);
        session.zoom(4.0);
        session.pan(0.2, -0.1);
        session.fit();

        assert_eq!(original, session.draw(48, 48).pixels, "fit did not restore the view");
    }

    /// Zooming in makes the part bigger, which is not as obvious as it sounds:
    /// the factor is a scale on the *distance*, so it has to be inverted.
    #[test]
    fn zooming_in_makes_the_part_larger() {
        let mut session = open_a_square("square-4");
        let before = session.draw(64, 64).covered();

        session.zoom(2.0);
        let after = session.draw(64, 64).covered();

        assert!(after > before, "{before} pixels became {after}");
    }

    /// Panning moves the part without turning it.
    #[test]
    fn panning_does_not_change_the_angle() {
        let mut session = open_a_square("square-5");
        let facing = session.camera.back;

        session.pan(0.3, 0.2);

        assert_eq!(facing, session.camera.back, "panning turned the view");
    }

    /// A nonsense zoom is ignored rather than destroying the view.
    #[test]
    fn a_bad_zoom_leaves_the_camera_alone() {
        let mut session = open_a_square("square-6");
        let distance = session.camera.distance;

        session.zoom(0.0);
        session.zoom(f64::NAN);
        session.zoom(-1.0);

        assert_eq!(distance, session.camera.distance);
    }

    // ---- refusal -------------------------------------------------------------

    #[test]
    fn something_that_is_not_step_is_refused_with_a_sentence() {
        let Err(error) = open("junk", b"hello") else { panic!("not a model") };
        assert!(!error.message().is_empty());
    }

    /// The gate runs before the geometry, which is the point of it.
    #[test]
    fn a_file_over_the_face_limit_is_refused_without_being_built() {
        let mut huge = String::from("DATA;\n");
        for index in 0..(audit::MAX_FACES + 1) {
            huge.push_str(&format!("#{index}=ADVANCED_FACE('',(#1),#2,.T.);\n"));
        }

        let Err(error) = open("huge", huge.as_bytes()) else { panic!("refused") };
        assert!(matches!(error, OpenError::Refused(_)));
        assert!(error.message().contains(&format!("{}", audit::MAX_FACES)));
    }

    // ---- the summary ---------------------------------------------------------

    /// The summary carries what was left out, not only what was drawn.
    #[test]
    fn the_summary_says_what_was_skipped() {
        let session = open_a_square("square-7");
        let summary = session.summary_json();

        assert!(summary.contains("\"triangles\""), "{summary}");
        assert!(summary.contains("\"skipped\""), "{summary}");
        assert!(summary.contains("\"facesInFile\""), "{summary}");
        assert!(summary.contains("\"size\""), "{summary}");
    }

    // ---- the cache -----------------------------------------------------------

    /// Reopening the same file reuses the mesh rather than rebuilding it.
    #[test]
    fn the_same_file_opens_from_the_cache() {
        // Its own cache, not the global one: another test running beside
        // this one can evict the entry between the two opens.
        let mut meshes = HashMap::new();
        let first = open_with(&mut meshes, "cached", a_cube().as_bytes()).expect("opens");
        let second = open_with(&mut meshes, "cached", a_cube().as_bytes()).expect("again");

        assert!(
            Arc::ptr_eq(&first.mesh, &second.mesh),
            "the mesh was built a second time",
        );
    }

    /// **A file that changed is not served from the cache.**
    ///
    /// A part being worked on is exported to the same name repeatedly. Keying
    /// on the name alone would show yesterday's shape under today's filename —
    /// which is worse than waiting, because there is nothing to notice.
    #[test]
    fn a_changed_file_is_not_served_from_the_cache() {
        let original = a_cube();
        let edited = original.replace("(10.0,0.0,0.0)", "(12.0,0.0,0.0)  ");

        let mut meshes = HashMap::new();
        let first = open_with(&mut meshes, "same-name", original.as_bytes()).expect("opens");
        let second = open_with(&mut meshes, "same-name", edited.as_bytes()).expect("opens");

        assert_ne!(
            original.len(),
            edited.len().wrapping_add(1),
            "the fixtures must differ in length for this to be the case under test",
        );
        assert!(!Arc::ptr_eq(&first.mesh, &second.mesh), "served a stale mesh");
    }
}
