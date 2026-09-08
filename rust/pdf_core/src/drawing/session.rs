//! An open drawing: its shapes, where it is being looked at, and its pixels.
//!
//! The phone holds a handle to one of these and asks it to draw. Reading
//! happens once, on opening; panning and zooming afterwards is only stroking
//! the shapes again, which is why a drag can be smooth even on a plan of
//! twenty-seven thousand lines.
//!
//! The same shape as [`crate::step::session`] — open, resize, move, draw, and a
//! summary the screen can read — and deliberately its own type rather than a
//! generalisation of it. The two hold different things and move in different
//! ways, and one abstraction over two examples fits neither.

use tiny_skia::Pixmap;

use super::model::Drawing;
use super::raster::{self, Style, View};

/// A drawing, open and ready to be looked at.
pub struct DrawingSession {
    pub drawing: Drawing,
    pub view: View,
    style: Style,
    /// What the file is called, for the screen's title.
    pub name: String,
}

/// Why a file could not be opened.
#[derive(Debug, Clone, PartialEq)]
pub enum OpenError {
    /// The reader could not make sense of it. The message is the reader's own.
    Unreadable(String),
    /// It read, and there was nothing in it to draw.
    Empty,
}

impl OpenError {
    /// What to put in front of somebody, rather than in a log.
    pub fn message(&self) -> String {
        match self {
            Self::Unreadable(why) => format!("This drawing could not be read: {why}"),
            Self::Empty => "This drawing has nothing in it to show.".to_string(),
        }
    }
}

/// Open a drawing, choosing the reader by what the file is called.
///
/// **By extension, not by sniffing the contents.** DXF is text and DWG is
/// binary, so they are easy to tell apart — but a file named `.dxf` that is
/// really a DWG is somebody's mistake, and quietly reading it anyway hides the
/// mistake rather than fixing it.
pub fn open(path: &std::path::Path) -> Result<DrawingSession, OpenError> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "drawing".into());

    let dwg = path
        .extension()
        .map(|e| e.eq_ignore_ascii_case("dwg"))
        .unwrap_or(false);

    let drawing = if dwg {
        super::dwg::read(path).map_err(OpenError::Unreadable)?
    } else {
        let bytes = std::fs::read(path).map_err(|e| OpenError::Unreadable(e.to_string()))?;
        // Lossy on purpose: a drawing with one stray byte in a layer name is
        // still a drawing, and refusing it over an encoding is not a service.
        super::dxf::read(&String::from_utf8_lossy(&bytes)).map_err(OpenError::Unreadable)?
    };

    if drawing.entities.is_empty() {
        return Err(OpenError::Empty);
    }

    Ok(DrawingSession {
        view: View { centre: super::model::Point::new(0.0, 0.0), scale: 1.0 },
        drawing,
        style: Style::default(),
        name,
    })
}

impl DrawingSession {
    /// Zoom about a point on the screen.
    ///
    /// Above one is closer. Multiplied rather than added so a pinch feels the
    /// same however far in it already is.
    ///
    /// **The point under the fingers stays under the fingers.** Zooming about
    /// the middle instead is the difference between a viewer that goes where it
    /// is pointed and one that has to be dragged back after every pinch:
    /// somebody spreads two fingers over a stair detail in the corner and the
    /// middle of the sheet comes up at them instead. Work out which point of
    /// the drawing is under the touch, scale, then move the centre so that same
    /// point lands back where it was.
    pub fn zoom_about(&mut self, by: f64, at_x: f64, at_y: f64, width: u32, height: u32) {
        if !by.is_finite() || by <= 0.0 || self.view.scale <= 0.0 {
            return;
        }
        let was = self.view.scale;
        let now = (was * by).clamp(1e-9, 1e12);
        if now == was {
            return;
        }
        self.view.scale = now;

        // How far the touch is from the middle, in pixels. The sheet's y runs
        // up and the screen's down, so the second term is negated.
        let across = at_x - width as f64 / 2.0;
        let down = at_y - height as f64 / 2.0;
        self.view.centre.x += across * (1.0 / was - 1.0 / now);
        self.view.centre.y -= down * (1.0 / was - 1.0 / now);
    }

    /// Zoom about the middle, for anything with no particular point in mind.
    pub fn zoom(&mut self, by: f64) {
        if by.is_finite() && by > 0.0 {
            self.view.scale = (self.view.scale * by).clamp(1e-9, 1e12);
        }
    }

    /// The font the sheet's text is drawn with.
    pub fn use_font(&mut self, bytes: std::sync::Arc<Vec<u8>>) {
        self.style.font = Some(bytes);
    }

    /// Which point of the drawing is under a pixel.
    ///
    /// Exposed so a test can state the property that matters — that a pinch
    /// leaves the same place under the fingers — rather than re-deriving the
    /// arithmetic it is checking.
    pub fn under(&self, at_x: f64, at_y: f64, width: u32, height: u32) -> super::model::Point {
        super::model::Point::new(
            self.view.centre.x + (at_x - width as f64 / 2.0) / self.view.scale,
            self.view.centre.y - (at_y - height as f64 / 2.0) / self.view.scale,
        )
    }

    /// Slide the sheet. Both distances are fractions of the view, not pixels,
    /// so a gesture means the same thing on any screen.
    pub fn pan(&mut self, across: f64, down: f64, width: u32, height: u32) {
        if self.view.scale <= 0.0 {
            return;
        }
        // Dragging right moves the *sheet* right, which moves the point at the
        // centre left. And the screen's y runs the other way from the sheet's.
        self.view.centre.x -= across * width as f64 / self.view.scale;
        self.view.centre.y += down * height as f64 / self.view.scale;
    }

    /// Show the whole drawing.
    pub fn fit(&mut self, width: u32, height: u32) {
        self.view = View::fitted(&self.drawing, width.max(1), height.max(1));
    }

    pub fn draw(&self, width: u32, height: u32) -> Pixmap {
        self.draw_scaled(width, height, 1.0)
    }

    /// Draw the same view into a bitmap `by` times the size of the screen's.
    ///
    /// **A sheet does not reframe itself when the canvas grows.** Its scale is
    /// pixels per drawing unit, fixed, so drawing into a bitmap twice as wide
    /// shows twice as much of the drawing rather than the same part of it in
    /// twice the detail. A camera in space has no such problem — its projection
    /// comes from the canvas it is given — which is why the model viewer needs
    /// no equivalent of this, and why sharing its capture code quietly did the
    /// wrong thing here: the region cut out of the larger picture was not the
    /// region anybody had drawn a box around.
    pub fn draw_scaled(&self, width: u32, height: u32, by: f64) -> Pixmap {
        let mut sheet = Pixmap::new(width.max(1), height.max(1)).unwrap_or_else(|| {
            // Only when the size is absurd; a one-pixel sheet is better than a
            // panic crossing the JNI boundary.
            Pixmap::new(1, 1).expect("a single pixel")
        });
        let mut view = self.view;
        let mut style = self.style.clone();
        if by.is_finite() && by > 0.0 {
            view.scale *= by;
            // **The strokes grow with it.** Line width is a constant number of
            // screen pixels, not a property of the drawing, so leaving it
            // alone makes a capture a picture of thinner lines rather than the
            // same picture larger — and the text, which does scale, comes out
            // heavy beside them.
            style.line_width *= by as f32;
        }
        raster::draw(&self.drawing, &view, &style, &mut sheet);
        sheet
    }

    /// What the screen shows about the file, as JSON.
    ///
    /// The same contract the model viewer's summary follows: what is in the
    /// file, what is drawn, and — named one by one — what is not.
    pub fn summary_json(&self) -> String {
        let mut skipped = String::from("[");
        for (at, entry) in self.drawing.skipped.iter().enumerate() {
            if at > 0 {
                skipped.push(',');
            }
            skipped.push_str(&format!(
                r#"{{"what":{},"count":{}}}"#,
                quoted(&entry.what),
                entry.count,
            ));
        }
        skipped.push(']');

        let extent = match self.drawing.bounds() {
            Some((low, high)) => format!(
                r#","size":{{"x":{:.3},"y":{:.3}}}"#,
                high.x - low.x,
                high.y - low.y,
            ),
            None => String::new(),
        };

        format!(
            r#"{{"name":{},"shapes":{},"layers":{},"notShown":{},"metresPerUnit":{},"unitsDeclared":{},"skipped":{}{}}}"#,
            quoted(&self.name),
            self.drawing.kept(),
            self.drawing.layers.len(),
            self.drawing.lost(),
            self.drawing.units.metres_per_unit,
            self.drawing.units.declared,
            skipped,
            extent,
        )
    }

    /// Which layers there are, and whether each is being drawn.
    pub fn layers_json(&self) -> String {
        let mut out = String::from("[");
        for (at, layer) in self.drawing.layers.iter().enumerate() {
            if at > 0 {
                out.push(',');
            }
            // A longer delimiter, because the colour's own `#` immediately
            // before a quote would otherwise end the raw string.
            out.push_str(&format!(
                r##"{{"name":{},"visible":{},"colour":"#{:02x}{:02x}{:02x}"}}"##,
                quoted(&layer.name),
                layer.visible,
                layer.colour[0],
                layer.colour[1],
                layer.colour[2],
            ));
        }
        out.push(']');
        out
    }

    /// Turn one layer on or off.
    pub fn show_layer(&mut self, at: usize, visible: bool) {
        if let Some(layer) = self.drawing.layers.get_mut(at) {
            layer.visible = visible;
        }
    }
}

/// A string as JSON, with the characters that would break it escaped.
///
/// Layer names come from the file and are somebody else's text: they contain
/// quotes and backslashes often enough that building JSON by hand without this
/// produces something the phone cannot parse, on exactly the drawings that
/// matter most.
fn quoted(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_sheet() -> DrawingSession {
        let mut drawing = Drawing::default();
        drawing.layer_for("0");
        drawing.entities.push(super::super::model::Entity {
            layer: 0,
            shape: super::super::model::Shape::Line {
                a: super::super::model::Point::new(0.0, 0.0),
                b: super::super::model::Point::new(100.0, 80.0),
            },
        });
        DrawingSession {
            drawing,
            view: View { centre: super::super::model::Point::new(50.0, 40.0), scale: 4.0 },
            style: Style::default(),
            name: "plan.dxf".into(),
        }
    }

    /// **A pinch leaves the same place under the fingers.**
    ///
    /// Zooming about the middle instead is not a failure anybody sees as one:
    /// the sheet does get bigger. It is just never the part that was being
    /// pinched, so every zoom has to be followed by a drag to find the detail
    /// again. Stated as the property rather than as the arithmetic, so the
    /// test cannot agree with a wrong formula by copying it.
    #[test]
    fn zooming_keeps_the_point_under_the_fingers() {
        let (width, height) = (1000u32, 600u32);
        for (x, y) in [(120.0, 90.0), (880.0, 510.0), (500.0, 300.0), (0.0, 0.0)] {
            for by in [1.5, 0.5, 4.0] {
                let mut sheet = a_sheet();
                let before = sheet.under(x, y, width, height);

                sheet.zoom_about(by, x, y, width, height);
                let after = sheet.under(x, y, width, height);

                assert!(
                    (before.x - after.x).abs() < 1e-9 && (before.y - after.y).abs() < 1e-9,
                    "pinching {by}x at ({x}, {y}) moved {before:?} to {after:?}",
                );
            }
        }
    }

    /// And it does actually zoom.
    #[test]
    fn zooming_changes_the_scale() {
        let mut sheet = a_sheet();
        let was = sheet.view.scale;

        sheet.zoom_about(2.0, 100.0, 100.0, 1000, 600);

        assert!((sheet.view.scale - was * 2.0).abs() < 1e-9);
    }

    /// Zooming about the middle is the one case where both agree.
    #[test]
    fn a_pinch_at_the_middle_does_not_move_the_centre() {
        let mut sheet = a_sheet();
        let was = sheet.view.centre;

        sheet.zoom_about(3.0, 500.0, 300.0, 1000, 600);

        assert!((sheet.view.centre.x - was.x).abs() < 1e-9);
        assert!((sheet.view.centre.y - was.y).abs() < 1e-9);
    }

    /// **Whatever a layer is called, the summary is still JSON.**
    ///
    /// Layer names come out of somebody else's file and contain quotes,
    /// backslashes and the occasional control character often enough that
    /// building JSON by hand without escaping produces something the phone
    /// cannot parse — on exactly the drawings that matter most. Checked by
    /// parsing it back rather than by comparing text, which tests the property
    /// that matters instead of one spelling of it.
    #[test]
    fn any_layer_name_survives_being_written_and_read_back() {
        let quote = char::from(34);
        let backslash = char::from(92);
        let bell = char::from(7);

        let awkward = [
            format!("WALL {quote}outer{quote} {backslash} 2"),
            format!("A{bell}B"),
            "two\nlines".to_string(),
            "a\ttab".to_string(),
            "ordinary".to_string(),
        ];

        for name in awkward {
            let json = quoted(&name);
            let back: String = serde_json::from_str(&json)
                .unwrap_or_else(|why| panic!("{json} is not JSON: {why}"));
            assert_eq!(name, back, "came back changed");
        }
    }

    /// And the summary as a whole parses, not just one field of it.
    #[test]
    fn the_summary_is_json() {
        let mut drawing = Drawing::default();
        let quote = char::from(34);
        drawing.layer_for(&format!("A{quote}B"));
        drawing.note("text");
        drawing.entities.push(super::super::model::Entity {
            layer: 0,
            shape: super::super::model::Shape::Line {
                a: super::super::model::Point::new(0.0, 0.0),
                b: super::super::model::Point::new(1.0, 1.0),
            },
        });

        let session = DrawingSession {
            drawing,
            view: View { centre: super::super::model::Point::new(0.0, 0.0), scale: 1.0 },
            style: Style::default(),
            name: format!("plan{quote}.dxf"),
        };

        let value: serde_json::Value = serde_json::from_str(&session.summary_json())
            .expect("the summary is JSON");
        assert_eq!(1, value["shapes"]);
        assert_eq!(1, value["notShown"]);

        let layers: serde_json::Value = serde_json::from_str(&session.layers_json())
            .expect("the layers are JSON");
        assert!(layers.is_array());
    }
}

#[cfg(test)]
mod real {
    /// What a real drawing loses, in either format.
    ///
    /// ```text
    /// PAGIFY_DRAWING_FILE="/path/plan.dwg" \
    ///   cargo test --release --lib drawing::session::real -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "needs a real drawing; set PAGIFY_DRAWING_FILE"]
    fn what_a_real_drawing_loses() {
        let path = std::env::var("PAGIFY_DRAWING_FILE").expect("set PAGIFY_DRAWING_FILE");
        let sheet = super::open(std::path::Path::new(&path)).expect("it opens");

        println!("--- {path}");
        println!("  {} shapes, {} layers", sheet.drawing.kept(), sheet.drawing.layers.len());
        for entry in &sheet.drawing.skipped {
            println!("  {:>6} {}", entry.count, entry.what);
        }
    }
}
