//! The tool that is waiting for clicks: what it still wants, what to say about
//! it, and whether it stays armed once it has what it wanted.
//!
//! **Still in its current shape, not redesigned.** This is the state the
//! mentor's review calls tool-state sprawl — the fields it belongs with
//! (`pending`, `markup_armed`, `editing_run`, `grab`, `handle`, `selected_*`,
//! and the rest) are still on `PagifyApp`/`DocTab`, and the transition
//! functions (`arm`, `resolve`, `leave_editor_by_click`, …) are still where
//! they were. Collapsing all of that into one `Tool` enum with its own
//! transitions is later work; this is only the file it moves to first.

use pagify_shell::page_space::AppPoint;
use pagify_shell::tools;
use pagify_shell::verbs::MeasureKind;

/// What a command is still waiting for from the pointer.
///
/// Objects and points are collected separately because they are not the same
/// thing: fillet wants two *objects* clicked on, move wants two *points*, and
/// offset wants an object and then a point saying which side. Treating both as
/// "clicks" is what made fillet perform a move.
pub(crate) struct Pending {
    pub(crate) kind: PendingKind,
    pub(crate) page: usize,
    pub(crate) objects: Vec<(usize, AppPoint)>,
    pub(crate) points: Vec<AppPoint>,
}

/// A tool armed outside the big [`Pending`] dispatch — the `Tool` state
/// machine the mentor's review calls for (`DESIGN_REVIEW.md` §3.2), built up
/// one slice at a time. `Draw` is the first kind that ends on Enter rather
/// than a fixed point count (see `wants_points`/`ends_on_enter`) — every
/// kind before it wanted a fixed number of points. Every other
/// `PendingKind` variant is still exactly where it was; `PendingKind` is
/// deleted only once it is empty.
pub(crate) struct ArmedTool {
    pub(crate) kind: Tool,
    pub(crate) page: usize,
    /// Collected so far — unlike [`Pending`], never objects: no `Tool` kind
    /// has needed one yet.
    pub(crate) points: Vec<AppPoint>,
}

pub(crate) enum Tool {
    /// Where a drawn signature is to sit — on the line that is clicked.
    Signature,
    /// A decoded picture waiting for a point to be centred on.
    PlaceImage { rgba: Vec<u8>, width: u32, height: u32 },
    /// Two corners of a box brand new text is composed into — see
    /// [`crate::NewTextBox`].
    PlaceText,
    /// Two points a known real-world distance apart, used to scale every
    /// later measurement on this document.
    Calibrate { distance: f64, unit: String },
    /// A point to put a tick, a cross or a dot at.
    Fill(pdf_core::document::FillMark),
    /// Two corners of an area whose contents are to be destroyed.
    Redact,
    /// Two corners of an area to paint over.
    ///
    /// **Covers; does not remove.** Kept apart from `Redact` for the same
    /// reason the verbs are: the difference is the whole point, and a flag
    /// on one is how somebody ends up with the other.
    Whiteout,
    /// Two corners of a box to draw while filling a form in.
    SignRectangle,
    /// The two ends of a line to rule while filling a form in.
    SignLine,
    /// Words waiting for a point to be written at.
    Write(String),
    Draw(DrawKind),
}

impl Tool {
    /// How many points it still wants. `usize::MAX` means "until Enter" —
    /// the same convention `PendingKind::wants` uses, now that `Draw` is
    /// the first `Tool` kind to need it.
    pub(crate) fn wants_points(&self) -> usize {
        match self {
            Tool::Signature | Tool::PlaceImage { .. } | Tool::Fill(_) | Tool::Write(_) => 1,
            Tool::PlaceText
            | Tool::Calibrate { .. }
            | Tool::Redact
            | Tool::Whiteout
            | Tool::SignRectangle
            | Tool::SignLine => 2,
            Tool::Draw(DrawKind::Line | DrawKind::Circle | DrawKind::Rectangle | DrawKind::Arrow) => 2,
            Tool::Draw(DrawKind::Polyline | DrawKind::Spline) => usize::MAX,
        }
    }

    /// Whether Enter can end it early — see `PendingKind::ends_on_enter`'s
    /// own doc.
    pub(crate) fn ends_on_enter(&self) -> bool {
        self.wants_points() == usize::MAX
    }

    /// Mirrors `PendingKind::prompt` for these kinds, minus the
    /// `objects_done` there is never anything to thread through.
    pub(crate) fn prompt(&self, points_done: usize) -> String {
        match self {
            // Says where the click lands, because a signature that appears
            // above or below the line is the thing to get right first time.
            Tool::Signature => "signature: click the line to sign on".into(),
            Tool::PlaceImage { .. } => "click where the picture goes".into(),
            Tool::PlaceText => match points_done {
                0 => "text: first corner of the box".into(),
                _ => "text: opposite corner".into(),
            },
            Tool::Calibrate { .. } => {
                if points_done == 0 {
                    "calibrate: first of the two points".into()
                } else {
                    "calibrate: second point".into()
                }
            }
            Tool::Fill(mark) => format!("fill: click where the {} goes", mark.describe()),
            Tool::Redact => match points_done {
                0 => "redact: first corner of the area to destroy".into(),
                _ => "redact: opposite corner".into(),
            },
            Tool::Whiteout => match points_done {
                0 => "whiteout: first corner — this covers, it does not remove".into(),
                _ => "whiteout: opposite corner".into(),
            },
            // Says which of the two rectangles/lines this is, because the
            // other one is a drawing that can be picked up again and this
            // one is not.
            Tool::SignRectangle => match points_done {
                0 => "rectangle: first corner — a mark on the form, not a drawing".into(),
                _ => "rectangle: opposite corner".into(),
            },
            Tool::SignLine => match points_done {
                0 => "line: from — a mark on the form, not a drawing".into(),
                _ => "line: to".into(),
            },
            Tool::Write(text) => {
                let short: String = text.chars().take(24).collect();
                format!(
                    "click where \"{short}{}\" goes",
                    if text.chars().count() > 24 { "…" } else { "" }
                )
            }
            Tool::Draw(kind) => match (kind, points_done) {
                (DrawKind::Line, 0) => "line: from".into(),
                (DrawKind::Line, _) => "line: to".into(),
                (DrawKind::Circle, 0) => "circle: centre".into(),
                (DrawKind::Circle, _) => "circle: a point on it".into(),
                (DrawKind::Rectangle, 0) => "rectangle: first corner".into(),
                (DrawKind::Rectangle, _) => "rectangle: opposite corner".into(),
                (DrawKind::Arrow, 0) => "arrow: from".into(),
                (DrawKind::Arrow, _) => "arrow: to — the point the head lands on".into(),
                (DrawKind::Polyline, n) => {
                    format!("polyline: point {} — Enter to finish", n + 1)
                }
                (DrawKind::Spline, n) => {
                    format!("spline: point {} — Enter to finish", n + 1)
                }
            },
        }
    }

    /// Whether finishing it should arm it again — see `PendingKind::
    /// repeats`'s own doc for the full reasoning. `Fill`/`Redact`/
    /// `Whiteout`/`SignRectangle`/`SignLine` are a straight port of it (a
    /// run of stamps, redactions or form marks should not mean a trip to
    /// the ribbon between each one). The other three `Tool` kinds answer a
    /// question or place one thing to immediately adjust, not stamp a
    /// mark, so they stay `false`.
    pub(crate) fn repeats(&self) -> bool {
        matches!(
            self,
            Tool::Fill(_)
                | Tool::Redact
                | Tool::Whiteout
                | Tool::SignRectangle
                | Tool::SignLine
                | Tool::Write(_)
                | Tool::Draw(_)
        )
    }

    /// The ribbon command that arms this, so its button can show itself lit
    /// while it is waiting for its clicks — see `PendingKind::command`'s own
    /// doc. `PlaceImage` has never lit a button: it names a file, not a
    /// repeatable command.
    pub(crate) fn command(&self) -> Option<&'static str> {
        match self {
            Tool::Signature => Some("signature"),
            Tool::PlaceImage { .. } => None,
            Tool::PlaceText => Some("addtext"),
            Tool::Calibrate { .. } => Some("calibrate"),
            // No ribbon button lights up per mark; the tool is one word
            // with an argument — see `PendingKind::command`'s own doc.
            Tool::Fill(_) => None,
            Tool::Redact => Some("redact"),
            Tool::Whiteout => Some("whiteout"),
            Tool::SignRectangle => Some("signrectangle"),
            Tool::SignLine => Some("signline"),
            Tool::Write(_) => Some("addtext"),
            Tool::Draw(DrawKind::Line) => Some("line"),
            Tool::Draw(DrawKind::Circle) => Some("circle"),
            Tool::Draw(DrawKind::Polyline) => Some("pline"),
            Tool::Draw(DrawKind::Rectangle) => None,
            Tool::Draw(DrawKind::Arrow) => Some("arrow"),
            Tool::Draw(DrawKind::Spline) => Some("spline"),
        }
    }

    /// Whether the pointer should be pulled to nearby geometry — see
    /// `PendingKind::wants_snapping`'s own doc. A calibration or draw point
    /// is placing a point on known geometry, unlike a signature, picture or
    /// text box's corner, which is just "about here."
    pub(crate) fn wants_snapping(&self) -> bool {
        matches!(self, Tool::Calibrate { .. } | Tool::Draw(_))
    }
}

pub(crate) enum PendingKind {
    /// Waiting for a click on the words to change.
    PickText,
    /// Waiting for a click on a highlight, underline, strike-out or squiggle to
    /// take it off the page — what the Eraser arms when nothing drawn is
    /// selected. Stays in hand, so a run of marks can be rubbed out in a row.
    EraseMark,
    Modify(tools::Pick),
    Measure(MeasureKind),
    /// Two corners of an area to hide, sealed under a passcode.
    Lock,
    /// Two corners of a labelled region — see [`PendingArticleBox`] for the
    /// title, asked for once the area is drawn.
    ArticleBox,
}

/// A drawn Article Box rectangle, waiting for the title that names it.
#[derive(Debug, Clone)]
pub(crate) struct PendingArticleBox {
    pub(crate) page: usize,
    pub(crate) rect: pdf_core::document::Rect,
    pub(crate) title: String,
}

/// A text selection waiting for the address to link it to.
#[derive(Debug, Clone, Default)]
pub(crate) struct PendingLink {
    pub(crate) page: usize,
    /// One rect per line — see [`pdf_core::document::Annotation::Link`]'s own
    /// doc for why applying this writes one link per entry rather than one
    /// link covering all of them.
    pub(crate) rects: Vec<pdf_core::document::Rect>,
    pub(crate) url: String,
}

/// The sample "Match Properties" copies from — see
/// [`crate::PagifyApp::match_properties_sample`].
///
/// Built once, when the sample is picked, rather than re-read before every
/// target: `alternate_objects` is one pass over the whole page, and running
/// that again for every separate target selection would be the same
/// per-selection page walk `compute_right_click_text_actions`'s own doc
/// already found seconds long on a real few-hundred-run document.
#[derive(Debug, Clone)]
pub(crate) struct MatchPropertiesSample {
    pub(crate) page: usize,
    /// The sample run's own registered face name, if it has an embedded copy
    /// at all — `None` means only size and colour can carry over.
    pub(crate) face: Option<String>,
    pub(crate) size: f32,
    pub(crate) color: pdf_core::document::Color,
    /// The sample's own family, subset tag stripped — see
    /// `strip_subset_prefix`. A target already in this family is left alone
    /// rather than retyped.
    pub(crate) family: Option<String>,
    /// One representative object per distinct on-page font name sharing
    /// `family` — tried in turn when `face`'s own embedded copy cannot spell
    /// a target's text. See `match_font_to_first_selected`'s own doc, which
    /// this is ported from.
    pub(crate) alternate_objects: Vec<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum DrawKind {
    Line,
    Circle,
    Rectangle,
    Polyline,
    /// A line with an arrowhead pre-picked on its end — see `PagifyApp::
    /// draw_shape_properties`'s "ends" control for turning one on or off
    /// after the fact, on this or any other line.
    Arrow,
    /// A NURBS curve — `cad_kernel::Geom::Spline`. Picked the same way a
    /// Polyline is (any number of points, Enter to finish); the difference
    /// is only in what the points become.
    Spline,
}

/// How many of each a pick still wants. `usize::MAX` means "until Enter".
impl PendingKind {
    pub(crate) fn wants(&self) -> (usize, usize) {
        match self {
            PendingKind::PickText | PendingKind::EraseMark => (0, 1),
            PendingKind::Modify(pick) => (pick.objects, pick.points),
            PendingKind::Measure(MeasureKind::Distance) => (0, 2),
            PendingKind::Measure(MeasureKind::Area) => (0, usize::MAX),
            PendingKind::Lock => (0, 2),
            PendingKind::ArticleBox => (0, 2),
        }
    }

    pub(crate) fn prompt(&self, objects_done: usize, points_done: usize) -> String {
        match self {
            PendingKind::PickText => "click the words to change".into(),
            PendingKind::EraseMark => {
                "click a highlight, underline or strike-out to erase it — Escape puts the eraser down".into()
            }
            // Says which of the two rectangles this is, because the other one
            // is a drawing that can be picked up again and this one is not.
            PendingKind::Lock => match points_done {
                0 => "lock: first corner of the area to hide".into(),
                _ => "lock: opposite corner".into(),
            },
            PendingKind::ArticleBox => match points_done {
                0 => "article box: first corner".into(),
                _ => "article box: opposite corner".into(),
            },
            PendingKind::Modify(pick) => pick.prompt(objects_done, points_done),
            PendingKind::Measure(MeasureKind::Distance) => {
                if points_done == 0 { "measure: from".into() } else { "measure: to".into() }
            }
            PendingKind::Measure(MeasureKind::Area) => {
                format!("measure area: corner {} — Enter to close", points_done + 1)
            }
        }
    }

    /// Whether finishing it should arm it again — trim and extend are used on
    /// one piece after another and re-arming by hand each time is miserable.
    /// Whether the tool stays in hand after it has been used.
    ///
    /// **Nearly all of them do.** A tool is something you pick up and keep
    /// using until you put it down; one that lets go after a single line means
    /// going back to the ribbon between every line, and drawing four sides of a
    /// box becomes four trips.
    ///
    /// The exceptions are the ones that answer a question rather than make
    /// a mark, or whose mark is immediately the thing to keep working on
    /// rather than repeat: calibration is set once; and a placed signature
    /// is the same — what someone wants right after placing one is almost
    /// always to move, resize or turn the one just placed, not stamp
    /// another, and a tool left in hand would swallow that very click,
    /// reading it as the start of a second signature instead of a pick on
    /// the first (reported from use: "the scaling and rotating isn't
    /// working" was this, not the drag math).
    ///
    /// **`PickText` used to be a third exception, on the reasoning that
    /// picking a run opens an editor where the attention now belongs — but
    /// the attention belongs there only until the *next* word someone means
    /// to change, and every one after the first needed `edittext` retyped by
    /// hand to reach.** Reported from use: editing a column of a datasheet
    /// field by field took a fresh `edittext` before every single one.
    /// Repeating here only matters together with `interact_page`'s own
    /// click-elsewhere handler, which re-arms and resolves a fresh pick at
    /// that same click — this flag is what lets that re-arm survive a click
    /// that lands on bare paper instead of another run, rather than putting
    /// the tool down right back where `edittext` would have to undo it.
    pub(crate) fn repeats(&self) -> bool {
        // Every exception — calibration, a placed signature, picture and
        // text box — has moved to `Tool`, which never re-arms at all (see
        // its own doc), so there is nothing left here that does not repeat.
        true
    }

    /// Whether Enter can end it early.
    /// The ribbon command that arms this, so the button can show itself lit
    /// while it is collecting clicks.
    ///
    /// `None` where no button arms it — a pick started from a typed command
    /// with no ribbon equivalent has nothing to light up.
    pub(crate) fn command(&self) -> Option<&'static str> {
        Some(match self {
            PendingKind::Lock => "lock",
            PendingKind::ArticleBox => "articlebox",
            PendingKind::PickText => "edittext",
            PendingKind::EraseMark => "erase",
            PendingKind::Measure(MeasureKind::Distance) => "measure distance",
            PendingKind::Measure(MeasureKind::Area) => "measure area",
            PendingKind::Modify(_) => return None,
        })
    }

    /// Whether the pointer should be pulled to nearby geometry.
    ///
    /// **Only while a tool is placing points.** Snapping, ortho and the grid
    /// exist to put a line exactly on the end of another line; applied to
    /// ordinary clicking they drag the pointer away from whatever the user was
    /// aiming at — a word, a run to edit, somebody else's highlight — and the
    /// page feels like it is fighting them.
    ///
    /// Picking a run of text is not placing a point: it means "the words
    /// there", and the nearest drawn line has nothing to do with it.
    pub(crate) fn wants_snapping(&self) -> bool {
        matches!(self, PendingKind::Modify(_) | PendingKind::Measure(_))
    }

    pub(crate) fn ends_on_enter(&self) -> bool {
        let (_, points) = self.wants();
        points == usize::MAX
    }
}

impl Pending {
    pub(crate) fn ready(&self) -> bool {
        let (objects, points) = self.kind.wants();
        objects != usize::MAX
            && points != usize::MAX
            && self.objects.len() >= objects
            && self.points.len() >= points
    }

    /// Whether the next click should pick an object rather than a free point.
    pub(crate) fn wants_object(&self) -> bool {
        let (objects, _) = self.kind.wants();
        self.objects.len() < objects
    }

    pub(crate) fn prompt(&self) -> String {
        self.kind.prompt(self.objects.len(), self.points.len())
    }
}
