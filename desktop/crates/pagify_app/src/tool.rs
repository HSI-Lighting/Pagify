//! The tool that is waiting for clicks: what it still wants, what to say
//! about it, and whether it stays armed once it has what it wanted.
//!
//! **Still in its current shape, not redesigned.** This is the state the
//! mentor's review calls tool-state sprawl — the fields `Tool` belongs with
//! (`editing_run`, `grab`, `handle`, `selected_*`, and the
//! rest) are still on `PagifyApp`/`DocTab`, not folded in here. `Tool`
//! itself was built up one slice at a time, from the big `PendingKind`
//! dispatch this file used to also define: `Draw` was the first kind that
//! ends on Enter rather than a fixed point count (see
//! `wants_points`/`ends_on_enter`); `Modify` was the first that wants
//! *objects* too (see `wants_objects`); `PickText`, the last of the
//! original 17 `PendingKind` variants, finished that migration and took
//! `PendingKind`/`Pending` down with it. `Markup`/`Link`/`MatchProperties`
//! moved in afterward from their own scattered fields (`markup_armed`,
//! `link_armed`, `match_properties_armed`/`match_properties_sample`) — the
//! first kinds resolved by a text selection rather than by collecting
//! clicks (see `wants_selection`), so `canvas.rs`'s own selection-drag
//! handling resolves them directly instead of through `take_pick`/
//! `resolve_tool`.
//!
//! `resolve_tool` (`picking.rs`) is still one large match over `Tool`, not
//! the `Tool::on_click` returning a `ToolEffect` that is supposed to
//! replace it — collapsing *how* transitions are expressed is later work;
//! this phase only collapsed *where* the state lives. [`ToolId`] is a
//! first, partial step on the *how*: `Tool::id()` returns it instead of a
//! bare ribbon-command string, so the ribbon-side half of "which button is
//! lit" is now the one place left that has to know the matching literal —
//! see its own doc for exactly how partial. This file used to be
//! `pending.rs`; renamed once `PendingKind`/`Pending` were gone and the
//! name no longer fit (and `tools.rs`, the mentor's own suggested name,
//! collides with `pagify_shell::tools`, already imported everywhere in
//! `main.rs`).

use pagify_shell::page_space::AppPoint;
use pagify_shell::tools;
use pagify_shell::verbs::{MeasureKind, Markup};

pub(crate) struct ArmedTool {
    pub(crate) kind: Tool,
    pub(crate) page: usize,
    pub(crate) objects: Vec<(usize, AppPoint)>,
    pub(crate) points: Vec<AppPoint>,
}

#[derive(Debug)]
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
    /// Waiting for a click on a highlight, underline, strike-out or
    /// squiggle to take it off the page — what the Eraser arms when
    /// nothing drawn is selected. Stays in hand, so a run of marks can be
    /// rubbed out in a row.
    EraseMark,
    Measure(MeasureKind),
    Modify(tools::Pick),
    /// Two corners of an area to hide, sealed under a passcode.
    Lock,
    /// Two corners of a labelled region — see [`PendingArticleBox`] for the
    /// title, asked for once the area is drawn.
    ArticleBox,
    /// Waiting for a click on the words to change. The last `PendingKind`
    /// variant to move — `PendingKind`/`Pending` are gone now that it is
    /// empty.
    PickText,
    /// A highlighter, underline, strike-out or squiggle waiting for a text
    /// selection — resolved by a drag, not a click, so it never goes
    /// through `wants_points`/`take_pick` at all; see
    /// [`Self::wants_selection`].
    Markup(Markup),
    /// Waiting for a text selection to link. A selection already made is
    /// linked at once instead of arming this — see `begin_web_link`.
    Link,
    /// Waiting for a text selection to serve as Match Properties' own
    /// sample, once picked. While `sample` is held, completing a further
    /// selection is matched to it at once and the tool stays armed for the
    /// next one — carrying the sample here, rather than in a field beside
    /// `tool`, is what lets `tool.is_some()` alone answer "is Match
    /// Properties still in hand" across both the "waiting for a sample"
    /// and "has one, applying it" phases.
    MatchProperties { sample: Option<MatchPropertiesSample> },
}

impl Tool {
    /// How many points it still wants. `usize::MAX` means "until Enter".
    pub(crate) fn wants_points(&self) -> usize {
        match self {
            Tool::Signature
            | Tool::PlaceImage { .. }
            | Tool::Fill(_)
            | Tool::Write(_)
            | Tool::EraseMark
            | Tool::PickText => 1,
            Tool::PlaceText
            | Tool::Calibrate { .. }
            | Tool::Redact
            | Tool::Whiteout
            | Tool::SignRectangle
            | Tool::SignLine
            | Tool::Lock
            | Tool::ArticleBox => 2,
            Tool::Draw(DrawKind::Line | DrawKind::Circle | DrawKind::Rectangle | DrawKind::Arrow) => 2,
            Tool::Draw(DrawKind::Polyline | DrawKind::Spline) => usize::MAX,
            Tool::Measure(MeasureKind::Distance) => 2,
            Tool::Measure(MeasureKind::Area) => usize::MAX,
            Tool::Modify(pick) => pick.points,
            Tool::Markup(_) | Tool::Link | Tool::MatchProperties { .. } => 0,
        }
    }

    /// How many objects it still wants, clicked before any points. Zero
    /// for every kind except `Modify`: fillet wants two objects clicked
    /// on, move wants two points, offset wants an object and then a point
    /// saying which side — objects and points are collected separately
    /// because they are not the same thing, and treating both as "clicks"
    /// is what made fillet perform a move.
    pub(crate) fn wants_objects(&self) -> usize {
        match self {
            Tool::Modify(pick) => pick.objects,
            _ => 0,
        }
    }

    /// Whether Enter can end it early — true exactly when there is no
    /// fixed number of points to wait for.
    pub(crate) fn ends_on_enter(&self) -> bool {
        self.wants_points() == usize::MAX
    }

    /// What to say about what it is still waiting for. `objects_done`
    /// only ever varies `Modify`'s own answer — every other kind ignores
    /// it.
    pub(crate) fn prompt(&self, objects_done: usize, points_done: usize) -> String {
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
            Tool::EraseMark => {
                "click a highlight, underline or strike-out to erase it — Escape puts the eraser down".into()
            }
            Tool::Measure(MeasureKind::Distance) => {
                if points_done == 0 { "measure: from".into() } else { "measure: to".into() }
            }
            Tool::Measure(MeasureKind::Area) => {
                format!("measure area: corner {} — Enter to close", points_done + 1)
            }
            Tool::Modify(pick) => pick.prompt(objects_done, points_done),
            Tool::Lock => match points_done {
                0 => "lock: first corner of the area to hide".into(),
                _ => "lock: opposite corner".into(),
            },
            Tool::ArticleBox => match points_done {
                0 => "article box: first corner".into(),
                _ => "article box: opposite corner".into(),
            },
            Tool::PickText => "click the words to change".into(),
            Tool::Markup(kind) => format!(
                "{} — drag across the text to mark it. Escape puts it down.",
                match kind {
                    Markup::Highlight => "highlighter",
                    Markup::Underline => "underline",
                    Markup::StrikeOut => "strikeout",
                    Markup::Squiggly => "squiggly",
                }
            ),
            Tool::Link => "web link — drag across the text to link it (typed text works too, \
                            once it is on the page). Escape puts it down."
                .into(),
            Tool::MatchProperties { sample: None } => {
                "match properties — drag across the sample text to copy from. Escape puts it down.".into()
            }
            Tool::MatchProperties { sample: Some(_) } => {
                "match properties — drag across text to change it. Escape puts it down.".into()
            }
        }
    }

    /// Whether finishing it should arm it again — nearly all kinds do. A
    /// tool is something you pick up and keep using until you put it down;
    /// one that let go after a single line would mean a trip to the ribbon
    /// between every line, and a box would become four trips.
    /// `Fill`/`Redact`/`Whiteout`/`SignRectangle`/`SignLine`/`Lock`/
    /// `ArticleBox` all follow this: a run of stamps, redactions or form
    /// marks should not mean a trip to the ribbon between each one.
    ///
    /// The exceptions — `Signature`, `PlaceImage`, `PlaceText`, `Calibrate`
    /// — answer a question or place one thing to immediately adjust, not
    /// stamp a mark, so they stay `false`.
    ///
    /// **`PickText` is a deliberate reversal, not an exception.** It used
    /// to be one-shot, on the reasoning that picking a run opens an editor
    /// where the attention now belongs — but the attention belongs there
    /// only until the *next* word someone means to change, and every one
    /// after the first needed `edittext` retyped by hand to reach.
    /// Reported from use: editing a column of a datasheet field by field
    /// took a fresh `edittext` before every single one. Repeating here only
    /// matters together with `canvas.rs`'s own click-elsewhere handler,
    /// which re-arms and resolves a fresh pick at that same click — this
    /// is what lets that re-arm survive a click that lands on bare paper
    /// instead of another run, rather than putting the tool down right
    /// back where `edittext` would have to undo it.
    pub(crate) fn repeats(&self) -> bool {
        !matches!(self, Tool::Signature | Tool::PlaceImage { .. } | Tool::PlaceText | Tool::Calibrate { .. })
    }

    /// The identity that arms this, so its ribbon button can show itself
    /// lit while it is waiting for its clicks. `None` where no button arms
    /// it — a pick started from a typed command with no ribbon equivalent
    /// has nothing to light up. `PlaceImage` has never lit a button: it
    /// names a file, not a repeatable command.
    ///
    /// Returns a [`ToolId`], not a bare string — see its own doc for why
    /// (DESIGN_REVIEW.md §2.8.4: comparing bare strings between here and
    /// the ribbon table let either side drift with a typo nothing caught).
    pub(crate) fn id(&self) -> Option<ToolId> {
        match self {
            Tool::Signature => Some(ToolId::Signature),
            Tool::PlaceImage { .. } => None,
            Tool::PlaceText => Some(ToolId::AddText),
            Tool::Calibrate { .. } => Some(ToolId::Calibrate),
            // No ribbon button lights up per mark; the tool is one word
            // with an argument.
            Tool::Fill(_) => None,
            Tool::Redact => Some(ToolId::Redact),
            Tool::Whiteout => Some(ToolId::Whiteout),
            Tool::SignRectangle => Some(ToolId::SignRectangle),
            Tool::SignLine => Some(ToolId::SignLine),
            Tool::Write(_) => Some(ToolId::AddText),
            Tool::Draw(DrawKind::Line) => Some(ToolId::Line),
            Tool::Draw(DrawKind::Circle) => Some(ToolId::Circle),
            Tool::Draw(DrawKind::Polyline) => Some(ToolId::Polyline),
            Tool::Draw(DrawKind::Rectangle) => None,
            Tool::Draw(DrawKind::Arrow) => Some(ToolId::Arrow),
            Tool::Draw(DrawKind::Spline) => Some(ToolId::Spline),
            Tool::EraseMark => Some(ToolId::EraseMark),
            Tool::Measure(MeasureKind::Distance) => Some(ToolId::MeasureDistance),
            Tool::Measure(MeasureKind::Area) => Some(ToolId::MeasureArea),
            Tool::Modify(_) => None,
            Tool::Lock => Some(ToolId::Lock),
            Tool::ArticleBox => Some(ToolId::ArticleBox),
            Tool::PickText => Some(ToolId::EditText),
            Tool::Markup(kind) => Some(match kind {
                Markup::Highlight => ToolId::Highlight,
                Markup::Underline => ToolId::Underline,
                Markup::StrikeOut => ToolId::StrikeOut,
                Markup::Squiggly => ToolId::Squiggly,
            }),
            Tool::Link => Some(ToolId::Link),
            Tool::MatchProperties { .. } => Some(ToolId::MatchProperties),
        }
    }

    /// Resolved by a text selection (a drag), not by collecting clicks —
    /// `Markup`/`Link`/`MatchProperties`, the three kinds `canvas.rs`'s own
    /// selection-drag handling resolves directly rather than through
    /// `take_pick`/`resolve_tool`. Used to keep those three out of the
    /// click/drag-to-points gesture handling built for every other kind,
    /// and to exempt them from the page-binding a part-way point/object
    /// pick needs (they never collect either, so there is nothing to bind).
    pub(crate) fn wants_selection(&self) -> bool {
        matches!(self, Tool::Markup(_) | Tool::Link | Tool::MatchProperties { .. })
    }

    /// Whether the pointer should be pulled to nearby geometry.
    ///
    /// **Only while a tool is placing points.** Snapping, ortho and the
    /// grid exist to put a line exactly on the end of another line;
    /// applied to ordinary clicking they drag the pointer away from
    /// whatever the user was aiming at — a word, a run to edit, somebody
    /// else's highlight — and the page feels like it is fighting them. A
    /// calibration, draw or measure point is placing a point on known
    /// geometry, unlike a signature, picture or text box's corner, which
    /// is just "about here" — and picking a run of text is not placing a
    /// point at all: it means "the words there", and the nearest drawn
    /// line has nothing to do with it.
    pub(crate) fn wants_snapping(&self) -> bool {
        matches!(self, Tool::Calibrate { .. } | Tool::Draw(_) | Tool::Measure(_) | Tool::Modify(_))
    }
}

/// A ribbon-visible tool identity — the typed replacement for comparing
/// bare strings between [`Tool::id`] and the ribbon table's own command
/// string, which is what the ribbon's "which button is lit" query used to
/// do directly (DESIGN_REVIEW.md §2.8.4 names this: `PendingKind::command()
/// -> &'static str`, now `Tool::id() -> Option<ToolId>`, compared against
/// the ribbon table's string through [`Self::ribbon_command`]).
///
/// **Only a partial fix, by design — not the full §2.8.4 ask.** The ribbon
/// tables themselves (`ribbon.rs`'s many `const` button arrays) still store
/// their third field as a bare `&'static str`, not a `ToolId` — rewriting
/// every one of those literals (most of which are one-shot commands with
/// no `Tool` behind them at all, like `"save"` or `"import"`, and were
/// never the actual problem) was judged a materially bigger, separate
/// piece of work than this slice's scope. What this *does* fix: the
/// `Tool`-side half of the comparison is now exhaustively enumerated and
/// checked by the compiler — a newly added `Tool` variant that should
/// light a ribbon button cannot compile without a matching `ToolId`
/// arm in [`Tool::id`], and [`Self::ribbon_command`] is the one place
/// left that still has to know the literal the ribbon table uses for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolId {
    Signature,
    AddText,
    Calibrate,
    Redact,
    Whiteout,
    SignRectangle,
    SignLine,
    Line,
    Circle,
    Polyline,
    Arrow,
    Spline,
    EraseMark,
    MeasureDistance,
    MeasureArea,
    Lock,
    ArticleBox,
    EditText,
    Highlight,
    Underline,
    StrikeOut,
    Squiggly,
    Link,
    MatchProperties,
}

impl ToolId {
    /// The ribbon table's own command string for this identity — the one
    /// place left that has to know it matches the table's literal, now
    /// that every caller compares through here rather than against a bare
    /// string of its own.
    pub(crate) fn ribbon_command(&self) -> &'static str {
        match self {
            ToolId::Signature => "signature",
            ToolId::AddText => "addtext",
            ToolId::Calibrate => "calibrate",
            ToolId::Redact => "redact",
            ToolId::Whiteout => "whiteout",
            ToolId::SignRectangle => "signrectangle",
            ToolId::SignLine => "signline",
            ToolId::Line => "line",
            ToolId::Circle => "circle",
            ToolId::Polyline => "pline",
            ToolId::Arrow => "arrow",
            ToolId::Spline => "spline",
            ToolId::EraseMark => "erase",
            ToolId::MeasureDistance => "measure distance",
            ToolId::MeasureArea => "measure area",
            ToolId::Lock => "lock",
            ToolId::ArticleBox => "articlebox",
            ToolId::EditText => "edittext",
            ToolId::Highlight => "highlight",
            ToolId::Underline => "underline",
            ToolId::StrikeOut => "strikeout",
            ToolId::Squiggly => "squiggly",
            ToolId::Link => "weblinks",
            ToolId::MatchProperties => "matchproperties",
        }
    }
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
/// [`Tool::MatchProperties`].
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

