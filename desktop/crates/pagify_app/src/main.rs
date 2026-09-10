//! Pagify Desktop.
//!
//! The window. Everything that can be decided without one lives in
//! `pagify_shell`, which is where the tests are — this file lays out panels,
//! turns pointer events into page coordinates, and hands verbs to the shell.
//!
//! The ribbon buttons all run command strings. That is not a shortcut: §7's
//! claim is that the command box *is* the interface, so anything clickable must
//! be typeable and anything typeable must be scriptable. A button that called a
//! function directly would be a fourth thing that the Automate tab could not
//! record.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod mac_open;
mod focus;
mod home;
mod logo;
mod overlay;
mod theme;

use std::collections::HashMap;
use std::path::PathBuf;

use focus::Focus;
use overlay::PageView;
use pagify_shell::automate::{Recorder, Script};
use pagify_shell::command::{CommandBox, Dispatch, Escaped, Kind, Submit};
use pagify_shell::markup::{Markup, HIT_TOLERANCE_PT};
use pagify_shell::measure::{self, Calibration};
use pagify_shell::page_space::AppPoint;
use pagify_shell::reader::{prefetch_targets, Strip, PAGE_GAP_PT};
use pagify_shell::recent::Recent;
use pagify_shell::tools::{self, SnapSet};
use pagify_shell::verbs::{self, MeasureKind, PageTarget, Verb, ZoomTarget};
use pagify_shell::{PageRaster, Session};
use pdf_core::document::Color;
use pdf_core::error::PdfError;
use pdf_core::Rotation;

const COMMAND_INPUT: &str = "pagify::command_input";
const THUMB_SCALE: f32 = 0.12;
const MARKUP_INK: Color = Color { r: 0xFF, g: 0x5C, b: 0x8A, a: 0xFF };
/// The colour a signature is drawn in.
///
/// Blue-black rather than the markup pink, and rather than plain black: the
/// convention on paper is that a signature in blue is the original and one in
/// black is a photocopy, and a signature that looks like a highlighter mark
/// reads as an annotation somebody added rather than as a name.
const SIGNATURE_INK: Color = Color { r: 0x14, g: 0x2B, b: 0x63, a: 0xFF };

fn main() -> eframe::Result<()> {
    // `pagify [FILE] [--run "<command>"]…`
    //
    // `--run` exists because §7's claim — anything typeable is scriptable —
    // is only worth something if something other than a person can type. It is
    // also the only way to drive the program from a test or a shell script
    // without a pointer.
    // Before anything else, because AppKit hands over a double-clicked
    // document *before* the window exists — see `mac_open`.
    mac_open::watch_for_delegate();

    let mut path = None;
    let mut startup: Vec<String> = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--run" => {
                if let Some(command) = args.next() {
                    startup.push(command);
                }
            }
            _ if path.is_none() => path = Some(arg),
            _ => {}
        }
    }
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1240.0, 860.0])
        .with_min_inner_size([640.0, 480.0])
        .with_title("Pagify");
    if let Some(icon) = logo::icon() {
        viewport = viewport.with_icon(icon);
    }

    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native(
        "Pagify",
        options,
        Box::new(move |cc| {
            // Finder hands a double-clicked document over by Apple Event, not
            // in `argv`. Registered **here** rather than before the event loop:
            // AppKit installs its own handler for that event while the
            // application finishes launching, and whichever handler is
            // registered last wins. Registered first, ours was simply
            // overwritten — measured, it never fired once.
            mac_open::listen();

            theme::apply(&cc.egui_ctx);
            let mut app = PagifyApp::new(path.as_deref());
            for command in &startup {
                app.submit(command);
            }
            Ok(Box::new(app))
        }),
    )
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ZoomMode {
    Factor(f32),
    Fit,
    Width,
}

/// What a command is still waiting for from the pointer.
///
/// Objects and points are collected separately because they are not the same
/// thing: fillet wants two *objects* clicked on, move wants two *points*, and
/// offset wants an object and then a point saying which side. Treating both as
/// "clicks" is what made fillet perform a move.
struct Pending {
    kind: PendingKind,
    page: usize,
    objects: Vec<(usize, AppPoint)>,
    points: Vec<AppPoint>,
}

enum PendingKind {
    /// Waiting for a click on the words to change.
    PickText,
    /// Words waiting for a point to be written at.
    Write(String),
    Draw(DrawKind),
    Modify(tools::Pick),
    Calibrate { distance: f64, unit: String },
    Measure(MeasureKind),
    /// Two corners of an area whose contents are to be destroyed.
    Redact,
    /// A point to put a tick, a cross or a dot at.
    Fill(pdf_core::document::FillMark),
    /// Where a drawn signature is to sit — on the line that is clicked.
    Signature,
    /// Two corners of a box to draw while filling a form in.
    SignRectangle,
    /// Something on the page, and where it is to go.
    ///
    /// `pictures_first` is which of two overlapping things is meant. The Move
    /// tool takes the smallest — a caption on a photograph is the caption. Edit
    /// Object sits beside a separate text tool, so there it is the picture.
    Move { pictures_first: bool },
    /// The two ends of a line to rule while filling a form in.
    SignLine,
    /// Two corners of an area to paint over.
    ///
    /// **Covers; does not remove.** Kept apart from `Redact` for the same
    /// reason the verbs are: the difference is the whole point, and a flag on
    /// one is how somebody ends up with the other.
    Whiteout,
    /// Two corners of an area to hide, sealed under a passcode.
    Lock,
}

/// A signature being drawn.
///
/// Strokes in the pad's own coordinates; only their proportions are kept when
/// it is saved — see [`pagify_shell::signatures::Signature::from_drawing`].
#[derive(Debug, Default, Clone)]
struct SignaturePad {
    strokes: Vec<Vec<(f32, f32)>>,
    /// What to call it. A name for a list, not a claim about anybody.
    name: String,
    /// Whether to arm placement once it is saved — true when the pad was
    /// opened by reaching for the tool with nothing drawn yet, so the one
    /// action somebody took carries on to the thing they wanted.
    then_place: bool,
}

/// The Manage Signatures panel, while it is open.
#[derive(Debug, Default, Clone)]
struct SignatureList {
    /// Which signature is being renamed, and what has been typed for it.
    renaming: Option<(String, String)>,
    /// A signature waiting for a second press before it is forgotten.
    ///
    /// **Because this one does not come back.** Undo reaches into the
    /// document; it does not reach into a drawing kept beside it, and no
    /// document holds a copy to draw the signature back from.
    doomed: Option<String>,
}

/// The Predefined Text panel, while it is open.
#[derive(Debug, Default, Clone)]
struct SnippetList {
    /// What is being typed into the box that adds one.
    adding: String,
}

/// What a press in the Manage Signatures panel asked for.
///
/// Gathered while the panel is drawn and acted on afterwards: the panel holds
/// the list borrowed, and every one of these changes it.
enum ListAction {
    Use(String),
    Forget(String),
    Rename(String, String),
    Draw,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum DrawKind {
    Line,
    Circle,
    Rectangle,
    Polyline,
}

/// How many of each a pick still wants. `usize::MAX` means "until Enter".
impl PendingKind {
    fn wants(&self) -> (usize, usize) {
        match self {
            PendingKind::PickText => (0, 1),
            PendingKind::Write(_) => (0, 1),
            PendingKind::Draw(DrawKind::Line | DrawKind::Circle | DrawKind::Rectangle) => (0, 2),
            PendingKind::Draw(DrawKind::Polyline) => (0, usize::MAX),
            PendingKind::Modify(pick) => (pick.objects, pick.points),
            PendingKind::Calibrate { .. } => (0, 2),
            PendingKind::Whiteout => (0, 2),
            PendingKind::Fill(_) => (0, 1),
            PendingKind::Signature => (0, 1),
            PendingKind::SignRectangle => (0, 2),
            PendingKind::Move { .. } => (0, 2),
            PendingKind::SignLine => (0, 2),
            PendingKind::Measure(MeasureKind::Distance) => (0, 2),
            PendingKind::Measure(MeasureKind::Area) => (0, usize::MAX),
            PendingKind::Redact | PendingKind::Lock => (0, 2),
        }
    }

    fn prompt(&self, objects_done: usize, points_done: usize) -> String {
        match self {
            PendingKind::Write(text) => {
                let short: String = text.chars().take(24).collect();
                format!(
                    "click where \"{short}{}\" goes",
                    if text.chars().count() > 24 { "…" } else { "" }
                )
            }
            PendingKind::PickText => "click the words to change".into(),
            PendingKind::Redact => match points_done {
                0 => "redact: first corner of the area to destroy".into(),
                _ => "redact: opposite corner".into(),
            },
            // Says what it does *not* do, because that is the thing somebody
            // reaching for it might be wrong about.
            PendingKind::Whiteout => match points_done {
                0 => "whiteout: first corner — this covers, it does not remove".into(),
                _ => "whiteout: opposite corner".into(),
            },
            PendingKind::Fill(mark) => {
                format!("fill: click where the {} goes", mark.describe())
            }
            // Says where the click lands, because a signature that appears
            // above or below the line is the thing to get right first time.
            PendingKind::Signature => "signature: click the line to sign on".into(),
            // Says which of the two rectangles this is, because the other one
            // is a drawing that can be picked up again and this one is not.
            PendingKind::SignRectangle => match points_done {
                0 => "rectangle: first corner — a mark on the form, not a drawing".into(),
                _ => "rectangle: opposite corner".into(),
            },
            PendingKind::Move { pictures_first } => match (pictures_first, points_done) {
                (true, 0) => "edit object: click the picture to move".into(),
                (false, 0) => "move: click the words or the picture to move".into(),
                (_, _) => "click where it goes".into(),
            },
            PendingKind::SignLine => match points_done {
                0 => "line: from — a mark on the form, not a drawing".into(),
                _ => "line: to".into(),
            },
            PendingKind::Lock => match points_done {
                0 => "lock: first corner of the area to hide".into(),
                _ => "lock: opposite corner".into(),
            },
            PendingKind::Draw(kind) => match (kind, points_done) {
                (DrawKind::Line, 0) => "line: from".into(),
                (DrawKind::Line, _) => "line: to".into(),
                (DrawKind::Circle, 0) => "circle: centre".into(),
                (DrawKind::Circle, _) => "circle: a point on it".into(),
                (DrawKind::Rectangle, 0) => "rectangle: first corner".into(),
                (DrawKind::Rectangle, _) => "rectangle: opposite corner".into(),
                (DrawKind::Polyline, n) => {
                    format!("polyline: point {} — Enter to finish", n + 1)
                }
            },
            PendingKind::Modify(pick) => pick.prompt(objects_done, points_done),
            PendingKind::Calibrate { .. } => {
                if points_done == 0 {
                    "calibrate: first of the two points".into()
                } else {
                    "calibrate: second point".into()
                }
            }
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
    /// The exceptions are the two that answer a question rather than make a
    /// mark: calibration is set once, and picking a run of text opens an editor
    /// which is where the attention now belongs.
    fn repeats(&self) -> bool {
        !matches!(self, PendingKind::Calibrate { .. } | PendingKind::PickText)
    }

    /// Whether Enter can end it early.
    /// The ribbon command that arms this, so the button can show itself lit
    /// while it is collecting clicks.
    ///
    /// `None` where no button arms it — a pick started from a typed command
    /// with no ribbon equivalent has nothing to light up.
    fn command(&self) -> Option<&'static str> {
        Some(match self {
            PendingKind::Draw(DrawKind::Line) => "line",
            PendingKind::Draw(DrawKind::Circle) => "circle",
            PendingKind::Draw(DrawKind::Polyline) => "pline",
            PendingKind::Draw(DrawKind::Rectangle) => return None,
            PendingKind::Redact => "redact",
            PendingKind::Whiteout => "whiteout",
            // No ribbon button lights up per mark; the tool is one word with
            // an argument.
            PendingKind::Fill(_) => return None,
            PendingKind::Signature => "signature",
            PendingKind::SignRectangle => "signrectangle",
            PendingKind::Move { pictures_first: true } => "editobject",
            PendingKind::Move { pictures_first: false } => "moveobject",
            PendingKind::SignLine => "signline",
            PendingKind::Lock => "lock",
            PendingKind::PickText => "edittext",
            PendingKind::Write(_) => "addtext",
            PendingKind::Measure(MeasureKind::Distance) => "measure distance",
            PendingKind::Measure(MeasureKind::Area) => "measure area",
            PendingKind::Calibrate { .. } => "calibrate",
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
    fn wants_snapping(&self) -> bool {
        matches!(
            self,
            PendingKind::Draw(_)
                | PendingKind::Modify(_)
                | PendingKind::Measure(_)
                | PendingKind::Calibrate { .. }
        )
    }

    fn ends_on_enter(&self) -> bool {
        let (_, points) = self.wants();
        points == usize::MAX
    }
}

impl Pending {
    fn ready(&self) -> bool {
        let (objects, points) = self.kind.wants();
        objects != usize::MAX
            && points != usize::MAX
            && self.objects.len() >= objects
            && self.points.len() >= points
    }

    /// Whether the next click should pick an object rather than a free point.
    fn wants_object(&self) -> bool {
        let (objects, _) = self.kind.wants();
        self.objects.len() < objects
    }

    fn prompt(&self) -> String {
        self.kind.prompt(self.objects.len(), self.points.len())
    }
}

struct Doc {
    /// Shared, so recognition can run off the UI thread.
    ///
    /// `Session` is just a handle; every call through it takes the registry
    /// lock, which is what makes PDFium access from two threads safe at all.
    /// The `Arc` is about *lifetime* — the worker must not outlive the document
    /// it is reading — not about safety.
    session: std::sync::Arc<Session>,
    strip: Strip,
    page_count: usize,
    /// One texture per (page, quantised scale). Bounded by eviction of pages
    /// that have scrolled out — a fifty-page document at full zoom would
    /// otherwise hold fifty full-size rasters on the GPU.
    textures: HashMap<(usize, u32, u8), egui::TextureHandle>,
    thumbs: HashMap<usize, egui::TextureHandle>,
}

impl Doc {
    /// Throw away everything already drawn from this document.
    ///
    /// **Both caches, always.** Reported from use: a locked page still showed
    /// its contents in the thumbnail strip. Thirteen places cleared `textures`
    /// after an edit and two cleared `thumbs`, so almost every edit left a
    /// stale thumbnail — and for a lock or a redaction that is not a cosmetic
    /// lag, it is the hidden content still on screen.
    ///
    /// One method rather than two calls at each site, because the next edit
    /// added will call this and be right by default.
    fn rendered_is_stale(&mut self) {
        self.textures.clear();
        self.thumbs.clear();
    }
}

struct PagifyApp {
    doc: Option<Doc>,
    cmd: CommandBox,
    markup: Markup,
    calibration: Calibration,
    recorder: Recorder,
    pending: Option<Pending>,

    page: usize,
    zoom: ZoomMode,
    rotation: Rotation,
    scroll_pt: f32,
    /// Where the current page was last drawn on screen.
    ///
    /// Recorded because it is the only place that knows it: the mapping is
    /// built inside the draw loop from the scroll offset and the zoom, and
    /// nothing outside can reconstruct where a point on the page ended up. The
    /// UI tests need it to put the pointer on a character.
    last_view: Option<PageView>,
    /// Where the page strip is scrolled to, kept so zooming can hold the point
    /// under the cursor still.
    scroll_offset: egui::Vec2,
    /// Set when a zoom needs the scroll offset moved with it, applied on the
    /// next frame's `ScrollArea`.
    anchor_offset: Option<egui::Vec2>,
    /// A run of text being retyped **on the page**, where it sits.
    ///
    /// Not in the command box. Editing a word is a thing you do to the word,
    /// and looking somewhere else to do it means holding the page in your head
    /// while you type — which is exactly what an editor should not ask of you.
    editing_run: Option<EditingRun>,
    /// The id for the next words written onto a page.
    ///
    /// Distinct per mark, because `remove_text` removes **every** object
    /// carrying an id — undoing one line of writing would otherwise take every
    /// other line with it. Well clear of `TEXT_LAYER_ID`, which recognition
    /// owns.
    next_text_id: i32,
    /// A markup tool waiting for text to be selected.
    ///
    /// A highlighter is a thing you pick up and then use, not a thing you
    /// reach for after the fact. Requiring the selection first means the tool
    /// can only ever be applied once per selection, and reads as a button that
    /// scolds you.
    markup_armed: Option<pagify_shell::verbs::Markup>,
    /// A foreign annotation the pointer is on, as its position in `marks`.
    ///
    /// Cached per page so hit-testing does not re-read every annotation on
    /// every frame of a mouse move.
    foreign: Option<(usize, Vec<(usize, Vec<pdf_core::document::Rect>)>)>,
    /// The page the current text selection belongs to.
    ///
    /// The selection used to be dropped whenever the current page changed, and
    /// the current page now follows the scroll — so nudging the wheel after
    /// highlighting something threw the highlight away, and ⌘C then had nothing
    /// to copy. It survives; it just remembers which page it is on.
    selection_page: usize,
    /// Frames to leave the current page alone after a jump.
    ///
    /// A programmatic scroll takes a frame or two to arrive, and the
    /// follow-the-scroll rule would read the *old* position in the meantime and
    /// put the page straight back — which is why clicking a thumbnail appeared
    /// to do nothing at all.
    settling: u8,
    /// The mapping for the page under the pointer, and which page it is.
    ///
    /// Not always the current page, and the index matters: a `PageView` maps to
    /// coordinates *within its own page*, so two views cannot be compared
    /// without knowing which pages they belong to.
    hover_view: Option<(usize, PageView)>,
    /// The scroll area's own viewport, from the last frame.
    ///
    /// Zoom anchoring has to measure the pointer against the rectangle that is
    /// actually scrolling, not the panel around it — they differ by the margins
    /// and the scrollbar, and the difference shows up as the page creeping away
    /// from the cursor as you zoom.
    viewport_rect: Option<egui::Rect>,
    /// A copy was asked for by something with no `egui::Context` to hand — a
    /// typed `copy`, a ribbon button, a menu item. Carried out at the end of
    /// the frame, where the context is available.
    copy_wanted: bool,
    /// Screen pixels the view should move by, accumulated from a pan gesture
    /// and applied to the scroll area on the next frame.
    ///
    /// It has to go through the scroll area, because the scroll area owns the
    /// offset. Hand mode used to write to `scroll_pt`, which nothing reads —
    /// so dragging with the Hand tool moved nothing at all.
    pan_by: Option<egui::Vec2>,
    /// A page to bring into view, in strip points from the top.
    ///
    /// `page next` set `scroll_pt` and nothing ever read it — the scroll area
    /// owns its own offset — so going to a page changed which page was
    /// *current* without moving the window to it.
    scroll_to_pt: Option<f32>,
    canvas_pt: egui::Vec2,

    recent: Recent,

    /// Extra fonts a reader has added for outlined-text recognition, beyond
    /// `BUNDLED_OUTLINED_FONTS` — see `outlined_font_bytes`.
    outlined_fonts: pagify_shell::outlined_fonts::OutlinedFonts,

    /// The image a click landed on, and the page it is on.
    ///
    /// Held so a right-click can offer to lock it: the menu opens on a later
    /// frame than the click that selected it, so what was under the pointer has
    /// to survive in between.
    selected_image: Option<(usize, pdf_core::document::PageImage)>,

    /// The current page's characters, kept because extracting them costs a
    /// text-page load and a walk, and a selection drag asks on every frame.
    text: Option<(usize, pagify_shell::reader::Characters)>,
    text_selection: Option<std::ops::Range<usize>>,
    /// Where a text drag began. `None` means a drag is selecting marks instead.
    text_drag: Option<AppPoint>,

    find_needle: String,
    /// Every match, as (page, character range).
    find_hits: Vec<(usize, std::ops::Range<usize>)>,
    find_at: usize,
    defaults: tools::Defaults,
    /// The markup revision at the last successful save. Anything above it is
    /// work that closing would throw away.
    saved_revision: u64,
    /// Decoded on the first frame — a `Context` is needed to upload it and
    /// there is none when the app is constructed.
    mark: Option<egui::TextureHandle>,
    snaps: SnapSet,
    ortho: bool,
    grid_pt: f64,
    show_thumbs: bool,
    /// Whether the command box shows its history, or is the single line the
    /// mockup draws. Collapsed by default — the history is worth seeing when
    /// you are working in it and is dead space when you are reading.
    command_open: bool,
    ribbon: Tab,

    /// A page being read, off the UI thread.
    ///
    /// Recognition is about a second of solid CPU per page. Run in `update` it
    /// stops the window dead — no repaint, no scrolling, no way to cancel —
    /// and on a twenty-page document that is twenty seconds of a frozen app.
    reading: Option<Reading>,
    /// Loaded on first use and kept. The models are twelve megabytes and take
    /// a moment to memory-map; doing that per page would make the second page
    /// as slow as the first for no reason.
    recogniser: Option<std::sync::Arc<pdf_core::ocr::engine::OcrsRecogniser>>,
    /// What the user asked to do, held while they decide what to do about
    /// unsaved marks.
    closing: Option<Closing>,
    /// A redaction the survey found something in the way of, waiting on an
    /// answer.
    ///
    /// Held rather than refused, for the reason measuring found: of the pages
    /// where an image blocks a redaction on the 2026 catalogue, most had their
    /// text come out perfectly cleanly and were stopped by a photograph beside
    /// it. Whether that photograph matters is not a question this program can
    /// answer and is an easy one for whoever is looking at the page.
    asking_to_redact: Option<PendingRedaction>,
    /// What the next line typed is a passcode **for**.
    ///
    /// Never the passcode itself. Only what is waiting on one — a path, an area
    /// to lock — so that the secret goes from the input to the engine and is
    /// dropped, never touching the visible history, the recorder, or this
    /// struct.
    awaiting_password: Option<Awaiting>,
    /// What has been typed into the password window.
    password_typed: String,
    /// Whether the window's field has been given the caret yet.
    password_field_focused: bool,
    /// What the window should say went wrong, if anything.
    password_problem: Option<String>,
    /// Whether the password being chosen is Pagify's own rather than PDF's.
    ///
    /// Held here rather than on the `Awaiting` because it is a toggle in the
    /// window, changed while the same question is being asked.
    password_plus: bool,
    /// The signatures this person has drawn, and where they are kept.
    ///
    /// The path is held rather than asked for each time so a test can point it
    /// at a scratch file: writing a test signature into somebody's real
    /// settings would be a poor way to find out this works.
    signatures: pagify_shell::signatures::Signatures,
    signatures_path: Option<std::path::PathBuf>,
    /// The pad, while a signature is being drawn on it.
    pad: Option<SignaturePad>,
    /// The Manage Signatures panel, while it is open.
    signature_list: Option<SignatureList>,
    /// The words kept for writing again, and where they are kept.
    predefined: pagify_shell::predefined::Predefined,
    predefined_path: Option<std::path::PathBuf>,
    /// The Predefined Text panel, while it is open.
    snippets: Option<SnippetList>,
    /// The document face installed for the editor, and whether egui has
    /// rebuilt its atlas with it yet.
    ///
    /// **Two fields because the family does not exist until the next frame.**
    /// `set_fonts` takes effect at the start of the frame after it is called,
    /// and drawing in a family nothing is bound to panics rather than falling
    /// back — the same trap `install_fonts` warns about for the icons. So the
    /// face is asked for on one frame and used on the next.
    editor_face: Option<u64>,
    editor_face_ready: bool,
    /// A face asked for and not yet installed, handed to `install_fonts` at the
    /// top of the next frame.
    pending_face: Option<Vec<u8>>,
    /// The words a page draws rather than writes, and which page they are for.
    ///
    /// Recognising them costs real work, and a pick asks for them on every
    /// click that lands on no text.
    drawn_words: Option<(usize, Vec<pdf_core::document::RecognisedWord>)>,
    /// What a drag on the page means. Set by Hand and Select.
    pointer: pagify_shell::verbs::PointerMode,
    drag_from: Option<AppPoint>,
    last_snap: Option<tools::Snapped>,
}

/// The rectangle two dragged corners describe, or `None` if it has no area.
///
/// Shared by the two tools that draw one, so that a lock and a redaction cannot
/// disagree about what the same gesture meant.
fn area_between(a: AppPoint, b: AppPoint) -> Option<pdf_core::document::Rect> {
    let area = pdf_core::document::Rect {
        left: a.x.min(b.x) as f32,
        top: a.y.min(b.y) as f32,
        right: a.x.max(b.x) as f32,
        bottom: a.y.max(b.y) as f32,
    };
    // A point is not an area. Below a point across, the gesture was a click that
    // wandered, and clearing "everything inside it" would mean clearing nothing
    // while looking as though something had happened.
    ((area.right - area.left) >= 1.0 && (area.bottom - area.top) >= 1.0).then_some(area)
}

/// One ribbon button: the glyph, the name under it, and the command it runs.
///
/// A command string rather than a callback, because §7 makes the command box
/// the single way anything happens — a button that did something the box could
/// not would be a second, undiscoverable interface.
pub type Tool = (&'static str, &'static str, &'static str);

/// Hand and Select, which the reference toolbar repeats at the head of every
/// tab.
///
/// Written once and rendered on each tab rather than copied into all fifteen:
/// they are the same two tools, and fifteen copies is fifteen chances for them
/// to drift apart. The File tab is the exception — it is a backstage, not a
/// toolbar.
const ALWAYS: &[Tool] = &[
    ("\u{E925}", "Hand", "hand"),
    ("\u{EF52}", "Select", "selecttool"),
];

/// A run of text open for editing, in place.
#[derive(Clone)]
struct EditingRun {
    page: usize,
    object: usize,
    /// What it said when it was picked, so undo and "unchanged" both have
    /// something true to compare against.
    original: String,
    /// Where it sits, in page points — the box the editor is drawn over.
    rect: pdf_core::document::Rect,
    /// What is being typed.
    buffer: String,
    /// The appearance, as it will be applied. Seeded from the run so that
    /// leaving the controls alone changes nothing.
    style: pdf_core::document::TextStyle,
    /// The appearance as it was, so "did anything change" is answerable
    /// without asking the document again.
    was: pdf_core::document::TextStyle,
    /// Focus is asked for once. Asking every frame fights anything else that
    /// wants it, including the editor itself.
    focused: bool,
    /// How far the grip has dragged these words, in page points, before it is
    /// let go.
    ///
    /// Held rather than applied as it moves: every real move re-emits the
    /// page's content stream, and doing that sixty times a second while
    /// somebody drags would be unusable. The words follow the pointer as a
    /// preview; the page changes once, on release.
    drag_by: (f32, f32),
    /// The colour of the page *behind* these words.
    ///
    /// The field is painted over the page while it is open, because the words
    /// underneath are still there until the edit is applied and typing over
    /// them is unreadable. Painting the program's own panel colour turned that
    /// into a dark slab in the middle of a white page — the thing somebody is
    /// editing stopped looking like the thing they were looking at. Sampled
    /// from the page itself, once, when the run is picked.
    background: pdf_core::document::Color,
    /// Whether these words are *drawn* — type converted to outlines — rather
    /// than written as text.
    ///
    /// Kept from the moment of picking: what is on the page can change under an
    /// edit, and what has to happen afterwards is decided by what was picked.
    drawn: bool,
}

/// A page in the hands of the recogniser.
struct Reading {
    /// Pages still to read, and the one in hand — for the progress line.
    queue: Vec<usize>,
    at: usize,
    done: std::sync::mpsc::Receiver<PageResult>,
    /// Set when the user gives up.
    ///
    /// A **real** stop, not a declined answer: the pipeline checks it between
    /// lines and between tiles, so the worker puts the page down rather than
    /// finishing it for nobody. On one page that is a second of wasted CPU; on
    /// a 149-page catalogue it is two and a half minutes of fans after the user
    /// has said stop.
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// What has been written so far, for the closing report.
    words: usize,
    pages: usize,
}

/// One page, back from the worker.
struct PageResult {
    page: usize,
    reading: Result<pdf_core::ocr::pipeline::PageReading, String>,
    /// Set once, the run a page first needs OCR and nothing was cached yet —
    /// so `collect_reading` can warm `self.recogniser` for next time instead
    /// of every later call reloading the model files from disk.
    built_recogniser: Option<std::sync::Arc<pdf_core::ocr::engine::OcrsRecogniser>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Decision {
    Save,
    Discard,
    Cancel,
}

/// What is waiting for a passcode to be typed.
///
/// The variants exist so that one interception path covers every secret the
/// program asks for. A second route that only *mostly* kept a passcode out of
/// the history would be worse than none, because the first one's care would
/// make it look handled.
#[derive(Debug, Clone, PartialEq)]
enum Awaiting {
    /// A file that would not open without one.
    Open(String),
    /// An area to hide, once there is a passcode to seal it under.
    ///
    /// `require_complete` carries which gesture asked. A dragged rectangle
    /// means "this area" and must refuse what it cannot clear; a text selection
    /// means "these words" and must not refuse on account of an image that was
    /// never selected — see `Session::lock_area`.
    Lock { page: usize, area: pdf_core::document::Rect, require_complete: bool },
    /// Whole pages to hide, once there is a passcode to seal them under.
    LockPages(Vec<usize>),
    /// The password on a certificate file, asked for before signing with it.
    ///
    /// A password being *used*, not chosen — the certificate already has one —
    /// so no rule applies and it is not typed twice.
    Certificate(std::path::PathBuf),
    /// The password a document **already has**, asked for before a new one is
    /// chosen.
    ///
    /// Changing a password needs the current one — otherwise anyone walking
    /// past an open document could change it — and the old route said to save
    /// a plain copy first, which was both laborious and, as it turned out, did
    /// not work.
    SecureCurrent(pagify_shell::verbs::SecureOptions),
    /// A password to put on the file itself, and what it will still permit.
    ///
    /// Kept apart from the locking variants because it is a different promise:
    /// those hide content inside a document anyone can open, this shuts the
    /// document to everyone without the password.
    Secure(pagify_shell::verbs::SecureOptions),
    /// The same passcode again, when one is being chosen for the first time.
    ///
    /// Only on the **first** lock in a document. After that the passcode
    /// already exists and is being *used* rather than chosen, so asking twice
    /// would be asking somebody to confirm a fact.
    LockAgain { first: String, then: Box<Awaiting> },
    /// The same password again.
    ///
    /// **Asked because a mistyped password here cannot be discovered later.**
    /// Every other passcode in this program guards something the document also
    /// still contains; this one *is* the document. Somebody who mistypes it
    /// finds out when they next open the file, by which time nothing can be
    /// done — reported from use exactly that way.
    SecureAgain { first: String, options: pagify_shell::verbs::SecureOptions },
    /// One image to hide, once there is a passcode to seal it under.
    LockImage { page: usize, object: usize },
    /// One sealed object to bring back — what clicking a lock badge asks for.
    UnlockItem(String),
    /// Bring back everything this document has sealed.
    Unlock,
}

/// A redaction waiting on the user, with what the survey found.
#[derive(Debug, Clone)]
struct PendingRedaction {
    page: usize,
    area: pdf_core::document::Rect,
    report: pdf_core::document::RedactionReport,
}

/// What was being attempted when unsaved marks got in the way.
///
/// Held rather than refused. Telling somebody their work is unsaved and then
/// giving them no way through except a command they have to be told about is
/// not a safeguard, it is a trap — the window simply will not close, and the
/// only escape is typing something.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Closing {
    /// Close the document, keep the program.
    Document,
    /// Close the program.
    Program,
    /// Open something else in its place — which discards the current document
    /// just as surely as closing it, and was refused the same way.
    Open(PathBuf),
    OpenDialog,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    File,
    Home,
    Convert,
    Edit,
    Organize,
    Comment,
    View,
    Form,
    Protect,
    PagiSign,
    Share,
    Accessibility,
    Help,
    Draw,
    Automate,
}

impl Tab {
    const ALL: [Tab; 15] = [
        Tab::File, Tab::Home, Tab::Convert, Tab::Edit,
        Tab::Organize, Tab::Comment, Tab::View, Tab::Form,
        Tab::Protect, Tab::PagiSign, Tab::Share, Tab::Accessibility,
        Tab::Help, Tab::Draw, Tab::Automate,
    ];

    fn label(self) -> &'static str {
        match self {
            Tab::File => "File",
            Tab::Home => "Home",
            Tab::Convert => "Convert",
            Tab::Edit => "Edit",
            Tab::Organize => "Organize",
            Tab::Comment => "Comment",
            Tab::View => "View",
            Tab::Form => "Form",
            Tab::Protect => "Protect",
            Tab::PagiSign => "PagiSign",
            Tab::Share => "Share",
            Tab::Accessibility => "Accessibility",
            Tab::Help => "Help",
            Tab::Draw => "Draw",
            Tab::Automate => "Automate",
        }
    }

    /// Hand and Select come first on every tab except File.
    ///
    /// Select runs `selecttool`, not `select`. `select` belongs to the kernel,
    /// where it picks geometry to modify — shadowing it would take that away
    /// from anyone drawing, to give the name to a mode switch that had not been
    /// built yet. `every_collision_with_the_kernel_is_deliberate` is what
    /// caught it.
    fn leading(self) -> &'static [Tool] {
        if self == Tab::File { &[] } else { ALWAYS }
    }

    fn buttons(self) -> &'static [Tool] {
        match self {
            Tab::File => &[
                ("\u{E2C8}", "Open…", "open"),
                ("\u{E5CD}", "Close", "close"),
                ("\u{E161}", "Save", "save"),
                ("\u{E8B8}", "PDFium", "pdfium"),
                ("\u{F8C7}", "Quit", "quit"),
            ],
            Tab::Home => &[
                ("\u{E412}", "Snapshot", "snapshot"),
                ("\u{E14D}", "Copy", "copy"),
                ("\u{E8E7}", "Bookmark", "bookmark"),
                ("\u{E8FF}", "Zoom In", "zoom in"),
                ("\u{E900}", "Zoom Out", "zoom out"),
                ("\u{EA10}", "Fit Page", "zoom fit"),
                ("\u{F779}", "Fit Width", "zoom width"),
                ("\u{E3F4}", "Actual Size", "zoom actual"),
                ("\u{E5FA}", "Extract Text", "extracttext"),
                ("\u{E41A}", "Rotate View", "rotate"),
                ("\u{E262}", "Edit Text", "edittext"),
                ("\u{E162}", "Edit Object", "editobject"),
                ("\u{E89F}", "Move", "moveobject"),
                ("\u{F82B}", "Highlight", "highlight"),
                ("\u{E312}", "Typewriter", "addtext "),
                ("\u{E418}", "Rotate Pages", "rotatepages"),
                ("\u{E145}", "Insert", "insertpage"),
                ("\u{E329}", "From Scanner", "fromscanner"),
                ("\u{E746}", "Fill & Sign", "fillsign"),
                // The two a form actually asks for, one press away.
                ("\u{E668}", "Tick", "fillsign tick"),
                ("\u{E5CD}", "Cross", "fillsign cross"),
            ],
            Tab::Convert => &[
                ("\u{E873}", "From Files", "fromfiles"),
                ("\u{E329}", "From Scanner", "fromscanner"),
                ("\u{E14F}", "From Clipboard", "fromclipboard"),
                ("\u{E0EE}", "Form", "createform"),
                ("\u{EBBD}", "PDF Portfolio", "portfolio"),
                ("\u{EB98}", "Combine Files", "combine"),
                ("\u{E66D}", "Blank", "blankdoc"),
                ("\u{E99B}", "From Template", "fromtemplate"),
                ("\u{EFA2}", "Export All Images", "exportimages"),
                ("\u{F1BE}", "To MS Office", "tooffice"),
                ("\u{E3F4}", "To Image", "toimage"),
                ("\u{EB7E}", "To HTML", "tohtml"),
                ("\u{F720}", "To Other", "toother"),
                ("\u{F0C5}", "Preflight", "preflight"),
            ],
            Tab::Edit => &[
                ("\u{E262}", "Edit Text", "edittext"),
                ("\u{E162}", "Edit Object", "editobject"),
                ("\u{E236}", "Link & Join Text", "jointext"),
                ("\u{E8CE}", "Check Spelling", "spelling"),
                ("\u{E881}", "Search & Replace", "replace"),
                ("\u{EAE2}", "Add Text", "addtext"),
                ("\u{E43E}", "Add Images", "addimage"),
                ("\u{E72C}", "Add Shapes", "addshape"),
                ("\u{E8EC}", "Add Article Box", "articlebox"),
                ("\u{EA07}", "Web Links", "weblinks"),
                ("\u{E250}", "Link", "addlink"),
                ("\u{E8E7}", "Bookmark", "bookmark"),
                ("\u{F184}", "Cross Reference", "crossref"),
                ("\u{E661}", "Auto Create Bookmarks", "autobookmarks"),
                ("\u{E226}", "File Attachment", "attach"),
                ("\u{E439}", "Image Annotation", "imageannotation"),
                ("\u{E404}", "Audio & Video", "media"),
                ("\u{EFC9}", "Add 3D", "add3d"),
            ],
            Tab::Organize => &[
                ("\u{E9B0}", "Thumbnail View", "thumbnails"),
                ("\u{E145}", "Insert", "insertpage"),
                ("\u{E92E}", "Delete", "deletepage"),
                ("\u{E14E}", "Extract", "extract"),
                ("\u{E8D5}", "Reverse", "reversepages"),
                ("\u{E8FE}", "Rearrange", "rearrange"),
                ("\u{E89F}", "Move", "movepage"),
                ("\u{E173}", "Duplicate", "duplicatepage"),
                ("\u{F232}", "Replace", "replacepage"),
                ("\u{E0B6}", "Split", "split"),
                ("\u{E8D4}", "Swap", "swappages "),
                ("\u{EAF4}", "Interleaving", "interleave"),
                ("\u{E418}", "Rotate Pages", "rotatepages"),
                ("\u{E3BE}", "Crop Pages", "croppages all "),
                ("\u{E85B}", "Resize Pages", "resizepages all "),
                ("\u{E53C}", "Flatten", "flatten"),
                ("\u{E41C}", "Page Marks", "pagemarks"),
            ],
            Tab::Comment => &[
                ("\u{F82B}", "Highlight", "highlight"),
                ("\u{E249}", "Underline", "underline"),
                ("\u{E246}", "Strikeout", "strikeout"),
                ("\u{E155}", "Squiggly", "squiggly"),
                ("\u{E0D7}", "Replace Text", "replacetext"),
                ("\u{F735}", "Insert Text", "inserttext"),
                ("\u{F1FC}", "Note", "note "),
                ("\u{E2BC}", "File", "attachcomment"),
                ("\u{E312}", "Typewriter", "addtext "),
                ("\u{E3BC}", "Textbox", "textbox"),
                ("\u{E0CB}", "Callout", "callout"),
                ("\u{EBBB}", "Drawing", "drawing"),
                ("\u{F097}", "Pencil", "pencil"),
                ("\u{E6D0}", "Eraser", "erase"),
                ("\u{E162}", "Area Highlight", "areahighlight"),
                ("\u{F02F}", "Search & Highlight", "searchhighlight"),
                ("\u{EA5F}", "Accounting Calculator", "calculator"),
                ("\u{EA49}", "Measure", "measure distance"),
                ("\u{E982}", "Stamp", "stamp"),
                ("\u{EF76}", "Custom Stamp", "customstamp"),
                ("\u{E668}", "TickMark", "tickmark"),
                ("\u{E8AF}", "Manage Comments", "managecomments"),
                ("\u{F10D}", "Keep Tool Selected", "keeptool"),
            ],
            Tab::View => &[
                ("\u{E40A}", "Change Color", "changecolor"),
                ("\u{E3E8}", "Reverse View", "reverseview"),
                ("\u{E41A}", "Rotate View", "rotate"),
                ("\u{E41C}", "Toggle Ruler", "ruler"),
                ("\u{E3C5}", "Single Page", "viewsingle"),
                ("\u{E0E0}", "Facing", "viewfacing"),
                ("\u{E8ED}", "Continuous", "viewcontinuous"),
                ("\u{E666}", "Continuous Facing", "viewcontinuousfacing"),
                ("\u{E86E}", "Separate Cover", "viewcover"),
                ("\u{E0B6}", "Split", "viewsplit"),
                ("\u{E235}", "Reflow", "reflow"),
                ("\u{E71C}", "Page Transitions", "transitions"),
                ("\u{E258}", "AutoScroll", "autoscroll"),
                ("\u{E87A}", "Assistant", "assistant"),
                ("\u{E050}", "Read", "readaloud"),
                ("\u{E3B9}", "Compare", "compare"),
                ("\u{EAC7}", "Word Count", "wordcount"),
                ("\u{E429}", "View Setting", "viewsetting"),
            ],
            Tab::Form => &[
                ("\u{F04C}", "Run Form Field Recognition", "formrecognise"),
                ("\u{F10A}", "Designer Assistant", "formdesigner"),
                ("\u{F1C1}", "Push Button", "fieldbutton"),
                ("\u{E9DE}", "Check Box", "fieldcheckbox"),
                ("\u{E837}", "Radio Button", "fieldradio"),
                ("\u{E262}", "Text Field", "fieldtext"),
                ("\u{E896}", "List Box", "fieldlist"),
                ("\u{E5C6}", "Combo Box", "fieldcombo"),
                ("\u{E3F4}", "Image Field", "fieldimage"),
                ("\u{EBCC}", "Date Field", "fielddate"),
                ("\u{F74C}", "Signature Field", "fieldsignature"),
                ("\u{E70B}", "Barcode", "fieldbarcode"),
                ("\u{E66B}", "Page Templates", "pagetemplates"),
                ("\u{F88C}", "Edit Static XFA Form", "editxfa"),
                ("\u{E24A}", "Calculation Order", "calcorder"),
                ("\u{E8FD}", "Add Tooltip", "tooltip"),
                ("\u{F053}", "Reset Form", "resetform"),
                ("\u{E265}", "Form to sheet", "formtosheet"),
                ("\u{F090}", "Import", "formimport"),
                ("\u{F09B}", "Export", "formexport"),
                ("\u{E86F}", "JavaScript", "javascript"),
                ("\u{E8B9}", "Tool Settings", "toolsettings"),
            ],
            Tab::Protect => &[
                // First, because it is the one that works and the one the tab is
                // for. "Mark for Redaction" beside it would be two names for the
                // same intention where only one of them destroys anything.
                ("\u{E243}", "Redact", "redact"),
                // Named beside Redact rather than under Secure Document, because
                // the choice between them is the one a user actually makes and
                // they need to be read together. Lock hides; Redact destroys.
                ("\u{E63F}", "Lock Text", "lock"),
                ("\u{E3C2}", "Lock Area", "lockarea"),
                // Beside Lock Area because it is the same promise at a
                // different scale, and because area locking refuses the scanned
                // and outlined pages this one takes — a reader turned away by
                // the first needs to see the second without hunting for it.
                ("\u{F686}", "Lock Pages", "lock all"),
                ("\u{E898}", "Unlock", "unlock"),
                // Grouped with the three above rather than after the stubs:
                // this is what makes Redact and Lock Area actually succeed on
                // a page whose words are drawn as outlines instead of falling
                // back to slow OCR — the tool a reader needs at the exact
                // moment one of those two does not work the way they expect.
                ("\u{E167}", "Outlined-Text Fonts", "outlinedfont"),
                ("\u{E8F5}", "Smart Redact", "smartredact"),
                // Beside it, because finding is the safe half and acting is
                // the one people came for — and because redaction destroys.
                ("\u{F74F}", "Redact Found", "smartredact redact"),
                ("\u{E23B}", "Whiteout", "whiteout"),
                ("\u{EA17}", "Hidden Data", "hiddendata"),
                // Beside the survey rather than hidden behind it: reporting is
                // the safe half and removing is the one people came for.
                ("\u{E16C}", "Remove Hidden Data", "hiddendata clean"),
                ("\u{E899}", "Secure Document", "secure"),
                // Beside it because it is the same action with the permissions
                // turned down, and because a reader who wants "they can read it
                // but not lift the artwork" should not have to find the words.
                ("\u{E593}", "Secure Read-only", "secure readonly"),
                ("\u{F03F}", "Remove Password", "unsecure"),
                ("\u{F0C6}", "Sensitivity", "sensitivity"),
                // The one most people reach for, one press away — and the verb
                // takes the others.
                ("\u{E948}", "Mark Confidential", "sensitivity confidential"),
                ("\u{E746}", "Fill & Sign", "fillsign"),
                ("\u{E7AF}", "Sign & Certify", "certify"),
                ("\u{EFD6}", "Time Stamp Document", "timestamp"),
                ("\u{F013}", "Validate", "validate"),
            ],
            Tab::PagiSign => &[
                ("\u{F603}", "Signature", "signature"),
                ("\u{F775}", "Manage Signatures", "managesignatures"),
                ("\u{E877}", "Apply All Signatures", "applysignatures"),
                ("\u{EAE2}", "Add Text", "addtext"),
                ("\u{E8F3}", "Comb Field", "combfield"),
                ("\u{EF6C}", "Predefined Text", "predefinedtext"),
                ("\u{EB54}", "Rectangle", "signrectangle"),
                ("\u{E668}", "Check", "signcheck"),
                ("\u{EF4A}", "Dot", "signdot"),
                ("\u{E5CD}", "Cross", "signcross"),
                ("\u{F108}", "Line", "signline"),
                ("\u{F0D2}", "Request Signature", "requestsignature"),
                ("\u{F187}", "Send in Bulk", "sendbulk"),
                ("\u{F728}", "Create Online Form", "onlineform"),
                ("\u{EF3E}", "Document Status", "documentstatus"),
                ("\u{E06B}", "Add E-Sign Branding", "signbranding"),
            ],
            Tab::Share => &[
                ("\u{E159}", "Email", "email"),
                ("\u{EA5E}", "Attach to Email", "emailattach"),
                ("\u{E80D}", "Share Link", "sharelink"),
                ("\u{E560}", "Send for Review", "sendreview"),
                ("\u{E8E1}", "Track Reviews", "trackreviews"),
                ("\u{F15C}", "Cloud Storage", "cloudstorage"),
            ],
            Tab::Accessibility => &[
                ("\u{E6B1}", "Full Check", "accesscheck"),
                ("\u{F071}", "Accessibility Report", "accessreport"),
                ("\u{E893}", "Autotag Document", "autotag"),
                ("\u{E242}", "Reading Order", "readingorder"),
                ("\u{E43F}", "Set Alternate Text", "alttext"),
                ("\u{F05B}", "Tags Panel", "tagspanel"),
                ("\u{E92C}", "Reading Options", "readingoptions"),
            ],
            Tab::Help => &[
                ("\u{EA19}", "User Manual", "help"),
                ("\u{EB9B}", "Quick Start", "quickstart"),
                ("\u{EAE7}", "Keyboard Shortcuts", "shortcuts"),
                ("\u{E923}", "Check for Updates", "checkupdates"),
                ("\u{E868}", "Report an Issue", "reportissue"),
                ("\u{E88E}", "About Pagify", "about"),
            ],
            Tab::Draw => &[
                ("\u{F108}", "Line", "line"),
                ("\u{EF4A}", "Circle", "circle"),
                ("\u{E922}", "Polyline", "pline"),
                ("\u{E14E}", "Trim", "trim"),
                ("\u{E920}", "Fillet", "fillet"),
                ("\u{E2EC}", "Offset", "offset"),
                ("\u{E6D0}", "Erase", "erase"),
                ("\u{E41C}", "Calibrate", "calibrate "),
                ("\u{EB95}", "Distance", "measure distance"),
                ("\u{EA49}", "Area", "measure area"),
                ("\u{EAF6}", "Page Scale", "pagescale"),
            ],
            Tab::Automate => &[
                ("\u{E837}", "Record", "record"),
                ("\u{EF71}", "Stop", "stop"),
                ("\u{E037}", "Replay", "replay "),
            ],
        }
    }
}


/// The scale a page is actually rasterised at, for a given zoom.
///
/// Quantised to third-of-an-octave steps, and this is the whole reason zooming
/// stopped being smooth. Every distinct scale is a full PDFium re-render of the
/// page; a continuous pinch produces a different scale on every frame, so the
/// engine was asked to redraw the page sixty times a second at sixty slightly
/// different sizes.
///
/// Rounding to a step means a pinch crosses only a handful of them, and egui
/// scales the existing texture in between — at most about 12% off true size,
/// which is not visible on a page of text, against a redraw that very much is.
fn raster_scale(device_scale: f32) -> f32 {
    const STEPS_PER_OCTAVE: f32 = 3.0;
    let clamped = device_scale.clamp(0.05, 32.0);
    2f32.powf((clamped.log2() * STEPS_PER_OCTAVE).round() / STEPS_PER_OCTAVE)
}

/// Where a mark sits, for a listing.
///
/// Said in page points because that is the only handle a user has on a mark
/// this program cannot otherwise address: "the second highlight" means nothing
/// until you know which one is near the top.
fn describe_where(annotation: &pdf_core::document::Annotation) -> String {
    use pdf_core::document::Annotation as A;
    let first = match annotation {
        A::Highlight { rects, .. }
        | A::Underline { rects, .. }
        | A::StrikeOut { rects, .. }
        | A::Squiggly { rects, .. } => rects.first().copied(),
        A::Note { rect, .. } => Some(*rect),
        A::Ink { .. } | A::Text { .. } => None,
    };
    match first {
        Some(r) => format!(" at {:.0},{:.0}", r.left, r.top),
        None => String::new(),
    }
}

/// A ribbon button: the glyph above, the name under it.
///
/// Painted rather than assembled out of a `Button` and a `LayoutJob`, because
/// the two lines have to be centred **independently**. Laid out as one galley
/// they share a bounding box, so a name that wraps to two lines — "Link & Join
/// Text" — widens the box and drags the glyph off to one side of it. The glyph
/// is the thing the eye lands on, and it has to sit in the middle of the button
/// whatever the name under it does.
///
/// `LayoutJob::halign` is not the fix, and was the original bug: it centres text
/// around x = 0, and the button then draws the galley from its top-left corner,
/// so everything lands half a width to the right.
/// The icon font, as its own family.
///
/// Material Symbols, Apache 2.0, **subsetted to the 174 glyphs this program
/// actually draws** — 10.6 MB of variable font instanced to one weight and cut
/// down to 28 kB. Embedded rather than loaded from disk: an icon set that can
/// go missing is a toolbar that can come up blank.
///
/// It replaces a set assembled from the 236 characters egui's bundled fonts can
/// draw, which had no magnifier, no tick, no plus and almost no arrows — so
/// Search was a detective and Highlight was a shading block.
const ICON_FAMILY: &str = "pagify-icons";

/// Candidate faces for matching type converted to outlines — see
/// `third_party/fonts/README.md` for provenance, licence, and the measured
/// case for bundling this specific face rather than a generic system one.
///
/// **Named, not guessed at.** HSI's own catalogue was set in Montserrat, so
/// that is what ships — a generic system font would not resolve their brand
/// typeface at all, which is the actual document this feature exists for.
/// Both weights are handed to every call together: matching merges every
/// candidate's glyphs into one catalogue and the shape distance decides which
/// one a given letter matches, so there is no "try Regular, then Bold" retry
/// to get wrong.
const BUNDLED_OUTLINED_FONTS: &[&[u8]] = &[
    include_bytes!("../../../third_party/fonts/Montserrat-Regular.ttf"),
    include_bytes!("../../../third_party/fonts/Montserrat-Bold.ttf"),
];

/// The family a document's own face is bound to while a run is being edited.
const RUN_FAMILY: &str = "pagify-run-face";

/// Install the program's fonts, plus the document's own face when there is one.
///
/// **One function rather than two calls**, because `set_fonts` replaces the lot:
/// installing a document's face separately would take the ribbon's icons out
/// with it, and the ribbon would then draw in a family nothing is bound to.
fn install_fonts(ctx: &egui::Context, run_face: Option<Vec<u8>>) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        ICON_FAMILY.to_owned(),
        std::sync::Arc::new(egui::FontData::from_static(include_bytes!(
            "../../../third_party/icons/pagify-icons.ttf"
        ))),
    );

    // Appended to the end of the existing families rather than bound to one of
    // its own.
    //
    // A named family does not exist until egui rebuilds its fonts, which
    // happens at the *start of the next frame* — so the frame that installs it
    // then draws the ribbon in a family nothing is bound to, and epaint panics
    // rather than falling back. Appending has no such ordering: the icons live
    // at private-use codepoints, which no text ever asks for, so being last in
    // the fallback chain costs nothing and cannot fail.
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts.families.entry(family).or_default().push(ICON_FAMILY.to_owned());
    }

    // The document's face, in a family of its own — bound *and* appended to the
    // proportional fallback, so a character the run's own subset lacks still
    // draws rather than vanishing as somebody types it.
    if let Some(bytes) = run_face {
        fonts
            .font_data
            .insert(RUN_FAMILY.to_owned(), std::sync::Arc::new(egui::FontData::from_owned(bytes)));
        fonts.families.insert(
            egui::FontFamily::Name(RUN_FAMILY.into()),
            vec![RUN_FAMILY.to_owned(), "Ubuntu-Light".to_owned()],
        );
    }
    ctx.set_fonts(fonts);
}

fn install_icons(ctx: &egui::Context) {
    install_fonts(ctx, None);
}

fn icon_font(size: f32) -> egui::FontId {
    egui::FontId::proportional(size)
}

fn tool_button(
    ui: &mut egui::Ui,
    glyph: &str,
    label: &str,
    command: &str,
    active: bool,
) -> egui::Response {
    let size = egui::vec2(TOOL_WIDTH, TOOL_HEIGHT);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
    });

    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);
        // Only drawn once there is something to react to. A ribbon of two
        // hundred outlined boxes is a wall; the frame belongs to the button the
        // pointer is on — or to the tool that is currently in force, which has
        // to be visible without hovering to find it.
        if active {
            ui.painter().rect(
                rect,
                visuals.corner_radius,
                theme::VIOLET_DEEP,
                egui::Stroke::new(1.0, theme::VIOLET_BRIGHT),
                egui::StrokeKind::Inside,
            );
        } else if response.hovered() || response.is_pointer_button_down_on() {
            ui.painter().rect(
                rect,
                visuals.corner_radius,
                visuals.weak_bg_fill,
                visuals.bg_stroke,
                egui::StrokeKind::Inside,
            );
        }

        let painter = ui.painter();
        let glyph_galley =
            painter.layout_no_wrap(glyph.to_owned(), icon_font(19.0), theme::INK);
        // Wrapped inside the button rather than allowed to set its own width,
        // so every button in the row stays the same size.
        let label_galley = painter.layout(
            label.to_owned(),
            egui::FontId::proportional(10.0),
            if active || response.hovered() { theme::INK } else { theme::INK_DIM },
            size.x - 8.0,
        );

        let gap = 3.0;
        let total = glyph_galley.size().y + gap + label_galley.size().y;
        let top = rect.center().y - total / 2.0;

        for (galley, y) in [
            (&glyph_galley, top),
            (&label_galley, top + glyph_galley.size().y + gap),
        ] {
            painter.galley(
                egui::pos2(rect.center().x - galley.size().x / 2.0, y),
                galley.clone(),
                theme::INK,
            );
        }
    }

    // What to type to get the same thing. §7 makes the command box the only way
    // anything happens, so a button that never says its own name leaves the box
    // undiscoverable to anyone who only ever clicks.
    response.on_hover_text(format!("{label}  —  `{}`", command.trim()))
}

/// Sized so a two-line name still leaves the glyph centred, and so a row of
/// them reads as a grid rather than as a ragged line.
const TOOL_WIDTH: f32 = 82.0;
const TOOL_HEIGHT: f32 = 58.0;

impl PagifyApp {
    fn new(path: Option<&str>) -> Self {
        let mut app = PagifyApp {
            doc: None,
            cmd: CommandBox::default(),
            markup: Markup::default(),
            calibration: Calibration::default(),
            recorder: Recorder::default(),
            pending: None,
            page: 0,
            zoom: ZoomMode::Fit,
            rotation: Rotation::None,
            scroll_pt: 0.0,
            last_view: None,
            scroll_offset: egui::Vec2::ZERO,
            anchor_offset: None,
            editing_run: None,
            next_text_id: 0x0100_0000,
            markup_armed: None,
            foreign: None,
            selection_page: 0,
            settling: 0,
            hover_view: None,
            viewport_rect: None,
            copy_wanted: false,
            pan_by: None,
            scroll_to_pt: None,
            canvas_pt: egui::vec2(800.0, 600.0),
            recent: Recent::load(),
            outlined_fonts: pagify_shell::outlined_fonts::OutlinedFonts::load(),
            selected_image: None,
            text: None,
            text_selection: None,
            text_drag: None,
            find_needle: String::new(),
            find_hits: Vec::new(),
            find_at: 0,
            defaults: tools::Defaults::default(),
            saved_revision: 0,
            mark: None,
            snaps: SnapSet::defaults(),
            ortho: false,
            grid_pt: 0.0,
            show_thumbs: true,
            command_open: false,
            ribbon: Tab::Home,
            closing: None,
            asking_to_redact: None,
            reading: None,
            recogniser: None,
            awaiting_password: None,
            password_typed: String::new(),
            password_field_focused: false,
            password_problem: None,
            password_plus: false,
            signatures: pagify_shell::signatures::Signatures::load(),
            signatures_path: pagify_shell::signatures::Signatures::path(),
            pad: None,
            signature_list: None,
            predefined: pagify_shell::predefined::Predefined::load(),
            predefined_path: pagify_shell::predefined::Predefined::path(),
            snippets: None,
            drawn_words: None,
            editor_face: None,
            editor_face_ready: false,
            pending_face: None,
            pointer: Default::default(),
            drag_from: None,
            last_snap: None,
        };
        app.say_info("Pagify — type `help`, or `open <path.pdf>`.");
        if let Some(path) = path {
            app.open(path);
        }
        // Nothing to look at means starting backstage, where the wizard and the
        // recent documents are. `open` moves us to Home.
        if app.doc.is_none() {
            app.ribbon = Tab::File;
        }
        app
    }

    fn say_info(&mut self, text: impl Into<String>) {
        self.cmd.say(Kind::Info, text);
    }
    fn say_error(&mut self, text: impl Into<String>) {
        // Open the box if it is shut. A command that failed silently because
        // its explanation was collapsed out of view is worse than one that
        // never ran.
        self.command_open = true;
        self.cmd.say(Kind::Error, text);
    }

    // -- document -----------------------------------------------------------

    /// Move the view by a drag, in screen pixels.
    ///
    /// Dragging the paper moves the paper, so the offset goes the other way.
    fn pan(&mut self, by: egui::Vec2) {
        let total = self.pan_by.unwrap_or(egui::Vec2::ZERO) - by;
        self.pan_by = Some(total);
    }

    /// Ask what to do about unsaved marks, and do it.
    ///
    /// Three ways out, all of them reachable with the mouse. **Discarding is
    /// one of them.** A guard that only offers "save" or "go back" does not
    /// protect anybody: work gets abandoned on purpose all the time — a mark
    /// put down to measure something, a line drawn to check a distance — and a
    /// program that will not let go of it is a program you have to kill.
    fn ask_about_unsaved(&mut self, ctx: &egui::Context) {
        let Some(intent) = self.closing.clone() else { return };
        let password_waiting = self.unsaved_password();
        let edited = self.unsaved_edits();
        let (marks, pages) = self.unsaved().unwrap_or((0, 0));
        if marks == 0 && !password_waiting && !edited {
            // Saved out from under the dialog — carry on with what was asked.
            self.closing = None;
            self.finish_closing(intent, ctx);
            return;
        }
        let mut decision: Option<Decision> = None;
        egui::Modal::new(egui::Id::new("unsaved")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.heading(match intent {
                Closing::Document => "Close this document?",
                Closing::Program => "Quit Pagify?",
                Closing::Open(_) | Closing::OpenDialog => "Open another document?",
            });
            ui.add_space(6.0);
            if marks > 0 {
                ui.label(format!(
                    "{marks} mark{} on {pages} page{} {} not been saved into the file.",
                    if marks == 1 { "" } else { "s" },
                    if pages == 1 { "" } else { "s" },
                    if marks == 1 { "has" } else { "have" },
                ));
            }
            if edited {
                ui.label(
                    "This document has been changed and not saved. Closing without \
                     saving leaves the file as it was.",
                );
            }
            if password_waiting {
                ui.label(
                    "A password has been set and not yet written. Closing without \
                     saving throws it away, and the file stays as it was.",
                );
            }
            ui.add_space(12.0);

            ui.horizontal(|ui| {
                if ui.button("Save and close").clicked() {
                    decision = Some(Decision::Save);
                }
                // Named for what it does. "Don't save" describes the thing not
                // happening; "Discard" describes the thing that does.
                if ui.button("Discard and close").clicked() {
                    decision = Some(Decision::Discard);
                }
                if ui.button("Cancel").clicked() {
                    decision = Some(Decision::Cancel);
                }
            });
        });

        // Escape is Cancel, which is the safe one — the same key everywhere
        // else in this program means "stop what you are doing".
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            decision = Some(Decision::Cancel);
        }

        match decision {
            None => {}
            Some(Decision::Cancel) => {
                self.closing = None;
                self.say_info("still open.");
            }
            Some(Decision::Save) => {
                self.closing = None;
                self.save(None);
                // **Not `would_lose_work`.** A chosen password stays on the
                // document after it is written, deliberately — every later save
                // has to re-apply it, so the engine keeps it. It is therefore no
                // evidence about whether this save took. Marks and edits are,
                // and both are cleared by one that did.
                if self.unsaved().is_some() || self.unsaved_edits() {
                    // The save did not take. Closing now would throw away the
                    // very work the dialog was protecting.
                    self.say_error("could not save — nothing was closed.");
                    return;
                }
                self.finish_closing(intent, ctx);
            }
            Some(Decision::Discard) => {
                self.closing = None;
                self.finish_closing(intent, ctx);
            }
        }
    }

    fn finish_closing(&mut self, intent: Closing, ctx: &egui::Context) {
        match intent {
            Closing::Open(path) => self.open(&path.to_string_lossy()),
            Closing::OpenDialog => self.open_dialog(),
            Closing::Document => {
                if self.doc.take().is_some() {
                    self.markup.clear();
                    self.text = None;
                    self.cmd.prompt_mut().document = None;
                    self.ribbon = Tab::File;
                    self.say_info("closed.");
                }
            }
            Closing::Program => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                std::process::exit(0);
            }
        }
    }

    /// Where the verified recognition models are.
    ///
    /// Verification is not optional and not a separate step a caller could
    /// forget: `models::find` returns a directory only when the files in it are
    /// the exact ones this build was tested against. A model is data fed
    /// straight into a numeric runtime — a corrupted one is a crash and a
    /// substituted one is arbitrary behaviour inside the process reading the
    /// user's documents.
    fn model_directory() -> Option<std::path::PathBuf> {
        pagify_shell::models::find().ok()
    }

    /// Find and load the OCR models fresh, with no cache — see `extract_text`,
    /// which keeps `self.recogniser` warm across calls and only reaches this
    /// once a page in the batch actually turns out to need OCR. A page the
    /// bundled outline fonts already read needs neither the search nor the
    /// load, so neither belongs ahead of that check.
    fn load_recogniser() -> Result<std::sync::Arc<pdf_core::ocr::engine::OcrsRecogniser>, String> {
        // The refusals come back with the search, so the message can say what
        // was wrong rather than only that nothing worked. "No models found"
        // when a file is sitting right there one byte short is the kind of
        // message that costs an afternoon.
        let dir = pagify_shell::models::find().map_err(|refused| {
            let mut why = String::from("no usable recognition models.");
            for (dir, problem) in refused.iter().take(3) {
                why.push_str(&format!("\n  {}: {problem}", dir.display()));
            }
            why
        })?;

        let loaded = pdf_core::ocr::engine::OcrsRecogniser::from_files(
            &dir.join("text-detection.rten"),
            &dir.join("text-recognition.rten"),
        )
        .map_err(|e| format!("could not load the recognition models: {e}"))?;

        Ok(std::sync::Arc::new(loaded))
    }

    /// Read this page with OCR and write the words back as an invisible text
    /// layer, so what is on the page can be selected.
    ///
    /// For the two kinds of page that look like text and are not: a scan, and a
    /// page whose words were drawn as glyph outlines. Both render perfectly and
    /// neither has one character in it to select. This is the only thing that
    /// helps either of them, and it is asked for rather than done automatically
    /// — it takes about a second a page and it changes the document.
    fn extract_text(&mut self, spec: &str) {
        let Some(doc) = &self.doc else {
            self.say_error("nothing open.");
            return;
        };
        if let Some(busy) = &self.reading {
            self.say_error(format!("already reading page {}.", busy.at + 1));
            return;
        }

        // No range means the page in front of you. That is what a button press
        // means, and a button that silently read 149 pages would be a trap.
        let queue: Vec<usize> = if spec.trim().is_empty() {
            vec![self.page]
        } else {
            match pagify_shell::organize::parse_range(spec, doc.page_count) {
                Ok(pages) => pages,
                Err(why) => {
                    self.say_error(why);
                    return;
                }
            }
        };

        // **A page whose words are already text does not need reading.**
        //
        // Reported from use: extracting on such a page ran OCR for over a
        // minute and then laid a *second*, invisible copy of the words over the
        // real ones. Editing afterwards picked the copy — so the reported text
        // changed, the page did not, and deleting a word left it plainly
        // visible underneath. "The text embedded below still shows."
        //
        // `Native` and `Hybrid` are exactly the two the classifier already
        // calls readable; `Hybrid`'s own documentation says the image on it "is
        // not a reason to re-recognise it". Taken at its word.
        let mut already: Vec<usize> = Vec::new();
        let queue: Vec<usize> = queue
            .into_iter()
            .filter(|page| {
                let readable = doc
                    .session
                    .classify(*page)
                    .map(|c| {
                        matches!(
                            c.kind,
                            pdf_core::document::PageTextKind::Native
                                | pdf_core::document::PageTextKind::Hybrid
                        )
                    })
                    .unwrap_or(false);
                if readable {
                    already.push(*page + 1);
                }
                !readable
            })
            .collect();

        if queue.is_empty() {
            let pages = already.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", ");
            self.say_info(match already.len() {
                0 => "nothing to read.".to_string(),
                1 => format!("page {pages} already has text — select and edit it directly."),
                _ => format!("pages {pages} already have text — select and edit it directly."),
            });
            return;
        }
        if !already.is_empty() {
            let pages = already.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", ");
            self.say_info(format!("skipping page{} {pages} — the text is already there.",
                if already.len() == 1 { "" } else { "s" }));
        }

        // A read, not a resolve: finding and loading the models is deferred
        // into the worker below, and only reached at all once some page in
        // the batch turns out not to be covered by the bundled outline fonts.
        // A build from an earlier call is still reused here rather than
        // repeated.
        let cached_recogniser = self.recogniser.clone();
        // Read now, on this thread, while `self` is still reachable — the
        // worker below is a plain `move` closure with no way back to it.
        let outlined_fonts = self.outlined_font_bytes();

        let options = pdf_core::ocr::pipeline::Options::default();
        let session = self.doc.as_ref().expect("checked above").session.clone();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (tx, done) = std::sync::mpsc::channel();

        let work = queue.clone();
        let worker_stop = stop.clone();
        // One thread for the whole run, one page at a time. Pages in parallel
        // would multiply the peak memory, and recognition already measured at
        // 482 MB for a single 300 dpi page.
        std::thread::spawn(move || {
            let font_refs: Vec<&[u8]> = outlined_fonts.iter().map(|f| f.as_slice()).collect();
            let mut recogniser = cached_recogniser;
            for page in work {
                if worker_stop.load(std::sync::atomic::Ordering::Relaxed) {
                    return;
                }

                // Geometric matching first: no rasterising, no neural net, and
                // — where it applies at all — better signal than a pixel-based
                // recogniser can have, per the same reasoning `recognise_outlined`
                // was built on. `Ok(None)` covers both "not outlined type" and
                // "the bundled faces do not genuinely resolve this page", which
                // is exactly OCR's job either way, so this only ever *adds* a
                // faster path — it never takes the slower one away.
                let outlined = session
                    .recognise_outlined_words_if_trustworthy(page, &font_refs)
                    .unwrap_or(None);

                let mut built_recogniser = None;
                let reading = if let Some(words) = outlined {
                    Ok(pdf_core::ocr::pipeline::PageReading {
                        words,
                        // The three fields below describe *rasterising and
                        // recognising an image*, which this path never does.
                        // Harmless placeholders rather than measurements
                        // dressed up as real ones: `collect_reading` reports
                        // `lines`/`already_there` only on the *empty*-result
                        // branch, which a trustworthy, non-empty match can
                        // never reach.
                        skew: 0.0,
                        lines: 0,
                        already_there: 0,
                        image_size: (0, 0),
                    })
                } else {
                    if recogniser.is_none() {
                        match PagifyApp::load_recogniser() {
                            Ok(r) => {
                                built_recogniser = Some(r.clone());
                                recogniser = Some(r);
                            }
                            Err(why) => {
                                let sent = tx.send(PageResult {
                                    page,
                                    reading: Err(why),
                                    built_recogniser: None,
                                });
                                if sent.is_err() {
                                    return;
                                }
                                continue;
                            }
                        }
                    }
                    let recogniser = recogniser.as_deref().expect("just resolved above");
                    session
                        .recognise_page_until(page, recogniser, &options, &|| {
                            worker_stop.load(std::sync::atomic::Ordering::Relaxed)
                        })
                        .map_err(|e| format!("{e}"))
                };

                // The receiver is gone if the document was closed. Nothing to
                // do about that, and nothing that needs doing.
                if tx.send(PageResult { page, reading, built_recogniser }).is_err() {
                    return;
                }
            }
        });

        let first = queue[0];
        let total = queue.len();
        self.reading =
            Some(Reading { queue, at: first, done, stop, words: 0, pages: 0 });
        self.say_info(if total == 1 {
            format!("reading page {}…", first + 1)
        } else {
            format!("reading {total} pages…")
        });
    }

    /// Block until the page being read comes back.
    ///
    /// Tests only. The application never waits — that is the entire point of
    /// moving the work off this thread — but a test that asserts on the result
    /// has to have one.
    #[cfg(test)]
    fn wait_for_reading(&mut self) {
        let ctx = egui::Context::default();
        for _ in 0..1200 {
            if self.reading.is_none() {
                return;
            }
            self.collect_reading(&ctx);
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        panic!("recognition did not finish within a minute");
    }

    /// Collect a finished page, if one has finished.
    ///
    /// The layer is written **here**, on the UI thread, not in the worker. It
    /// is an edit: it goes through `execute` so it lands in the undo stack, and
    /// it invalidates caches the UI owns.
    fn collect_reading(&mut self, ctx: &egui::Context) {
        let Some(reading) = &self.reading else { return };

        let result = match reading.done.try_recv() {
            Ok(result) => result,
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                // Keep the frames coming while it works, or the answer arrives
                // and nothing notices until the user moves the mouse.
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
                return;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                // The worker is finished — either it read everything, or it was
                // stopped and put the page down.
                let done = self.reading.take().expect("checked above");
                self.report_reading(&done);
                return;
            }
        };

        if let Some(recogniser) = &result.built_recogniser {
            self.recogniser = Some(recogniser.clone());
        }

        let page = result.page;
        match result.reading {
            Err(why) => {
                // One bad page must not end a run over a hundred of them.
                self.say_error(format!("page {}: {why}", page + 1));
            }
            Ok(reading) if reading.words.is_empty() => {
                if reading.already_there == 0 {
                    self.say_info(format!(
                        "page {}: nothing to read ({} lines detected).",
                        page + 1,
                        reading.lines
                    ));
                }
            }
            Ok(reading) => {
                let words = reading.words.len();
                let Some(doc) = &self.doc else { return };
                // Written here, on the UI thread: it is an edit, it goes
                // through `execute` for undo, and it invalidates caches the UI
                // owns.
                match doc.session.execute(pdf_core::command::Command::AddTextLayer {
                    page_index: page,
                    words: reading.words,
                }) {
                    Ok(_) => {
                        if let Some(r) = &mut self.reading {
                            r.words += words;
                            r.pages += 1;
                        }
                        if let Some(doc) = &mut self.doc {
                            doc.rendered_is_stale();
                        }
                        self.text = None;
                        self.text_selection = None;
                        self.find_hits.clear();
                    }
                    Err(e) => self.say_error(format!("page {}: {e}", page + 1)),
                }
            }
        }

        // Move the progress line on, and let the next frame collect the next
        // page.
        if let Some(r) = &mut self.reading {
            let next = r.queue.iter().position(|p| *p == page).map(|i| i + 1);
            if let Some(page) = next.and_then(|i| r.queue.get(i).copied()) {
                r.at = page;
            }
        }
        ctx.request_repaint();
    }

    /// What a finished run has to say for itself.
    fn report_reading(&mut self, done: &Reading) {
        if done.stop.load(std::sync::atomic::Ordering::Relaxed) {
            self.say_info(format!(
                "stopped after {} page{}.",
                done.pages,
                if done.pages == 1 { "" } else { "s" }
            ));
            return;
        }
        if done.pages == 0 {
            self.say_info("nothing became selectable.");
            return;
        }
        self.say_info(format!(
            "read {} word{} across {} page{}. They are selectable now — `undo` takes a \
             layer back off.",
            done.words,
            if done.words == 1 { "" } else { "s" },
            done.pages,
            if done.pages == 1 { "" } else { "s" }
        ));
    }

    /// A file asked for a password; the next line typed is it.
    ///
    /// Intercepted **before** the command box sees it, which is the whole
    /// point: `CommandBox::submit` echoes every line into the visible history
    /// and the recorder keeps it for replay. A password must reach PDFium and
    /// nothing else — not the history, not the recording, not the recent list.
    ///
    /// Returns whether the line was taken.
    fn consume_password_line(&mut self) -> bool {
        // Opening asks in a window; everything else asks here. Without this the
        // command box would take a line meant for the window, and the person
        // would be typing their password into two places at once.
        // Opening and locking both ask in a window; only the `secure` prompts
        // still use the command box. Without this the box would take a line
        // meant for a window, and somebody would be typing a passcode into two
        // places at once.
        // Every password is asked for in a window now, so the command box
        // never takes one — otherwise a password typed while a window is up
        // goes somewhere nobody expected.
        if self.awaiting_password.is_some() {
            return false;
        }
        let typed = self.cmd.input_mut().trim().to_string();
        if typed.is_empty() {
            return false;
        }
        self.cmd.input_mut().clear();

        match self.awaiting_password.take().expect("checked above") {
            Awaiting::Open(path) => self.open_with(&path, Some(&typed)),
            Awaiting::Lock { page, area, require_complete } => {
                match self.lock_area(page, area, typed.as_bytes(), require_complete) {
                    Ok(said) => self.say_info(said),
                    Err(e) => self.say_error(e),
                }
            }
            Awaiting::LockPages(pages) => match self.lock_pages(&pages, typed.as_bytes()) {
                Ok(said) => self.say_info(said),
                Err(e) => self.say_error(e),
            },
            Awaiting::LockAgain { first, then } => {
                if typed != first {
                    self.say_error("those did not match — nothing was locked. Try again.");
                } else {
                    self.awaiting_password = Some(*then);
                    // The passcode is already known to be right; hand it on to
                    // whichever lock was waiting for it.
                    self.answer_lock_passcode(&typed);
                }
            }
            Awaiting::SecureCurrent(options) => {
                // Handled in `answer_passcode`; the command box no longer takes
                // passwords, so this only exists to keep the match whole.
                self.awaiting_password = Some(Awaiting::SecureCurrent(options));
            }
            Awaiting::Certificate(path) => {
                self.awaiting_password = Some(Awaiting::Certificate(path));
            }
            Awaiting::Secure(options) => {
                self.awaiting_password =
                    Some(Awaiting::SecureAgain { first: typed.clone(), options });
                self.say_info("type the same password again, so a slip cannot lock you out.");
            }
            Awaiting::SecureAgain { first, options } => {
                if typed != first {
                    self.say_error(
                        "those did not match — nothing was set. Run `secure` again.",
                    );
                } else {
                    match self.secure_document(typed.as_bytes(), options) {
                        Ok(said) => self.say_info(said),
                        Err(e) => self.say_error(e),
                    }
                }
            }
            Awaiting::LockImage { page, object } => {
                match self.lock_image(page, object, typed.as_bytes()) {
                    Ok(said) => self.say_info(said),
                    Err(e) => self.say_error(e),
                }
            }
            Awaiting::UnlockItem(id) => match self.unlock_item(&id, typed.as_bytes()) {
                Ok(said) => self.say_info(said),
                Err(e) => self.say_error(e),
            },
            Awaiting::Unlock => match self.unlock(typed.as_bytes()) {
                Ok(said) => self.say_info(said),
                Err(e) => self.say_error(e),
            },
        }
        true
    }

    fn open(&mut self, path: &str) {
        self.open_with(path, None)
    }

    /// Open, asking for a password if the file turns out to need one.
    ///
    /// A refusal is not the right answer here. A great many working documents
    /// are encrypted — a catalogue extract saved out of another editor, a
    /// drawing issued under restriction — and "document is password protected"
    /// with no way to supply one is a dead end in a program whose whole
    /// interface is a place to type things.
    fn open_with(&mut self, path: &str, password: Option<&str>) {
        let typing = self.outlined_font_bytes();
        let opened = Session::open_with_password(path, password).and_then(|session| {
            let count = session.page_count()?;
            let sizes = session.page_sizes()?;
            // The same fonts the outline matcher uses — the bundled ones plus
            // whatever the reader has added. A font good enough to recognise a
            // page's letters by is a font good enough to write them with, and
            // asking somebody to add the same file twice would be a poor joke.
            session.set_typing_fonts(typing)?;
            Ok((session, count, sizes))
        });

        match opened {
            Err(PdfError::PasswordRequired) | Err(PdfError::IncorrectPassword) => {
                let again = password.is_some();
                self.awaiting_password = Some(Awaiting::Open(path.to_string()));
                self.say_info(if again {
                    "that password was not accepted — type it again, or Escape to give up."
                } else {
                    "this file is encrypted. Type its password, or Escape to give up."
                });
            }
            Ok((session, page_count, sizes)) => {
                let name = session
                    .path()
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.to_string());

                self.markup.clear();
                self.calibration = Calibration::default();

                // Anything this document already carries comes back as live
                // geometry, not as ink — phase 8's whole point.
                let mut restored = 0;
                for page in 0..page_count {
                    if let Ok(Some(layer)) = session.restore_markup(page) {
                        restored += layer.len();
                        let height = layer.space().height_pt();
                        *self.markup.page(page, height) = layer;
                    }
                }

                // Land on the document rather than leaving the backstage view
                // covering the thing that was just opened.
                if self.ribbon == Tab::File {
                    self.ribbon = Tab::Home;
                }
                self.recent.record(session.path(), page_count, pagify_shell::recent::now());
                self.recent.save();
                self.cmd.prompt_mut().document = Some(name.clone());
                self.say_info(format!(
                    "{name} — {page_count} page{}.{}",
                    if page_count == 1 { "" } else { "s" },
                    if restored > 0 { format!(" {restored} marks restored.") } else { String::new() }
                ));

                self.doc = Some(Doc {
                    session: std::sync::Arc::new(session),
                    strip: Strip::new(&sizes, PAGE_GAP_PT),
                    page_count,
                    textures: HashMap::new(),
                    thumbs: HashMap::new(),
                });
                self.page = 0;
                self.saved_revision = self.markup.revision();
                // Said once, on opening, and only when there is something wrong
                // — otherwise selection silently doing nothing is left for the
                // reader to work out.
                self.report_text_layer(false);
                self.scroll_pt = 0.0;
                self.pending = None;
            }
            Err(e) => {
                self.say_error(format!("{e}"));
                let where_ = pagify_shell::pdfium::describe();
                self.say_info(format!("PDFium was looked for at: {where_}"));
            }
        }
    }

    /// Ask for a file.
    ///
    /// Blocks the frame while the dialog is up, which is what a modal open
    /// dialog is. Doing it asynchronously would mean holding a half-open
    /// document across frames for no gain — nobody expects to keep working in
    /// the window behind an open dialog.
    /// Marks that closing right now would discard, and the pages they are on.
    fn unsaved(&self) -> Option<(usize, usize)> {
        if self.markup.revision() == self.saved_revision {
            return None;
        }
        let (marks, pages) = self.markup.unsaved();
        (marks > 0).then_some((marks, pages))
    }

    /// Sign the document with a certificate.
    fn sign_with(&mut self, path: &std::path::Path, password: &str) -> Result<String, String> {
        let pkcs12 = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let Some(doc) = &self.doc else { return Err("nothing open.".into()) };

        let who = doc
            .session
            .sign_document(&pkcs12, password, &pdf_core::pdf::sign::Reason::default())
            .map_err(|e| e.to_string())?;

        if let Some(doc) = &mut self.doc {
            doc.rendered_is_stale();
        }
        self.text = None;
        // Says the thing somebody will otherwise learn the hard way: the
        // signature is over the file as it now stands, and the next edit
        // breaks it.
        Ok(format!(
            "signed as {who}. The signature covers the file as it is now — \
             anything changed after this breaks it."
        ))
    }

    /// Everything this document's protections amount to, in a few lines.
    ///
    /// **What it deliberately does not do is go looking.** A hidden-data survey
    /// reads the whole file, and a status readout somebody presses out of
    /// curiosity on an 80 MB catalogue must not stop to do that. It says which
    /// tool looks, instead of pretending it already has.
    fn document_status(&self) -> Vec<String> {
        let Some(doc) = &self.doc else { return vec!["nothing open.".into()] };
        let session = &doc.session;
        let mut lines = Vec::new();

        let name = doc
            .session
            .path()
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "this document".into());
        lines.push(format!(
            "{name} — {} page{}{}",
            doc.page_count,
            if doc.page_count == 1 { "" } else { "s" },
            if session.is_dirty().unwrap_or(false) { ", with unsaved changes" } else { "" }
        ));

        // -- the password, and what it is worth -------------------------------
        //
        // Said in terms of who can open the file, which is the question, rather
        // than in terms of the cipher, which is the answer to a different one.
        let pending = session.is_secured();
        let on_file = session.had_password_on_open();
        lines.push(match (on_file || pending, session.is_secure_plus()) {
            (false, _) => "password: none — anyone who has the file can open it".into(),
            (true, true) => format!(
                "password: Secure Plus{} — only Pagify opens this, and only with the password",
                if pending && !on_file { ", not written until you save" } else { "" }
            ),
            (true, false) => format!(
                "password: AES-256{} — any PDF reader opens it with the password",
                if pending && !on_file { ", not written until you save" } else { "" }
            ),
        });
        if let Some(permissions) = session.permissions() {
            lines.push(format!("permissions: {}", permissions.describe()));
        }

        // -- signatures, and whether they still hold --------------------------
        match session.validate_signatures() {
            Ok(found) if found.is_empty() => lines.push("signed: no".into()),
            Ok(found) => {
                for signature in &found {
                    let what = if signature.timestamp { "timestamp" } else { "signed" };
                    let who = if signature.name.is_empty() {
                        String::new()
                    } else {
                        format!(" by {}", signature.name)
                    };
                    lines.push(format!("{what}{who}: {}", signature.verdict.describe()));
                }
            }
            // Not fatal to the readout: everything else about the document is
            // still worth saying.
            Err(e) => lines.push(format!("signed: could not be checked — {e}")),
        }

        // -- marks placed but not yet part of the page ------------------------
        let mut placed = 0usize;
        let mut on_pages = Vec::new();
        for page in 0..doc.page_count {
            match session.signature_marks(page) {
                Ok(marks) if !marks.is_empty() => {
                    placed += marks.len();
                    on_pages.push(page + 1);
                }
                _ => {}
            }
        }
        if placed > 0 {
            lines.push(format!(
                "{placed} drawn signature{} placed on page{} {} — still an annotation \
                 anyone can delete; `applysignatures` makes {} part of the page",
                if placed == 1 { "" } else { "s" },
                if on_pages.len() == 1 { "" } else { "s" },
                on_pages.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", "),
                if placed == 1 { "it" } else { "them" },
            ));
        }

        // -- what is hidden under a passcode ----------------------------------
        let locked = session.locked_pages();
        if !locked.is_empty() {
            lines.push(format!(
                "locked: {} page{} — `unlock` with the passcode brings {} back",
                locked.len(),
                if locked.len() == 1 { "" } else { "s" },
                if locked.len() == 1 { "it" } else { "them" },
            ));
        }

        if let Some(level) = session.sensitivity() {
            lines.push(format!(
                "sensitivity: {} — a label, which marks rather than protects",
                level.describe()
            ));
        }

        if session.must_save_full_copy() {
            lines.push(
                "saving: this document must be saved as a full copy — an appended \
                 revision would leave what was removed in the file"
                    .into(),
            );
        }

        // Said last, because it is the one thing here that is a pointer rather
        // than a fact.
        lines.push("hidden data: not checked here — `hiddendata` reads the file and says".into());
        lines
    }

    /// Write the kept words out.
    ///
    /// Reported rather than swallowed, for the same reason the signatures are:
    /// "kept" when nothing was kept is worse than saying so.
    fn keep_snippets(&self) -> Result<(), String> {
        let Some(path) = &self.predefined_path else { return Ok(()) };
        self.predefined
            .save_to(path)
            .map_err(|e| format!("could not write {}: {e}", path.display()))
    }

    /// Keep some words, and arm the click that writes them.
    fn use_snippet(&mut self, text: &str) {
        let text = text.trim().to_string();
        if text.is_empty() {
            self.say_error("predefinedtext: the words to keep, as in `predefinedtext Jane Smith`.");
            return;
        }
        let is_new = self.predefined.remember(&text);
        if let Err(e) = self.keep_snippets() {
            self.say_error(e);
            return;
        }
        if is_new {
            // Said once, when it starts being kept — not every time it is used.
            self.say_info(format!(
                "kept \"{}\" on this computer. `predefinedtext` offers it again.",
                short(&text)
            ));
        }
        self.add_text(text);
    }

    /// Write the signature list out.
    ///
    /// **Reported rather than swallowed**, unlike the recents file: a settings
    /// file that fails to save costs somebody a line in a menu, and this one
    /// costs them a drawing they were told was safe.
    fn keep_signatures(&self) -> Result<(), String> {
        let Some(path) = &self.signatures_path else { return Ok(()) };
        self.signatures
            .save_to(path)
            .map_err(|e| format!("could not write {}: {e}", path.display()))
    }

    /// Say what is kept, newest last — the last one being the one a click uses.
    fn signature_list_lines(&self) -> String {
        if self.signatures.is_empty() {
            return "no signatures drawn yet — `signature draw` makes one.".into();
        }
        let current = self.signatures.current().map(|s| s.name.clone()).unwrap_or_default();
        let names: Vec<String> = self
            .signatures
            .names()
            .iter()
            .map(|name| {
                if *name == current {
                    format!("{name} (the one a click places)")
                } else {
                    (*name).to_string()
                }
            })
            .collect();
        names.join(", ")
    }

    /// Keep what was drawn on the pad.
    ///
    /// Takes the strokes rather than reading them off the pad so a test can
    /// hand it a signature without an egui context — the drawing is the one
    /// part of this that needs a pointer, and it is not the part worth
    /// leaving untested.
    fn save_drawn_signature(
        &mut self,
        name: &str,
        strokes: &[Vec<(f32, f32)>],
    ) -> Result<String, String> {
        let name = match name.trim() {
            "" => "Signature".to_string(),
            given => given.to_string(),
        };
        let Some(signature) = pagify_shell::signatures::Signature::from_drawing(&name, strokes)
        else {
            return Err("that is not enough of a signature to keep — draw across the pad.".into());
        };

        self.signatures.add(signature);
        if let Err(e) = self.keep_signatures() {
            self.signatures.remove(&name);
            return Err(e);
        }
        // Says where it went, because a signature is about as personal as a
        // file gets and somebody is entitled to know it is on their machine
        // and nowhere else.
        Ok(format!(
            "kept \"{name}\" on this computer{}. `signature` places it.",
            match &self.signatures_path {
                Some(path) => format!(", in {}", path.display()),
                None => String::new(),
            }
        ))
    }

    /// Put the drawn signature on the line somebody clicked.
    fn place_signature(&mut self, page: usize, at: AppPoint) -> Result<String, String> {
        // A signature on a form is around two inches across; wider looks like
        // a banner and narrower like initials.
        const WIDTH: f32 = 144.0;
        let Some(signature) = self.signatures.current().cloned() else {
            return Err("no signature has been drawn yet — `signature draw` makes one.".into());
        };
        let strokes: Vec<Vec<pdf_core::document::Point>> = signature
            .placed(at.x as f32, at.y as f32, WIDTH)
            .into_iter()
            .map(|stroke| {
                stroke
                    .into_iter()
                    .map(|(x, y)| pdf_core::document::Point { x, y })
                    .collect()
            })
            .collect();

        let Some(doc) = &self.doc else { return Err("nothing open.".into()) };
        // Placed *and* marked as a signature in one call, so `applysignatures`
        // can tell it from a drawing later — including after the document has
        // been closed and reopened.
        doc.session
            .place_signature(page, strokes, SIGNATURE_INK, 1.6, &signature.name)
            .map_err(|e| e.to_string())?;

        if let Some(doc) = &mut self.doc {
            doc.rendered_is_stale();
        }
        self.foreign = None;
        // **The sentence that keeps the two kinds of signature apart.** Somebody
        // who thinks this is the cryptographic one is worse off than somebody
        // with no signature at all, and this is the line they will read.
        Ok(format!(
            "signed \"{}\" on page {}. This is ink — it shows a name, it does not \
             prove one. `certify` is what signs with a certificate.",
            signature.name,
            page + 1
        ))
    }

    /// Put a tick, a cross or a dot where somebody clicked.
    fn stamp_mark(
        &mut self,
        page: usize,
        mark: pdf_core::document::FillMark,
        at: AppPoint,
    ) -> Result<String, String> {
        // Sized to sit inside a printed tick box, which is around ten points on
        // most forms.
        const SIZE: f32 = 12.0;
        let Some(doc) = &self.doc else { return Err("nothing open.".into()) };
        doc.session
            .stamp_mark(
                page,
                mark,
                pdf_core::document::Point { x: at.x as f32, y: at.y as f32 },
                SIZE,
            )
            .map_err(|e| e.to_string())?;

        if let Some(doc) = &mut self.doc {
            doc.rendered_is_stale();
        }
        Ok(format!("{} on page {}.", mark.describe(), page + 1))
    }

    /// Rule a line while filling a form in.
    fn stamp_line(&mut self, page: usize, a: AppPoint, b: AppPoint) -> Result<String, String> {
        let Some(doc) = &self.doc else { return Err("nothing open.".into()) };
        doc.session
            .stamp_line(
                page,
                pdf_core::document::Point { x: a.x as f32, y: a.y as f32 },
                pdf_core::document::Point { x: b.x as f32, y: b.y as f32 },
            )
            .map_err(|e| e.to_string())?;

        if let Some(doc) = &mut self.doc {
            doc.rendered_is_stale();
        }
        Ok(format!("line on page {}.", page + 1))
    }

    /// What is under a point, as something that could be picked up.
    ///
    /// Words first, then pictures. A caption sits *on* a photograph, and
    /// somebody clicking the caption means the caption — the smaller, more
    /// specific thing is what was aimed at, which is the same rule
    /// `pick_text_run` follows among overlapping runs.
    fn thing_at(
        &self,
        page: usize,
        at: AppPoint,
        pictures_first: bool,
    ) -> Option<(usize, pdf_core::document::Rect, &'static str)> {
        let near = HIT_TOLERANCE_PT as f32;
        let (x, y) = (at.x as f32, at.y as f32);
        let holds = |r: &pdf_core::document::Rect| {
            x >= r.left.min(r.right) - near
                && x <= r.left.max(r.right) + near
                && y >= r.top.min(r.bottom) - near
                && y <= r.top.max(r.bottom) + near
        };
        let doc = self.doc.as_ref()?;

        let words = || {
            doc.session
                .text_runs(page)
                .ok()?
                .into_iter()
                .filter(|run| holds(&run.rect))
                .min_by(|a, b| {
                    let area = |r: &pdf_core::document::TextRun| {
                        ((r.rect.right - r.rect.left) * (r.rect.bottom - r.rect.top)).abs()
                    };
                    area(a).total_cmp(&area(b))
                })
                .map(|run| (run.object, run.rect, "the words"))
        };
        let pictures = || {
            doc.session
                .images_on(page)
                .ok()?
                .into_iter()
                .filter(|image| holds(&image.rect))
                .min_by(|a, b| {
                    let area = |r: &pdf_core::document::PageImage| {
                        ((r.rect.right - r.rect.left) * (r.rect.bottom - r.rect.top)).abs()
                    };
                    area(a).total_cmp(&area(b))
                })
                .map(|image| (image.object, image.rect, "the picture"))
        };

        if pictures_first {
            pictures().or_else(words)
        } else {
            words().or_else(pictures)
        }
    }

    /// Pick something up and put it down somewhere else.
    fn move_thing(
        &mut self,
        page: usize,
        from: AppPoint,
        to: AppPoint,
        pictures_first: bool,
    ) -> Result<String, String> {
        let Some((object, _, what)) = self.thing_at(page, from, pictures_first) else {
            return Err("nothing to move there — click on some words or a picture.".into());
        };
        let by = pdf_core::document::Point {
            x: (to.x - from.x) as f32,
            y: (to.y - from.y) as f32,
        };
        if by.x.abs() < 0.1 && by.y.abs() < 0.1 {
            return Err("move: that is where it already is.".into());
        }

        let Some(doc) = &self.doc else { return Err("nothing open.".into()) };
        doc.session.move_object(page, object, by).map_err(|e| e.to_string())?;

        if let Some(doc) = &mut self.doc {
            doc.rendered_is_stale();
        }
        self.text = None;
        self.text_selection = None;
        self.find_hits.clear();
        Ok(format!(
            "moved {what} by {:.0} across and {:.0} down on page {}.",
            by.x,
            by.y,
            page + 1
        ))
    }

    /// Draw a box around something while filling a form in.
    fn stamp_box(&mut self, page: usize, a: AppPoint, b: AppPoint) -> Result<String, String> {
        let Some(area) = area_between(a, b) else {
            return Err("rectangle: that area has no size.".into());
        };
        let Some(doc) = &self.doc else { return Err("nothing open.".into()) };
        doc.session.stamp_box(page, area).map_err(|e| e.to_string())?;

        if let Some(doc) = &mut self.doc {
            doc.rendered_is_stale();
        }
        Ok(format!("box on page {}.", page + 1))
    }

    /// Paint over an area, covering what is there.
    ///
    /// **Says what it did not do.** Somebody reaching for this may believe it
    /// removes what it covers; the one place they are certain to read is the
    /// line that comes back when it works.
    fn whiteout(&mut self, page: usize, a: AppPoint, b: AppPoint) -> Result<String, String> {
        let Some(area) = area_between(a, b) else {
            return Err("whiteout: that area has no size.".into());
        };
        let Some(doc) = &self.doc else { return Err("nothing open.".into()) };
        doc.session
            .whiteout(page, area, pdf_core::document::Color { r: 255, g: 255, b: 255, a: 255 })
            .map_err(|e| e.to_string())?;

        if let Some(doc) = &mut self.doc {
            doc.rendered_is_stale();
        }
        Ok(format!(
            "painted over an area of page {} — the words underneath are still \
             in the file and still findable. `redact` is what destroys.",
            page + 1
        ))
    }

    /// Whether a password has been chosen and not yet written.
    ///
    /// **Unsaved work of a different shape.** A password is set on the document
    /// in memory and only reaches the file on the next save, so closing without
    /// saving throws it away — silently, and after somebody has typed it twice
    /// and been told it was set. Reported from use.
    fn unsaved_password(&self) -> bool {
        self.doc.as_ref().is_some_and(|d| d.session.is_secured())
    }

    /// Whether the document itself has been changed and not written.
    ///
    /// **The third shape of unsaved work, and the one nothing was asking
    /// about.** Marks live in this program until a save puts them in the file,
    /// and a password does too — both were checked. Everything else that
    /// changes a document changes it *in the document*: edited words, a
    /// whiteout, a redaction, a signature, a box, a page moved or deleted. All
    /// of it is only in memory until a save, and closing threw the lot away
    /// without a word. Reported from use.
    fn unsaved_edits(&self) -> bool {
        self.doc.as_ref().is_some_and(|d| d.session.is_dirty().unwrap_or(false))
    }

    /// Whether closing, or opening something else, would throw work away.
    ///
    /// One question with one answer, asked everywhere that closes something.
    /// Six places asked their own version of it, and every one of them knew
    /// about marks while five knew nothing about edits.
    fn would_lose_work(&self) -> bool {
        self.unsaved().is_some() || self.unsaved_password() || self.unsaved_edits()
    }


    /// This page's characters, extracted once and kept.
    fn characters(&mut self, page: usize) -> Option<&pagify_shell::reader::Characters> {
        if self.text.as_ref().map(|(p, _)| *p) != Some(page) {
            let chars = self.doc.as_ref()?.session.characters(page).ok()?;
            self.text = Some((page, chars));
        }
        self.text.as_ref().map(|(_, chars)| chars)
    }

    /// Rebuild this page's reading order and show the result.
    ///
    /// Asked for rather than applied. The disorder score is reported alongside,
    /// because on real documents it is a poor guide: a CAD drawing scores 0.637
    /// and reads perfectly, while the deliberately scrambled fixture scores
    /// 0.267 and does not.
    fn reflow(&mut self) {
        use pdf_core::document::layout;

        let page = self.page;
        let Some(doc) = &self.doc else {
            self.say_error("nothing open.");
            return;
        };

        let glyphs = match doc.session.with_engine(|s| s.document.page(page)?.glyphs()) {
            Ok(glyphs) if !glyphs.is_empty() => glyphs,
            Ok(_) => {
                self.say_info("this page has no text to reflow.");
                return;
            }
            Err(e) => {
                self.say_error(format!("reflow: {e}"));
                return;
            }
        };

        let measured = layout::trust(&glyphs);
        let rebuilt = layout::reconstruct(&glyphs);

        self.command_open = true;
        self.say_info(format!(
            "reflowed into {} block(s) — disorder was {:.2}{}",
            rebuilt.blocks.len(),
            measured.disorder(),
            if measured.is_noteworthy() { ", which is high" } else { "" },
        ));
        for line in rebuilt.plain().lines().take(40) {
            self.say_info(line.to_string());
        }
    }

    /// Search every page, and go to the first match.
    fn find(&mut self, needle: &str) {
        let Some(doc) = &self.doc else {
            self.say_error("nothing open.");
            return;
        };

        self.find_hits.clear();
        self.find_at = 0;
        self.find_needle = needle.to_string();

        for page in 0..doc.page_count {
            let Ok(chars) = doc.session.characters(page) else { continue };
            for hit in chars.find(needle) {
                self.find_hits.push((page, hit));
            }
        }

        if self.find_hits.is_empty() {
            self.say_info(format!("`{needle}` — no matches."));
            return;
        }

        let pages = {
            let mut seen: Vec<usize> = self.find_hits.iter().map(|(p, _)| *p).collect();
            seen.dedup();
            seen.len()
        };
        self.say_info(format!(
            "{} match{} on {pages} page{}. `findnext` steps through them.",
            self.find_hits.len(),
            if self.find_hits.len() == 1 { "" } else { "es" },
            if pages == 1 { "" } else { "s" },
        ));
        self.go_to_hit(0);
    }

    fn find_step(&mut self, forward: bool) {
        if self.find_hits.is_empty() {
            self.say_error("nothing to step through — `find <text>` first.");
            return;
        }
        let count = self.find_hits.len();
        let next = if forward {
            (self.find_at + 1) % count
        } else {
            (self.find_at + count - 1) % count
        };
        self.go_to_hit(next);
    }

    fn go_to_hit(&mut self, index: usize) {
        let Some((page, range)) = self.find_hits.get(index).cloned() else { return };
        self.find_at = index;

        if page != self.page {
            // Through `go_to` rather than by hand, so the scroll position, the
            // page-size cache and the raster cache all move together — they are
            // three things that must not disagree about which page is showing.
            self.go_to(PageTarget::Number(page + 1));
        }
        // The match is also the selection, so ⌘C copies what was found.
        self.text_selection = Some(range);
        self.selection_page = self.page;
        self.say_info(format!("match {} of {}", index + 1, self.find_hits.len()));
    }

    fn copy_selection(&mut self, ctx: &egui::Context) {
        // The page the selection was made on, not whichever one happens to be
        // in view now.
        let page = self.selection_page;
        let Some(range) = self.text_selection.clone() else {
            self.say_info("nothing selected.");
            return;
        };
        let Some(text) = self.characters(page).map(|c| c.text_of(range)) else { return };
        if text.trim().is_empty() {
            self.say_info("nothing selected.");
            return;
        }
        let length = text.chars().count();
        ctx.copy_text(text);
        self.say_info(format!("{length} characters copied."));
    }

    fn open_dialog(&mut self) {
        let start = self
            .recent
            .present()
            .first()
            .and_then(|e| e.path.parent().map(|p| p.to_path_buf()))
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from));

        let mut dialog = rfd::FileDialog::new()
            .set_title("Open a PDF")
            .add_filter("PDF", &["pdf"]);
        if let Some(start) = start {
            dialog = dialog.set_directory(start);
        }

        match dialog.pick_file() {
            Some(path) => self.open(&path.to_string_lossy()),
            None => self.say_info("nothing chosen."),
        }
    }

    /// Every candidate outlined-text font: what this build bundles, plus
    /// whatever a reader has added — see `Verb::OutlinedFont`.
    ///
    /// Read fresh on every call rather than cached: `redact`, `lock_area` and
    /// `extract_text` each need this, and a font added between two of those
    /// calls must take effect on the very next one, not after a restart.
    fn outlined_font_bytes(&self) -> Vec<Vec<u8>> {
        let mut fonts: Vec<Vec<u8>> = BUNDLED_OUTLINED_FONTS.iter().map(|f| f.to_vec()).collect();
        fonts.extend(self.outlined_fonts.bytes());
        fonts
    }

    fn outlined_font_dialog(&mut self) {
        let dialog = rfd::FileDialog::new()
            .set_title("Add a font for outlined-text recognition")
            .add_filter("Fonts", &["ttf", "otf", "ttc"]);

        match dialog.pick_file() {
            Some(path) => self.add_outlined_font(path),
            None => self.say_info("nothing chosen."),
        }
    }

    fn add_outlined_font(&mut self, path: PathBuf) {
        match self.outlined_fonts.add(path) {
            Ok(()) => {
                self.outlined_fonts.save();
                self.say_info("font added — tried on outlined pages from now on.");
            }
            Err(why) => self.say_error(why),
        }
    }

    fn remove_outlined_font(&mut self, path: PathBuf) {
        self.outlined_fonts.remove(&path);
        self.outlined_fonts.save();
        self.say_info("removed.");
    }

    fn clear_outlined_fonts(&mut self) {
        self.outlined_fonts.clear();
        self.outlined_fonts.save();
        self.say_info("cleared — only the bundled fonts will be tried now.");
    }

    fn outlined_font_action(&mut self, action: pagify_shell::verbs::OutlinedFontAction) {
        use pagify_shell::verbs::OutlinedFontAction;
        match action {
            OutlinedFontAction::Dialog => self.outlined_font_dialog(),
            OutlinedFontAction::Add(path) => self.add_outlined_font(path),
            OutlinedFontAction::Remove(path) => self.remove_outlined_font(path),
            OutlinedFontAction::Clear => self.clear_outlined_fonts(),
        }
    }

    /// Say what kind of text the current page has.
    ///
    /// `asked` distinguishes the command from the automatic note on opening a
    /// page: a reader who typed `textlayer` wants an answer either way, and one
    /// who is just turning pages wants to be told only when something is wrong.
    fn report_text_layer(&mut self, asked: bool) {
        use pdf_core::document::PageTextKind;

        let Some(doc) = &self.doc else {
            if asked {
                self.say_error("nothing open.");
            }
            return;
        };

        let Ok(verdict) = doc.session.classify(self.page) else {
            if asked {
                self.say_error("could not read this page's text layer.");
            }
            return;
        };

        let (trouble, note) = match verdict.kind {
            PageTextKind::Native => (false, "this page has selectable text.".to_string()),
            // Two different pages arrive here, and they need different answers.
            //
            // Text over a picture is ordinary and worth no alarm. Text *beside
            // text drawn as glyph outlines* means part of the page cannot be
            // selected, searched or copied at all — and the reader has no way
            // to tell, because what they see looks like the rest of the page.
            // That one is volunteered rather than kept for anyone who asks.
            PageTextKind::Hybrid if verdict.image_coverage >= 0.6 => (
                false,
                "this page has selectable text over an image — a scan that already \
                 carries a text layer, or type over a picture."
                    .to_string(),
            ),
            PageTextKind::Hybrid => (
                true,
                format!(
                    "part of this page is drawn as shapes rather than text ({} runs), so \
                     selecting and searching will miss it. Only the {} characters that are \
                     real text can be found.",
                    verdict.glyph_paths, verdict.chars
                ),
            ),
            PageTextKind::Scanned => (
                true,
                "this page has no text layer — it is a picture of text. Selection and \
                 search will find nothing until it is recognised."
                    .to_string(),
            ),
            PageTextKind::Unmappable => (
                true,
                format!(
                    "this page has text, but {:.0}% of it has no Unicode mapping, so it \
                     copies as nonsense. This wants encoding repair rather than \
                     recognition.",
                    verdict.unmappable_ratio() * 100.0
                ),
            ),
            PageTextKind::Outlined => (
                true,
                format!(
                    "this page has no text at all — {} shapes that look like letters. \
                     Type converted to outlines, as print production does.",
                    verdict.glyph_paths
                ),
            ),
            PageTextKind::Empty => (true, "this page has nothing on it to select.".to_string()),
        };

        // Whether this page's text is likely to read out of order.
        //
        // Volunteered rather than acted on. The score cannot tell paint order
        // from positioned layout — a CAD drawing scores higher than a
        // deliberately scrambled page — so the useful thing to do with it is
        // mention `reflow` and let someone looking at the page decide.
        let reflow_hint = self
            .doc
            .as_ref()
            .and_then(|d| d.session.with_engine(|s| s.document.page(self.page)?.glyphs()).ok())
            .map(|glyphs| pdf_core::document::layout::trust(&glyphs))
            .filter(|t| t.is_noteworthy())
            .map(|t| {
                format!(
                    "this page's text may not read in order (disorder {:.2}) — `reflow` \
                     rebuilds it from the layout if the copy comes out jumbled.",
                    t.disorder()
                )
            });

        if asked {
            self.say_info(note);
            self.say_info(format!(
                "  {} characters, {} unmappable, {:.0}% image, {} paths",
                verdict.chars,
                verdict.unmappable,
                verdict.image_coverage * 100.0,
                verdict.paths
            ));
            if let Some(hint) = reflow_hint {
                self.say_info(hint);
            }
        } else if trouble {
            self.say_info(note);
        } else if let Some(hint) = reflow_hint {
            self.say_info(hint);
        }
    }

    fn page_extent(&self, page: usize) -> (f32, f32) {
        let Some(doc) = &self.doc else { return (612.0, 792.0) };
        let (w, h) = doc.strip.size_of(page).unwrap_or((612.0, 792.0));
        if self.rotation.swaps_axes() { (h, w) } else { (w, h) }
    }

    fn resolved_zoom(&self) -> f32 {
        let (w, h) = self.page_extent(self.page);
        let available = (self.canvas_pt - egui::vec2(24.0, 24.0)).max(egui::vec2(1.0, 1.0));
        match self.zoom {
            ZoomMode::Factor(f) => f,
            ZoomMode::Width => (available.x / w).clamp(0.05, 16.0),
            ZoomMode::Fit => (available.x / w).min(available.y / h).clamp(0.05, 16.0),
        }
    }

    // -- running verbs ------------------------------------------------------

    /// Run a command line as though it had been typed and submitted.
    pub fn submit(&mut self, line: &str) {
        // A run open for editing takes the line: the words being typed are
        // text, not a command, and this is the path tests and recordings use in
        // place of typing into the box on the page.
        if let Some(edit) = &mut self.editing_run {
            edit.buffer = line.to_string();
            self.apply_edited_run();
            return;
        }
        if self.awaiting_password.is_some() {
            self.cmd.input_mut().clear();
            self.cmd.input_mut().push_str(line);
            self.consume_password_line();
            return;
        }
        if let Some(dispatch) = pagify_shell::command::dispatch(line) {
            self.cmd.say(Kind::Echo, line);
            self.recorder.observe(line);
            self.run(dispatch);
        }
    }

    fn run(&mut self, dispatch: Dispatch) {
        match dispatch {
            Dispatch::Pagify(Verb::Help(topic)) => {
                for l in verbs::help_text(topic.as_deref()) {
                    self.say_info(l);
                }
            }
            Dispatch::Pagify(Verb::Planned { verb, phase }) => {
                self.say_info(format!("`{verb}` is planned for {phase}, and not built yet."));
            }
            Dispatch::Pagify(verb) => self.act(verb),
            Dispatch::Kernel(command) => self.draw(*command),
            ref other => self.cmd.report(other),
        }
    }

    fn act(&mut self, verb: Verb) {
        match verb {
            // Opening another document drops this one's markup just as surely
            // as closing it does.
            Verb::Open(path) => {
                if self.would_lose_work() {
                    self.closing = Some(Closing::Open(path.clone()));
                } else {
                    self.open(&path.to_string_lossy());
                }
            }
            Verb::OpenDialog => {
                if self.would_lose_work() {
                    self.closing = Some(Closing::OpenDialog);
                } else {
                    self.open_dialog();
                }
            }
            Verb::Close { force } => {
                if !force && self.would_lose_work() {
                    self.closing = Some(Closing::Document);
                    return;
                }
                if self.doc.take().is_some() {
                    self.markup.clear();
                    self.cmd.prompt_mut().document = None;
                    self.ribbon = Tab::File;
                    self.say_info("closed.");
                } else {
                    self.say_error("nothing open.");
                }
            }
            Verb::Quit { force } => {
                if !force && self.would_lose_work() {
                    self.closing = Some(Closing::Program);
                    return;
                }
                std::process::exit(0);
            }
            Verb::Page(target) => self.go_to(target),
            Verb::Zoom(target) => self.set_zoom(target),
            Verb::RotatePage(degrees) => self.rotate_view(degrees),
            Verb::TextLayer => self.report_text_layer(true),
            Verb::Pdfium => {
                let d = pagify_shell::pdfium::describe();
                self.say_info(d);
            }
            Verb::Pick(at) => {
                if self.pending.is_none() {
                    self.say_error("nothing is waiting for a click.");
                    return;
                }
                // A typed pick is in the *same* coordinates as a typed draw
                // command — `l 30,250 170,250` and `pick 100,250` must refer to
                // the same place, or every scripted pick misses. That is the
                // kernel's page space, y up from the bottom-left, which is also
                // the PDF's own convention.
                //
                // Pointer picks arrive in app space instead, so this is the one
                // place that converts, through the module that owns the flip.
                let page = self.page;
                let height = self
                    .doc
                    .as_ref()
                    .and_then(|d| d.strip.size_of(page))
                    .map(|(_, h)| h as f64)
                    .unwrap_or(792.0);
                let space = pagify_shell::page_space::PageSpace::new(height);
                let in_app = space.from_kernel(cad_kernel::Vec2::new(at.x, at.y));
                self.take_pick(in_app);
            }
            Verb::Sensitivity(what) => {
                use pdf_core::document::sensitivity::Sensitivity;
                let Some(doc) = &self.doc else {
                    self.say_error("nothing open.");
                    return;
                };
                match what {
                    // Reporting.
                    None => {
                        let said = match doc.session.sensitivity() {
                            Some(level) => format!(
                                "marked {} — {}. `sensitivity none` takes it off.",
                                level.stamp(),
                                level.describe()
                            ),
                            None => "not marked. Try `sensitivity confidential`, or \
                                     public, internal or secret."
                                .to_string(),
                        };
                        self.say_info(said);
                    }
                    // Taking it off.
                    Some(word) if word.is_empty() => match doc.session.clear_sensitivity() {
                        Ok(()) => {
                            if let Some(doc) = &mut self.doc {
                                doc.rendered_is_stale();
                            }
                            self.text = None;
                            self.say_info("the marking is off. Save to write it out.");
                        }
                        Err(e) => self.say_error(e.to_string()),
                    },
                    // Marking.
                    Some(word) => {
                        let Some(level) = Sensitivity::parse(&word) else {
                            self.say_error(format!("sensitivity: don't know {word:?}."));
                            return;
                        };
                        match doc.session.set_sensitivity(level) {
                            Ok(()) => {
                                if let Some(doc) = &mut self.doc {
                                    doc.rendered_is_stale();
                                }
                                self.text = None;
                                // Says plainly what a marking is and is not.
                                // Somebody reaching for it may believe it
                                // protects the document; it does not.
                                self.say_info(format!(
                                    "marked {} on every page — {}. This says what you \
                                     intend, it does not enforce it: `secure` is what \
                                     withholds a document. Save to write it out.",
                                    level.stamp(),
                                    level.describe()
                                ));
                            }
                            Err(e) => self.say_error(e.to_string()),
                        }
                    }
                }
            }
            Verb::FillSign(what) => {
                if self.doc.is_none() {
                    self.say_error("nothing open.");
                    return;
                }
                let page = self.page;
                match what.as_deref().and_then(pdf_core::document::FillMark::parse) {
                    // A tick, a cross or a dot: one click each.
                    Some(mark) => self.arm(PendingKind::Fill(mark), page),
                    // Bare `fillsign` types where you click, which is the other
                    // half of filling a form in by hand.
                    None => {
                        self.arm(PendingKind::PickText, page);
                        self.say_info(
                            "fill: click a line of text to change it, or use \
                             `addtext <words>` to write somewhere new — and \
                             `fillsign tick`, `cross` or `dot` for a box.",
                        );
                    }
                }
            }
            Verb::PredefinedText(words) => match words {
                Some(text) => self.use_snippet(&text),
                None => {
                    self.snippets = Some(SnippetList::default());
                    if self.predefined.is_empty() {
                        self.say_info(
                            "nothing kept yet — type some words into the window, or \
                             `predefinedtext Jane Smith` to keep and write them at once.",
                        );
                    }
                }
            },
            Verb::EditObject => {
                let page = self.page;
                if self.doc.is_none() {
                    self.say_error("nothing open.");
                    return;
                }
                self.arm(PendingKind::Move { pictures_first: true }, page);
            }
            Verb::MoveThing => {
                let page = self.page;
                if self.doc.is_none() {
                    self.say_error("nothing open.");
                    return;
                }
                self.arm(PendingKind::Move { pictures_first: false }, page);
            }
            Verb::SignLine => {
                let page = self.page;
                if self.doc.is_none() {
                    self.say_error("nothing open.");
                    return;
                }
                self.arm(PendingKind::SignLine, page);
            }
            Verb::SignRectangle => {
                let page = self.page;
                if self.doc.is_none() {
                    self.say_error("nothing open.");
                    return;
                }
                self.arm(PendingKind::SignRectangle, page);
            }
            Verb::DocumentStatus => {
                for line in self.document_status() {
                    self.say_info(line);
                }
            }
            Verb::ApplySignatures => {
                let Some(doc) = &self.doc else {
                    self.say_error("nothing open.");
                    return;
                };
                let pages = doc.page_count;
                let mut applied = 0usize;
                let mut touched = Vec::new();
                for page in 0..pages {
                    match doc.session.apply_signatures(page) {
                        Ok(0) => {}
                        Ok(count) => {
                            applied += count;
                            touched.push(page + 1);
                        }
                        Err(e) => {
                            self.say_error(format!("page {}: {e}", page + 1));
                            return;
                        }
                    }
                }

                if applied == 0 {
                    // Not a failure: a document with no signatures placed on it
                    // is the ordinary case for this verb being pressed by
                    // mistake.
                    self.say_info(
                        "no signatures are placed on this document. `signature` places one.",
                    );
                    return;
                }
                if let Some(doc) = &mut self.doc {
                    doc.rendered_is_stale();
                }
                self.foreign = None;
                // **Says the two things that matter and are not obvious**: that
                // they can no longer be picked up, and that the way back is to
                // close without saving rather than to press undo.
                self.say_info(format!(
                    "{applied} signature{} on page{} {} {} part of the page now — \
                     nothing can select or delete {} any more. Undo does not reach \
                     this; closing without saving does.",
                    if applied == 1 { "" } else { "s" },
                    if touched.len() == 1 { "" } else { "s" },
                    touched.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", "),
                    if applied == 1 { "is" } else { "are" },
                    if applied == 1 { "it" } else { "them" },
                ));
            }
            Verb::ManageSignatures(what) => {
                use pagify_shell::verbs::Signatures as What;
                match what {
                    What::Open => {
                        self.signature_list = Some(SignatureList::default());
                        if self.signatures.is_empty() {
                            self.say_info("no signatures yet — the window has a button to draw one.");
                        }
                    }
                    What::List => {
                        let said = self.signature_list_lines();
                        self.say_info(said);
                    }
                    What::Use(name) => {
                        if self.signatures.choose(&name) {
                            match self.keep_signatures() {
                                Ok(()) => {
                                    let chosen = self
                                        .signatures
                                        .current()
                                        .map(|s| s.name.clone())
                                        .unwrap_or(name);
                                    self.say_info(format!("`signature` now places \"{chosen}\"."));
                                }
                                Err(e) => self.say_error(e),
                            }
                        } else {
                            // Says what there is, rather than choosing for them.
                            self.say_error(format!(
                                "no signature called \"{name}\". There is: {}",
                                self.signature_list_lines()
                            ));
                        }
                    }
                    What::Forget(name) => {
                        if self.signatures.remove(&name) {
                            match self.keep_signatures() {
                                // Said plainly: this one does not come back.
                                Ok(()) => self.say_info(format!(
                                    "\"{name}\" is gone — a drawing is not something undo \
                                     reaches. {}",
                                    self.signature_list_lines()
                                )),
                                Err(e) => self.say_error(e),
                            }
                        } else {
                            self.say_error(format!(
                                "no signature called \"{name}\". There is: {}",
                                self.signature_list_lines()
                            ));
                        }
                    }
                    What::Rename(to) => {
                        let Some(from) = self.signatures.current().map(|s| s.name.clone()) else {
                            self.say_error(
                                "no signatures drawn yet — `signature draw` makes one.",
                            );
                            return;
                        };
                        match self.signatures.rename(&from, &to) {
                            Ok(()) => match self.keep_signatures() {
                                Ok(()) => self.say_info(format!("\"{from}\" is now \"{to}\".")),
                                Err(e) => self.say_error(e),
                            },
                            Err(why) => self.say_error(format!(
                                "{} — {}",
                                why.describe(),
                                self.signature_list_lines()
                            )),
                        }
                    }
                }
            }
            Verb::Signature { draw } => {
                // Drawing needs no document — somebody can make their
                // signature before they have anything to sign. Placing does.
                if draw || self.signatures.current().is_none() {
                    let first = self.signatures.current().is_none();
                    self.pad = Some(SignaturePad {
                        name: if first {
                            "Signature".to_string()
                        } else {
                            format!("Signature {}", self.signatures.entries().len() + 1)
                        },
                        // Somebody who typed `signature` wanted to sign, not to
                        // draw; carrying on to the click is the rest of that.
                        then_place: !draw,
                        ..SignaturePad::default()
                    });
                    self.say_info(if first {
                        "draw your signature in the window — it is kept on this \
                         computer, and nowhere else."
                    } else {
                        "draw a signature in the window."
                    });
                    return;
                }
                let page = self.page;
                if self.doc.is_none() {
                    self.say_error("nothing open.");
                    return;
                }
                self.arm(PendingKind::Signature, page);
            }
            Verb::Validate => {
                let Some(doc) = &self.doc else {
                    self.say_error("nothing open.");
                    return;
                };
                match doc.session.validate_signatures() {
                    Ok(found) if found.is_empty() => {
                        // Not a failure, and not phrased as one.
                        self.say_info("this document carries no signatures.");
                    }
                    Ok(found) => {
                        let said: Vec<String> = found
                            .iter()
                            .map(|s| {
                                let what = if s.timestamp { "timestamp" } else { "signature" };
                                let who = if s.name.is_empty() {
                                    String::new()
                                } else {
                                    format!(" by {}", s.name)
                                };
                                format!("{what}{who}: {}", s.verdict.describe())
                            })
                            .collect();
                        // Anything other than unaltered is a warning, not news.
                        let all_well = found
                            .iter()
                            .all(|s| s.verdict == pdf_core::pdf::validate::Verdict::Unaltered);
                        let line = said.join("; ");
                        if all_well {
                            self.say_info(line);
                        } else {
                            self.say_error(line);
                        }
                    }
                    Err(e) => self.say_error(e.to_string()),
                }
            }
            Verb::TimeStamp(authority) => {
                let Some(doc) = &self.doc else {
                    self.say_error("nothing open.");
                    return;
                };
                let Some(authority) = authority else {
                    // **No default address.** Naming one here would make the
                    // program contact somewhere the person never chose, and
                    // which authority to trust is not ours to decide.
                    self.say_info(
                        "timestamp <http://address-of-a-time-authority> — a digest of \
                         this document is sent there and nothing else, and the \
                         signed answer goes into the file. There is no default \
                         address; the choice of who to trust is yours.",
                    );
                    return;
                };
                match doc.session.timestamp_document(&authority) {
                    Ok(()) => {
                        if let Some(doc) = &mut self.doc {
                            doc.rendered_is_stale();
                        }
                        self.text = None;
                        self.say_info(
                            "timestamped. It covers the file as it is now — anything \
                             changed after this breaks it.",
                        );
                    }
                    Err(e) => self.say_error(e.to_string()),
                }
            }
            Verb::Certify(path) => {
                let Some(doc) = &self.doc else {
                    self.say_error("nothing open.");
                    return;
                };
                match path {
                    // Reporting.
                    None => {
                        let count = doc.session.signature_count();
                        self.say_info(if count == 0 {
                            "not signed. `certify <certificate.p12>` signs it.".to_string()
                        } else {
                            format!(
                                "{count} signature{}. Anything written after a \
                                 signature breaks it.",
                                if count == 1 { "" } else { "s" }
                            )
                        });
                    }
                    Some(path) => {
                        if !path.is_file() {
                            self.say_error(format!("{}: no such file.", path.display()));
                            return;
                        }
                        self.awaiting_password = Some(Awaiting::Certificate(path));
                        self.say_info("type the certificate's password, or Escape to give up.");
                    }
                }
            }
            Verb::Whiteout => {
                if self.doc.is_none() {
                    self.say_error("nothing open.");
                } else {
                    let page = self.page;
                    self.arm(PendingKind::Whiteout, page);
                }
            }
            Verb::Redact => {
                if self.doc.is_none() {
                    self.say_error("nothing open.");
                } else {
                    let page = self.page;
                    self.arm(PendingKind::Redact, page);
                }
            }
            Verb::Lock => {
                if self.doc.is_none() {
                    self.say_error("nothing open.");
                    return;
                }
                // **Selecting the words is the obvious way to say which words.**
                // Asking for two opposite corners of a rectangle is a drawing
                // gesture, and this is not a drawing operation — reported from
                // use as not being what anyone reaches for. A selection already
                // on the page is taken as the answer; otherwise it says to make
                // one, and the rectangle stays available for the cases a
                // selection cannot express, like an area of a scan.
                if self.text_selection.is_some() {
                    self.lock_selection();
                    return;
                }
                self.say_info(
                    "lock: select the words to hide, then `lock` again — or drag a rectangle with `lockarea`.",
                );
            }
            Verb::Secure(options) => {
                let Some(doc) = &self.doc else {
                    self.say_error("nothing open.");
                    return;
                };
                // **Refused before a password is typed, not after.**
                //
                // Two passwords cannot both be written — the content would be
                // encrypted twice and open for nobody — and the engine has
                // always refused that. But it refused at *save*, by which time
                // somebody had chosen a password, met the rule and typed it
                // twice for nothing.
                // A document that already has one is *changed*, not refused:
                // the current password first, then the new one.
                if doc.session.already_has_password() {
                    self.awaiting_password = Some(Awaiting::SecureCurrent(options));
                    self.say_info("this document has a password — type it to change it.");
                    return;
                }
                if doc.session.is_secured() {
                    self.say_info(
                        "this document already has a password waiting — `unsecure` takes it off.",
                    );
                    return;
                }
                let allowed = options.describe();
                self.awaiting_password = Some(Awaiting::Secure(options));
                self.say_info(format!(
                    "type a password for this document, or Escape to give up. \
                     Anyone opening the file will be asked for it — {allowed}."
                ));
            }
            Verb::SmartRedact { redact } => {
                let Some(doc) = &self.doc else {
                    self.say_error("nothing open.");
                    return;
                };
                // Every page, because the thing somebody is looking for is
                // rarely on the one they happen to be reading.
                let mut found = Vec::new();
                for page in 0..doc.page_count {
                    match doc.session.sensitive_on(page) {
                        Ok(on_page) => found.extend(on_page),
                        Err(e) => {
                            self.say_error(format!("page {}: {e}", page + 1));
                            return;
                        }
                    }
                }
                if found.is_empty() {
                    self.say_info(
                        "nothing found that can be checked — addresses, card numbers, \
                         account numbers and telephone numbers are what this looks for.",
                    );
                    return;
                }

                if !redact {
                    // Named, with their pages, so a person can look before
                    // anything happens to them.
                    let mut said: Vec<String> = found
                        .iter()
                        .take(12)
                        .map(|f| {
                            format!("p{} {}: {}", f.page_index + 1, f.kind.describe(), f.text)
                        })
                        .collect();
                    if found.len() > said.len() {
                        said.push(format!("and {} more", found.len() - said.len()));
                    }
                    self.say_info(format!(
                        "{} found — {}. `smartredact redact` blacks them out.",
                        found.len(),
                        said.join("; ")
                    ));
                    return;
                }

                // Destroyed, not hidden: this is redaction, and there is no
                // passcode to bring any of it back.
                let mut done = 0usize;
                let mut refused: Vec<String> = Vec::new();
                let faces = self.outlined_font_bytes();
                let Some(doc) = &self.doc else { return };
                for item in &found {
                    // Through the command, so each one lands in the history and
                    // can be undone one at a time — the same as a redaction
                    // somebody drew by hand.
                    let outcome = doc.session.execute(pdf_core::command::Command::Redact {
                        page_index: item.page_index,
                        area: item.area,
                        fill: Some(pdf_core::document::Color { r: 0, g: 0, b: 0, a: 255 }),
                        // These were found *by* their text, so there is text to
                        // clear; an image crossing the edge must not stop the
                        // rest of the page being done.
                        allow_incomplete: true,
                        outlined_fonts: faces.clone(),
                    });
                    match outcome {
                        Ok(_) => done += 1,
                        Err(e) => refused.push(format!("p{}: {e}", item.page_index + 1)),
                    }
                }
                if let Some(doc) = &mut self.doc {
                    doc.rendered_is_stale();
                }
                self.text = None;
                self.text_selection = None;
                self.find_hits.clear();
                self.say_info(if refused.is_empty() {
                    format!("{done} redacted — gone for good. Save to write it out.")
                } else {
                    format!(
                        "{done} redacted; {} refused ({}). Save to write it out.",
                        refused.len(),
                        refused.join(", ")
                    )
                });
            }
            Verb::HiddenData { clean } => {
                let Some(doc) = &self.doc else {
                    self.say_error("nothing open.");
                    return;
                };
                let outcome = if clean {
                    doc.session.remove_hidden_data()
                } else {
                    doc.session.hidden_data()
                };
                match outcome {
                    Ok(found) if clean => {
                        if let Some(doc) = &mut self.doc {
                            doc.rendered_is_stale();
                        }
                        self.text = None;
                        self.text_selection = None;
                        self.find_hits.clear();
                        self.say_info(if found.is_empty() {
                            "there was nothing hidden to remove.".to_string()
                        } else {
                            format!("removed: {}. Save to write it out.", found.describe())
                        });
                    }
                    Ok(found) => self.say_info(if found.is_empty() {
                        found.describe()
                    } else {
                        format!("{} — `hiddendata clean` takes it out.", found.describe())
                    }),
                    Err(e) => self.say_error(e.to_string()),
                }
            }
            Verb::Unsecure => {
                let Some(doc) = &mut self.doc else {
                    self.say_error("nothing open.");
                    return;
                };
                match doc.session.unsecure_document() {
                    Ok(()) => self.say_info("the password is off; save to write it out."),
                    Err(e) => self.say_error(e.to_string()),
                }
            }
            Verb::LockArea => {
                if self.doc.is_none() {
                    self.say_error("nothing open.");
                } else {
                    let page = self.page;
                    self.arm(PendingKind::Lock, page);
                }
            }
            Verb::LockPages(spec) => {
                let Some(doc) = &self.doc else {
                    self.say_error("nothing open.");
                    return;
                };
                match pagify_shell::organize::parse_range(&spec, doc.page_count) {
                    Ok(pages) => {
                        self.awaiting_password = Some(Awaiting::LockPages(pages));
                        self.say_info(
                            "type a passcode to lock these pages with, or Escape to give up.",
                        );
                    }
                    Err(why) => self.say_error(why),
                }
            }
            Verb::Unlock => {
                if self.doc.is_none() {
                    self.say_error("nothing open.");
                } else if self.doc.as_ref().is_some_and(|d| d.session.locked_pages().is_empty()) {
                    self.say_error("nothing in this document is locked.");
                } else {
                    self.awaiting_password = Some(Awaiting::Unlock);
                    self.say_info("type the passcode this was locked with, or Escape to give up.");
                }
            }
            Verb::Undo | Verb::Redo => self.undo_redo(matches!(verb, Verb::Undo)),

            Verb::Pointer(mode) => {
                // Switching tools abandons whatever was half-picked. Leaving a
                // pending operation armed under a new tool is how a click meant
                // for one thing lands in another.
                if self.pending.take().is_some() {
                    if let Some(layer) = self.markup.existing_mut(self.page) {
                        layer.forget_last_step();
                    }
                }
                self.pointer = mode;
                self.say_info(match mode {
                    pagify_shell::verbs::PointerMode::Select =>
                        "select — drag over text to select it, or over paper to select marks.",
                    pagify_shell::verbs::PointerMode::Pan => "hand — drag to move the page.",
                });
            }

            Verb::CopyText => self.copy_wanted = true,
            Verb::EditText => self.edit_text(),
            Verb::AddText(text) => self.add_text(text),
            Verb::SetLayout(layout) => self.set_layout(layout),
            Verb::ReversePages => self.reverse_pages(),
            Verb::DuplicatePages(spec) => self.duplicate_pages(&spec),
            Verb::CropPages { pages, margin } => self.crop_pages(&pages, margin),
            Verb::ResizePages { pages, width_pt, height_pt } => {
                self.resize_pages(&pages, width_pt, height_pt)
            }
            Verb::SwapPages { a, b } => self.swap_pages(a, b),
            Verb::RotatePages { pages, quarters } => self.rotate_pages(&pages, quarters),
            Verb::ExtractText(spec) => self.extract_text(&spec),
            Verb::OutlinedFont(action) => self.outlined_font_action(action),

            // The typed form of pressing Enter over the page.
            Verb::Finish => {
                let closeable = self
                    .pending
                    .as_ref()
                    .is_some_and(|p| p.kind.ends_on_enter() && p.points.len() >= 2);
                if closeable {
                    self.resolve();
                } else if self.pending.is_some() {
                    self.say_error("not enough points yet.");
                } else {
                    self.say_error("nothing to finish.");
                }
            }
            Verb::Save => self.save(None),
            Verb::SaveAs(path) => self.save(Some(path)),

            Verb::Extract { pages, dest } => self.extract(&pages, &dest),
            Verb::Import { source, pages } => self.import(&source, &pages),
            Verb::DeletePages(pages) => self.delete_pages(&pages),
            Verb::InsertPage => self.insert_page(),
            Verb::MovePages { pages, before } => self.move_pages(&pages, before),

            Verb::Reflow => self.reflow(),
            Verb::Find(needle) => self.find(&needle),
            Verb::FindStep { forward } => self.find_step(forward),
            Verb::Copy => {}
            Verb::MarkText(kind) => self.mark_selection(kind),
            Verb::ListMarks => self.list_marks(),
            Verb::RemoveMark(n) => self.remove_mark(n),
            Verb::Note(text) => self.add_note(text),

            Verb::Calibrate { distance, unit } => {
                let page = self.page;
                self.arm(PendingKind::Calibrate { distance, unit }, page);
            }
            Verb::Scale => {
                let d = self.calibration.describe();
                self.say_info(d);
            }
            Verb::Measure(kind) => {
                let page = self.page;
                self.arm(PendingKind::Measure(kind), page);
            }

            Verb::Record(name) => {
                let name = if name.trim().is_empty() { "script".to_string() } else { name };
                self.recorder.start(name.clone());
                self.say_info(format!("recording `{name}` — every command from here is a step."));
            }
            Verb::StopRecording => self.stop_recording(),
            Verb::Replay(path) => self.replay(&path),

            Verb::Help(_) | Verb::Planned { .. } => unreachable!("handled in run()"),
        }
    }

    fn draw(&mut self, command: cad_kernel::parser::Command) {
        let Some(doc) = &self.doc else {
            self.say_error("open a document before drawing on it.");
            return;
        };
        let height = doc.strip.size_of(self.page).map(|(_, h)| h as f64).unwrap_or(792.0);
        let page = self.page;

        // **Nothing new goes over a lock — and it has to be said here.**
        //
        // Reported from use: ink stayed visible on a locked page. The engine
        // already refuses a mark over a lock, but drawn ink does not reach the
        // engine until the document is saved — it lives in this layer and is
        // painted over the page meanwhile. So the refusal arrived far too late
        // to mean anything, and in between the page looked marked-up despite
        // being locked.
        //
        // Whole-page locks are what this catches: an area lock cannot be
        // judged until the geometry exists, and `add_annotation` catches those
        // when the layer is committed.
        if self.page_is_locked(page) {
            self.say_error(
                "this page is locked — unlock it before drawing on it.",
            );
            return;
        }

        let layer = self.markup.page(page, height);
        let applied = tools::apply(layer, &command, &mut self.defaults);

        match applied {
            tools::Applied::Added(n) => self.say_info(format!("{n} added.")),
            tools::Applied::Changed(n) => self.say_info(format!("{n} changed.")),
            tools::Applied::Removed(n) => self.say_info(format!("{n} removed.")),
            tools::Applied::Nothing(why) => self.say_info(why),
            tools::Applied::Failed(why) => self.say_error(why),

            tools::Applied::Tool(kind) => {
                use cad_kernel::parser::ToolKind;
                let draw = match kind {
                    ToolKind::Line => Some(DrawKind::Line),
                    ToolKind::Circle => Some(DrawKind::Circle),
                    ToolKind::Rectangle => Some(DrawKind::Rectangle),
                    ToolKind::Polyline => Some(DrawKind::Polyline),
                    _ => None,
                };
                match draw {
                    Some(kind) => self.arm(PendingKind::Draw(kind), page),
                    None => self.say_info(
                        "that tool draws from typed coordinates for now — e.g. `arc3p 0,0 50,50 100,0`.",
                    ),
                }
            }

            tools::Applied::Interactive(pick) => {
                if pick.needs_selection {
                    let empty = self
                        .markup
                        .existing(page)
                        .map(|l| l.selection().is_empty())
                        .unwrap_or(true);
                    if empty {
                        self.say_error("nothing selected — click a mark first, or drag a box round some.");
                        return;
                    }
                }
                self.arm(PendingKind::Modify(pick), page);
            }
        }
    }

    /// Start collecting clicks for an operation.
    /// Stop whatever is going on: abandon a half-finished tool, drop back to
    /// the pointer, and hand the command box its own escape.
    ///
    /// A method rather than a block inside the key handler because it is the
    /// one piece of behaviour every other one has to defer to, and because a
    /// key handler needs a live `egui::Context` to reach — which meant the most
    /// important key in the program could not be tested.
    ///
    /// Returns what the command box made of it, since only the caller has the
    /// focus to surrender.
    fn escape(&mut self) -> Escaped {
        // Escape means "stop what you are doing", and being left in Hand
        // afterwards is not stopping.
        self.pointer = Default::default();
        if let Some(what) = self.awaiting_password.take() {
            self.say_info(match what {
                Awaiting::Open(_) => "gave up on the password.",
                Awaiting::Lock { .. } | Awaiting::LockPages(_) | Awaiting::LockImage { .. } => {
                    "nothing was locked."
                }
                Awaiting::Unlock | Awaiting::UnlockItem(_) => "nothing was unlocked.",
                Awaiting::Secure(_)
                | Awaiting::SecureAgain { .. }
                | Awaiting::SecureCurrent(_) => "no password was set.",
                Awaiting::LockAgain { .. } => "nothing was locked.",
                Awaiting::Certificate(_) => "nothing was signed.",
            });
        }
        if self.editing_run.take().is_some() {
            self.say_info("left as it was.");
        }
        if self.markup_armed.take().is_some() {
            self.say_info("tool put down.");
        }
        if let Some(reading) = &self.reading {
            // A real stop: the worker checks this between lines and puts the
            // page down.
            reading.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        if self.pending.take().is_some() {
            self.say_info("cancelled.");
            if let Some(layer) = self.markup.existing_mut(self.page) {
                layer.forget_last_step();
            }
        }
        self.cmd.escape()
    }

    /// Hide an area, sealed under a passcode.
    ///
    /// **The passcode arrives borrowed and leaves with the call.** It reaches
    /// the engine and nothing else — not the command history, which is visible;
    /// not the recorder, which replays; and not the undo stack, where it would
    /// outlive the moment it was typed.
    fn lock_area(
        &mut self,
        page: usize,
        area: pdf_core::document::Rect,
        passcode: &[u8],
        require_complete: bool,
    ) -> Result<String, String> {
        if passcode.is_empty() {
            return Err("a lock needs a passcode.".into());
        }
        let Some(doc) = &self.doc else { return Err("nothing open.".into()) };
        let fonts = self.outlined_font_bytes();
        let font_refs: Vec<&[u8]> = fonts.iter().map(|f| f.as_slice()).collect();
        match doc.session.lock_area(page, area, passcode, &font_refs, require_complete) {
            Ok(report) => {
                if let Some(doc) = &mut self.doc {
                    doc.rendered_is_stale();
                }
                self.text = None;
                self.text_selection = None;
                self.find_hits.clear();
                Ok(format!(
                    "locked {} character{} on page {} — `unlock` and the passcode bring them back.",
                    report.characters,
                    if report.characters == 1 { "" } else { "s" },
                    page + 1
                ))
            }
            Err(e) => Err(format!("lock: {e}")),
        }
    }

    /// Hide whole pages, sealed under a passcode.
    ///
    /// The passcode arrives borrowed and leaves with the call, exactly as
    /// [`Self::lock_area`]'s does.
    /// Write any drawn ink on these pages into the document before locking.
    ///
    /// **Reported from use: ink stayed visible on a locked page.** Drawn marks
    /// live in the app's own layer until the document is saved, so a lock — which
    /// seals the page *as the document has it* — sealed a page that did not
    /// include them, and the layer went on painting them over the result.
    ///
    /// Committing first makes them part of the page, so they are sealed with it
    /// and come back with it. The alternative was to throw them away, which is
    /// not something to do with somebody's unsaved work.
    fn commit_marks_on(&mut self, pages: &[usize]) -> Result<(), String> {
        let Some(doc) = &self.doc else { return Ok(()) };
        for page in pages {
            let Some(layer) = self.markup.existing(*page) else { continue };
            doc.session
                .commit_markup(*page, layer, MARKUP_INK, 1.5)
                .map_err(|e| format!("could not write the marks on page {}: {e}", page + 1))?;
        }
        // Written into the document now, so the layer must stop drawing them —
        // otherwise every mark appears twice.
        for page in pages {
            self.markup.forget(*page);
        }
        Ok(())
    }

    /// Whether a lock covers the whole of a page.
    ///
    /// Cheap to ask — the lock is cached — but asked only where a person is
    /// about to change something, never while drawing.
    fn page_is_locked(&self, page: usize) -> bool {
        let Some(doc) = &self.doc else { return false };
        let items = doc.session.locked_items_on(page);
        let Some((width, height)) = doc.strip.size_of(page) else { return false };
        items.iter().any(|item| {
            item.rect.left <= 0.5
                && item.rect.top <= 0.5
                && item.rect.right >= width - 0.5
                && item.rect.bottom >= height - 0.5
        })
    }

    /// Put a password on the file, applied when it is next saved.
    /// Put Pagify's own password on the file, which nothing else can read.
    fn secure_document_plus(&mut self, password: &[u8]) -> Result<String, String> {
        let doc = self.doc.as_mut().ok_or_else(|| "nothing open.".to_string())?;
        doc.session.secure_document_plus(password).map_err(|e| e.to_string())?;
        Ok("Secure Plus password set — it is written when you save. \
            No other reader will open the file afterwards."
            .to_string())
    }

    fn secure_document(
        &mut self,
        password: &[u8],
        options: pagify_shell::verbs::SecureOptions,
    ) -> Result<String, String> {
        // The permission bits are written from the other direction — a set bit
        // *permits* — which is why this reads as a chain of allowances rather
        // than of prohibitions.
        let permissions = pdf_core::pdf::encrypt::Permissions::all()
            .allow_printing(options.printing)
            .allow_copying(options.copying)
            .allow_editing(options.editing)
            .allow_annotating(options.annotating);

        let doc = self.doc.as_mut().ok_or_else(|| "nothing open.".to_string())?;
        doc.session
            .secure_document(password, None, permissions)
            .map_err(|e| e.to_string())?;

        // What the permissions actually buy is worth saying plainly. They are
        // honoured by readers that choose to; the password is what withholds
        // the document.
        let note = if options == pagify_shell::verbs::SecureOptions::default() {
            String::new()
        } else {
            format!(
                " Readers are asked to allow {} — a courtesy most honour, not a lock.",
                options.describe()
            )
        };
        Ok(format!(
            "password set — it is written when you save.{note} \
             Save somewhere new if you want to keep an unsecured copy."
        ))
    }

    fn lock_pages(&mut self, pages: &[usize], passcode: &[u8]) -> Result<String, String> {
        if passcode.is_empty() {
            return Err("a lock needs a passcode.".into());
        }
        self.commit_marks_on(pages)?;
        let Some(doc) = &self.doc else { return Err("nothing open.".into()) };
        match doc.session.lock_pages(pages, passcode) {
            Ok(locked) => {
                if let Some(doc) = &mut self.doc {
                    doc.rendered_is_stale();
                }
                self.text = None;
                self.text_selection = None;
                self.find_hits.clear();
                // Says how many were *newly* locked, which can be fewer than
                // were asked for: a page already locked keeps the way back it
                // has rather than being sealed again over its blank self.
                let already = pages.len() - locked;
                let mut said = format!(
                    "locked {locked} page{} — `unlock` and the passcode bring them back.",
                    if locked == 1 { "" } else { "s" }
                );
                if already > 0 {
                    said.push_str(&format!(" {already} were already locked."));
                }
                Ok(said)
            }
            Err(e) => Err(format!("lock: {e}")),
        }
    }

    /// The images on a page, or none if they cannot be read.
    ///
    /// Read fresh rather than cached: locking one rewrites the page, and a
    /// cached list would go on offering an image that is no longer there.
    fn images_on(&self, page: usize) -> Vec<pdf_core::document::PageImage> {
        self.doc
            .as_ref()
            .and_then(|d| d.session.images_on(page).ok())
            .unwrap_or_default()
    }

    /// What is sealed on a page, for drawing its padlocks.
    fn locked_items_on(&self, page: usize) -> Vec<pdf_core::document::LockedItem> {
        self.doc.as_ref().map(|d| d.session.locked_items_on(page)).unwrap_or_default()
    }

    /// Lock the selected text, from the right-click menu.
    ///
    /// Goes through `lock_area` over the selection's own bounds — the words
    /// come off the page and the page as it was is sealed. **Not an item seal
    /// like an image gets**: a run's font is a document resource, and this
    /// module's `crypto::vault` explains at length why a text run restored on
    /// its own cannot be trusted to come back in the typeface it left in.
    fn lock_selection(&mut self) {
        let page = self.selection_page;
        let Some(range) = self.text_selection.clone() else {
            self.say_error("nothing selected.");
            return;
        };
        let Some(chars) = self.characters(page) else {
            self.say_error("nothing selected.");
            return;
        };
        let boxes = chars.line_rects(range);
        let Some(first) = boxes.first() else {
            self.say_error("nothing selected.");
            return;
        };
        // The union of the selected lines, which is what a reader means by "the
        // part I highlighted" even when it spans several of them.
        let area = boxes.iter().fold(
            pdf_core::document::Rect {
                left: first.left,
                top: first.top,
                right: first.right,
                bottom: first.bottom,
            },
            |acc, r| pdf_core::document::Rect {
                left: acc.left.min(r.left),
                top: acc.top.min(r.top),
                right: acc.right.max(r.right),
                bottom: acc.bottom.max(r.bottom),
            },
        );

        // `require_complete: false` — the selection is the words, and an
        // image beneath them was never part of it.
        self.awaiting_password =
            Some(Awaiting::Lock { page, area, require_complete: false });
        self.say_info("type a passcode to lock the selection with, or Escape to give up.");
    }

    /// Take one image off the page, sealed under a passcode.
    fn lock_image(&mut self, page: usize, object: usize, passcode: &[u8]) -> Result<String, String> {
        if passcode.is_empty() {
            return Err("a lock needs a passcode.".into());
        }
        let Some(doc) = &self.doc else { return Err("nothing open.".into()) };
        match doc.session.lock_image(page, object, passcode) {
            Ok(_id) => {
                self.after_locked_items_changed();
                Ok("image locked — click the padlock on the page to bring it back.".into())
            }
            Err(e) => Err(format!("lock: {e}")),
        }
    }

    /// Bring one sealed object back, leaving the rest sealed.
    fn unlock_item(&mut self, id: &str, passcode: &[u8]) -> Result<String, String> {
        let Some(doc) = &self.doc else { return Err("nothing open.".into()) };
        match doc.session.unlock_item(id, passcode) {
            Ok(()) => {
                self.after_locked_items_changed();
                Ok("unlocked.".into())
            }
            Err(e) => Err(format!("unlock: {e}")),
        }
    }

    /// What every lock and unlock of an item has to invalidate.
    ///
    /// The page has been rewritten, so the rendered tiles and anything holding
    /// its text are stale — and the badges themselves are re-read rather than
    /// cached, so nothing else has to be told.
    fn after_locked_items_changed(&mut self) {
        if let Some(doc) = &mut self.doc {
            doc.rendered_is_stale();
        }
        self.text = None;
        self.text_selection = None;
        self.find_hits.clear();
        self.selected_image = None;
    }

    /// Bring back everything the passcode has sealed.
    ///
    /// Decryption happens here; the pages go back **through the command stack**,
    /// so the restore undoes like any other edit and no passcode goes near it.
    fn unlock(&mut self, passcode: &[u8]) -> Result<String, String> {
        let Some(doc) = &self.doc else { return Err("nothing open.".into()) };
        // Every tag is verified inside this call, before a single page is
        // handed back.
        let pages = doc.session.open_lock(passcode).map_err(|e| format!("unlock: {e}"))?;
        if pages.is_empty() {
            return Ok("there was nothing locked.".into());
        }

        let restored = pages.len();
        for (index, pdf) in pages {
            doc.session
                .execute(pdf_core::command::Command::ReplacePage { index, pdf })
                .map_err(|e| format!("unlock: page {} could not be put back: {e}", index + 1))?;
        }

        if let Some(doc) = &mut self.doc {
            doc.rendered_is_stale();
        }
        self.text = None;
        self.text_selection = None;
        self.find_hits.clear();
        Ok(format!(
            "unlocked {restored} page{}.",
            if restored == 1 { "" } else { "s" }
        ))
    }

    /// Survey the area, then either destroy it or ask.
    ///
    /// **The survey runs first, always.** Where nothing is in the way this
    /// applies at once; where something is, the report goes into a dialog rather
    /// than into an error, and nothing has been touched in the meantime.
    fn redact(&mut self, page: usize, a: AppPoint, b: AppPoint) -> Result<String, String> {
        let Some(area) = area_between(a, b) else {
            return Err("redact: that area has no size.".into());
        };
        let Some(doc) = &self.doc else { return Err("nothing open.".into()) };

        // The same candidate faces `apply_redaction` below will actually use —
        // the preview has to agree with what applying would do, or it could
        // promise a clean redaction the apply step then cannot deliver.
        let fonts = self.outlined_font_bytes();
        let font_refs: Vec<&[u8]> = fonts.iter().map(|f| f.as_slice()).collect();
        let report = doc
            .session
            .preview_redaction(page, area, &font_refs)
            .map_err(|e| format!("redact: {e}"))?;

        if !report.blockers().is_empty() {
            self.asking_to_redact = Some(PendingRedaction { page, area, report });
            // Not an error — the question is on screen.
            return Ok(String::new());
        }
        self.apply_redaction(page, area, false)
    }

    /// Run the redaction for real, through the command stack so it undoes.
    fn apply_redaction(
        &mut self,
        page: usize,
        area: pdf_core::document::Rect,
        allow_incomplete: bool,
    ) -> Result<String, String> {
        let Some(doc) = &self.doc else { return Err("nothing open.".into()) };
        match doc.session.execute(pdf_core::command::Command::Redact {
            page_index: page,
            area,
            fill: Some(pdf_core::document::Color { r: 0, g: 0, b: 0, a: 255 }),
            allow_incomplete,
            // Copied into the command so redo re-executes against the same
            // faces later, not against whatever this build happens to bundle
            // — or a reader happens to have added — by then.
            outlined_fonts: self.outlined_font_bytes(),
        }) {
            Ok(_) => {
                if let Some(doc) = &mut self.doc {
                    doc.rendered_is_stale();
                }
                // The words are gone, so anything holding on to them is stale.
                self.text = None;
                self.text_selection = None;
                self.find_hits.clear();
                Ok(format!(
                    "redacted an area of page {} — saving will rewrite the whole file.",
                    page + 1
                ))
            }
            Err(e) => Err(format!("redact: {e}")),
        }
    }

    /// Put what could not be cleared to the one person who can judge it.
    ///
    /// The wording matters more than usual here. "Some content could not be
    /// removed" is true of every case and useful in none; what the person
    /// deciding needs is whether their *words* came out, and whether the thing
    /// left behind might be words itself.
    fn ask_about_redaction(&mut self, ctx: &egui::Context) {
        let Some(asking) = self.asking_to_redact.clone() else { return };
        let mut decision: Option<bool> = None;

        egui::Modal::new(egui::Id::new("redact")).show(ctx, |ui| {
            ui.set_width(440.0);
            ui.heading("Part of this area cannot be destroyed");
            ui.add_space(6.0);

            if asking.report.characters > 0 {
                ui.label(format!(
                    "{} character{} of text will be removed permanently.",
                    asking.report.characters,
                    if asking.report.characters == 1 { "" } else { "s" },
                ));
                ui.add_space(6.0);
            }

            ui.label("These will still be in the file afterwards:");
            ui.add_space(4.0);
            for item in asking.report.blockers() {
                ui.label(format!("  •  {}", item.describe()));
            }

            if asking
                .report
                .uncleared
                .iter()
                .any(|u| matches!(u, pdf_core::document::Uncleared::Image { may_hold_text: true, .. }))
            {
                ui.add_space(8.0);
                // The case an acknowledgement must not read as routine.
                ui.colored_label(
                    egui::Color32::from_rgb(200, 80, 40),
                    "This page mixes text with scanned content. The image may contain \
                     words of its own, which will not be removed.",
                );
            }

            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.button("Redact the text anyway").clicked() {
                    decision = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    decision = Some(false);
                }
            });
        });

        // Escape cancels, as it does everywhere else in this program.
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            decision = Some(false);
        }

        match decision {
            None => {}
            Some(false) => {
                self.asking_to_redact = None;
                self.say_info("nothing was redacted.");
            }
            Some(true) => {
                self.asking_to_redact = None;
                match self.apply_redaction(asking.page, asking.area, true) {
                    Ok(said) => self.say_info(said),
                    Err(e) => self.say_error(e),
                }
            }
        }
    }

    fn arm(&mut self, kind: PendingKind, page: usize) {
        let pending = Pending { kind, page, objects: Vec::new(), points: Vec::new() };
        self.say_info(pending.prompt());
        self.pending = Some(pending);
    }

    fn go_to(&mut self, target: PageTarget) {
        let Some(doc) = &self.doc else {
            self.say_error("nothing open.");
            return;
        };
        let last = doc.page_count.saturating_sub(1);
        let wanted = match target {
            PageTarget::Number(n) => n - 1,
            PageTarget::Next => self.page.saturating_add(1),
            PageTarget::Previous => self.page.saturating_sub(1),
            PageTarget::First => 0,
            PageTarget::Last => last,
        };
        if wanted > last {
            self.say_error(format!("there are only {} pages.", doc.page_count));
            return;
        }
        self.page = wanted;
        self.scroll_pt = doc.strip.scroll_to(wanted);
        self.scroll_to_pt = Some(self.scroll_pt);
        self.settling = 3;
    }

    fn set_zoom(&mut self, target: ZoomTarget) {
        let current = self.resolved_zoom();
        self.zoom = match target {
            ZoomTarget::Factor(f) => ZoomMode::Factor(f),
            ZoomTarget::In => ZoomMode::Factor((current * 1.25).min(16.0)),
            ZoomTarget::Out => ZoomMode::Factor((current / 1.25).max(0.05)),
            ZoomTarget::Actual => ZoomMode::Factor(1.0),
            ZoomTarget::Fit => ZoomMode::Fit,
            ZoomTarget::Width => ZoomMode::Width,
        };
    }

    fn rotate_view(&mut self, degrees: i32) {
        let quarters = |r: Rotation| match r {
            Rotation::None => 0,
            Rotation::Clockwise90 => 1,
            Rotation::Clockwise180 => 2,
            Rotation::Clockwise270 => 3,
        };
        let turned = (quarters(self.rotation) + degrees.rem_euclid(360) / 90) % 4;
        self.rotation = match turned {
            0 => Rotation::None,
            1 => Rotation::Clockwise90,
            2 => Rotation::Clockwise180,
            _ => Rotation::Clockwise270,
        };
        if let Some(doc) = &mut self.doc {
            doc.rendered_is_stale();
        }
    }

    fn undo_redo(&mut self, undo: bool) {
        // The markup layer first. It is where almost every edit happens, and
        // answering "nothing to undo" while three freshly drawn lines sit on
        // the page is worse than not offering undo at all — it is an app
        // telling the user something they can see is untrue.
        let page = self.page;
        if let Some(layer) = self.markup.existing_mut(page) {
            let stepped = if undo { layer.undo() } else { layer.redo() };
            if let Some(what) = stepped {
                self.say_info(format!(
                    "{} {what}.",
                    if undo { "undid" } else { "redid" }
                ));
                return;
            }
        }

        let Some(doc) = &self.doc else {
            self.say_error("nothing open.");
            return;
        };
        let outcome = if undo { doc.session.undo() } else { doc.session.redo() };
        match outcome {
            Ok((true, state)) => {
                let label = if undo { state.redo_label.clone() } else { state.undo_label.clone() };
                self.say_info(label.unwrap_or_else(|| "done.".into()));
                if let Some(doc) = &mut self.doc {
                    doc.rendered_is_stale();
                }
            }
            Ok((false, _)) => self.say_info(if undo {
                "nothing to undo — neither the marks on this page nor the document."
            } else {
                "nothing to redo."
            }),
            Err(e) => self.say_error(format!("{e}")),
        }
    }

    /// Hand a passcode to whichever lock is waiting for it.
    ///
    /// Its own method so the window and a test take one path — the alternative
    /// is a dialog nothing exercises.
    fn answer_lock_passcode(&mut self, typed: &str) {
        let Some(what) = self.awaiting_password.take() else { return };
        match what {
            Awaiting::Lock { page, area, require_complete } => {
                match self.lock_area(page, area, typed.as_bytes(), require_complete) {
                    Ok(said) => self.say_info(said),
                    Err(e) => self.say_error(e),
                }
            }
            Awaiting::LockPages(pages) => match self.lock_pages(&pages, typed.as_bytes()) {
                Ok(said) => self.say_info(said),
                Err(e) => self.say_error(e),
            },
            Awaiting::LockImage { page, object } => {
                match self.lock_image(page, object, typed.as_bytes()) {
                    Ok(said) => self.say_info(said),
                    Err(e) => self.say_error(e),
                }
            }
            Awaiting::UnlockItem(id) => match self.unlock_item(&id, typed.as_bytes()) {
                Ok(said) => self.say_info(said),
                Err(e) => self.say_error(e),
            },
            Awaiting::Unlock => match self.unlock(typed.as_bytes()) {
                Ok(said) => self.say_info(said),
                Err(e) => self.say_error(e),
            },
            // Not a lock; put it back for whoever does handle it.
            other => self.awaiting_password = Some(other),
        }
    }

    /// Whether this document already has a lock in it.
    ///
    /// Decides whether a passcode is being **chosen** — new, so it must be
    /// strong and typed twice — or **used**, which is neither. A document
    /// locked before this rule existed has to stay lockable.
    fn lock_exists(&self) -> bool {
        self.doc.as_ref().is_some_and(|d| !d.session.locked_pages().is_empty())
    }

    /// Ask for a password, whatever it is for, in one window.
    ///
    /// **Every password in this program is asked for the same way.** There were
    /// three ways at one point — a window for opening, a window for locking, and
    /// the command box for `secure` — which meant three sets of wording, three
    /// places to look, and a rule that applied to one of them. Reported from use
    /// as wanting it uniform, and rightly.
    ///
    /// The one distinction that survives is real: a password being **chosen**
    /// has to be strong and is typed twice, because there is nothing to check it
    /// against and a slip cannot be discovered later. A password being **used**
    /// gets neither, because the document already knows the answer.
    /// The words somebody keeps for writing again.
    ///
    /// Shown in full rather than as labels: these *are* the words, and a list
    /// that named them would be a list of names for things that are already
    /// their own name.
    fn draw_snippet_list(&mut self, ctx: &egui::Context) {
        let Some(mut panel) = self.snippets.take() else { return };

        let mut done = false;
        let mut chosen: Option<String> = None;
        let mut doomed: Option<String> = None;
        let mut add = false;
        let kept: Vec<String> = self.predefined.entries().to_vec();

        egui::Modal::new(egui::Id::new("snippet-list")).show(ctx, |ui| {
            ui.set_width(520.0);
            ui.heading("Predefined text");
            ui.add_space(6.0);
            ui.label(
                "Words kept for filling forms in, on this computer. Only what you \
                 put here is kept — nothing you type onto a page reaches this list.",
            );
            ui.add_space(12.0);

            if kept.is_empty() {
                ui.label("Nothing kept yet.");
                ui.add_space(8.0);
            }

            egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
                for entry in &kept {
                    egui::Frame::new()
                        .fill(theme::PANEL)
                        .stroke(egui::Stroke::new(1.0, theme::LINE))
                        .corner_radius(egui::CornerRadius::same(5))
                        .inner_margin(8.0)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.set_width(340.0);
                                    // Every line of it: an address is why this
                                    // exists, and showing the first line only
                                    // would hide what is about to be written.
                                    for line in entry.lines() {
                                        ui.label(line);
                                    }
                                });
                                if ui.button("Write").clicked() {
                                    chosen = Some(entry.clone());
                                }
                                if ui.button("Forget").clicked() {
                                    doomed = Some(entry.clone());
                                }
                            });
                        });
                    ui.add_space(6.0);
                }
            });

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(8.0);
            ui.label("Keep something new");
            let field = ui.add(
                egui::TextEdit::multiline(&mut panel.adding)
                    .desired_width(f32::INFINITY)
                    .desired_rows(2)
                    .hint_text("Jane Smith"),
            );
            // Enter finishes it, since a snippet is usually one line — and
            // Shift-Enter is still how a second line is typed.
            let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let something = !panel.adding.trim().is_empty();
                if ui.add_enabled(something, egui::Button::new("Keep it")).clicked() || entered {
                    add = true;
                }
                if ui.button("Done").clicked() {
                    done = true;
                }
            });
        });

        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            done = true;
        }

        if add {
            let typed = panel.adding.trim().to_string();
            if !typed.is_empty() {
                self.predefined.remember(&typed);
                match self.keep_snippets() {
                    Ok(()) => {
                        panel.adding.clear();
                        self.say_info(format!("kept \"{}\".", short(&typed)));
                    }
                    Err(e) => {
                        self.predefined.forget(&typed);
                        self.say_error(e);
                    }
                }
            }
        }
        if let Some(text) = doomed {
            if self.predefined.forget(&text) {
                match self.keep_snippets() {
                    Ok(()) => self.say_info(format!("forgot \"{}\".", short(&text))),
                    Err(e) => self.say_error(e),
                }
            }
        }
        if let Some(text) = chosen {
            // The panel closes: what comes next is a click on the page, and a
            // window over the page would be in the way of it.
            self.use_snippet(&text);
            return;
        }
        if done {
            return;
        }
        self.snippets = Some(panel);
    }

    /// The list of signatures somebody has drawn.
    ///
    /// Each is drawn rather than named, because a list of names says nothing
    /// about which scrawl is which — and it is drawn by the same code that
    /// places one on a page, so the preview is the thing itself rather than an
    /// impression of it.
    fn draw_signature_list(&mut self, ctx: &egui::Context) {
        let Some(mut panel) = self.signature_list.take() else { return };

        let mut done = false;
        let mut action: Option<ListAction> = None;
        let current = self.signatures.current().map(|s| s.name.clone());
        let entries = self.signatures.entries().to_vec();

        egui::Modal::new(egui::Id::new("signature-list")).show(ctx, |ui| {
            ui.set_width(560.0);
            ui.heading("Your signatures");
            ui.add_space(6.0);
            // Says which list this is: the drawings, not the marks on a page.
            ui.label(
                "The signatures you have drawn, kept on this computer. A signature \
                 already placed in a document is part of that document.",
            );
            ui.add_space(12.0);

            if entries.is_empty() {
                ui.label("Nothing drawn yet.");
                ui.add_space(10.0);
            }

            for entry in &entries {
                let is_current = current.as_deref() == Some(entry.name.as_str());
                egui::Frame::new()
                    .fill(if is_current { theme::RAISED } else { theme::PANEL })
                    .stroke(egui::Stroke::new(
                        1.0,
                        if is_current { theme::VIOLET } else { theme::LINE },
                    ))
                    .corner_radius(egui::CornerRadius::same(6))
                    .inner_margin(10.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let (response, painter) = ui.allocate_painter(
                                egui::vec2(220.0, 64.0),
                                egui::Sense::hover(),
                            );
                            let area = response.rect;
                            painter.rect_filled(
                                area,
                                4.0,
                                egui::Color32::from_rgb(0xFA, 0xFA, 0xFC),
                            );
                            paint_signature(&painter, area, entry);

                            ui.vertical(|ui| {
                                match &mut panel.renaming {
                                    Some((which, typed)) if which == &entry.name => {
                                        let field = ui.add(
                                            egui::TextEdit::singleline(typed)
                                                .desired_width(180.0),
                                        );
                                        let entered = field.lost_focus()
                                            && ui.input(|i| i.key_pressed(egui::Key::Enter));
                                        ui.horizontal(|ui| {
                                            if ui.button("Save").clicked() || entered {
                                                action = Some(ListAction::Rename(
                                                    entry.name.clone(),
                                                    typed.clone(),
                                                ));
                                            }
                                            if ui.button("Cancel").clicked() {
                                                action = Some(ListAction::Rename(
                                                    entry.name.clone(),
                                                    entry.name.clone(),
                                                ));
                                            }
                                        });
                                    }
                                    _ => {
                                        ui.label(egui::RichText::new(&entry.name).strong());
                                        if is_current {
                                            ui.colored_label(theme::VIOLET, "a click places this");
                                        }
                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            if ui
                                                .add_enabled(
                                                    !is_current,
                                                    egui::Button::new("Use"),
                                                )
                                                .clicked()
                                            {
                                                action = Some(ListAction::Use(entry.name.clone()));
                                            }
                                            if ui.button("Rename").clicked() {
                                                panel.renaming = Some((
                                                    entry.name.clone(),
                                                    entry.name.clone(),
                                                ));
                                            }
                                            // Twice, because it does not come
                                            // back — and the second press says
                                            // what it is about to do.
                                            if panel.doomed.as_deref() == Some(&entry.name) {
                                                if ui
                                                    .add(
                                                        egui::Button::new("Delete for good")
                                                            .fill(theme::DANGER),
                                                    )
                                                    .clicked()
                                                {
                                                    action = Some(ListAction::Forget(
                                                        entry.name.clone(),
                                                    ));
                                                }
                                            } else if ui.button("Delete").clicked() {
                                                panel.doomed = Some(entry.name.clone());
                                            }
                                        });
                                    }
                                }
                            });
                        });
                    });
                ui.add_space(8.0);
            }

            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.button("Draw a new one").clicked() {
                    action = Some(ListAction::Draw);
                }
                if ui.button("Done").clicked() {
                    done = true;
                }
            });
        });

        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            done = true;
        }

        match action {
            Some(ListAction::Use(name)) => {
                if self.signatures.choose(&name) {
                    match self.keep_signatures() {
                        Ok(()) => self.say_info(format!("`signature` now places \"{name}\".")),
                        Err(e) => self.say_error(e),
                    }
                }
            }
            Some(ListAction::Forget(name)) => {
                panel.doomed = None;
                if self.signatures.remove(&name) {
                    match self.keep_signatures() {
                        Ok(()) => self.say_info(format!(
                            "\"{name}\" is gone — a drawing is not something undo reaches."
                        )),
                        Err(e) => self.say_error(e),
                    }
                }
            }
            Some(ListAction::Rename(from, to)) => {
                panel.renaming = None;
                if !from.eq_ignore_ascii_case(&to) {
                    match self.signatures.rename(&from, &to) {
                        Ok(()) => match self.keep_signatures() {
                            Ok(()) => self.say_info(format!("\"{from}\" is now \"{to}\".")),
                            Err(e) => self.say_error(e),
                        },
                        Err(why) => self.say_error(why.describe()),
                    }
                }
            }
            Some(ListAction::Draw) => {
                self.pad = Some(SignaturePad {
                    name: format!("Signature {}", self.signatures.entries().len() + 1),
                    then_place: false,
                    ..SignaturePad::default()
                });
                return;
            }
            None => {}
        }

        if done {
            return;
        }
        self.signature_list = Some(panel);
    }

    /// The pad a signature is drawn on.
    ///
    /// A white sheet with a line across it, because that is what a person is
    /// used to signing and because the line is what the placed signature will
    /// sit on — drawn here so the shape somebody sees is the shape they get.
    fn draw_signature_pad(&mut self, ctx: &egui::Context) {
        let Some(mut pad) = self.pad.take() else { return };

        let mut keep = false;
        let mut gave_up = false;
        egui::Modal::new(egui::Id::new("signature-pad")).show(ctx, |ui| {
            ui.set_width(560.0);
            ui.heading("Draw your signature");
            ui.add_space(6.0);
            // Said where it cannot be missed, not in a manual. Somebody who
            // believes this is the cryptographic kind is worse off than
            // somebody with no signature at all.
            ui.label(
                "Drag to write. This is ink — it shows a name, it does not prove \
                 one, and `certify` is what signs with a certificate.",
            );
            ui.add_space(10.0);

            let (response, painter) =
                ui.allocate_painter(egui::vec2(520.0, 200.0), egui::Sense::drag());
            let area = response.rect;
            painter.rect_filled(area, 6.0, egui::Color32::from_rgb(0xFA, 0xFA, 0xFC));
            // The line, a third up from the bottom — where a signature sits on
            // a form.
            let line = area.bottom() - area.height() * 0.28;
            painter.line_segment(
                [
                    egui::pos2(area.left() + 24.0, line),
                    egui::pos2(area.right() - 24.0, line),
                ],
                egui::Stroke::new(1.0, egui::Color32::from_rgb(0xC8, 0xC8, 0xD4)),
            );

            if response.drag_started() {
                pad.strokes.push(Vec::new());
            }
            if let Some(at) = response.interact_pointer_pos() {
                if let Some(stroke) = pad.strokes.last_mut() {
                    let point = (at.x - area.left(), at.y - area.top());
                    // Repeated points are the pointer resting, not writing —
                    // and thousands of them would be stored as the signature.
                    if stroke.last().is_none_or(|last: &(f32, f32)| {
                        (last.0 - point.0).hypot(last.1 - point.1) > 0.75
                    }) {
                        stroke.push(point);
                    }
                }
            }

            let ink = egui::Color32::from_rgb(0x14, 0x2B, 0x63);
            for stroke in &pad.strokes {
                if stroke.len() < 2 {
                    continue;
                }
                let points: Vec<egui::Pos2> = stroke
                    .iter()
                    .map(|(x, y)| egui::pos2(area.left() + x, area.top() + y))
                    .collect();
                painter.add(egui::Shape::line(points, egui::Stroke::new(2.2, ink)));
            }

            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.label("Called");
                ui.add(
                    egui::TextEdit::singleline(&mut pad.name)
                        .desired_width(200.0)
                        .hint_text("Signature"),
                );
            });

            ui.add_space(12.0);
            ui.horizontal(|ui| {
                // Disabled rather than hidden while there is nothing to keep,
                // so the button that will work is the one in the same place.
                let drawn = pad.strokes.iter().any(|s| s.len() >= 2);
                if ui.add_enabled(drawn, egui::Button::new("Keep it")).clicked() {
                    keep = true;
                }
                if ui.button("Clear").clicked() {
                    pad.strokes.clear();
                }
                if ui.button("Cancel").clicked() {
                    gave_up = true;
                }
            });
        });

        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            gave_up = true;
        }

        if keep {
            let (name, strokes) = (pad.name.clone(), pad.strokes.clone());
            match self.save_drawn_signature(&name, &strokes) {
                Ok(said) => {
                    self.say_info(said);
                    // Straight on to the click, for somebody who reached for
                    // the tool wanting to sign rather than to draw.
                    if pad.then_place && self.doc.is_some() {
                        let page = self.page;
                        self.arm(PendingKind::Signature, page);
                    }
                }
                Err(e) => {
                    self.say_error(e);
                    // Kept open with the strokes intact: throwing away what
                    // somebody drew because it was too small is the wrong way
                    // round.
                    self.pad = Some(pad);
                }
            }
            return;
        }
        if gave_up {
            self.say_info("nothing was kept.");
            return;
        }
        self.pad = Some(pad);
    }

    fn draw_passcode_dialog(&mut self, ctx: &egui::Context) {
        use pagify_shell::passphrase;

        let Some(waiting) = self.awaiting_password.clone() else { return };

        // What is being asked, in three questions.
        let confirming =
            matches!(waiting, Awaiting::LockAgain { .. } | Awaiting::SecureAgain { .. });
        let using = matches!(
            waiting,
            Awaiting::Open(_) | Awaiting::Unlock | Awaiting::UnlockItem(_)
        ) || matches!(waiting, Awaiting::Lock { .. } | Awaiting::LockPages(_) | Awaiting::LockImage { .. })
            && self.lock_exists();
        let choosing = !using && !confirming
            || matches!(waiting, Awaiting::LockAgain { .. } | Awaiting::SecureAgain { .. });

        // A rule applies only where one is being chosen.
        let ruled = !using;

        let heading = match &waiting {
            Awaiting::Open(_) => "This document needs a password".to_string(),
            Awaiting::Unlock | Awaiting::UnlockItem(_) => "Unlock".to_string(),
            Awaiting::LockAgain { .. } | Awaiting::SecureAgain { .. } => {
                "Type it again".to_string()
            }
            Awaiting::Certificate(_) => "Sign this document".to_string(),
            Awaiting::SecureCurrent(_) => "Change this document's password".to_string(),
            Awaiting::Secure(_) => "Choose a password for this file".to_string(),
            _ if using => "Unlock to add to this document".to_string(),
            _ => "Choose a passcode for this document".to_string(),
        };
        let explains = match &waiting {
            Awaiting::Open(path) => std::path::Path::new(path)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.clone()),
            Awaiting::Unlock | Awaiting::UnlockItem(_) => {
                "The passcode this document was locked with.".into()
            }
            Awaiting::LockAgain { .. } | Awaiting::SecureAgain { .. } => {
                "The same one, so a slip cannot lock you out.".into()
            }
            Awaiting::Certificate(path) => format!(
                "The password on {}. The signature covers the file as it is now.",
                std::path::Path::new(path)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string())
            ),
            Awaiting::SecureCurrent(_) => {
                "Type the password it has now. The new one comes next.".into()
            }
            Awaiting::Secure(_) => {
                "It encrypts the whole file. Nobody can open it without this — \
                 in Pagify or anywhere else."
                    .into()
            }
            _ if using => "The passcode this document is already locked with.".into(),
            _ => "It locks the text and the pictures alike, and it is the only way \
                  back to what is hidden."
                .into(),
        };
        let act = match &waiting {
            Awaiting::Open(_) => "Open",
            Awaiting::Unlock | Awaiting::UnlockItem(_) => "Unlock document",
            Awaiting::Certificate(_) => "Sign",
            Awaiting::SecureCurrent(_) => "Continue",
            Awaiting::Secure(_) | Awaiting::SecureAgain { .. } => "Set password",
            _ => "Lock document",
        };

        let mut submitted = false;
        let mut gave_up = false;
        egui::Modal::new(egui::Id::new("passcode")).show(ctx, |ui| {
            ui.set_width(440.0);
            ui.heading(&heading);
            ui.add_space(6.0);
            ui.label(&explains);
            ui.add_space(10.0);

            let field = ui.add(
                egui::TextEdit::singleline(&mut self.password_typed)
                    .password(true)
                    .desired_width(f32::INFINITY)
                    .hint_text("password"),
            );
            if !self.password_field_focused {
                field.request_focus();
                self.password_field_focused = true;
            }
            if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                submitted = true;
            }

            // What is still needed, ticked off as it arrives — shown while one
            // is being chosen, which is the only time it can be acted on.
            if ruled && !confirming {
                ui.add_space(8.0);
                let missing = passphrase::unmet(&self.password_typed);
                let here = self.password_typed.chars().count();
                for (wanted, said) in [
                    (
                        passphrase::Unmet::TooShort { need: passphrase::LEAST, have: here },
                        format!("{} characters or more", passphrase::LEAST),
                    ),
                    (passphrase::Unmet::NoUppercase, "an upper-case letter".into()),
                    (passphrase::Unmet::NoLowercase, "a lower-case letter".into()),
                    (passphrase::Unmet::NoDigit, "a number".into()),
                    (passphrase::Unmet::NoSymbol, "a symbol".into()),
                ] {
                    let met = !missing
                        .iter()
                        .any(|m| std::mem::discriminant(m) == std::mem::discriminant(&wanted));
                    ui.horizontal(|ui| {
                        ui.colored_label(
                            if met { theme::SNAP } else { theme::INK_FAINT },
                            if met { "\u{2713}" } else { "\u{2022}" },
                        );
                        ui.colored_label(if met { theme::INK } else { theme::INK_DIM }, said);
                    });
                }
            }

            // Which handler writes it. Offered only where a password is being
            // chosen for the *file* — locking has no such choice, and a
            // password being used has already been decided.
            if matches!(waiting, Awaiting::Secure(_)) {
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.password_plus, false, "Secure");
                    ui.selectable_value(&mut self.password_plus, true, "Secure Plus");
                });
                ui.add_space(4.0);
                if self.password_plus {
                    // Said before it is chosen, not discovered afterwards.
                    ui.colored_label(
                        theme::DANGER,
                        "Nothing else will open this file. Not Preview, not Acrobat, \
                         not a browser — only Pagify, with this password. There is no \
                         way back without it.",
                    );
                } else {
                    ui.colored_label(
                        theme::INK_DIM,
                        "Standard PDF encryption. Any reader will ask for this password.",
                    );
                }
            }

            if let Some(said) = &self.password_problem {
                ui.add_space(6.0);
                ui.colored_label(theme::DANGER, said);
            }

            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let ready = !self.password_typed.is_empty()
                    && (!ruled || confirming || passphrase::is_strong_enough(&self.password_typed));
                if ui.add_enabled(ready, egui::Button::new(act)).clicked() {
                    submitted = true;
                }
                if ui.button("Cancel").clicked() {
                    gave_up = true;
                }
            });
        });

        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            gave_up = true;
        }
        if gave_up {
            self.awaiting_password = None;
            self.password_typed.clear();
            self.password_problem = None;
            self.password_field_focused = false;
            self.say_info(match waiting {
                Awaiting::Open(_) => "left it unopened.",
                Awaiting::Unlock | Awaiting::UnlockItem(_) => "nothing was unlocked.",
                Awaiting::Secure(_)
                | Awaiting::SecureAgain { .. }
                | Awaiting::SecureCurrent(_) => "no password was set.",
                _ => "nothing was locked.",
            });
            return;
        }
        if !submitted || self.password_typed.is_empty() {
            return;
        }

        let typed = std::mem::take(&mut self.password_typed);
        let _ = choosing;
        self.answer_passcode(&typed);
    }

    /// Act on a password somebody has typed, whatever it was asked for.
    ///
    /// **The decision, separated from the window.** Whether a password is
    /// strong enough, whether it needs typing again, and what it finally does
    /// are all decided here — so a test can exercise them without rendering,
    /// and the window cannot drift from what those tests check.
    fn answer_passcode(&mut self, typed: &str) {
        use pagify_shell::passphrase;

        let Some(waiting) = self.awaiting_password.clone() else { return };
        self.password_field_focused = false;
        self.password_problem = None;

        let confirming =
            matches!(waiting, Awaiting::LockAgain { .. } | Awaiting::SecureAgain { .. });
        // A password is *used* when the document already knows the answer:
        // opening, unlocking, or adding to a document that is already locked.
        let using = matches!(
            waiting,
            Awaiting::Open(_)
                | Awaiting::Unlock
                | Awaiting::UnlockItem(_)
                | Awaiting::SecureCurrent(_)
                | Awaiting::Certificate(_)
        ) || (matches!(
            waiting,
            Awaiting::Lock { .. } | Awaiting::LockPages(_) | Awaiting::LockImage { .. }
        ) && self.lock_exists());

        // A password being chosen goes round once more before it does anything.
        if !using && !confirming {
            if let Some(problem) = passphrase::problem(typed) {
                self.password_problem = Some(problem);
                return;
            }
            let Some(then) = self.awaiting_password.take() else { return };
            self.awaiting_password = Some(match then {
                Awaiting::Secure(options) => {
                    Awaiting::SecureAgain { first: typed.to_string(), options }
                }
                other => Awaiting::LockAgain { first: typed.to_string(), then: Box::new(other) },
            });
            return;
        }

        match self.awaiting_password.clone() {
            Some(Awaiting::LockAgain { first, then }) => {
                if typed != first {
                    self.awaiting_password = None;
                    self.say_error("those did not match — nothing was locked. Try again.");
                    return;
                }
                self.awaiting_password = Some(*then);
                self.answer_lock_passcode(typed);
            }
            Some(Awaiting::SecureAgain { first, options }) => {
                if typed != first {
                    self.awaiting_password = None;
                    self.say_error("those did not match — nothing was set. Run `secure` again.");
                    return;
                }
                self.awaiting_password = None;
                let outcome = if self.password_plus {
                    self.secure_document_plus(typed.as_bytes())
                } else {
                    self.secure_document(typed.as_bytes(), options)
                };
                match outcome {
                    Ok(said) => self.say_info(said),
                    Err(e) => self.say_error(e),
                }
            }
            Some(Awaiting::SecureCurrent(options)) => {
                if !self.doc.as_ref().is_some_and(|d| d.session.password_matches(typed.as_bytes()))
                {
                    self.password_problem = Some("That is not this document's password.".into());
                    return;
                }
                // Right, so the old one comes off and a new one is chosen.
                if let Some(doc) = &self.doc {
                    if let Err(e) = doc.session.unsecure_document() {
                        self.awaiting_password = None;
                        self.say_error(e.to_string());
                        return;
                    }
                }
                self.awaiting_password = Some(Awaiting::Secure(options));
            }
            Some(Awaiting::Certificate(path)) => {
                self.awaiting_password = None;
                match self.sign_with(&path, typed) {
                    Ok(said) => self.say_info(said),
                    Err(e) => self.say_error(e),
                }
            }
            Some(Awaiting::Open(path)) => self.answer_open_password(&path, typed),
            Some(_) => self.answer_lock_passcode(typed),
            None => {}
        }
    }

    /// Try a password against the document that is waiting for one.
    ///
    /// Its own method so that the window and a test take the same path — the
    /// alternative is a dialog nothing exercises, which is how the drawn-ink
    /// bug earlier in this program survived a test suite that passed.
    fn answer_open_password(&mut self, path: &str, typed: &str) {
        self.awaiting_password = None;
        self.password_field_focused = false;
        self.open_with(path, Some(typed));
        // Still asking means it was not accepted, and the window says so rather
        // than the status line underneath it.
        self.password_problem = matches!(self.awaiting_password, Some(Awaiting::Open(_)))
            .then(|| "That password was not accepted.".to_string());
    }

    /// The path a document is waiting on a password for, if any.
    fn awaiting_open(&self) -> Option<String> {
        match &self.awaiting_password {
            Some(Awaiting::Open(path)) => Some(path.clone()),
            _ => None,
        }
    }

    fn save(&mut self, dest: Option<PathBuf>) {
        let Some(doc) = &self.doc else {
            self.say_error("nothing open.");
            return;
        };
        let over_the_original = dest.is_none();
        let path = dest.unwrap_or_else(|| doc.session.path().to_path_buf());

        // **A password is never written over the only copy.**
        //
        // Securing a document and saving it turns the file into one nobody can
        // read without the password — including the person who typed it, if
        // they mistype it or forget it. Doing that to the original, in place,
        // on a plain `save`, is not a thing to do quietly: it happened to a
        // real document during this program's own development, and there is no
        // way back from it.
        //
        // `saveas` still does it, because that is the person saying where the
        // secured copy goes.
        // **Only when the original is a plain file.**
        //
        // Reported from use: changing a password, saving and reopening left the
        // *old* password working — because this refused the save, the file was
        // never written, and the error scrolled past. A document that already
        // had a password loses nothing by being written over: it was encrypted
        // before and it is encrypted after. What this guards is the one
        // irreversible case, which is putting a first password over somebody's
        // only plain copy.
        if over_the_original
            && doc.session.is_secured()
            && !doc.session.had_password_on_open()
        {
            self.say_error(
                "this document has a password waiting — use `saveas <path>` so the \
                 unsecured original is kept. A password written over the only copy \
                 cannot be undone.",
            );
            return;
        }

        // Commit every marked page before writing: real ink for any reader,
        // plus the live geometry so it is still editable next time.
        let mut stored = 0;
        let mut skipped = Vec::new();
        for page in self.markup.marked_pages() {
            if let Some(layer) = self.markup.existing(page) {
                match doc.session.commit_markup(page, layer, MARKUP_INK, 1.5) {
                    Ok(committed) => {
                        stored += committed.objects_stored;
                        skipped.extend(committed.skipped.into_iter().map(|s| s.what));
                    }
                    Err(e) => {
                        self.say_error(format!("could not write the markup on page {}: {e}", page + 1));
                        return;
                    }
                }
            }
        }

        // Incremental, always, unless saving somewhere new: a signature covers a
        // byte range, and a full rewrite relocates every object in the file.
        //
        // **Except after a redaction**, where incremental is the one thing that
        // must not happen: it keeps the original bytes verbatim and appends a
        // delta, so every word the redaction removed is still in the file at its
        // old offset. The engine refuses it outright; this asks the same
        // question first so the answer is a rewritten file rather than an error
        // the user has to interpret.
        //
        // Nothing is lost by rewriting here. The redaction has already changed
        // the page's content stream, so any signature over this document is void
        // whichever way it is written.
        let incremental = !doc.session.must_save_full_copy();
        match doc.session.save_to(&path, incremental) {
            Ok(()) => {
                self.saved_revision = self.markup.revision();
                self.say_info(format!(
                    "saved {} ({stored} marks){}.",
                    path.display(),
                    if incremental { "" } else { " — the file was rewritten" }
                ));
                if !skipped.is_empty() {
                    self.say_error(format!(
                        "{} mark{} could not be stored and will not come back: {}",
                        skipped.len(),
                        if skipped.len() == 1 { "" } else { "s" },
                        skipped.join(", ")
                    ));
                }
            }
            Err(e) => self.say_error(format!("save failed: {e}")),
        }
    }

    // -- organize -----------------------------------------------------------

    fn with_pages<F>(&mut self, spec: &str, what: &str, mut f: F)
    where
        F: FnMut(&Session, Vec<usize>) -> Result<String, String>,
    {
        let Some(doc) = &self.doc else {
            self.say_error("nothing open.");
            return;
        };
        match pagify_shell::organize::parse_range(spec, doc.page_count) {
            Ok(pages) => match f(&doc.session, pages) {
                Ok(said) => {
                    self.say_info(said);
                    self.refresh_after_page_change();
                }
                Err(e) => self.say_error(format!("{what}: {e}")),
            },
            Err(e) => self.say_error(e),
        }
    }

    fn refresh_after_page_change(&mut self) {
        let Some(doc) = &mut self.doc else { return };
        doc.rendered_is_stale();
        if let (Ok(count), Ok(sizes)) = (doc.session.page_count(), doc.session.page_sizes()) {
            doc.page_count = count;
            // Rebuilt in the layout the reader chose. `Strip::new` is
            // single-page, and using it here would quietly put a spread back to
            // one-up every time a page was added or removed.
            doc.strip = Strip::with_layout(&sizes, PAGE_GAP_PT, doc.strip.layout());
            self.page = self.page.min(count.saturating_sub(1));
        }
    }

    fn extract(&mut self, spec: &str, dest: &std::path::Path) {
        let dest = dest.to_path_buf();
        self.with_pages(spec, "extract", move |session, pages| {
            session
                .extract_to(&pages, &dest)
                .map(|n| format!("extracted {n} page(s) to {}", dest.display()))
                .map_err(|e| e.to_string())
        });
    }

    fn delete_pages(&mut self, spec: &str) {
        self.with_pages(spec, "deletepage", |session, pages| {
            // Back to front: deleting page 2 renumbers everything after it.
            let mut removed = 0;
            for page in pages.iter().rev() {
                session
                    .execute(pdf_core::command::Command::DeletePage { index: *page })
                    .map_err(|e| e.to_string())?;
                removed += 1;
            }
            Ok(format!("{removed} page(s) deleted."))
        });
    }

    fn move_pages(&mut self, spec: &str, before: usize) {
        let Some(doc) = &self.doc else {
            self.say_error("nothing open.");
            return;
        };
        let count = doc.page_count;
        self.with_pages(spec, "movepage", move |session, pages| {
            let order = pagify_shell::organize::order_for_move(count, &pages, before - 1)?;
            session
                .execute(pdf_core::command::Command::ReorderPages { order })
                .map(|_| "moved.".to_string())
                .map_err(|e| e.to_string())
        });
    }

    /// Change the sheet size, scaling the content to match.
    fn resize_pages(&mut self, spec: &str, width_pt: f32, height_pt: f32) {
        let Some(doc) = &self.doc else {
            self.say_error("nothing open.");
            return;
        };
        let pages = match pagify_shell::organize::parse_range(spec, doc.page_count) {
            Ok(pages) => pages,
            Err(why) => {
                self.say_error(why);
                return;
            }
        };

        let mut done = 0usize;
        for page in &pages {
            if doc
                .session
                .execute(pdf_core::command::Command::SetPageSize {
                    index: *page,
                    width_pt,
                    height_pt,
                })
                .is_ok()
            {
                done += 1;
            }
        }

        if done == 0 {
            self.say_error("nothing was resized.");
            return;
        }
        self.after_page_change(format!(
            "{done} page{} resized to {width_pt:.0}×{height_pt:.0}pt. The content was scaled to \
             fit and centred.",
            if done == 1 { "" } else { "s" }
        ));
    }

    /// Trim what pages show by an inset from each edge.
    ///
    /// The crop box is a window onto the sheet, not a knife: what falls outside
    /// it stays in the file, and a wider crop brings it back. Said plainly
    /// because "crop" in most programs means *discard*, and somebody expecting
    /// that would be right to be careful.
    ///
    /// Each page is trimmed from **its own** current crop, so cropping twice
    /// trims twice — and a document of mixed page sizes keeps its proportions
    /// rather than being forced to one rectangle.
    fn crop_pages(&mut self, spec: &str, margin: f32) {
        let Some(doc) = &self.doc else {
            self.say_error("nothing open.");
            return;
        };
        let pages = match pagify_shell::organize::parse_range(spec, doc.page_count) {
            Ok(pages) => pages,
            Err(why) => {
                self.say_error(why);
                return;
            }
        };

        let mut done = 0usize;
        let mut refused = 0usize;
        for page in &pages {
            let Ok(now) = doc.session.page_crop(*page) else { continue };
            let crop = pdf_core::document::Rect {
                left: now.left.min(now.right) + margin,
                top: now.top.min(now.bottom) + margin,
                right: now.left.max(now.right) - margin,
                bottom: now.top.max(now.bottom) - margin,
            };
            match doc
                .session
                .execute(pdf_core::command::Command::SetPageCrop { index: *page, crop })
            {
                Ok(_) => done += 1,
                // A page too small to take the inset is left alone rather than
                // collapsed to nothing.
                Err(_) => refused += 1,
            }
        }

        if done == 0 {
            self.say_error("nothing was cropped — is the inset larger than the page?");
            return;
        }
        self.after_page_change(format!(
            "{done} page{} cropped by {margin:.0}pt{}. What is outside is still in the file — \
             `undo` puts it back.",
            if done == 1 { "" } else { "s" },
            if refused > 0 { format!(", {refused} too small to trim") } else { String::new() }
        ));
    }

    /// Copy pages, placing the copies straight after the last one copied.
    ///
    /// After rather than before, and after the *whole* run rather than each
    /// page after itself: duplicating 1-3 gives 1,2,3,1,2,3 — the block kept
    /// together, which is what somebody duplicating a section means. Interleaved
    /// copies would be a different operation and nobody asks for it by this
    /// name.
    fn duplicate_pages(&mut self, spec: &str) {
        let Some(doc) = &self.doc else {
            self.say_error("nothing open.");
            return;
        };
        let pages = if spec.trim().is_empty() {
            vec![self.page]
        } else {
            match pagify_shell::organize::parse_range(spec, doc.page_count) {
                Ok(pages) => pages,
                Err(why) => {
                    self.say_error(why);
                    return;
                }
            }
        };

        let after = pages.iter().copied().max().map(|p| p + 1).unwrap_or(0);
        let count = pages.len();
        match doc.session.duplicate_pages(&pages, after) {
            Ok(_) => self.after_page_change(format!(
                "{count} page{} duplicated.",
                if count == 1 { "" } else { "s" }
            )),
            Err(e) => self.say_error(format!("{e}")),
        }
    }

    /// Change words that are already on the page.
    fn edit_text(&mut self) {
        if self.doc.is_none() {
            self.say_error("nothing open.");
            return;
        }
        let page = self.page;
        self.arm(PendingKind::PickText, page);
    }

    /// The run of text under a point, offered for retyping.
    ///
    /// The run is the unit because it is what the file holds — one may be a
    /// whole paragraph, or a single letter that needed different spacing.
    /// Offering "the word you clicked" would mean rewriting the content stream
    /// around it, which is a different and much larger feature.
    fn pick_text_run(&mut self, page: usize, at: AppPoint) -> Result<String, String> {
        let Some(doc) = &self.doc else { return Err("nothing open.".into()) };
        let runs = doc.session.text_runs(page).map_err(|e| format!("{e}"))?;

        let (x, y) = (at.x as f32, at.y as f32);
        // How far outside its own box a run will still answer to a click.
        //
        // **Because the target is a few pixels tall.** Measured at the zoom
        // these documents open at: the median run is 5.1 pixels high in one and
        // 6.7 in the other. Asking somebody to land inside a five-pixel band is
        // not a reasonable thing to ask, and missing it looks exactly like the
        // tool being broken — the click is simply reported as landing on no
        // text.
        let near = HIT_TOLERANCE_PT as f32;
        let inside = |r: &pdf_core::document::TextRun, slack: f32| {
            x >= r.rect.left.min(r.rect.right) - slack
                && x <= r.rect.left.max(r.rect.right) + slack
                && y >= r.rect.top.min(r.rect.bottom) - slack
                && y <= r.rect.top.max(r.rect.bottom) + slack
        };
        let area = |r: &pdf_core::document::TextRun| {
            ((r.rect.right - r.rect.left) * (r.rect.bottom - r.rect.top)).abs()
        };

        // The smallest run containing the point. Runs overlap — a heading's box
        // can swallow a caption inside it — and the smaller one is always the
        // more specific thing to have clicked on.
        //
        // Exact containment first, so a click that lands squarely on a word
        // still picks that word and nothing else. Only a click that hit nothing
        // looks at the neighbours.
        let found = runs
            .iter()
            .filter(|r| inside(r, 0.0))
            .min_by(|a, b| area(a).total_cmp(&area(b)))
            .or_else(|| {
                runs.iter()
                    .filter(|r| inside(r, near))
                    .min_by(|a, b| {
                        // Nearest, not smallest: among runs the click merely
                        // came close to, the one whose edge it came closest to
                        // is the one that was being aimed at.
                        let gap = |r: &pdf_core::document::TextRun| {
                            let dx = (r.rect.left.min(r.rect.right) - x)
                                .max(x - r.rect.left.max(r.rect.right))
                                .max(0.0);
                            let dy = (r.rect.top.min(r.rect.bottom) - y)
                                .max(y - r.rect.top.max(r.rect.bottom))
                                .max(0.0);
                            dx.hypot(dy)
                        };
                        gap(a).total_cmp(&gap(b))
                    })
            })
            .cloned();

        let Some(run) = found else {
            // Nothing written here — but the page may *draw* words, and one of
            // those is still something a person can point at and mean.
            if let Some(word) = self.drawn_word_at(page, at) {
                return Ok(word);
            }
            // A page whose words are drawn says so, and says what would let
            // them be read — rather than "no text", which is true of the file
            // and useless to the person looking at the words.
            let drawn = self
                .doc
                .as_ref()
                .and_then(|d| d.session.classify(page).ok())
                .is_some_and(|c| {
                    matches!(
                        c.kind,
                        pdf_core::document::PageTextKind::Outlined
                            | pdf_core::document::PageTextKind::Hybrid
                    )
                });
            if drawn {
                return Err(
                    "no text there — the words here look drawn rather than written, and \
                     reading them needs the typeface they were set in. \
                     `outlinedfont add <file.ttf>` supplies it."
                        .into(),
                );
            }
            return Err("no text there — click on some words.".into());
        };

        let unreadable = run.text.trim().is_empty();
        self.editing_run = Some(EditingRun {
            page,
            object: run.object,
            original: run.text.clone(),
            rect: run.rect,
            buffer: run.text.clone(),
            // Seeded from the run, so leaving the controls alone changes
            // nothing about how it looks.
            //
            // **From the baseline, not the top of the box.** Those differ by
            // the font's ascent, and `at` means the baseline — seeding it from
            // `rect.top` moved a restyled run up by most of its own height,
            // onto the line above. See `TextRun::origin`.
            style: pdf_core::document::TextStyle {
                size: Some(run.size),
                color: Some(run.color),
                at: Some((run.origin.x, run.origin.y)),
            },
            was: pdf_core::document::TextStyle {
                size: Some(run.size),
                color: Some(run.color),
                at: Some((run.origin.x, run.origin.y)),
            },
            background: self.page_behind(page, run.rect),
            drag_by: (0.0, 0.0),
            drawn: run.color.a == 0,
            focused: false,
        });
        // The face these words are already in, for the field to be set in. Asked
        // for here and used a frame or two later — see `want_document_face`.
        self.want_document_face(page, run.object);

        // **Invisible words are an extracted layer, not the printed page.**
        //
        // `extracttext` writes what it recognises as fully transparent text
        // over the artwork, so the page can be searched and copied from. The
        // words a reader *sees* on such a page are vector outlines — type
        // converted to paths when the file was made — and nothing done to the
        // layer touches them.
        //
        // Reported from use, and it is the worst kind of failure: the edit
        // appears to work, the reported text changes, and the page looks
        // exactly as it did. Said at the moment of picking, before anything is
        // typed.
        if run.color.a == 0 {
            return Ok(
                "these words are drawn, not written — type converted to outlines. \
                 Changing them takes the drawn shapes off the page and writes real \
                 text in their place, in a face that will not match. Escape to \
                 leave them."
                    .into(),
            );
        }

        if unreadable {
            // Visible on the page, and the file cannot say what it says: the
            // font carries no `/ToUnicode`, so there is nothing to read out.
            // Offered anyway — it can still be replaced — but not pretending
            // the box is showing what is there.
            return Ok("these words are on the page but the file does not say what they \
                       are. Type a replacement, or Escape to leave them."
                .into());
        }
        Ok("edit the words on the page — Enter to keep, Escape to leave them.".into())
    }

    /// Ask for the document's own face to be used in the editor.
    ///
    /// Takes effect on the *next* frame — see `editor_face`. Returns nothing:
    /// a face that cannot be read is simply not installed, and the editor draws
    /// in the program's own font as it did before.
    fn want_document_face(&mut self, page: usize, object: usize) {
        let bytes = self
            .doc
            .as_ref()
            .and_then(|d| d.session.run_font_data(page, object).ok())
            .flatten();
        let Some(bytes) = bytes else {
            self.editor_face = None;
            self.editor_face_ready = false;
            return;
        };
        // **Checked before it is handed to the atlas builder.** A PDF may carry
        // a Type 1 program, or a subset cut in a way nothing else reads; egui
        // is not the place to find that out.
        if pdf_core::pdf::embed::metrics(&bytes).is_none() {
            self.editor_face = None;
            self.editor_face_ready = false;
            return;
        }

        let key = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            bytes.len().hash(&mut hasher);
            bytes[..bytes.len().min(4096)].hash(&mut hasher);
            hasher.finish()
        };
        if self.editor_face == Some(key) {
            return;
        }
        self.editor_face = Some(key);
        self.editor_face_ready = false;
        self.pending_face = Some(bytes);
    }

    /// What colour the page is around a run.
    ///
    /// Sampled from a band just outside the words themselves, so the ink does
    /// not vote, and taken as the commonest colour rather than the average —
    /// an average of black text on white paper is grey, which is neither.
    ///
    /// Rendered small on purpose: a quarter scale is a few tens of thousands of
    /// pixels for a whole page, and the answer is a flat colour either way.
    fn page_behind(&self, page: usize, rect: pdf_core::document::Rect) -> pdf_core::document::Color {
        const WHITE: pdf_core::document::Color =
            pdf_core::document::Color { r: 255, g: 255, b: 255, a: 255 };
        const SCALE: f32 = 0.25;

        let Some(doc) = &self.doc else { return WHITE };
        let Ok(raster) = doc.session.render_page(page, SCALE) else { return WHITE };
        let (width, height) = (raster.width as i32, raster.height as i32);

        // **Sampled inside the words' own box, not beside it.**
        //
        // A band above and below looked safer and was worse: on a page where
        // the text sits in a tinted strip over a photograph, just outside the
        // line is the photograph, and the editor painted a dark bar across a
        // pale one. Inside the box the ink is a minority of the pixels, so the
        // commonest colour there *is* the paper behind the words.
        let mut seen: std::collections::HashMap<(u8, u8, u8), usize> = Default::default();
        let (top, bottom) = (
            (rect.top.min(rect.bottom) * SCALE) as i32,
            (rect.top.max(rect.bottom) * SCALE) as i32,
        );
        let (from, to) = (
            (rect.left.min(rect.right) * SCALE) as i32,
            (rect.left.max(rect.right) * SCALE) as i32,
        );
        for y in top.max(0)..bottom.min(height - 1).max(top.max(0) + 1) {
            for x in from.max(0)..to.min(width - 1).max(from.max(0) + 1) {
                let at = ((y * width + x) * 4) as usize;
                if at + 2 < raster.pixels.len() {
                    // Quantised, so near-identical shades of one background
                    // count as the same background.
                    let key = (
                        raster.pixels[at] & 0xF8,
                        raster.pixels[at + 1] & 0xF8,
                        raster.pixels[at + 2] & 0xF8,
                    );
                    *seen.entry(key).or_default() += 1;
                }
            }
        }
        match seen.into_iter().max_by_key(|(_, count)| *count) {
            Some(((r, g, b), _)) => pdf_core::document::Color { r, g, b, a: 255 },
            None => WHITE,
        }
    }

    /// Pick a word the page draws rather than writes.
    ///
    /// Returns what to say, or `None` when the click was not on one.
    fn drawn_word_at(&mut self, page: usize, at: AppPoint) -> Option<String> {
        let near = HIT_TOLERANCE_PT as f32;
        let (x, y) = (at.x as f32, at.y as f32);
        let found = self
            .drawn_words_on(page)
            .iter()
            .find(|w| {
                x >= w.rect.left - near
                    && x <= w.rect.right + near
                    && y >= w.rect.top - near
                    && y <= w.rect.bottom + near
            })
            .cloned()?;
        let placed = found.placement()?;

        self.editing_run = Some(EditingRun {
            page,
            // No page object owns these words; they are shapes.
            object: usize::MAX,
            original: found.text.clone(),
            rect: found.rect,
            buffer: found.text.clone(),
            style: pdf_core::document::TextStyle {
                size: Some(placed.size),
                color: Some(pdf_core::document::Color { r: 20, g: 20, b: 20, a: 255 }),
                at: Some((placed.left, placed.baseline)),
            },
            was: pdf_core::document::TextStyle::default(),
            background: self.page_behind(page, found.rect),
            drag_by: (0.0, 0.0),
            focused: false,
            drawn: true,
        });
        Some(
            "these words are drawn, not written — type converted to outlines. \
             Changing them takes the drawn shapes off the page and writes real text \
             in their place, in a face that will not match. Escape to leave them."
                .into(),
        )
    }

    /// The words this page *draws*, recognised without changing it.
    ///
    /// **Deliberately not `extracttext`.** That writes a transparent text layer
    /// over the artwork so the page can be searched — and writing it re-emits
    /// the page, which puts its paths inside a form. Objects nested in a form
    /// cannot be taken off by a redaction at all, so extracting first is
    /// precisely what makes a drawn word unremovable afterwards. Measured: the
    /// same word, in the same box, came off a freshly opened document and was
    /// refused on one that had been extracted.
    ///
    /// So this asks the recogniser for the words and keeps them here, in
    /// memory. The page is not touched, and stays as removable as it was.
    fn drawn_words_on(&mut self, page: usize) -> &[pdf_core::document::RecognisedWord] {
        if self.drawn_words.as_ref().map(|(at, _)| *at) != Some(page) {
            let faces = self.outlined_font_bytes();
            let borrowed: Vec<&[u8]> = faces.iter().map(Vec::as_slice).collect();
            let found = self
                .doc
                .as_ref()
                .and_then(|d| {
                    d.session.recognise_drawn_words(page, &borrowed).ok()
                })
                .flatten()
                .unwrap_or_default();
            // **Not filtered for plausibility, deliberately.** Matching shapes
            // against a face the document was not set in produces fragments —
            // measured on a real report: 44 "words", the longest `000`. But the
            // same is true of a *correct* match on a page the recogniser splits
            // finely: `extru`, `Th`, `us` are what a good read of a real
            // fixture looks like. No statistic told the two apart.
            //
            // So the judgement is left where it can actually be made: the
            // editor opens holding the word as read, and somebody who sees
            // `000` presses Escape. Nothing changes until it is applied.
            self.drawn_words = Some((page, found));
        }
        self.drawn_words.as_ref().map(|(_, words)| words.as_slice()).unwrap_or(&[])
    }

    /// The next line submitted replaces the run that was picked.
    ///
    /// Intercepted before the command box sees it, the same way a password is:
    /// the words being typed are text, not a command, and dispatching them
    /// would answer `unknown command` to somebody's sentence.
    fn apply_edited_run(&mut self) {
        let Some(edit) = self.editing_run.take() else { return };
        let typed = edit.buffer.trim().to_string();

        if typed.is_empty() {
            self.say_info("left as it was.");
            return;
        }
        let changed_look = edit.style != edit.was;
        if typed == edit.original.trim() && !changed_look {
            self.say_info("unchanged.");
            return;
        }

        // **Words that are artwork are replaced, not edited.**
        //
        // There is no text on the page to change — see `replace_outlined_word`,
        // which takes the drawn shapes off and writes real words in their place.
        if edit.drawn {
            let at = edit.style.at.unwrap_or((edit.rect.left, edit.rect.bottom));
            let size = edit.style.size.unwrap_or(12.0);
            match self
                .replace_outlined_word(edit.page, edit.rect, at, size, &edit.original, &typed)
            {
                Ok(said) => self.say_info(said),
                Err(e) => self.say_error(e),
            }
            return;
        }

        let Some(doc) = &self.doc else { return };
        match doc.session.execute(pdf_core::command::Command::SetTextRun {
            page_index: edit.page,
            object: edit.object,
            text: typed.clone(),
            // **Nothing asked for, when nothing was changed.**
            //
            // The style here is *seeded* from the run so the controls open
            // showing what is there — a UI convenience. Sending it as an
            // instruction made every edit an instruction to restyle, and the
            // engine takes its safe path (swap the characters in the stream,
            // touch nothing else) only when the style asks for nothing. So the
            // safe path was unreachable, and every word-only edit went through
            // the one that re-emits the page.
            //
            // Reported from use twice, with a screenshot: one line of a
            // paragraph drawn over the line above it in a different font, its
            // own place left empty. This is why.
            style: if changed_look {
                edit.style
            } else {
                pdf_core::document::TextStyle::default()
            },
        }) {
            Ok(_) => {
                // Said when it happened, not discovered on a printed page: the
                // document's own font could not spell these words, so they are
                // in a face that is not their neighbours'.
                let face = self
                    .doc
                    .as_ref()
                    .and_then(|d| d.session.substituted_face());
                if let Some(doc) = &mut self.doc {
                    doc.rendered_is_stale();
                }
                self.text = None;
                self.text_selection = None;
                self.find_hits.clear();

                match face {
                    Some(face) => self.say_info(format!(
                        "changed to \"{typed}\", written in {face} — the document's own \
                         font here has only the letters it already uses, so this will \
                         not match its neighbours. `undo` puts it back."
                    )),
                    None => {
                        self.say_info(format!("changed to \"{typed}\" — `undo` puts it back."))
                    }
                }
            }
            Err(e) => self.say_error(format!("{e}")),
        }
    }

    /// The faces available to write with, registered under their own names.
    ///
    /// The same files the outline matcher reads a page *with* — a font good
    /// enough to recognise a page's letters by is the font that page was set
    /// in, and writing the replacement in anything else is why replaced words
    /// used to stand out. Registered once; the engine keeps them by name.
    fn writing_faces(&self) -> Vec<String> {
        let mut names = Vec::new();
        for bytes in self.outlined_font_bytes() {
            let Some(name) = pdf_core::pdf::embed::face_name(&bytes) else { continue };
            if !pdf_core::text::is_registered(&name)
                && pdf_core::text::register(&name, bytes).is_err()
            {
                continue;
            }
            names.push(name);
        }
        names
    }

    /// Replace a word that is *drawn* rather than written.
    ///
    /// # The page this exists for
    ///
    /// Type converted to outlines: the words are Bézier paths, indistinguishable
    /// from any other artwork, and there is no text on the page to edit.
    /// `extracttext` recognises them and lays a transparent text layer over the
    /// top so the page can be searched and copied from — but that layer is not
    /// the page. Editing it changed what could be found and left what could be
    /// *seen* exactly as it was, which is what "I deleted it and the text below
    /// still shows" means.
    ///
    /// So this does the two things that actually change the page: it takes the
    /// paths off, and it writes words in their place.
    ///
    /// **Removal first, and it must be complete.** `allow_incomplete` is false
    /// on purpose — a word half taken off, with new text written over the
    /// remains, is worse than a refusal, and a refusal costs nothing because
    /// the removal is what would have happened first anyway.
    fn replace_outlined_word(
        &mut self,
        page: usize,
        area: pdf_core::document::Rect,
        origin: (f32, f32),
        size: f32,
        was: &str,
        text: &str,
    ) -> Result<String, String> {
        use pdf_core::document::{Annotation, Color, Glyph};

        let faces = self.outlined_font_bytes();
        if faces.is_empty() {
            return Err(
                "these words are artwork, and matching them needs a font — \
                 `outlinedfont add <file.ttf>` supplies one."
                    .into(),
            );
        }
        let Some(doc) = &self.doc else { return Err("nothing open.".into()) };
        let borrowed: Vec<&[u8]> = faces.iter().map(Vec::as_slice).collect();

        // **A drawn word is often part of something bigger.**
        //
        // A heading converted to outlines is frequently one path holding every
        // letter of it — reported from use as "object 18 is text drawn as
        // curves, which this pass cannot remove". A rectangle around one word
        // merely *crosses* that path, and a crossed path stays; a contained one
        // comes off whatever its shape. So: ask what is in the way, and widen
        // to contain it.
        let mut area = area;
        let mut widened = false;
        if let Ok(report) = doc.session.preview_redaction(page, area, &borrowed) {
            for blocker in &report.uncleared {
                let pdf_core::document::Uncleared::OutlinedText { object } = blocker else {
                    continue;
                };
                let Ok(bounds) = doc.session.object_bounds(page, *object) else { continue };
                area = pdf_core::document::Rect {
                    left: area.left.min(bounds.left),
                    top: area.top.min(bounds.top),
                    right: area.right.max(bounds.right),
                    bottom: area.bottom.max(bounds.bottom),
                };
                widened = true;
            }
        }

        // But not without limit. Some pages draw everything on them as one
        // path, and widening to contain *that* would take the page with it.
        if widened {
            let size = doc.session.page_size(page).map_err(|e| e.to_string())?;
            let share = ((area.right - area.left) * (area.bottom - area.top)).abs()
                / (size.width_pt * size.height_pt).max(1.0);
            if share > 0.4 {
                return Err(
                    "these words are part of one drawn shape covering most of the page, \
                     so replacing them would mean replacing all of it. Nothing was changed."
                        .into(),
                );
            }
        }

        // Off the page, leaving the space blank rather than the black bar a
        // redaction paints: this is an edit, not a redaction, and a mark would
        // be a second answer to a question nobody asked.
        doc.session
            .execute(pdf_core::command::Command::Redact {
                page_index: page,
                area,
                fill: None,
                allow_incomplete: false,
                outlined_fonts: faces,
            })
            .map_err(|e| {
                format!("the drawn words could not be taken off the page, so nothing was changed — {e}")
            })?;

        // **The document's own face, where it can draw these letters.**
        //
        // The words being replaced were set in it — that is how they were
        // recognised at all — so writing the new ones in anything else leaves a
        // patch that reads as a repair. Falls back to Helvetica only where no
        // face offered can spell what was typed, and says which was used.
        let mut size = size.max(1.0);
        let mut baseline = origin.1;
        let mut pen_start = origin.0;
        let (face, font_asset, glyphs) = match self
            .writing_faces()
            .into_iter()
            .find(|name| pdf_core::text::covers(name, text))
        {
            Some(name) => {
                // **The size the box says, not the size the recogniser guessed.**
                //
                // Measured: replacing a word with *itself* came back a quarter
                // too small and sitting left of where it had been. The word
                // that was there occupies a known width, and the same word set
                // in the same face has a known width per point — so the ratio
                // is the point size, and it is a measurement rather than an
                // estimate. Only used where it is sane: a recogniser that read
                // the word wrongly must not resize the replacement to match its
                // mistake.
                let drawn = area.right - area.left;
                if let Ok(original) = pdf_core::text::shape(&name, was.trim()) {
                    let per_point = original.width();
                    if per_point > 0.01 && drawn > 0.5 {
                        let measured = drawn / per_point;
                        if measured > size * 0.4 && measured < size * 3.0 {
                            size = measured;
                        }
                    }
                }

                // **And the baseline the box says, for the same reason.**
                //
                // Measured: the recogniser's baseline put the replacement three
                // points above the line the word had been sitting on. The
                // bottom of a word's ink *is* its baseline unless something in
                // it descends, and how far the original word descended is a
                // fact about its letters — see `text::ink_depth`.
                if let Some(depth) = pdf_core::text::ink_depth(&name, was.trim()) {
                    baseline = area.bottom - depth * size;
                }

                // And the same horizontally: the box's left is where the ink
                // starts, not where the pen was. Every glyph has a left side
                // bearing, and starting the pen at the edge pushes the word
                // right by it — measured at two points on a real page.
                if let Some(bearing) = pdf_core::text::ink_start(&name, text) {
                    pen_start = area.left - bearing * size;
                }
                let shaped = pdf_core::text::shape(&name, text)
                    .map_err(|e| format!("{name} could not set those words — {e}"))?;
                // Where each glyph goes: the pen starts on the baseline and
                // moves by the font's own advances, which is the whole reason
                // for shaping rather than spacing by hand.
                let mut boundaries: Vec<usize> =
                    shaped.glyphs.iter().map(|g| g.cluster as usize).collect();
                boundaries.sort_unstable();
                boundaries.dedup();

                let mut pen = pen_start;
                let mut placed = Vec::with_capacity(shaped.glyphs.len());
                for glyph in &shaped.glyphs {
                    let from = (glyph.cluster as usize).min(text.len());
                    let to = boundaries
                        .iter()
                        .find(|&&b| b > from)
                        .copied()
                        .unwrap_or(text.len())
                        .min(text.len());
                    placed.push(Glyph {
                        // What this glyph stands for, so the words are
                        // searchable afterwards — a joined form has no
                        // character of its own to fall back on.
                        ch: text.get(from..to).unwrap_or_default().to_string(),
                        id: glyph.id,
                        x: pen + glyph.offset_x * size,
                        // Page space counts downwards; a glyph that hangs above
                        // the pen has a positive offset.
                        y: baseline - glyph.offset_y * size,
                        radians: 0.0,
                    });
                    pen += glyph.advance * size;
                }
                (name.clone(), Some(name), placed)
            }
            None => (
                "Helvetica".to_string(),
                None,
                vec![Glyph {
                    ch: text.to_string(),
                    id: 0,
                    x: origin.0,
                    y: baseline,
                    radians: 0.0,
                }],
            ),
        };

        let id = self.next_text_id;
        self.next_text_id += 1;
        let Some(doc) = &self.doc else { return Err("nothing open.".into()) };
        doc.session
            .execute(pdf_core::command::Command::AddAnnotation {
                page_index: page,
                annotation: Annotation::Text {
                    text: text.to_string(),
                    font: if font_asset.is_some() { face.clone() } else { "Helvetica".into() },
                    font_asset: font_asset.clone(),
                    // The size and baseline the recognised word had, so the new
                    // words sit on the line the old ones sat on.
                    size,
                    color: Color { r: 20, g: 20, b: 20, a: 255 },
                    glyphs,
                    id,
                    restore: String::new(),
                    frame: Vec::new(),
                    frame_width: 0.0,
                },
            })
            .map_err(|e| format!("the words came off but the new ones would not go on — {e}"))?;

        if let Some(doc) = &mut self.doc {
            doc.rendered_is_stale();
        }
        self.text = None;
        self.text_selection = None;
        self.find_hits.clear();
        Ok(format!(
            "replaced the drawn word with \"{text}\" on page {}, set in {face}.{} It is \
             real text now — and this took two steps, so `undo` twice puts the \
             artwork back.",
            page + 1,
            if widened {
                " The letters around it were part of the same drawn shape and came off \
                 with it."
            } else {
                ""
            }
        ))
    }

    /// Write words onto the page, where the next click lands.
    ///
    /// **Real text objects, not a picture of text.** They select, search and
    /// copy like anything else on the page — which is the whole reason the
    /// engine writes text rather than drawing letters, and the reason this is
    /// not simply an ink stroke shaped like writing.
    fn add_text(&mut self, text: String) {
        if self.doc.is_none() {
            self.say_error("nothing open.");
            return;
        }
        let text = text.trim().to_string();
        if text.is_empty() {
            self.say_error("addtext: the words to write, as in `addtext Draft`.");
            return;
        }
        let page = self.page;
        self.arm(PendingKind::Write(text), page);
    }

    /// Put words on the page at `at`.
    fn write_text_at(&mut self, page: usize, at: AppPoint, text: &str) -> Result<String, String> {
        use pdf_core::document::{Annotation, Color, Glyph};

        const SIZE: f32 = 14.0;
        let id = self.next_text_id;
        self.next_text_id += 1;
        let Some(doc) = &self.doc else { return Err("nothing open.".into()) };

        // One run, not one object per letter: a constant advance leaves a
        // readable gap after every narrow letter, and extraction reads those
        // gaps as word breaks. Handing the whole string over lets the font's own
        // metrics do the spacing.
        let glyphs = vec![Glyph {
            ch: text.to_string(),
            id: 0,
            // The baseline, not the top of the box — the click is where the
            // writing sits, and a reader points at the line, not above it.
            x: at.x as f32,
            y: at.y as f32,
            radians: 0.0,
        }];

        doc.session
            .execute(pdf_core::command::Command::AddAnnotation {
                page_index: page,
                annotation: Annotation::Text {
                    text: text.to_string(),
                    font: "Helvetica".into(),
                    font_asset: None,
                    size: SIZE,
                    color: Color { r: 20, g: 20, b: 20, a: 255 },
                    glyphs,
                    id,
                    restore: String::new(),
                    frame: Vec::new(),
                    frame_width: 0.0,
                },
            })
            .map_err(|e| format!("{e}"))?;

        Ok(format!("wrote \"{text}\" on page {}.", page + 1))
    }

    /// One page per row, or two as a spread.
    fn set_layout(&mut self, layout: pagify_shell::reader::Layout) {
        use pagify_shell::reader::Layout;
        let Some(doc) = &mut self.doc else {
            self.say_error("nothing open.");
            return;
        };
        if let Ok(sizes) = doc.session.page_sizes() {
            doc.strip = Strip::with_layout(&sizes, PAGE_GAP_PT, layout);
        }
        // The strip is a different shape, so where the window was pointing no
        // longer means the same thing. Going back to the page the reader was on
        // is the only answer that survives the change.
        let page = self.page;
        self.scroll_to_pt = doc.strip.top_of(page);
        self.settling = 3;

        self.say_info(match layout {
            Layout::Single => "one page at a time.",
            Layout::Facing => "two pages side by side.",
            Layout::FacingWithCover => "two pages side by side, the first one alone.",
        });
    }

    fn reverse_pages(&mut self) {
        let Some(doc) = &self.doc else {
            self.say_error("nothing open.");
            return;
        };
        let count = doc.page_count;
        if count < 2 {
            self.say_error("there is only one page.");
            return;
        }
        let order = pagify_shell::organize::order_for_reverse(count);
        if let Err(e) = doc.session.execute(pdf_core::command::Command::ReorderPages { order }) {
            self.say_error(format!("{e}"));
            return;
        }
        self.after_page_change(format!("{count} pages reversed."));
    }

    fn swap_pages(&mut self, a: usize, b: usize) {
        let Some(doc) = &self.doc else {
            self.say_error("nothing open.");
            return;
        };
        let order = match pagify_shell::organize::order_for_swap(doc.page_count, a, b) {
            Ok(order) => order,
            Err(why) => {
                self.say_error(why);
                return;
            }
        };
        if let Err(e) = doc.session.execute(pdf_core::command::Command::ReorderPages { order }) {
            self.say_error(format!("{e}"));
            return;
        }
        self.after_page_change(format!("pages {a} and {b} swapped."));
    }

    fn rotate_pages(&mut self, spec: &str, quarters: i32) {
        let Some(doc) = &self.doc else {
            self.say_error("nothing open.");
            return;
        };
        let pages = match pagify_shell::organize::parse_range(spec, doc.page_count) {
            Ok(pages) => pages,
            Err(why) => {
                self.say_error(why);
                return;
            }
        };

        // Each page turned from *its own* current rotation. A document can
        // arrive with pages already at different angles — a landscape drawing
        // among portrait sheets — and setting them all to one value straightens
        // some and turns others sideways.
        let mut done = 0usize;
        for page in &pages {
            let from = doc.session.page_rotation(*page).unwrap_or(0) as i32;
            // Wrapped into 0–3, for negative quarter-turns too: `-1` is a
            // quarter anticlockwise, not an error.
            let to = (((from + quarters) % 4) + 4) % 4;
            if doc
                .session
                .execute(pdf_core::command::Command::SetPageRotation {
                    index: *page,
                    quarter_turns: to as u8,
                })
                .is_ok()
            {
                done += 1;
            }
        }

        if done == 0 {
            self.say_error("nothing was rotated.");
            return;
        }
        self.after_page_change(format!(
            "{done} page{} rotated.",
            if done == 1 { "" } else { "s" }
        ));
    }

    /// Report a page operation, and rebuild what it invalidated.
    fn after_page_change(&mut self, said: String) {
        // Rasters, thumbnails, the page strip and the page count all describe a
        // document that has just changed shape. Gathered in one call because
        // forgetting one is not a crash — it is a stale thumbnail, or a
        // selection belonging to a page that has moved — and the symptom
        // appears well away from the cause.
        self.refresh_after_page_change();
        self.text = None;
        self.text_selection = None;
        self.find_hits.clear();
        self.foreign = None;
        self.say_info(said);
    }

    fn insert_page(&mut self) {
        let Some(doc) = &self.doc else {
            self.say_error("nothing open.");
            return;
        };
        let (w, h) = doc.strip.size_of(self.page).unwrap_or((612.0, 792.0));
        let at = self.page;
        let outcome = doc.session.execute(pdf_core::command::Command::InsertBlankPage {
            at,
            width_pt: w,
            height_pt: h,
            fill: None,
            ruling: 0,
        });
        match outcome {
            Ok(_) => {
                self.say_info("blank page inserted.");
                self.refresh_after_page_change();
            }
            Err(e) => self.say_error(format!("insertpage: {e}")),
        }
    }

    fn import(&mut self, source: &std::path::Path, spec: &str) {
        let Some(doc) = &self.doc else {
            self.say_error("nothing open.");
            return;
        };
        let at = self.page;
        // The source's own page count decides the range, so it is opened first.
        let pages = match Session::open(source).and_then(|s| s.page_count()) {
            Ok(count) => match pagify_shell::organize::parse_range(spec, count) {
                Ok(pages) => pages,
                Err(e) => {
                    self.say_error(e);
                    return;
                }
            },
            Err(e) => {
                self.say_error(format!("import: {e}"));
                return;
            }
        };

        match doc.session.import_from(source, &pages, at) {
            Ok(total) => {
                self.say_info(format!("imported {} page(s); {total} in all.", pages.len()));
                self.refresh_after_page_change();
            }
            Err(e) => self.say_error(format!("import: {e}")),
        }
    }

    /// The rectangles of every annotation on a page, with its listing number.
    ///
    /// Rebuilt when the page changes, not on every pointer move: reading
    /// annotations goes through PDFium and a mouse crossing a page would ask
    /// hundreds of times a second.
    fn foreign_marks(&mut self, page: usize) -> &[(usize, Vec<pdf_core::document::Rect>)] {
        if self.foreign.as_ref().map(|(p, _)| *p) != Some(page) {
            use pdf_core::document::Annotation as A;
            let marks = self
                .doc
                .as_ref()
                .and_then(|d| d.session.annotations(page).ok())
                .unwrap_or_default()
                .iter()
                .enumerate()
                .filter_map(|(n, m)| {
                    let rects = match &m.annotation {
                        A::Highlight { rects, .. }
                        | A::Underline { rects, .. }
                        | A::StrikeOut { rects, .. }
                        | A::Squiggly { rects, .. } => rects.clone(),
                        A::Note { rect, .. } => vec![*rect],
                        // Ink has no rectangles to hit-test against, and text is
                        // page content rather than an annotation.
                        A::Ink { .. } | A::Text { .. } => return None,
                    };
                    Some((n + 1, rects))
                })
                .collect();
            self.foreign = Some((page, marks));
        }
        self.foreign.as_ref().map(|(_, m)| m.as_slice()).unwrap_or(&[])
    }

    /// The annotation under a point, if any.
    fn foreign_at(&mut self, page: usize, at: AppPoint) -> Option<usize> {
        let (x, y) = (at.x as f32, at.y as f32);
        self.foreign_marks(page).iter().find_map(|(n, rects)| {
            rects
                .iter()
                .any(|r| {
                    x >= r.left.min(r.right)
                        && x <= r.left.max(r.right)
                        && y >= r.top.min(r.bottom)
                        && y <= r.top.max(r.bottom)
                })
                .then_some(*n)
        })
    }

    /// Every annotation on this page, whoever made it.
    ///
    /// Pagify's own marks come back as live geometry, because it wrote the
    /// `restore` blobs that describe them. **Everything else does not** — a
    /// highlight made in another editor has no such blob, so until now it was
    /// invisible to this program: drawn by the renderer, absent from the markup
    /// layer, and impossible to select or remove. It was on the page and not in
    /// the app.
    ///
    /// Listing them is the smallest honest first step. It cannot reshape a
    /// foreign mark — that would mean reconstructing geometry nobody recorded —
    /// but it can say what is there and take one away.
    fn list_marks(&mut self) {
        let Some(doc) = &self.doc else {
            self.say_error("nothing open.");
            return;
        };
        let page = self.page;
        let marks = match doc.session.annotations(page) {
            Ok(marks) => marks,
            Err(e) => {
                self.say_error(format!("{e}"));
                return;
            }
        };

        if marks.is_empty() {
            self.say_info(format!("no annotations on page {}.", page + 1));
            return;
        }

        self.say_info(format!(
            "{} annotation{} on page {}:",
            marks.len(),
            if marks.len() == 1 { "" } else { "s" },
            page + 1
        ));
        for (n, mark) in marks.iter().enumerate() {
            let where_ = describe_where(&mark.annotation);
            self.say_info(format!("  {}. {}{where_}", n + 1, mark.annotation.describe()));
        }
        self.say_info("`removemark <n>` takes one off.");
    }

    /// Remove an annotation by the number `marks` gave it.
    ///
    /// Addressed by position in that listing rather than by the engine's own
    /// index, because the engine skips annotations it cannot read and its
    /// indices therefore have gaps — a number the user can see is the only one
    /// they can act on.
    fn remove_mark(&mut self, n: usize) {
        let Some(doc) = &self.doc else {
            self.say_error("nothing open.");
            return;
        };
        let page = self.page;
        let marks = match doc.session.annotations(page) {
            Ok(marks) => marks,
            Err(e) => {
                self.say_error(format!("{e}"));
                return;
            }
        };
        let Some(mark) = marks.get(n - 1) else {
            self.say_error(format!(
                "there is no annotation {n} — `marks` lists {} on this page.",
                marks.len()
            ));
            return;
        };

        let what = mark.annotation.describe().to_lowercase();
        let outcome = doc.session.execute(pdf_core::command::Command::RemoveAnnotation {
            page_index: page,
            index: mark.index,
        });
        match outcome {
            Ok(_) => {
                if let Some(doc) = &mut self.doc {
                    doc.rendered_is_stale();
                }
                self.foreign = None;
                self.say_info(format!("{what} removed — `undo` puts it back."));
            }
            Err(e) => self.say_error(format!("{e}")),
        }
    }

    /// Mark the selected text — highlight, underline, strike out, squiggle.
    ///
    /// One annotation for the whole selection, however many lines it covers.
    /// A selection spanning three lines is one thing the reader made, so
    /// erasing it should be one action rather than three — which is why the
    /// engine's markup carries a *list* of rectangles.
    fn mark_selection(&mut self, kind: pagify_shell::verbs::Markup) {
        use pagify_shell::verbs::Markup;

        let Some(range) = self.text_selection.clone() else {
            // Nothing selected: pick the tool up rather than refuse. It stays
            // in hand until Escape or another tool, so a run of passages can be
            // marked without going back to the ribbon between each.
            self.markup_armed = Some(kind);
            self.say_info(format!(
                "{} — drag across the text to mark it. Escape puts it down.",
                match kind {
                    Markup::Highlight => "highlighter",
                    Markup::Underline => "underline",
                    Markup::StrikeOut => "strikeout",
                    Markup::Squiggly => "squiggly",
                }
            ));
            return;
        };
        let page = self.selection_page;

        // One rect per line covered, from the characters themselves: a single
        // box round a selection that wraps would cover the margins and the
        // words either side of it on the first and last lines.
        let rects = match self.characters(page).map(|c| c.line_rects(range)) {
            Some(rects) if !rects.is_empty() => rects,
            _ => {
                self.say_error("that selection has nothing to mark.");
                return;
            }
        };

        // A highlight sits *behind* the words and must be translucent or it
        // hides them. The line marks sit on top and are drawn solid.
        let color = match kind {
            Markup::Highlight => pdf_core::document::Color { r: 255, g: 224, b: 102, a: 128 },
            Markup::Underline => pdf_core::document::Color { r: 43, g: 110, b: 224, a: 255 },
            Markup::StrikeOut => pdf_core::document::Color { r: 214, g: 48, b: 49, a: 255 },
            Markup::Squiggly => pdf_core::document::Color { r: 214, g: 137, b: 16, a: 255 },
        };

        // The reader's rect and the engine's are the same numbers in the same
        // space, and two distinct types — deliberately, so a page-space rect
        // cannot be handed to something expecting screen pixels.
        let rects: Vec<pdf_core::document::Rect> = rects
            .into_iter()
            .map(|r| pdf_core::document::Rect {
                left: r.left,
                top: r.top,
                right: r.right,
                bottom: r.bottom,
            })
            .collect();

        let lines = rects.len();
        let annotation = match kind {
            Markup::Highlight => pdf_core::document::Annotation::Highlight { rects, color },
            Markup::Underline => pdf_core::document::Annotation::Underline { rects, color },
            Markup::StrikeOut => pdf_core::document::Annotation::StrikeOut { rects, color },
            Markup::Squiggly => pdf_core::document::Annotation::Squiggly { rects, color },
        };

        let Some(doc) = &self.doc else {
            self.say_error("nothing open.");
            return;
        };
        match doc
            .session
            .execute(pdf_core::command::Command::AddAnnotation { page_index: page, annotation })
        {
            Ok(_) => {
                if let Some(doc) = &mut self.doc {
                    doc.rendered_is_stale();
                }
self.foreign = None;
                self.say_info(format!(
                    "{} {lines} line{}.",
                    kind.name(),
                    if lines == 1 { "" } else { "s" }
                ));
            }
            Err(e) => self.say_error(format!("{e}")),
        }
    }

    fn add_note(&mut self, text: String) {
        let Some(doc) = &self.doc else {
            self.say_error("nothing open.");
            return;
        };
        let rect = pdf_core::document::Rect { left: 24.0, top: 24.0, right: 44.0, bottom: 44.0 };
        match doc.session.note(self.page, rect, text, MARKUP_INK) {
            Ok(_) => {
                self.say_info("note added at the top-left of the page.");
                if let Some(doc) = &mut self.doc {
                    doc.rendered_is_stale();
                }
            }
            Err(e) => self.say_error(format!("note: {e}")),
        }
    }

    // -- automate -----------------------------------------------------------

    fn stop_recording(&mut self) {
        match self.recorder.finish() {
            None => self.say_error("not recording."),
            Some(script) => {
                let path = PathBuf::from(format!("{}.json", script.name.replace(' ', "-")));
                match std::fs::write(&path, script.to_json()) {
                    Ok(()) => self.say_info(format!(
                        "{} step(s) written to {}",
                        script.steps.len(),
                        path.display()
                    )),
                    Err(e) => self.say_error(format!("could not write the script: {e}")),
                }
            }
        }
    }

    fn replay(&mut self, path: &std::path::Path) {
        let script = match std::fs::read_to_string(path).map_err(|e| e.to_string()).and_then(|t| Script::from_json(&t)) {
            Ok(script) => script,
            Err(e) => {
                self.say_error(format!("replay: {e}"));
                return;
            }
        };

        // Collected first, then run: `replay` itself must not be recorded into
        // whatever is recording, and the steps must not be borrowed from a
        // script this loop could replace.
        let steps = script.steps.clone();
        let mut ran = 0;
        let mut stopped = None;
        for (index, line) in steps.iter().enumerate() {
            // A replay stops at the first step that cannot run. Carrying on
            // would apply the rest of the script to a document in a state it
            // was never written for, which is how a batch run quietly ruins a
            // folder full of files.
            match pagify_shell::command::dispatch(line) {
                Some(Dispatch::Unknown(token)) => {
                    stopped = Some((index + 1, line.clone(), format!("unknown command '{token}'")));
                    break;
                }
                Some(Dispatch::Bad(why)) => {
                    stopped = Some((index + 1, line.clone(), why));
                    break;
                }
                Some(Dispatch::Refused { token, why }) => {
                    stopped = Some((index + 1, line.clone(), format!("{token} — {why}")));
                    break;
                }
                Some(dispatch) => {
                    self.run(dispatch);
                    ran += 1;
                }
                None => {}
            }
        }
        match stopped {
            None => self.say_info(format!("replayed {ran} step(s).")),
            Some((step, line, why)) => {
                self.say_error(format!("stopped at step {step} (`{line}`): {why}. {ran} ran first."))
            }
        }
    }

    // -- picks --------------------------------------------------------------

    fn take_pick(&mut self, at: AppPoint) {
        let Some(pending) = &self.pending else { return };
        let page = pending.page;

        if pending.wants_object() {
            // A generous tolerance: you are aiming at a line with a mouse, and
            // a miss here costs the whole operation.
            let hit = self
                .markup
                .existing(page)
                .and_then(|layer| layer.hit(at, HIT_TOLERANCE_PT * 3.0));

            match hit {
                Some(index) => {
                    if let Some(p) = self.pending.as_mut() {
                        p.objects.push((index, at));
                    }
                }
                None => {
                    self.say_info("nothing there — click on a mark.");
                    return;
                }
            }
        } else if let Some(p) = self.pending.as_mut() {
            p.points.push(at);
        }

        if self.pending.as_ref().is_some_and(Pending::ready) {
            self.resolve();
        } else if let Some(p) = &self.pending {
            let prompt = p.prompt();
            self.say_info(prompt);
        }
    }

    /// Carry out whatever has finished collecting its clicks.
    fn resolve(&mut self) {
        let Some(pending) = self.pending.take() else { return };
        let page = pending.page;
        let height = self
            .doc
            .as_ref()
            .and_then(|d| d.strip.size_of(page))
            .map(|(_, h)| h as f64)
            .unwrap_or(792.0);
        let repeats = pending.kind.repeats();

        let outcome: Result<String, String> = match &pending.kind {
            PendingKind::PickText => match pending.points.first().copied() {
                Some(at) => self.pick_text_run(page, at),
                None => Err("nothing was clicked.".into()),
            },
            PendingKind::Write(text) => {
                let text = text.clone();
                match pending.points.first().copied() {
                    Some(at) => self.write_text_at(page, at, &text),
                    None => Err("nowhere to write.".into()),
                }
            }
            PendingKind::Calibrate { distance, unit } => {
                match Calibration::from_two_points(pending.points[0], pending.points[1], *distance, unit) {
                    Ok(calibration) => {
                        self.calibration = calibration;
                        Ok(self.calibration.describe())
                    }
                    Err(e) => Err(e),
                }
            }
            PendingKind::Measure(MeasureKind::Distance) => Ok(measure::measure_distance(
                &self.calibration,
                pending.points[0],
                pending.points[1],
            )
            .render()),
            PendingKind::Measure(MeasureKind::Area) => {
                Ok(measure::measure_area(&self.calibration, &pending.points).render())
            }

            PendingKind::Redact => match (pending.points.first(), pending.points.get(1)) {
                (Some(a), Some(b)) => self.redact(page, *a, *b),
                _ => Err("redact: two corners are needed.".into()),
            },
            PendingKind::Whiteout => match (pending.points.first(), pending.points.get(1)) {
                (Some(a), Some(b)) => self.whiteout(page, *a, *b),
                _ => Err("whiteout: two corners are needed.".into()),
            },
            PendingKind::Fill(mark) => {
                let mark = *mark;
                match pending.points.first().copied() {
                    Some(at) => self.stamp_mark(page, mark, at),
                    None => Err("fill: nowhere was clicked.".into()),
                }
            }
            PendingKind::Signature => match pending.points.first().copied() {
                Some(at) => self.place_signature(page, at),
                None => Err("signature: nowhere was clicked.".into()),
            },
            PendingKind::SignRectangle => match (pending.points.first(), pending.points.get(1)) {
                (Some(a), Some(b)) => self.stamp_box(page, *a, *b),
                _ => Err("rectangle: two corners are needed.".into()),
            },
            PendingKind::Move { pictures_first } => {
                let pictures_first = *pictures_first;
                match (pending.points.first().copied(), pending.points.get(1).copied()) {
                    (Some(from), Some(to)) => self.move_thing(page, from, to, pictures_first),
                    _ => Err("a thing to move and somewhere to put it.".into()),
                }
            }
            PendingKind::SignLine => match (pending.points.first(), pending.points.get(1)) {
                (Some(a), Some(b)) => self.stamp_line(page, *a, *b),
                _ => Err("line: two ends are needed.".into()),
            },
            PendingKind::Lock => match (pending.points.first(), pending.points.get(1)) {
                (Some(a), Some(b)) => match area_between(*a, *b) {
                    Some(area) => {
                        // The passcode is asked for *after* the area is drawn, so
                        // it is typed once and used immediately rather than being
                        // held while the user aims.
                        self.awaiting_password =
                            Some(Awaiting::Lock { page, area, require_complete: true });
                        self.say_info("type a passcode to lock it with, or Escape to give up.");
                        Ok(String::new())
                    }
                    None => Err("lock: that area has no size.".into()),
                },
                _ => Err("lock: two corners are needed.".into()),
            },
            PendingKind::Draw(kind) => {
                let layer = self.markup.page(page, height);
                layer.begin("draw");
                let space = layer.space();
                let p: Vec<cad_kernel::Vec2> =
                    pending.points.iter().map(|q| space.to_kernel(*q)).collect();

                match kind {
                    DrawKind::Line => {
                        layer.add(cad_kernel::Geom::Line(cad_kernel::Line { a: p[0], b: p[1] }));
                        Ok("line added.".into())
                    }
                    DrawKind::Circle => {
                        let radius = (p[1] - p[0]).len();
                        if radius < 1e-6 {
                            Err("circle: that radius is zero.".into())
                        } else {
                            layer.add(cad_kernel::Geom::Circle(cad_kernel::Circle {
                                center: p[0],
                                radius,
                            }));
                            Ok("circle added.".into())
                        }
                    }
                    DrawKind::Rectangle => {
                        let (a, b) = (p[0], p[1]);
                        let corners = [
                            a,
                            cad_kernel::Vec2::new(b.x, a.y),
                            b,
                            cad_kernel::Vec2::new(a.x, b.y),
                        ];
                        layer.add(cad_kernel::Geom::Polyline(cad_kernel::Polyline {
                            vertices: corners
                                .iter()
                                .map(|v| cad_kernel::PolyVertex { pos: *v, bulge: 0.0 })
                                .collect(),
                            closed: true,
                            widths: Vec::new(),
                        }));
                        Ok("rectangle added.".into())
                    }
                    DrawKind::Polyline => {
                        if p.len() < 2 {
                            Err("polyline: needs at least two points.".into())
                        } else {
                            layer.add(cad_kernel::Geom::Polyline(cad_kernel::Polyline {
                                vertices: p
                                    .iter()
                                    .map(|v| cad_kernel::PolyVertex { pos: *v, bulge: 0.0 })
                                    .collect(),
                                closed: false,
                                widths: Vec::new(),
                            }));
                            Ok(format!("polyline of {} points added.", p.len()))
                        }
                    }
                }
            }

            PendingKind::Modify(pick) => {
                let layer = self.markup.page(page, height);
                let space = layer.space();
                let objects: Vec<(usize, cad_kernel::Vec2)> = pending
                    .objects
                    .iter()
                    .map(|(i, at)| (*i, space.to_kernel(*at)))
                    .collect();
                let points: Vec<cad_kernel::Vec2> =
                    pending.points.iter().map(|q| space.to_kernel(*q)).collect();

                tools::run(layer, pick.op, &objects, &points)
            }
        };

        // Close the checkpoint a draw opened, and drop it if the draw refused —
        // an undo step for an operation that changed nothing looks broken,
        // because nothing moves.
        if matches!(pending.kind, PendingKind::Draw(_)) {
            if let Some(layer) = self.markup.existing_mut(page) {
                layer.end();
                if outcome.is_err() {
                    layer.forget_last_step();
                }
            }
        }

        match outcome {
            Ok(said) => self.say_info(said),
            Err(problem) => self.say_error(problem),
        }

        // Back in hand, ready for the next one. Escape puts it down, and
        // choosing another tool replaces it.
        if repeats && self.editing_run.is_none() {
            self.arm(pending.kind, page);
        }
    }

    // -- rendering ----------------------------------------------------------

    /// A page raster at `scale`, capped to what the GPU will hold.
    ///
    /// The cap lives here rather than at the call sites, and that is the point:
    /// it was at one of the two call sites, so the prefetch — which warms pages
    /// nobody is looking at yet — happily asked for a 3000-pixel texture and
    /// egui panicked. A limit every caller has to remember is a limit one of
    /// them will forget.
    fn texture_for(&mut self, ctx: &egui::Context, page: usize, scale: f32) -> Option<egui::TextureHandle> {
        let scale = {
            let (w, h) = self
                .doc
                .as_ref()
                .and_then(|d| d.strip.size_of(page))
                .unwrap_or((612.0, 792.0));
            let (w, h) = (w.max(1.0), h.max(1.0));

            // Three ceilings, and the page vanishes if any is missed.
            //
            // 1. The GPU's largest texture side.
            // 2. The engine's own limits — it refuses a render wider than
            //    16,384 px or larger than 64 M pixels, and `texture_for`
            //    returns `None` on a refusal, so the page is simply **not
            //    drawn**. Zooming in far enough made the whole page disappear
            //    until you zoomed back out.
            // 3. A budget of our own, well under the engine's: 64 M pixels is a
            //    256 MB buffer for one page of a document the user is scrolling
            //    through.
            const BUDGET_PIXELS: f32 = 24.0 * 1024.0 * 1024.0;
            let max_side = (ctx.input(|i| i.max_texture_side) as f32)
                .min(pdf_core::render::bitmap::MAX_DIMENSION_PX as f32);

            let by_side = max_side / w.max(h);
            let by_area = (BUDGET_PIXELS / (w * h)).sqrt();
            scale.min(by_side).min(by_area).max(0.05)
        };

        let key = (page, (scale * 1000.0).round() as u32, self.rotation as u8);
        let doc = self.doc.as_mut()?;

        if let Some(existing) = doc.textures.get(&key) {
            return Some(existing.clone());
        }
        // A refusal must not mean a blank page. The ceilings above should make
        // this unreachable; if a limit is ever missed, a softer page is a far
        // better answer than no page.
        let mut scale = scale;
        let raster = loop {
            match doc.session.render_page_rotated(page, scale, self.rotation) {
                Ok(raster) => break raster,
                Err(_) if scale > 0.08 => scale *= 0.5,
                Err(_) => return None,
            }
        };
        let handle = ctx.load_texture(
            format!("page{page}"),
            page_to_image(&raster),
            egui::TextureOptions::LINEAR,
        );
        doc.textures.insert(key, handle.clone());
        Some(handle)
    }

    fn thumb_for(&mut self, ctx: &egui::Context, page: usize) -> Option<egui::TextureHandle> {
        let doc = self.doc.as_mut()?;
        if let Some(existing) = doc.thumbs.get(&page) {
            return Some(existing.clone());
        }
        let raster = doc.session.render_page(page, THUMB_SCALE).ok()?;
        let handle = ctx.load_texture(
            format!("thumb{page}"),
            page_to_image(&raster),
            egui::TextureOptions::LINEAR,
        );
        doc.thumbs.insert(page, handle.clone());
        Some(handle)
    }
}

fn page_to_image(raster: &PageRaster) -> egui::ColorImage {
    egui::ColorImage::from_rgba_unmultiplied(
        [raster.width as usize, raster.height as usize],
        &raster.pixels,
    )
}

impl eframe::App for PagifyApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let command_id = egui::Id::new(COMMAND_INPUT);

        if self.mark.is_none() {
            install_icons(&ctx);
            self.mark = logo::texture(&ctx);
        }

        // A face asked for while picking a run is installed here, at the top of
        // the frame after it was asked for — and marked usable only on the
        // frame after *that*, when egui has rebuilt its atlas. Drawing in a
        // family that does not exist yet panics; see `install_fonts`.
        if let Some(face) = self.pending_face.take() {
            install_fonts(&ctx, Some(face));
        } else if self.editor_face.is_some() {
            self.editor_face_ready = true;
        }

        // A document that wants a password asks for it in a window, not in the
        // command box — see `draw_password_dialog`.
        self.draw_passcode_dialog(&ctx);
        self.draw_signature_pad(&ctx);
        self.draw_signature_list(&ctx);
        self.draw_snippet_list(&ctx);

        // The close button is how people actually quit, and it bypasses every
        // verb. Without this the whole guard is decoration: `quit` refuses
        // politely while the red button throws the work away.
        if ctx.input(|i| i.viewport().close_requested()) && self.would_lose_work() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.closing = Some(Closing::Program);
        }
        self.ask_about_unsaved(&ctx);
        self.ask_about_redaction(&ctx);
        self.collect_reading(&ctx);

        // The focus guard — §5.5. Read once, before any widget runs.
        let focus = Focus::capture(&ctx);

        let mut submit_by_space = false;
        if focus.allows_submit(command_id)
            && self.cmd.space_submits()
            && !self.cmd.input().trim().is_empty()
        {
            ctx.input_mut(|i| {
                let before = i.events.len();
                i.events.retain(|e| {
                    !matches!(e, egui::Event::Text(t) if t == " ")
                        && !matches!(e, egui::Event::Key { key: egui::Key::Space, pressed: true, .. })
                });
                submit_by_space = i.events.len() != before;
            });
        }

        let keys = ctx.input(|i| Keys {
            escape: i.key_pressed(egui::Key::Escape),
            enter: i.key_pressed(egui::Key::Enter),
            up: i.key_pressed(egui::Key::ArrowUp),
            down: i.key_pressed(egui::Key::ArrowDown),
            left: i.key_pressed(egui::Key::ArrowLeft),
            right: i.key_pressed(egui::Key::ArrowRight),
            zoom_in: i.key_pressed(egui::Key::Plus) || i.key_pressed(egui::Key::Equals),
            zoom_out: i.key_pressed(egui::Key::Minus),
            delete: i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace),
            copy: i.modifiers.command && i.key_pressed(egui::Key::C),
            find_next: i.key_pressed(egui::Key::Enter) && i.modifiers.command,
            open: i.modifiers.command && i.key_pressed(egui::Key::O),
            save: i.modifiers.command && i.key_pressed(egui::Key::S),
            undo: i.modifiers.command && !i.modifiers.shift && i.key_pressed(egui::Key::Z),
            redo: i.modifiers.command && i.modifiers.shift && i.key_pressed(egui::Key::Z),
            search: i.modifiers.command && i.key_pressed(egui::Key::F),
        });

        if keys.escape && self.escape() == Escaped::ReturnedToPointer {
            ctx.memory_mut(|m| m.surrender_focus(command_id));
            if let Some(layer) = self.markup.existing_mut(self.page) {
                layer.clear_selection();
            }
        }

        // ⌘C and ⌘Enter are not gated on focus. The guard exists so that
        // *typing* cannot reach the document — Enter and Delete fired while a
        // number is being entered into a field. Copying takes nothing and
        // changes nothing, and a reader who has just dragged out a selection
        // has not necessarily clicked away from the command box first.
        // Taken once. Reading it inside a short-circuiting condition consumed
        // it before the branch that reports the empty case could see it, so
        // `copy` with nothing selected did nothing and said nothing.
        if keys.copy || std::mem::take(&mut self.copy_wanted) {
            self.copy_selection(&ctx);
        }
        if keys.find_next && !self.find_hits.is_empty() {
            self.find_step(true);
        }

        // The shortcuts a Mac expects. Command-modified keys are not gated on
        // focus: ⌘S while the cursor is in the command box still means save,
        // and the guard exists for *unmodified* Enter and Delete, which are the
        // ones a numeric field would otherwise swallow into the document.
        if keys.open {
            self.act(Verb::OpenDialog);
        }
        if keys.save {
            self.act(Verb::Save);
        }
        if keys.undo {
            self.undo_redo(true);
        }
        if keys.redo {
            self.undo_redo(false);
        }
        if keys.search {
            // Puts the cursor in the box with `find ` typed, which is the
            // nearest thing to a search field in an app whose interface is a
            // command line.
            self.cmd.input_mut().clear();
            self.cmd.input_mut().push_str("find ");
            self.command_open = true;
            ctx.memory_mut(|m| m.request_focus(command_id));
        }

        if focus.allows_submit(command_id) {
            if keys.up {
                self.cmd.recall_previous();
            }
            if keys.down {
                self.cmd.recall_next();
            }
        }

        // Document keys. Deny by default — see focus.rs.
        if focus.allows_document_keys() && self.doc.is_some() {
            if keys.right {
                self.act(Verb::Page(PageTarget::Next));
            }
            if keys.left {
                self.act(Verb::Page(PageTarget::Previous));
            }
            if keys.zoom_in {
                self.set_zoom(ZoomTarget::In);
            }
            if keys.zoom_out {
                self.set_zoom(ZoomTarget::Out);
            }
            if keys.delete {
                let page = self.page;
                if let Some(layer) = self.markup.existing_mut(page) {
                    layer.begin("erase");
                    let n = layer.erase_selection();
                    layer.end();
                    if n > 0 {
                        self.say_info(format!("{n} erased."));
                    } else {
                        layer.forget_last_step();
                    }
                }
            }
            // Enter closes a pick that has no fixed number of points — an area
            // measurement, or a polyline. `done` is the same thing typed.
            if keys.enter {
                let closeable = self
                    .pending
                    .as_ref()
                    .is_some_and(|p| p.kind.ends_on_enter() && p.points.len() >= 2);
                if closeable {
                    self.resolve();
                }
            }
        }

        let mut submitted = None;
        if submit_by_space && !self.consume_password_line() {
            submitted = self.cmd.submit(Submit::Space);
        }

        // A file dropped on the window is the other way people open things,
        // and the one they try after the menu.
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect()
        });
        if let Some(path) = dropped.first() {
            self.open(&path.to_string_lossy());
        }

        // And a document Finder handed over, which arrives by Apple Event
        // rather than in `argv` — drained here rather than in the handler,
        // because a document that wants a password has to be able to ask for
        // one, and an Apple Event handler is no place to hold that
        // conversation.
        if let Some(path) = mac_open::taken().first() {
            self.open(&path.to_string_lossy());
        }

        // -- title bar --------------------------------------------------------
        egui::Panel::top("titlebar")
            .frame(egui::Frame::new().fill(theme::CHROME).inner_margin(egui::Margin::symmetric(16, 10)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(32.0, 32.0), egui::Sense::hover());
                    match &self.mark {
                        Some(mark) => {
                            // Rounded to match the tiles elsewhere. The mark's
                            // own square corners would be the only hard ones in
                            // the whole window.
                            ui.painter().image(
                                mark.id(),
                                rect,
                                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                                egui::Color32::WHITE,
                            );
                        }
                        // A failed decode costs a nicer mark, not a title bar.
                        None => {
                            theme::icon_tile(ui.painter(), rect, theme::VIOLET_BRIGHT, theme::VIOLET_DEEP);
                            ui.painter().text(
                                rect.center(),
                                egui::Align2::CENTER_CENTER,
                                "Pa",
                                egui::FontId::proportional(14.0),
                                egui::Color32::WHITE,
                            );
                        }
                    }
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new("Pagify")
                            .color(theme::INK)
                            .font(egui::FontId::proportional(24.0)),
                    );

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if self.doc.is_some() {
                            ui.checkbox(&mut self.ortho, "Ortho");
                            ui.checkbox(&mut self.show_thumbs, "Pages");
                        }
                    });
                });
            });

        // -- ribbon ----------------------------------------------------------
        let mut ribbon_command: Option<String> = None;
        egui::Panel::top("ribbon")
            .frame(egui::Frame::new().fill(theme::CHROME).inner_margin(egui::Margin::symmetric(12, 6)))
            .show(ui, |ui| {
                // Wrapped, not scrolled. Sixteen tabs do not fit a narrow
                // window, and a tab that has scrolled out of sight is a tab
                // nobody knows is there — a second row is the cheaper cost.
                ui.horizontal_wrapped(|ui| {
                    for tab in Tab::ALL {
                        if tab_button(ui, tab.label(), self.ribbon == tab) {
                            self.ribbon = tab;
                        }
                    }
                });
            });

        egui::Panel::top("ribbon_actions")
            .frame(egui::Frame::new().fill(theme::PAPER).inner_margin(egui::Margin::symmetric(14, 8)))
            .show(ui, |ui| {
                let tab = self.ribbon;

                // Tight horizontally, loose vertically. The buttons already
                // carry their own padding, so spacing between them only adds
                // gaps to a grid that reads better closed up — but a row that
                // wraps needs air between the rows or the two run together.
                ui.spacing_mut().item_spacing = egui::vec2(2.0, 6.0);

                // Which tool is in force. The pointer mode, or whichever tool
                // is part-way through collecting its clicks — a user who armed
                // Line and looked away needs to see that it is still armed.
                let armed = self.pending.as_ref().and_then(|p| p.kind.command());
                let in_hand = self.markup_armed.map(|k| match k {
                    pagify_shell::verbs::Markup::Highlight => "highlight",
                    pagify_shell::verbs::Markup::Underline => "underline",
                    pagify_shell::verbs::Markup::StrikeOut => "strikeout",
                    pagify_shell::verbs::Markup::Squiggly => "squiggly",
                });
                let live = |command: &str| -> bool {
                    let c = command.trim();
                    in_hand == Some(c)
                        || armed.as_deref() == Some(c)
                        || match self.pointer {
                            pagify_shell::verbs::PointerMode::Select => c == "selecttool",
                            pagify_shell::verbs::PointerMode::Pan => c == "hand",
                        }
                };

                ui.horizontal_wrapped(|ui| {
                    for (glyph, label, command) in tab.leading() {
                        if tool_button(ui, glyph, label, command, live(command)).clicked() {
                            ribbon_command = Some((*command).to_string());
                        }
                    }
                    // The reference toolbar divides the two standing tools from
                    // the tab's own, and it is worth keeping: without it Hand
                    // and Select read as part of whichever tab is open.
                    if !tab.leading().is_empty() {
                        ui.add_space(4.0);
                        ui.separator();
                        ui.add_space(4.0);
                    }
                    for (glyph, label, command) in tab.buttons() {
                        if tool_button(ui, glyph, label, command, live(command)).clicked() {
                            // Every button runs a command string — §7.
                            ribbon_command = Some((*command).to_string());
                        }
                    }
                });

            });

        // -- command bar ------------------------------------------------------
        //
        // Two shapes: the single line the mockup draws, and an opened box with
        // the history above it. Resizable while open, because how much history
        // you want to see is not something this can know.
        let bar_frame = egui::Frame::new()
            .fill(theme::CHROME)
            .inner_margin(egui::Margin::symmetric(14, 8));

        let mut bar = egui::Panel::bottom("command_bar").frame(bar_frame);
        bar = if self.command_open {
            bar.resizable(true).default_size(210.0).size_range(96.0..=520.0)
        } else {
            bar.resizable(false)
        };

        bar.show(ui, |ui| {
            if self.command_open {
                // The history claims whatever the panel was dragged to, less
                // the three fixed rows below it.
                let rows = 74.0;
                let height = (ui.available_height() - rows).max(24.0);
                egui::ScrollArea::vertical()
                    .max_height(height)
                    .stick_to_bottom(true)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.set_min_height(height);
                        for entry in self.cmd.history() {
                            let (colour, text) = match entry.kind {
                                Kind::Echo => (theme::INK_DIM, format!("› {}", entry.text)),
                                Kind::Info => (theme::INK, entry.text.clone()),
                                Kind::Error => (theme::DANGER, entry.text.clone()),
                            };
                            ui.colored_label(colour, text);
                        }
                    });
                ui.separator();
            }

            // The name and what is wanted are drawn separately so the name can
            // be violet. `Prompt::render` joins them for callers that want one
            // string — using it here as well is what printed "pagify › pagify ›".
            let name = self
                .cmd
                .prompt()
                .document
                .clone()
                .unwrap_or_else(|| "pagify".to_string());
            let wants = match &self.pending {
                Some(p) => p.prompt(),
                None => self.cmd.prompt().wants.clone(),
            };

            if self.command_open {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(&name)
                            .color(theme::VIOLET_BRIGHT)
                            .font(egui::FontId::monospace(13.0)),
                    );
                    ui.colored_label(theme::INK_FAINT, "›");
                    ui.colored_label(theme::INK_DIM, &wants);
                });
            }

            ui.horizontal(|ui| {
                if !self.command_open {
                    ui.label(
                        egui::RichText::new(&name)
                            .color(theme::VIOLET_BRIGHT)
                            .font(egui::FontId::monospace(13.0)),
                    );
                    ui.colored_label(theme::INK_FAINT, ">");
                    // A tool that is waiting for clicks must say so even with
                    // the history folded away. Without this, arming a tool
                    // looked exactly like nothing happening.
                    if self.pending.is_some() {
                        ui.colored_label(theme::SNAP, &wants);
                    } else if let Some(last) = self.cmd.history().last() {
                        // **What the program just said**, with the history
                        // folded away — which it is by default.
                        //
                        // Without this, every button that is not built yet
                        // looked broken rather than unbuilt: the reply saying
                        // so went straight into a panel nobody had open, and a
                        // whole tab of them read as a tab that does nothing.
                        // The one place a user is guaranteed to be looking
                        // after pressing a button is the line under it.
                        let (colour, text) = match last.kind {
                            Kind::Error => (theme::DANGER, last.text.as_str()),
                            Kind::Echo => (theme::INK_FAINT, last.text.as_str()),
                            _ => (theme::INK_DIM, last.text.as_str()),
                        };
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(text)
                                    .color(colour)
                                    .font(egui::FontId::proportional(12.0)),
                            )
                            .truncate(),
                        );
                    }
                }

                // The toggle sits at the right so the input keeps the width it
                // has, rather than jumping when the chevron changes.
                let chevron = if self.command_open { "⌄" } else { "⌃" };
                let toggle_width = 26.0;
                let input_width = (ui.available_width() - toggle_width - 8.0).max(80.0);

                let response = ui.add_sized(
                    egui::vec2(input_width, 22.0),
                    egui::TextEdit::singleline(self.cmd.input_mut())
                        .id(command_id)
                        .font(egui::FontId::monospace(13.0))
                        .password(self.awaiting_password.is_some())
                        .hint_text(match &self.awaiting_password {
                            Some(
                                Awaiting::Lock { .. }
                                | Awaiting::LockPages(_)
                                | Awaiting::LockImage { .. },
                            ) => "a passcode to lock with",
                            Some(Awaiting::UnlockItem(_)) => "the passcode this was locked with",
                            Some(Awaiting::Unlock) => "the passcode this was locked with",
                            Some(Awaiting::Secure(_)) => "a password for this document",
                            Some(Awaiting::SecureAgain { .. }) => "the same password again",
                            Some(Awaiting::LockAgain { .. }) => "the same passcode again",
                            Some(Awaiting::SecureCurrent(_)) => "this document's password",
                            Some(Awaiting::Certificate(_)) => "the certificate's password",
                            // Opening asks in a window of its own, so the box
                            // is free for what it is usually for.
                            Some(Awaiting::Open(_)) if self.command_open => "type a command",
                            Some(Awaiting::Open(_)) => "type a command...",
                            None if self.command_open => "type a command",
                            None => "type a command...",
                        }),
                );
                if response.changed() {
                    self.cmd.note_edited();
                }
                if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    if !self.consume_password_line() {
                        submitted = self.cmd.submit(Submit::Enter);
                    }
                    response.request_focus();
                }

                if ui
                    .add_sized(egui::vec2(toggle_width, 22.0), egui::Button::new(chevron))
                    .on_hover_text(if self.command_open { "Hide the history" } else { "Show the history" })
                    .clicked()
                {
                    self.command_open = !self.command_open;
                }
            });

            if self.command_open {
                ui.horizontal(|ui| {
                    if ui.button("Run").clicked()
                        && !self.consume_password_line()
                       
                    {
                        submitted = self.cmd.submit(Submit::Button);
                    }
                    let marks = self.markup.existing(self.page).map(|l| l.len()).unwrap_or(0);
                    ui.small(match &self.doc {
                        Some(doc) => format!(
                            "page {} of {}   ·   {:.0}%   ·   {marks} mark{}{}{}",
                            self.page + 1,
                            doc.page_count,
                            self.resolved_zoom() * 100.0,
                            if marks == 1 { "" } else { "s" },
                            if self.calibration.is_calibrated() { "   ·   calibrated" } else { "" },
                            if self.recorder.is_recording() {
                                format!("   ·   recording ({})", self.recorder.steps())
                            } else {
                                String::new()
                            },
                        ),
                        None => "no document".to_string(),
                    });
                });
            }
        });

        // The File tab is *backstage*: it covers the document rather than
        // sitting beside it, the way File does in every ribbon application. So
        // the thumbnail rail and the page canvas both stand down while it is
        // showing, and the document stays open behind it untouched.
        //
        // **Only** the File tab. This used to also fire when nothing was open,
        // which meant every tab showed the wizard and the tab highlight was
        // lying about what you were looking at. Having no document is not a
        // reason to replace Home with File — it is a reason for Home to say it
        // is empty. The app opens *on* File instead, which is where the wizard
        // belongs and where someone with no document needs to be.
        let backstage = self.ribbon == Tab::File;

        // -- thumbnails --------------------------------------------------------
        let mut jump_to = None;
        // Nothing open means nothing to thumbnail — an empty rail is a strip of
        // furniture that does not do anything.
        if self.show_thumbs && !backstage && self.doc.is_some() {
            egui::Panel::left("thumbs")
                .resizable(true)
                .default_size(148.0)
                .size_range(104.0..=420.0)
                .frame(
                    egui::Frame::new()
                        .fill(theme::PAPER)
                        .inner_margin(egui::Margin::symmetric(8, 8)),
                )
                .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let count = self.doc.as_ref().map(|d| d.page_count).unwrap_or(0);
                    for page in 0..count {
                        let current = page == self.page;
                        ui.vertical_centered(|ui| {
                            if let Some(texture) = self.thumb_for(&ctx, page) {
                                let size = texture.size_vec2() / ctx.pixels_per_point();
                                let response = ui.add(
                                    egui::Image::new(&texture)
                                        .fit_to_exact_size(size)
                                        .sense(egui::Sense::click()),
                                );
                                if current {
                                    ui.painter().rect_stroke(
                                        response.rect.expand(2.0),
                                        2.0,
                                        egui::Stroke::new(2.0, theme::VIOLET),
                                        egui::StrokeKind::Outside,
                                    );
                                }
                                if response.clicked() {
                                    jump_to = Some(page);
                                }
                            }
                            ui.small(format!("{}", page + 1));
                        });
                        ui.add_space(6.0);
                    }
                });
            });
        }

        // -- the pages ---------------------------------------------------------
        let mut home_command: Option<String> = None;
        egui::CentralPanel::default_margins().show(ui, |ui| {
            self.canvas_pt = ui.available_size();
            if backstage {
                let chosen = egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| home::show(ui, &self.recent, &self.outlined_fonts))
                    .inner;
                if let Some(command) = chosen {
                    home_command = Some(command);
                }
                return;
            }

            if self.doc.is_none() {
                // Not the wizard. This tab has nothing to show because there is
                // nothing open, and it should say so rather than quietly
                // becoming a different tab.
                ui.centered_and_justified(|ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(ui.available_height() * 0.35);
                        ui.colored_label(theme::INK_FAINT, "No document open.");
                        ui.add_space(10.0);
                        if ui.button("Open a PDF…").clicked() {
                            home_command = Some("open".to_string());
                        }
                        ui.add_space(6.0);
                        ui.small(
                            egui::RichText::new("or drop one on the window")
                                .color(theme::INK_FAINT),
                        );
                    });
                });
                return;
            }

            self.draw_pages(ui, &ctx, command_id);
        });

        if let Some(page) = jump_to {
            self.act(Verb::Page(PageTarget::Number(page + 1)));
        }
        if let Some(command) = home_command.or(ribbon_command) {
            // Put it in the box rather than running it silently, so the user
            // sees the words the button stands for — which is the whole claim.
            self.cmd.input_mut().clear();
            self.cmd.input_mut().push_str(&command);
            if !command.ends_with(' ') {
                submitted = self.cmd.submit(Submit::Button);
            } else {
                ctx.memory_mut(|m| m.request_focus(command_id));
            }
        }
        if let Some(dispatch) = submitted {
            // A typed line cancels any half-collected pick. Letting it swallow
            // the click silently would mean an unrelated command finishing
            // someone else's measurement.
            if self.pending.take().is_some() {
                self.say_info("that pick was cancelled.");
            }
            let line = self
                .cmd
                .history()
                .last()
                .map(|e| e.text.clone())
                .unwrap_or_default();
            self.recorder.observe(&line);
            self.run(dispatch);
        }
    }
}

impl PagifyApp {
    fn draw_pages(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, command_id: egui::Id) {
        let mut zoom = self.resolved_zoom();
        let (strip_height, strip_width, page_count) = {
            let doc = self.doc.as_ref().expect("checked");
            (doc.strip.height_pt(), doc.strip.width_pt(), doc.page_count)
        };

        // Zoom at the pointer, before the scroll area gets the wheel.
        //
        // Anchored rather than centred: zooming towards the middle of the
        // window moves whatever you were looking at off it, and on a page of
        // small type that is the whole reason you were zooming. The point under
        // the cursor is the one the user is asking about, so it is the one that
        // stays still.
        let viewport = self.viewport_rect.unwrap_or_else(|| ui.max_rect());
        let pointer = ui.input(|i| i.pointer.hover_pos()).filter(|p| viewport.contains(*p));
        if let Some(p) = pointer {
            // egui folds ⌘/Ctrl-scroll and a trackpad pinch into the same
            // number, which is right: they are one gesture with two spellings.
            let factor = ui.input(|i| i.zoom_delta());
            if (factor - 1.0).abs() > 0.001 {
                let before = zoom;
                let after = (before * factor).clamp(0.05, 16.0);
                if (after - before).abs() > f32::EPSILON {
                    // Anchored against where the page **was actually drawn**,
                    // not against a model of where it ought to be.
                    //
                    // Reconstructing the content position from the viewport,
                    // the scroll offset and the strip padding means encoding
                    // the layout twice, and any term missed from the copy shows
                    // up as the page creeping away from the cursor — which it
                    // did, vertically, by a different amount at every scale.
                    // `last_view` is the mapping the previous frame really
                    // used, so there is nothing left to get wrong.
                    // The page under the pointer, or the current one when the
                    // pointer is beside the page rather than on it — the strip
                    // is wider than the paper.
                    let anchor_on =
                        self.hover_view.or_else(|| self.last_view.map(|v| (self.page, v)));
                    if let Some((index, view)) = anchor_on {
                        // Where this page sits in the strip, in strip points.
                        //
                        // This is the term the anchor was missing. A page's
                        // origin on screen is
                        //
                        //     viewport - offset + padding + strip_position * zoom
                        //
                        // so changing the zoom moves it **even at a fixed
                        // offset**, by `strip_position * (after - before)`.
                        // Correcting only for the offset leaves exactly that
                        // much error, and it grows with distance down the
                        // document: on the first page `strip_position` is zero
                        // and the anchor looks perfect, which is why every
                        // single-page test passed while a catalogue slid 200pt.
                        let (sx, sy) = self
                            .doc
                            .as_ref()
                            .map(|d| {
                                let (w, _) = d.strip.size_of(index).unwrap_or((0.0, 0.0));
                                (
                                    d.strip
                                        .left_of(index)
                                        .unwrap_or((d.strip.width_pt() - w) / 2.0),
                                    d.strip.top_of(index).unwrap_or(0.0),
                                )
                            })
                            .unwrap_or((0.0, 0.0));

                        let on_page = view.to_page(p);
                        let origin_after = egui::vec2(
                            p.x - on_page.x as f32 * after,
                            p.y - on_page.y as f32 * after,
                        );
                        let moved = view.origin.to_vec2() - origin_after;
                        let with_strip = egui::vec2(sx, sy) * (after - before);
                        self.anchor_offset = Some(self.scroll_offset + moved + with_strip);
                    }
                    self.zoom = ZoomMode::Factor(after);
                    zoom = after;

                    // The gesture belongs to the zoom. Left in place, the same
                    // wheel also scrolls the strip, and the two fight for the
                    // offset every frame — which reads as the page shuddering
                    // rather than zooming.
                    ui.input_mut(|i| i.smooth_scroll_delta = egui::Vec2::ZERO);
                }
            }
        }

        let mut area = egui::ScrollArea::both().auto_shrink([false, false]);
        // Whether this frame dictated the offset rather than observing it.
        let mut forced: Option<egui::Vec2> = None;
        if let Some(by) = self.pan_by.take() {
            // Clamped to what can actually be scrolled to.
            //
            // Without the upper bound the offset keeps growing past the end of
            // the document while the drag continues; the scroll area clamps
            // what it draws, and the accumulated excess springs back the moment
            // the drag reverses. That is the bounce at the edges.
            let content = egui::vec2(strip_width * zoom + 24.0, strip_height * zoom + 24.0);
            let room = (content - viewport.size()).max(egui::Vec2::ZERO);
            let to = (self.scroll_offset + by).clamp(egui::Vec2::ZERO, room);
            forced = Some(to);
            area = area.scroll_offset(to);
        } else if let Some(y) = self.scroll_to_pt.take() {
            // 12.0 is the strip's top padding, the same constant the page
            // origins are laid out from.
            area = area.scroll_offset(egui::vec2(self.scroll_offset.x, y * zoom));
        } else if let Some(offset) = self.anchor_offset.take() {
            let content = egui::vec2(strip_width * zoom + 24.0, strip_height * zoom + 24.0);
            let room = (content - viewport.size()).max(egui::Vec2::ZERO);
            let to = offset.clamp(egui::Vec2::ZERO, room);
            forced = Some(to);
            area = area.scroll_offset(to);
        }
        let scroll = area
            .show(ui, |ui| {
                let content =
                    egui::vec2(strip_width * zoom + 24.0, strip_height * zoom + 24.0);
                let (_, canvas) = ui.allocate_space(content);

                // Centred while it fits, pinned to the corner once it does not.
                //
                // A scroll area lays its content out from the top-left, so a
                // page smaller than the window sat against the left edge with
                // all the empty space on one side. Half the slack on each side
                // is what a reader expects of a page on a desk.
                //
                // This does not disturb zoom anchoring: the padding is only
                // non-zero on an axis where the content fits, and an axis that
                // fits cannot scroll, so there was never an offset to hold on
                // it anyway.
                let pad = ((viewport.size() - content) * 0.5).max(egui::Vec2::ZERO);

                // Which slice of the strip the window is over, in page points.
                let visible_top =
                    ((ui.clip_rect().top() - canvas.top() - pad.y) / zoom).max(0.0);
                let visible_bottom = (ui.clip_rect().bottom() - canvas.top() - pad.y) / zoom;
                let visible = {
                    let doc = self.doc.as_ref().expect("checked");
                    doc.strip.visible(visible_top, visible_bottom)
                };

                // The page you are looking at is the page you are working on.
                //
                // Nothing kept `self.page` in step with the scroll: it moved
                // only for `page next` and friends. On a one-page fixture that
                // is invisibly correct, and on a 149-page catalogue it means
                // the pointer talks to page 1 while the reader is on page 40 —
                // so clicking and dragging do nothing at all, which is exactly
                // what "selection does not work" looked like.
                //
                // Not while the pointer is down: re-deciding the current page
                // in the middle of a drag would drop the selection being made.
                if self.settling > 0 {
                    self.settling -= 1;
                } else if !ui.ctx().input(|i| i.pointer.any_down()) {
                    let clip = ui.clip_rect();
                    let mut best: Option<(usize, f32)> = None;
                    for page in visible.clone() {
                        let doc = self.doc.as_ref().expect("checked");
                        let Some(top) = doc.strip.top_of(page) else { continue };
                        let Some((_, h)) = doc.strip.size_of(page) else { continue };
                        let y0 = canvas.top() + pad.y + 12.0 + top * zoom;
                        let y1 = y0 + h * zoom;
                        // How much of the window this page fills.
                        let shown = (y1.min(clip.bottom()) - y0.max(clip.top())).max(0.0);
                        if best.map_or(true, |(_, b)| shown > b) {
                            best = Some((page, shown));
                        }
                    }
                    if let Some((page, _)) = best {
                        // The selection is *not* dropped here. It belongs to
                        // the page it was made on, which `selection_page`
                        // remembers, and scrolling past is not a decision to
                        // discard it.
                        self.page = page;
                    }
                }

                let hover = ui.ctx().input(|i| i.pointer.hover_pos());
                for page in visible.clone() {
                    let (top, (w, h)) = {
                        let doc = self.doc.as_ref().expect("checked");
                        (doc.strip.top_of(page).unwrap_or(0.0), doc.strip.size_of(page).unwrap_or((612.0, 792.0)))
                    };
                    // Placement comes from the strip, which is what knows
                    // whether this page shares its row with another.
                    let left = {
                        let doc = self.doc.as_ref().expect("checked");
                        doc.strip.left_of(page).unwrap_or((strip_width - w) / 2.0)
                    };
                    let origin = egui::pos2(
                        canvas.left() + pad.x + 12.0 + left * zoom,
                        canvas.top() + pad.y + 12.0 + top * zoom,
                    );

                    // Past the GPU's limit the page is still *drawn* at the
                    // asked-for size and simply gets softer, which is the right
                    // trade. `texture_for` applies that cap.
                    let device_scale = raster_scale(zoom * ctx.pixels_per_point());
                    if let Some(texture) = self.texture_for(ctx, page, device_scale) {
                        // **The page's true size at this zoom, not the
                        // texture's.** The raster is quantised so that a pinch
                        // does not re-render sixty times a second; drawing it at
                        // its own size passes that quantisation straight through
                        // to the screen, so the page jumps between a handful of
                        // sizes instead of zooming. Letting the GPU scale the
                        // texture to the asked-for rectangle is the entire point
                        // of quantising it.
                        //
                        // It was also wrong: `PageView` below maps clicks at the
                        // true `zoom`, so wherever the raster scale differed from
                        // it, the picture and the hit-testing disagreed.
                        let size = egui::vec2(w * zoom, h * zoom);
                        let rect = egui::Rect::from_min_size(origin, size);
                        ui.painter().image(
                            texture.id(),
                            rect,
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            egui::Color32::WHITE,
                        );

                        let view = PageView { origin, scale: zoom };
                        self.draw_text_highlights(ui.painter(), page, view);
                        if let Some(layer) = self.markup.existing(page) {
                            overlay::draw_layer(
                                ui.painter(),
                                layer,
                                view,
                                theme::MARKUP,
                                theme::SELECTED,
                            );
                        }
                        if page == self.page {
                            self.last_view = Some(view);
                        }
                        // Zoom anchors against the page under the cursor, which
                        // on a scrolling strip is often not the current one.
                        // Anchoring with another page's mapping puts the fixed
                        // point on the wrong page and the view slides.
                        if hover.is_some_and(|p| rect.contains(p)) {
                            self.hover_view = Some((page, view));
                        }
                        // Every visible page is live, not just the current one:
                        // you interact with what you are pointing at, and only
                        // the page under the pointer will answer anyway.
                        //
                        // The exception is a tool that is part-way through. Its
                        // picks belong to the page it was armed on, and letting
                        // a later click on a different page join them would put
                        // one end of the line on page 40 and the other on page
                        // 41 — in coordinates that mean nothing on either.
                        // **A tool that has collected nothing has not started.**
                        //
                        // The rule below binds a part-way pick to its page, for
                        // a good reason. But it also silently swallowed the
                        // *first* click of a one-click tool armed while another
                        // page was current — on a scrolling strip with two pages
                        // in view, arming and then clicking the other one did
                        // nothing whatever: no mark, no message, nothing to
                        // read. So a pick holding no points and no objects yet
                        // starts on whichever page is actually clicked.
                        if let Some(pending) = &mut self.pending {
                            if pending.page != page
                                && pending.points.is_empty()
                                && pending.objects.is_empty()
                                && hover.is_some_and(|at| rect.contains(at))
                            {
                                pending.page = page;
                            }
                        }
                        let owns = self.pending.as_ref().map_or(true, |p| p.page == page);
                        if owns {
                            self.interact(ui, rect, view, page, command_id);
                        }
                        // **After the page's own interaction, not before it.**
                        // egui gives a click to the last widget registered over
                        // that spot, and the page covers every badge on it — so
                        // declaring the badges first meant the page swallowed
                        // every click and a padlock could never be pressed.
                        self.draw_lock_badges(ui, page, view);
                        // On top of the page and its badges, under the editor:
                        // a tool part-way through is the most recent thing the
                        // reader did and the thing they are aiming with.
                        self.draw_pending_preview(ui.painter(), page, view, hover);
                        // The editor, for the same reason, learned twice.
                        //
                        // Reported from use: "if i click somewhere else the
                        // cursor is gone and i cant bring it back… then i cant
                        // apply the change." Declared before the page, its
                        // field and its Apply button were under a widget that
                        // takes every click over them — so the caret could be
                        // given once, by asking for it, and never again by
                        // clicking. Drawn last, it is simply clickable.
                        self.draw_run_editor(ui, page, view);
                    }
                }

                visible
            });

        // Prefetch the neighbours, so a scroll onto them is a copy rather than a
        // render. One per frame: the point is to be ready, not to stall now.
        // Where the strip ended up, so the next zoom can anchor from it.
        //
        // Observed, not the value asked for — and it has to be observed,
        // because the anchor pairs it with `hover_view.origin`, which is where
        // the page was *actually drawn* this frame. Mixing an intended offset
        // with an observed origin means the two describe different moments, and
        // the difference accumulates into the page sliding away as you zoom.
        let _ = forced;
        self.scroll_offset = scroll.state.offset;
        self.viewport_rect = Some(scroll.inner_rect);
        let visible = scroll.inner;
        if let Some(target) = prefetch_targets(&visible, page_count, 2).first().copied() {
            // The same quantised scale the draw uses. Prefetching at the raw
            // zoom would warm a texture the next frame does not ask for.
            let device_scale = raster_scale(zoom * ctx.pixels_per_point());
            let _ = self.texture_for(ctx, target, device_scale);
        }

        // Evict rasters for pages nowhere near the window.
        if let Some(doc) = &mut self.doc {
            let keep_from = visible.start.saturating_sub(3);
            let keep_to = visible.end + 3;
            doc.textures.retain(|(page, _, _), _| *page >= keep_from && *page < keep_to);
        }
    }

    /// The selection, and every search hit, drawn over the page.
    ///
    /// Under the marks rather than over them: a highlight is a wash on the
    /// paper, and drawing it on top would dim the ink it is meant to point at.
    /// The shape a half-finished tool would make, following the pointer.
    ///
    /// **Because a tool that shows nothing until it is finished asks you to
    /// aim blind.** Every one of these collects two points or more, and until
    /// the last click there was nothing on screen to say where the first one
    /// had landed or what was being made — reported from use as wanting the
    /// drawing to start at the first click.
    ///
    /// Drawn from the point that has been placed to wherever a click would land
    /// *now*, snapping included, so the preview is the thing that would happen
    /// rather than an impression of it.
    fn draw_pending_preview(
        &self,
        painter: &egui::Painter,
        page: usize,
        view: PageView,
        hover: Option<egui::Pos2>,
    ) {
        let Some(pending) = &self.pending else { return };
        if pending.page != page || pending.points.is_empty() {
            return;
        }
        let Some(cursor) = hover else { return };
        // Where a click would land, which is not the pointer when a snap has
        // taken it.
        let at = self
            .last_snap
            .as_ref()
            .map(|snapped| snapped.at)
            .unwrap_or_else(|| view.to_page(cursor));

        let on = |p: AppPoint| view.to_screen(p);
        let stroke = egui::Stroke::new(1.0, theme::VIOLET_BRIGHT);
        let first = pending.points[0];
        let last = *pending.points.last().expect("checked");

        let box_between = |a: AppPoint, b: AppPoint| {
            egui::Rect::from_two_pos(on(a), on(b))
        };

        match &pending.kind {
            PendingKind::Draw(DrawKind::Line) | PendingKind::SignLine => {
                painter.line_segment([on(first), on(at)], stroke);
            }
            PendingKind::Draw(DrawKind::Circle) => {
                let radius = (on(first) - on(at)).length();
                painter.circle_stroke(on(first), radius, stroke);
            }
            PendingKind::Draw(DrawKind::Rectangle) | PendingKind::SignRectangle => {
                painter.rect_stroke(
                    box_between(first, at),
                    egui::CornerRadius::ZERO,
                    stroke,
                    egui::StrokeKind::Inside,
                );
            }
            // A polyline keeps what is already placed and trails the last leg.
            PendingKind::Draw(DrawKind::Polyline) | PendingKind::Measure(MeasureKind::Area) => {
                let mut path: Vec<egui::Pos2> = pending.points.iter().map(|p| on(*p)).collect();
                path.push(on(at));
                painter.add(egui::Shape::line(path, stroke));
            }
            PendingKind::Measure(MeasureKind::Distance) | PendingKind::Calibrate { .. } => {
                painter.line_segment([on(first), on(at)], stroke);
            }
            // The ones that take an area, each in the colour of what it does.
            PendingKind::Redact => {
                painter.rect_stroke(
                    box_between(first, at),
                    egui::CornerRadius::ZERO,
                    egui::Stroke::new(1.0, theme::DANGER),
                    egui::StrokeKind::Inside,
                );
            }
            // What is being carried, drawn where it would land.
            PendingKind::Move { pictures_first } => {
                if let Some((_, rect, _)) = self.thing_at(page, first, *pictures_first) {
                    let by = egui::vec2(
                        (on(at).x - on(first).x),
                        (on(at).y - on(first).y),
                    );
                    let outline = egui::Rect::from_two_pos(
                        on(AppPoint { x: rect.left as f64, y: rect.top as f64 }),
                        on(AppPoint { x: rect.right as f64, y: rect.bottom as f64 }),
                    );
                    // Where it is now, faint; where it is going, solid.
                    painter.rect_stroke(
                        outline,
                        egui::CornerRadius::ZERO,
                        egui::Stroke::new(1.0, theme::INK_FAINT),
                        egui::StrokeKind::Inside,
                    );
                    painter.rect_stroke(
                        outline.translate(by),
                        egui::CornerRadius::ZERO,
                        stroke,
                        egui::StrokeKind::Inside,
                    );
                }
            }
            PendingKind::Whiteout | PendingKind::Lock => {
                painter.rect_stroke(
                    box_between(first, at),
                    egui::CornerRadius::ZERO,
                    stroke,
                    egui::StrokeKind::Inside,
                );
            }
            _ => {}
        }

        // Where the placed points are, so a long drag still shows what it is
        // anchored to.
        for point in &pending.points {
            painter.circle_filled(on(*point), 2.5, theme::VIOLET_BRIGHT);
        }
        let _ = last;
    }

    /// The editor for a run of text, drawn over the words themselves.
    ///
    /// Sized and placed from the run's own box, so what is being typed sits
    /// where what is being replaced sits. A field somewhere else would mean
    /// holding the page in your head while you type.
    fn draw_run_editor(&mut self, ui: &mut egui::Ui, page: usize, view: PageView) {
        let Some(edit) = &mut self.editing_run else { return };
        if edit.page != page {
            return;
        }

        // Wherever the grip has dragged them to, which is only a preview until
        // it is let go.
        let (shift_x, shift_y) = edit.drag_by;
        let top_left = view.to_screen(AppPoint {
            x: (edit.rect.left.min(edit.rect.right) + shift_x) as f64,
            y: (edit.rect.top.min(edit.rect.bottom) + shift_y) as f64,
        });
        let bottom_right = view.to_screen(AppPoint {
            x: (edit.rect.left.max(edit.rect.right) + shift_x) as f64,
            y: (edit.rect.top.max(edit.rect.bottom) + shift_y) as f64,
        });

        // Room to grow. A replacement is rarely the same length as what it
        // replaces, and a field cut to the old text cannot show the new.
        let height = (bottom_right.y - top_left.y).max(14.0);
        let rect = egui::Rect::from_min_size(
            egui::pos2(top_left.x, top_left.y),
            egui::vec2((bottom_right.x - top_left.x).max(120.0) + 80.0, height + 6.0),
        );

        // **Edited where it sits, looking like what it is.**
        //
        // The field has to cover the words: they are still on the page until
        // the edit is applied, and typing over them is unreadable. But it was
        // covering them with the program's own panel colour and setting them in
        // the program's own font — a dark slab in the middle of a white page,
        // so the thing being edited stopped looking like the thing on screen.
        //
        // Instead: the page's own colour behind, the run's own ink and size in
        // front, and the box kept off the words themselves.
        let paper = egui::Color32::from_rgb(
            edit.background.r,
            edit.background.g,
            edit.background.b,
        );
        let ink = edit
            .style
            .color
            .filter(|c| c.a > 0)
            .map(|c| egui::Color32::from_rgb(c.r, c.g, c.b))
            .unwrap_or(egui::Color32::BLACK);
        ui.painter().rect_filled(rect.expand(1.0), 0.0, paper);

        // **The size the words are *drawn*, not the number in the file.**
        //
        // Plenty of producers write `1 Tf` and put the real size in the text
        // matrix — this crate already knows that, and says so where `TJ`
        // displacements are scaled. Taking the nominal size set the editor at
        // one point and the words came out as a whisper. The box a run occupies
        // is its ink, and ink is most of an em.
        let drawn = (bottom_right.y - top_left.y).max(1.0);
        let nominal = edit.style.size.unwrap_or(0.0) * view.scale as f32;
        let on_screen = if nominal >= drawn * 0.5 { nominal } else { drawn * 0.92 };

        let face_ready = self.editor_face.is_some() && self.editor_face_ready;
        let id = egui::Id::new(("run-editor", page, edit.object));
        let response = {
            let style = ui.style_mut();
            style.visuals.override_text_color = Some(ink);
            style.visuals.extreme_bg_color = paper;
            style.visuals.selection.bg_fill = theme::VIOLET.gamma_multiply(0.35);
            ui.put(
                rect,
                egui::TextEdit::singleline(&mut edit.buffer)
                    .id(id)
                    .background_color(paper)
                    .margin(egui::Margin::ZERO)
                    // The document's own face where it could be read and egui
                    // has had a frame to build it; the program's own otherwise.
                    .font(if face_ready {
                        egui::FontId::new(
                            on_screen.clamp(6.0, 96.0),
                            egui::FontFamily::Name(RUN_FAMILY.into()),
                        )
                    } else {
                        egui::FontId::proportional(on_screen.clamp(6.0, 96.0))
                    }),
            )
        };
        ui.style_mut().visuals.override_text_color = None;

        // A hairline under the words rather than a box around them: enough to
        // say which words are being edited, little enough to leave the page
        // looking like the page.
        ui.painter().line_segment(
            [
                egui::pos2(rect.left(), rect.bottom() + 1.0),
                egui::pos2(rect.right(), rect.bottom() + 1.0),
            ],
            egui::Stroke::new(1.0, theme::VIOLET_BRIGHT),
        );

        // **The grip: drag the words where they should go.**
        //
        // Beside them rather than on them, so dragging it cannot be mistaken
        // for selecting the text it is next to. The page changes once, when it
        // is let go — see `drag_by`.
        let size = (rect.height() * 0.9).clamp(12.0, 22.0);
        let grip = egui::Rect::from_min_size(
            egui::pos2(rect.left() - size - 4.0, rect.top()),
            egui::vec2(size, size),
        );
        let held = ui.interact(
            grip,
            egui::Id::new(("run-editor-grip", page, edit.object)),
            egui::Sense::drag(),
        );
        ui.painter().rect_filled(grip, egui::CornerRadius::same(3), theme::VIOLET);
        ui.painter().text(
            grip.center(),
            egui::Align2::CENTER_CENTER,
            "\u{E89F}",
            icon_font(size * 0.62),
            egui::Color32::WHITE,
        );
        if held.hovered() || held.dragged() {
            ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::Move);
        }
        if held.dragged() {
            let by = held.drag_delta() / view.scale as f32;
            edit.drag_by = (edit.drag_by.0 + by.x, edit.drag_by.1 + by.y);
        }
        let dropped = held.drag_stopped();

        if !edit.focused {
            edit.focused = true;
            response.request_focus();
        }

        let done = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));

        // **The properties float; they do not sit on the page.**
        //
        // Laid out in the page's own `Ui`, the bar was a slab across the
        // paragraph being edited — the lines under the one in hand disappeared
        // behind it. It describes *this* run, so it stays near it, but on its
        // own layer, clear of the words, and only as wide as it needs to be.
        //
        // Below the line where there is room, above it near the foot of the
        // window, so it never covers what is being typed.
        // The visible page area, which is what the bar must stay inside.
        let visible = ui.clip_rect();
        let below = rect.bottom() + 8.0;
        let anchor = if below + 40.0 < visible.max.y {
            egui::pos2(rect.left(), below)
        } else {
            egui::pos2(rect.left(), rect.top() - 40.0)
        };
        let mut apply_now = false;
        egui::Area::new(egui::Id::new(("run-editor-controls", page, edit.object)))
            .order(egui::Order::Foreground)
            .fixed_pos(anchor)
            .show(ui.ctx(), |ui| {
                egui::Frame::new()
                    .fill(theme::CHROME)
                    .stroke(egui::Stroke::new(1.0, theme::LINE))
                    .shadow(egui::epaint::Shadow {
                        offset: [0, 2],
                        blur: 8,
                        spread: 0,
                        color: egui::Color32::from_black_alpha(90),
                    })
                    .inner_margin(egui::Margin::symmetric(8, 4))
                    .corner_radius(egui::CornerRadius::same(4))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                        let mut size = edit.style.size.unwrap_or(12.0);
                        ui.label(egui::RichText::new("size").color(theme::INK_DIM).size(11.0));
                        if ui
                            .add(
                                egui::DragValue::new(&mut size)
                                    .speed(0.25)
                                    .range(1.0..=400.0),
                            )
                            .changed()
                        {
                            edit.style.size = Some(size);
                        }

                        let c = edit.style.color.unwrap_or(pdf_core::document::Color {
                            r: 0,
                            g: 0,
                            b: 0,
                            a: 255,
                        });
                        let mut rgb = [c.r, c.g, c.b];
                        ui.label(egui::RichText::new("colour").color(theme::INK_DIM).size(11.0));
                        if ui.color_edit_button_srgb(&mut rgb).changed() {
                            edit.style.color = Some(pdf_core::document::Color {
                                r: rgb[0],
                                g: rgb[1],
                                b: rgb[2],
                                a: c.a,
                            });
                        }

                        let (mut x, mut y) = edit.style.at.unwrap_or((0.0, 0.0));
                        ui.label(egui::RichText::new("at").color(theme::INK_DIM).size(11.0));
                        let moved = ui
                            .add(egui::DragValue::new(&mut x).speed(0.5).prefix("x "))
                            .changed()
                            | ui.add(egui::DragValue::new(&mut y).speed(0.5).prefix("y "))
                                .changed();
                        if moved {
                            edit.style.at = Some((x, y));
                        }

                        if ui.button("Apply").clicked() {
                            apply_now = true;
                        }
                    });
                    });
            });

        // Let go: the words go where they were dragged, once.
        if dropped {
            let (by_x, by_y) = self
                .editing_run
                .as_ref()
                .map(|e| e.drag_by)
                .unwrap_or((0.0, 0.0));
            if by_x.abs() > 0.2 || by_y.abs() > 0.2 {
                let (page, object) = self
                    .editing_run
                    .as_ref()
                    .map(|e| (e.page, e.object))
                    .expect("checked above");
                let moved = self.doc.as_ref().map(|doc| {
                    doc.session.move_object(
                        page,
                        object,
                        pdf_core::document::Point { x: by_x, y: by_y },
                    )
                });
                match moved {
                    Some(Ok(())) => {
                        if let Some(edit) = &mut self.editing_run {
                            // The words are there now, so the box is too, and
                            // the drag starts again from nothing.
                            edit.rect.left += by_x;
                            edit.rect.right += by_x;
                            edit.rect.top += by_y;
                            edit.rect.bottom += by_y;
                            edit.drag_by = (0.0, 0.0);
                        }
                        if let Some(doc) = &mut self.doc {
                            doc.rendered_is_stale();
                        }
                        self.text = None;
                        self.say_info(format!("moved the words on page {}.", page + 1));
                    }
                    Some(Err(e)) => {
                        if let Some(edit) = &mut self.editing_run {
                            edit.drag_by = (0.0, 0.0);
                        }
                        self.say_error(e.to_string());
                    }
                    None => {}
                }
            } else if let Some(edit) = &mut self.editing_run {
                edit.drag_by = (0.0, 0.0);
            }
        }

        if done || apply_now {
            self.apply_edited_run();
        }
    }

    /// A padlock over every sealed object on this page, and the click that
    /// brings one back.
    ///
    /// **The badge is the only thing that says a lock is there.** What was
    /// removed leaves a gap, and a gap on a page reads as a design choice
    /// rather than as something hidden — so the mark has to be visible, has to
    /// sit where the thing was, and has to be the way back.
    fn draw_lock_badges(&mut self, ui: &mut egui::Ui, page: usize, view: PageView) {
        let items = self.locked_items_on(page);
        if items.is_empty() {
            return;
        }

        // Where this page's words are, on screen.
        //
        // **The chequerboard must not cover them.** A full-page photo with a
        // caption over it is an ordinary layout, and filling the image's whole
        // rectangle hid text that was never locked — it looked, exactly as it
        // was reported, like locking the picture had taken the words with it.
        // The words are still on the page; only the picture went.
        let words: Vec<egui::Rect> = self
            .characters(page)
            .map(|chars| {
                chars
                    .line_rects(0..chars.len())
                    .into_iter()
                    .map(|r| {
                        egui::Rect::from_min_max(
                            view.to_screen(AppPoint::new(r.left as f64, r.top as f64)),
                            view.to_screen(AppPoint::new(r.right as f64, r.bottom as f64)),
                        )
                        // A little room, so a square does not clip an ascender.
                        .expand(2.0)
                    })
                    .collect()
            })
            .unwrap_or_default();

        let painter = ui.painter();
        let mut asked: Option<String> = None;

        for item in &items {
            let min = view.to_screen(AppPoint::new(item.rect.left as f64, item.rect.top as f64));
            let max = view.to_screen(AppPoint::new(item.rect.right as f64, item.rect.bottom as f64));
            let area = egui::Rect::from_min_max(min, max);

            // A chequerboard, which every editor uses for "nothing here" — the
            // gap reads as deliberate emptiness rather than as a design
            // choice, and it is unmistakably not part of the document.
            //
            // Clipped to the area and drawn from its own top-left rather than
            // the screen's, so the squares hold still while the page scrolls
            // instead of the pattern swimming under a stationary image.
            // A locked *area* was redacted, so the page already carries the
            // black mark that says so. Drawing anything over it would be a
            // second answer to the same question — and would hide the very
            // thing it was trying to explain. Only the padlock goes on top.
            if !item.is_area {
                let squares = painter.with_clip_rect(area.intersect(painter.clip_rect()));

                // **The grid is laid out in page points, not screen pixels.**
                //
                // Reported from use: zooming made bits of the pattern appear
                // and disappear beside the caption. A screen-space grid moves
                // relative to the page as the zoom changes, so which squares
                // fall on a word keeps changing — the flicker was the
                // text-avoidance being recomputed against a moving grid. In
                // page space the answer is the same at every zoom.
                const SQUARE_PT: f32 = 6.0;
                let step = (SQUARE_PT * view.scale as f32).max(2.0);
                let (across, down) = (
                    (area.width() / step).ceil() as i32,
                    (area.height() / step).ceil() as i32,
                );

                // The words to keep clear, in page points, as one rectangle per
                // line — also zoom-independent, and expanded a little so a
                // square never clips an ascender.
                let clear: Vec<egui::Rect> = words
                    .iter()
                    .copied()
                    .filter(|w| w.intersects(area))
                    .collect();

                for row in 0..down {
                    for column in 0..across {
                        let at = area.min + egui::vec2(column as f32 * step, row as f32 * step);
                        let square =
                            egui::Rect::from_min_size(at, egui::Vec2::splat(step)).intersect(area);
                        // The text keeps its own background, whatever the page
                        // draws behind it — the chequerboard says "the picture
                        // is gone", and painting it over words that are still
                        // there would say something untrue.
                        if clear.iter().any(|w| w.intersects(square)) {
                            continue;
                        }
                        squares.rect_filled(
                            square,
                            egui::CornerRadius::ZERO,
                            if (row + column) % 2 == 0 {
                                theme::CHEQUER_LIGHT
                            } else {
                                theme::CHEQUER_DARK
                            },
                        );
                    }
                }
                painter.rect_stroke(
                    area,
                    egui::CornerRadius::same(2),
                    egui::Stroke::new(1.0, theme::VIOLET),
                    egui::StrokeKind::Inside,
                );
            }

            // A padlock big enough to hit, but never larger than what it marks
            // — a badge overflowing a small image would cover its neighbours.
            let size = 26.0_f32.min(area.width() * 0.8).min(area.height() * 0.8).max(12.0);
            let badge = egui::Rect::from_center_size(area.center(), egui::vec2(size, size));
            painter.rect_filled(badge, egui::CornerRadius::same(4), theme::VIOLET);
            painter.text(
                badge.center(),
                egui::Align2::CENTER_CENTER,
                "\u{E899}",
                icon_font(size * 0.62),
                egui::Color32::WHITE,
            );

            let hit = ui.interact(
                badge,
                egui::Id::new(("lock-badge", page, item.id.as_str())),
                egui::Sense::click(),
            );
            if hit.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if hit.clicked() {
                asked = Some(item.id.clone());
            }
        }

        if let Some(id) = asked {
            self.awaiting_password = Some(Awaiting::UnlockItem(id));
            self.say_info("type the passcode this was locked with, or Escape to give up.");
        }
    }

    fn draw_text_highlights(&mut self, painter: &egui::Painter, page: usize, view: PageView) {
        // Only on the page it was made on, now that a selection outlives the
        // page being scrolled past.
        let selection =
            (page == self.selection_page).then(|| self.text_selection.clone()).flatten();
        let hits: Vec<std::ops::Range<usize>> = self
            .find_hits
            .iter()
            .filter(|(p, _)| *p == page)
            .map(|(_, range)| range.clone())
            .collect();
        let current = self.find_hits.get(self.find_at).cloned();

        let Some(chars) = self.characters(page) else { return };

        let wash = |range: std::ops::Range<usize>, colour: egui::Color32| {
            for rect in chars.line_rects(range) {
                let min = view.to_screen(AppPoint::new(rect.left as f64, rect.top as f64));
                let max = view.to_screen(AppPoint::new(rect.right as f64, rect.bottom as f64));
                painter.rect_filled(
                    egui::Rect::from_min_max(min, max),
                    egui::CornerRadius::same(1),
                    colour,
                );
            }
        };

        for hit in hits {
            let is_current = current
                .as_ref()
                .is_some_and(|(p, r)| *p == page && *r == hit);
            // The one you are on is brighter, so stepping through is visible
            // without reading the count in the command bar.
            wash(
                hit,
                if is_current {
                    egui::Color32::from_rgba_unmultiplied(0xFF, 0xC1, 0x07, 150)
                } else {
                    egui::Color32::from_rgba_unmultiplied(0xFF, 0xC1, 0x07, 70)
                },
            );
        }

        if let Some(range) = selection {
            wash(range, egui::Color32::from_rgba_unmultiplied(0x4C, 0xC9, 0xF0, 90));
        }
    }

    fn interact(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        view: PageView,
        page: usize,
        command_id: egui::Id,
    ) {
        let response = ui.interact(rect, egui::Id::new(("page", page)), egui::Sense::click_and_drag());

        // Right-click, which is where a reader looks for Copy first.
        //
        // **Before the early return, not after it.** Moving the pointer towards
        // the menu takes it off the page, so the page stops being hovered, so
        // the function returned before re-declaring the menu — and the menu
        // vanished as you reached for it. A popup has to be offered on every
        // frame it is open, including the frames where the pointer has left the
        // widget that opened it.
        // What the pointer is over, kept before the menu opens: a right-click
        // is a press and a release, and the menu is built on a later frame than
        // the one that knew where the pointer was.
        if response.secondary_clicked() {
            if let Some(spot) = response.interact_pointer_pos() {
                let at = view.to_page(spot);
                self.selected_image = self
                    .images_on(page)
                    .into_iter()
                    .find(|i| {
                        at.x >= i.rect.left as f64
                            && at.x <= i.rect.right as f64
                            && at.y >= i.rect.top as f64
                            && at.y <= i.rect.bottom as f64
                    })
                    .map(|i| (page, i));
            }
        }

        let over_text = self.text_selection.is_some() && page == self.selection_page;
        let over_image = self.selected_image.as_ref().is_some_and(|(p, _)| *p == page);
        if over_text || over_image {
            response.context_menu(|ui| {
                if over_text && ui.button("Copy").clicked() {
                    self.copy_wanted = true;
                    ui.close();
                }
                // Locking is a Protect operation, so it is offered where the
                // Protect tools are rather than on every tab — the same reason
                // the ribbon has tabs at all.
                if self.ribbon == Tab::Protect {
                    if over_image {
                        if ui.button("🔒 Lock this image").clicked() {
                            if let Some((page, image)) = self.selected_image.clone() {
                                self.awaiting_password =
                                    Some(Awaiting::LockImage { page, object: image.object });
                                self.say_info(
                                    "type a passcode to lock this image with, or Escape to give up.",
                                );
                            }
                            ui.close();
                        }
                    }
                    if over_text && ui.button("🔒 Lock the selection").clicked() {
                        self.lock_selection();
                        ui.close();
                    }
                }
            });
        }

        let Some(pointer) = response.interact_pointer_pos().or_else(|| response.hover_pos()) else {
            self.last_snap = None;
            return;
        };
        let mut at = view.to_page(pointer);

        // Snap, then ortho, then grid — in that order, because a snap is an
        // explicit request for a specific point and must not then be nudged off
        // it by a constraint.
        self.last_snap = None;
        let snapping = self.pending.as_ref().is_some_and(|p| p.kind.wants_snapping());
        if let Some(layer) = self.markup.existing(page).filter(|_| snapping) {
            let radius = HIT_TOLERANCE_PT * 3.0;
            if let Some(snapped) = tools::snap_at(layer, at, radius, self.snaps, None, self.pending.as_ref().and_then(|p| p.points.first().copied())) {
                at = snapped.at;
                self.last_snap = Some(snapped);
            }
        }
        if snapping && self.last_snap.is_none() {
            if self.ortho {
                if let Some(anchor) = self.pending.as_ref().and_then(|p| p.points.last().copied()) {
                    at = tools::orthogonal(anchor, at);
                }
            }
            if self.grid_pt > 0.0 {
                at = tools::to_grid(at, self.grid_pt);
            }
        }

        if let Some(snapped) = &self.last_snap {
            overlay::draw_snap(ui.painter(), view.to_screen(snapped.at), snapped.kind, theme::SNAP);
        }

        // A click is *not* subject to the focus guard, and conflating the two
        // was a bug you could feel: every first click on the page was swallowed
        // to surrender focus, so every tool needed clicking twice to start.
        //
        // The guard exists so that *typing* cannot reach the document — Enter
        // and Delete fired while a number is being entered into a field. A
        // click on the canvas is an unambiguous pointer act aimed at the page,
        // and it also happens to be how someone leaves the command box.
        if response.clicked() {
            ui.memory_mut(|m| m.surrender_focus(command_id));
        }

        // Hand takes the drag before anything else looks at it: in this mode a
        // drag is a scroll, and must not also select.
        //
        // The cursor says so. A lit button in a ribbon of two hundred is not
        // enough of a signal for a mode that stops selection working: the
        // symptom is "text is not selectable any more", which reads as a broken
        // program rather than as the wrong tool being held.
        // The middle button pans whatever tool is held. That is the CAD
        // convention and it costs nothing to honour: it is a button no other
        // gesture here uses, so panning never has to be *selected*, and a tool
        // in progress is not disturbed by moving the paper under it.
        let middle = response.dragged_by(egui::PointerButton::Middle);
        if middle {
            ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::Grabbing);
            self.pan(response.drag_delta());
            return;
        }

        if self.pointer == pagify_shell::verbs::PointerMode::Pan {
            ui.output_mut(|o| {
                o.cursor_icon = if response.dragged() {
                    egui::CursorIcon::Grabbing
                } else {
                    egui::CursorIcon::Grab
                }
            });
            if response.dragged() {
                self.pan(response.drag_delta());
            }
            self.text_drag = None;
            self.drag_from = None;
            if !response.clicked() {
                return;
            }
        }

        // A text cursor wherever there is text under the pointer, which is the
        // other half of the same answer: on a page whose words are drawn as
        // outlines the cursor stays an arrow, and the reason selection does
        // nothing is visible before the drag rather than after it.
        if self.pending.is_none()
            && self
                .characters(page)
                .and_then(|chars| chars.hit(at.x as f32, at.y as f32))
                .is_some()
        {
            ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::Text);
        }

        // An armed tool owns the pointer, and takes the whole gesture.
        //
        // Two defects in one. A drag while a tool was waiting for clicks used
        // to *also* select text, so one gesture did two things at once. And
        // egui calls a press-move-release a drag rather than a click even when
        // the movement is a pixel or two — which mice do constantly — so a
        // click that wandered was thrown away and the tool appeared not to
        // respond. That is what "the tools do not work" looks like from the
        // outside: most clicks land, some vanish, and nothing says why.
        //
        // A gesture that stayed within the same tolerance used for hit-testing
        // is a click, however egui classified it.
        if self.pending.is_some() {
            self.text_drag = None;
            if response.drag_started() {
                self.drag_from = Some(at);
            }
            if response.drag_stopped() {
                if let Some(from) = self.drag_from.take() {
                    let wandered =
                        (from.x - at.x).abs().max((from.y - at.y).abs()) <= HIT_TOLERANCE_PT;
                    if wandered {
                        self.take_pick(at);
                    }
                }
            }
            if response.clicked() {
                self.take_pick(at);
            }
            return;
        }

        if response.drag_started() {
            // A drag that begins **on a character** selects text; one that
            // begins on empty paper selects marks. That is the rule every PDF
            // reader already teaches, and it needs no mode switch — which
            // matters, because a mode the user has to remember is a mode they
            // will be in the wrong one of.
            let on_text = self
                .characters(page)
                .and_then(|chars| chars.hit(at.x as f32, at.y as f32))
                .is_some();

            if on_text {
                self.text_drag = Some(at);
                self.text_selection = None;
                self.drag_from = None;
            } else {
                self.drag_from = Some(at);
                self.text_drag = None;
            }
        }

        if response.dragged() {
            if let Some(from) = self.text_drag {
                self.text_selection = self
                    .characters(page)
                    .and_then(|chars| {
                        chars.range_between(
                            (from.x as f32, from.y as f32),
                            (at.x as f32, at.y as f32),
                        )
                    });
                if self.text_selection.is_some() {
                    self.selection_page = page;
                }
            }
        }

        if response.drag_stopped() {
            // A tool in hand marks what was just selected.
            if let (Some(kind), true) = (self.markup_armed, self.text_drag.is_some()) {
                if self.text_selection.is_some() {
                    self.mark_selection(kind);
                    // Still in hand: a highlighter is not put down after one
                    // sentence. The selection goes, because the mark is now the
                    // thing on the page.
                    self.markup_armed = Some(kind);
                    self.text_selection = None;
                }
            }
            self.text_drag = None;
            if let Some(from) = self.drag_from.take() {
                let height = view_height(self, page);
                let layer = self.markup.page(page, height);
                if (from.x - at.x).abs() > 2.0 || (from.y - at.y).abs() > 2.0 {
                    let n = layer.select_box(from, at, ui.input(|i| i.modifiers.shift));
                    self.say_info(format!("{n} selected."));
                }
            }
        }

        if response.clicked() {
            if self.pending.is_some() {
                self.take_pick(at);
            } else if let Some(n) = self.foreign_at(page, at) {
                // A mark this program did not make. It cannot be reshaped —
                // that would mean reconstructing geometry nobody recorded — but
                // it can be named and taken away, which is the difference
                // between a document you can work on and one you can only look
                // at.
                self.say_info(format!(
                    "annotation {n} on this page — `removemark {n}` takes it off."
                ));
            } else {
                self.text_selection = None;
                let height = view_height(self, page);
                let shift = ui.input(|i| i.modifiers.shift);
                let layer = self.markup.page(page, height);
                layer.select_at(at, HIT_TOLERANCE_PT * 3.0, shift);
            }
        }
    }
}

fn view_height(app: &PagifyApp, page: usize) -> f64 {
    app.doc
        .as_ref()
        .and_then(|d| d.strip.size_of(page))
        .map(|(_, h)| h as f64)
        .unwrap_or(792.0)
}

/// A ribbon tab. Drawn rather than using `selectable_label`, so the chosen one
/// is the solid violet pill the mockup shows rather than a tinted background.
fn tab_button(ui: &mut egui::Ui, label: &str, chosen: bool) -> bool {
    let padding = egui::vec2(14.0, 7.0);
    let galley = ui.painter().layout_no_wrap(
        label.to_string(),
        egui::FontId::proportional(14.0),
        theme::INK,
    );
    let size = galley.size() + padding * 2.0;
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());

    let painter = ui.painter();
    if chosen {
        painter.rect_filled(rect, egui::CornerRadius::same(7), theme::VIOLET);
    } else if response.hovered() {
        painter.rect_filled(rect, egui::CornerRadius::same(7), theme::RAISED);
    }
    let colour = if chosen {
        egui::Color32::WHITE
    } else if response.hovered() {
        theme::INK
    } else {
        theme::INK_DIM
    };
    painter.galley(rect.min + padding, galley, colour);

    response.clicked()
}

struct Keys {
    escape: bool,
    enter: bool,
    up: bool,
    down: bool,
    left: bool,
    right: bool,
    zoom_in: bool,
    zoom_out: bool,
    delete: bool,
    copy: bool,
    find_next: bool,
    open: bool,
    save: bool,
    undo: bool,
    redo: bool,
    search: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        format!(
            "{}/../../../workspace/Pagify/rust/pdf_core/fixtures/{name}",
            env!("CARGO_MANIFEST_DIR")
        )
    }

    #[test]
    fn the_conversion_to_egui_keeps_the_page_the_right_way_up() {
        let session = Session::open(fixture("quadrants.pdf")).expect("open");
        let raster = session.render_page(0, 1.0).expect("render");
        let image = page_to_image(&raster);

        assert_eq!(image.width(), 400);
        assert_eq!(image.height(), 400);

        let (near, far) = (100, 300);
        let top_left = image[(near, near)];
        let top_right = image[(far, near)];
        let bottom_left = image[(near, far)];

        assert!(top_left.r() > top_left.b(), "top-left should be red, got {top_left:?}");
        assert!(top_right.g() > top_right.r(), "top-right should be green");
        assert!(bottom_left.b() > bottom_left.r(), "bottom-left should be blue");
    }

    #[test]
    fn a_page_uploads_as_a_texture() {
        let session = Session::open(fixture("text-lines.pdf")).expect("open");
        let raster = session.render_page(0, 2.0).expect("render");
        let ctx = egui::Context::default();
        let handle = ctx.load_texture("page", page_to_image(&raster), egui::TextureOptions::LINEAR);
        assert_eq!(handle.size(), [raster.width as usize, raster.height as usize]);
    }

    #[test]
    fn a_rotated_page_renders_at_the_swapped_size() {
        let session = Session::open(fixture("mixed-sizes.pdf")).expect("open");
        let upright = session.render_page(0, 1.0).expect("render");
        let turned = session
            .render_page_rotated(0, 1.0, Rotation::Clockwise90)
            .expect("render rotated");
        assert_eq!((turned.width, turned.height), (upright.height, upright.width));
        let _ = page_to_image(&turned);
    }

    /// The whole tool chain, driven the way a script would drive it.
    ///
    /// These exist because the tools were broken in a way no shell test could
    /// see: the geometry functions were all correct and individually tested,
    /// and the *app* was choosing the wrong one — deciding the operation from
    /// the prompt text, so Fillet performed a Move. The defect lived entirely
    /// in the glue between command and operation, which is exactly what these
    /// exercise.
    mod tools_chain {
        use super::*;
        use cad_kernel::Geom;

        fn app() -> PagifyApp {
            let app = PagifyApp::new(Some(&fixture("pages-ladder.pdf")));
            assert!(app.doc.is_some(), "the fixture did not open");
            app
        }

        fn marks(app: &PagifyApp) -> &[cad_kernel::DObject] {
            app.markup.existing(0).map(|l| l.objects()).unwrap_or(&[])
        }

        fn errors(app: &PagifyApp) -> Vec<String> {
            app.cmd
                .history()
                .iter()
                .filter(|e| e.kind == Kind::Error)
                .map(|e| e.text.clone())
                .collect()
        }

        /// **The acceptance test for the whole thing: nothing of the ink is
        /// left to draw.**
        ///
        /// The two tests below check the mechanism — the layer stops painting,
        /// and new marks are refused. This checks the *outcome* the person
        /// actually sees: draw, lock, then rasterise the page at the size the
        /// thumbnail strip draws it and count pixels of the ink's own colour.
        ///
        /// Asserted on the colour rather than on how much of the page is dark,
        /// because a locked page is blank either way — "less ink" would pass
        /// whether or not the stroke went.
        #[test]
        fn a_locked_page_has_no_trace_of_the_ink_drawn_on_it() {
            /// Pixels of the markup ink's own colour, at thumbnail size.
            fn ink_pixels(app: &PagifyApp) -> usize {
                let doc = app.doc.as_ref().expect("doc");
                let (width_pt, _) = doc.strip.size_of(0).expect("size");
                // 140px wide, which is what the strip draws.
                let raster = doc
                    .session
                    .render_page(0, 140.0 / width_pt)
                    .expect("render the page");
                let (r, g, b) = (MARKUP_INK.r, MARKUP_INK.g, MARKUP_INK.b);
                raster
                    .pixels
                    .chunks_exact(4)
                    .filter(|p| {
                        // Near the ink colour, allowing for antialiasing.
                        (p[0] as i16 - r as i16).abs() < 60
                            && (p[1] as i16 - g as i16).abs() < 60
                            && (p[2] as i16 - b as i16).abs() < 60
                    })
                    .count()
            }

            let mut app = app();
            app.submit("l 100,100 300,300");
            // Into the document, but **not** to disk: `save(None)` writes over
            // the fixture, which every other test in this module then reads.
            app.commit_marks_on(&[0]).expect("commit the ink");
            let drawn = ink_pixels(&app);
            assert!(drawn > 0, "the ink never reached the page, so this proves nothing");

            app.lock_pages(&[0], b"a good passcode").expect("lock");
            let left = ink_pixels(&app);
            println!("thumbnail-sized render: {drawn} ink pixels drawn, {left} after locking");
            assert_eq!(left, 0, "the locked page still draws {drawn} pixels of ink");
        }

        /// **Ink drawn before locking is sealed with the page, not left on top
        /// of it.**
        ///
        /// Reported from use: a locked page still showed pen strokes. Drawn
        /// marks live in this layer until the document is saved, so locking —
        /// which seals the page *as the document has it* — sealed a page that
        /// did not include them, and the layer went on painting them over the
        /// blank result.
        #[test]
        fn locking_a_page_takes_the_ink_drawn_on_it() {
            let mut app = app();
            app.submit("l 100,100 300,300");
            assert!(!marks(&app).is_empty(), "nothing was drawn, so this proves nothing");

            app.lock_pages(&[0], b"a good passcode").expect("lock");

            assert!(
                marks(&app).is_empty(),
                "the drawn ink is still being painted over the locked page"
            );
        }

        /// And nothing new can be drawn on it afterwards — said at the moment
        /// of drawing, not when the document is eventually saved.
        #[test]
        fn a_locked_page_refuses_to_be_drawn_on() {
            let mut app = app();
            app.lock_pages(&[0], b"a good passcode").expect("lock");

            app.submit("l 100,100 300,300");
            assert!(
                errors(&app).iter().any(|e| e.contains("locked")),
                "it did not say why: {:?}",
                errors(&app)
            );
            assert!(marks(&app).is_empty(), "it drew on a locked page anyway");
        }

        #[test]
        fn typed_coordinates_draw() {
            let mut app = app();
            app.submit("l 30,250 170,250");
            app.submit("ci 100,400 40");
            assert_eq!(marks(&app).len(), 2, "errors: {:?}", errors(&app));
        }

        #[test]
        fn a_tool_plus_typed_picks_draws() {
            // `line` arms the tool; the picks are the clicks.
            let mut app = app();
            app.submit("line");
            app.submit("pick 20,20");
            app.submit("pick 120,120");
            assert_eq!(marks(&app).len(), 1, "errors: {:?}", errors(&app));
            assert!(matches!(marks(&app)[0].geom, Geom::Line(_)));
        }

        #[test]
        fn a_polyline_collects_until_it_is_resolved() {
            let mut app = app();
            app.submit("pline");
            for p in ["pick 10,10", "pick 60,10", "pick 60,60"] {
                app.submit(p);
            }
            assert!(marks(&app).is_empty(), "a polyline must not commit early");
            app.resolve();
            match &marks(&app)[0].geom {
                Geom::Polyline(pl) => assert_eq!(pl.vertices.len(), 3),
                other => panic!("{other:?}"),
            }
        }

        /// The reported bug, end to end.
        #[test]
        fn fillet_fillets_rather_than_moving() {
            let mut app = app();
            app.submit("l 30,250 170,250");
            app.submit("l 170,250 170,350");
            let before: Vec<_> = marks(&app).iter().map(|o| o.bbox()).collect();

            app.submit("fillet 25");
            app.submit("pick 100,250");
            app.submit("pick 170,320");

            assert!(
                marks(&app).iter().any(|o| matches!(o.geom, Geom::Arc(_))),
                "no arc — fillet did not fillet. errors: {:?}",
                errors(&app)
            );
            assert_ne!(
                marks(&app).len(),
                before.len(),
                "nothing was added; a move would have left the count alone"
            );
        }

        #[test]
        fn trim_cuts_and_stays_armed_for_the_next_piece() {
            let mut app = app();
            app.submit("l 0,250 200,250");
            app.submit("l 100,200 100,300");
            app.submit("trim");
            app.submit("pick 180,250");

            assert!(errors(&app).is_empty(), "errors: {:?}", errors(&app));
            assert!(
                app.pending.is_some(),
                "trim is a repeating tool and must still be armed"
            );
        }

        #[test]
        fn offset_takes_an_object_and_then_a_side() {
            let mut app = app();
            app.submit("l 30,250 170,250");
            app.submit("offset 15");
            app.submit("pick 100,250"); // the object
            app.submit("pick 100,300"); // which side

            assert_eq!(marks(&app).len(), 2, "errors: {:?}", errors(&app));
        }

        #[test]
        fn move_needs_a_selection_and_says_so() {
            let mut app = app();
            app.submit("l 30,250 170,250");
            app.submit("move");
            assert!(app.pending.is_none(), "move armed with nothing selected");
            assert!(
                errors(&app).iter().any(|e| e.contains("selected")),
                "no explanation: {:?}",
                errors(&app)
            );
        }

        #[test]
        fn move_moves_once_something_is_selected() {
            let mut app = app();
            app.submit("l 30,250 170,250");
            app.submit("all");
            app.submit("move");
            app.submit("pick 0,0");
            app.submit("pick 0,100");

            match &marks(&app)[0].geom {
                Geom::Line(l) => assert!((l.a.y - 350.0).abs() < 1e-6, "moved to {}", l.a.y),
                other => panic!("{other:?}"),
            }
        }

        #[test]
        fn measuring_reports_a_distance_and_calibration_changes_it() {
            let mut app = app();
            app.submit("measure distance");
            app.submit("pick 0,0");
            app.submit("pick 100,0");
            // The tool stays in hand, so the last line is its fresh prompt and
            // the answer is the one before it.
            let said = app.cmd.history()[app.cmd.history().len() - 2].text.clone();
            assert!(said.contains("100"), "uncalibrated distance wrong: {said}");
            assert!(said.contains("not calibrated"), "should warn: {said}");

            app.escape();
            app.submit("calibrate 5 m");
            app.submit("pick 0,0");
            app.submit("pick 100,0");
            app.submit("measure distance");
            app.submit("pick 0,0");
            app.submit("pick 200,0");
            let said = app.cmd.history()[app.cmd.history().len() - 2].text.clone();
            assert!(said.contains("10.000 m"), "calibrated distance wrong: {said}");
        }

        #[test]
        fn escape_style_cancellation_leaves_nothing_half_done() {
            let mut app = app();
            app.submit("fillet");
            app.submit("pick 100,250"); // misses — nothing there
            assert!(app.pending.is_some(), "a miss must not cancel the tool");
            assert!(marks(&app).is_empty());
        }
    }

    /// Not a test — a listing. Run with
    /// `cargo test -p pagify_app font_coverage -- --nocapture --ignored`
    /// to see which characters the bundled fonts can actually draw, which is
    /// the only honest way to choose an icon set for them.
    #[test]
    #[ignore]
    fn font_coverage() {
        let ctx = egui::Context::default();
        let mut frame = ctx.run_ui(Default::default(), |_| {});
        frame.textures_delta.clear();

        let font = egui::FontId::proportional(19.0);
        let ranges: &[(&str, u32, u32)] = &[
            ("latin symbols", 0x2000, 0x206F),
            ("letterlike", 0x2100, 0x214F),
            ("arrows", 0x2190, 0x21FF),
            ("maths", 0x2200, 0x22FF),
            ("technical", 0x2300, 0x23FF),
            ("enclosed", 0x2460, 0x24FF),
            ("box drawing", 0x2500, 0x257F),
            ("blocks", 0x2580, 0x259F),
            ("geometric", 0x25A0, 0x25FF),
            ("misc symbols", 0x2600, 0x26FF),
            ("dingbats", 0x2700, 0x27BF),
            ("supplemental arrows", 0x2900, 0x297F),
            ("misc symbols b", 0x2B00, 0x2BFF),
            ("emoji: misc", 0x1F300, 0x1F5FF),
            ("emoji: transport", 0x1F680, 0x1F6FF),
        ];

        ctx.fonts_mut(|fonts| {
            for (name, from, to) in ranges {
                let have: String = (*from..=*to)
                    .filter_map(char::from_u32)
                    .filter(|c| fonts.has_glyph(&font, *c))
                    .collect();
                println!("\n== {name} ({} of {}) ==\n{have}", have.chars().count(), to - from + 1);
            }
        });
    }

    /// Every ribbon glyph must be a glyph the bundled fonts can draw.
    ///
    /// The failure this catches is silent and total: a character egui has no
    /// font for renders as an empty box, so a missing glyph does not look like
    /// a missing glyph — it looks like a broken button, on a toolbar of two
    /// hundred where nobody would notice which one. Emoji coverage is a
    /// property of the fonts egui bundles, not of the platform, so it can be
    /// asserted here rather than discovered by looking at a screen.
    #[test]
    fn every_ribbon_glyph_can_actually_be_drawn() {
        let ctx = egui::Context::default();
        // Fonts are built lazily on the first frame; without one there is
        // nothing to ask. The texture delta has to be taken rather than
        // dropped — epaint panics on an unhandled one, on the reasoning that a
        // real backend forgetting to upload it would be a bug.
        let mut frame = ctx.run_ui(Default::default(), |_| {});
        frame.textures_delta.clear();

        install_icons(&ctx);
        let mut frame = ctx.run_ui(Default::default(), |_| {});
        frame.textures_delta.clear();

        let font = icon_font(19.0);
        let mut missing: Vec<String> = Vec::new();

        ctx.fonts_mut(|fonts| {
            for tab in Tab::ALL {
                for (glyph, label, _) in tab.leading().iter().chain(tab.buttons()) {
                    for ch in glyph.chars() {
                        if !fonts.has_glyph(&font, ch) {
                            missing.push(format!(
                                "{}/{label}: U+{:04X} {ch:?}",
                                tab.label(),
                                ch as u32
                            ));
                        }
                    }
                }
            }
        });

        missing.sort();
        missing.dedup();
        assert!(
            missing.is_empty(),
            "{} glyphs would render as empty boxes:\n  {}",
            missing.len(),
            missing.join("\n  ")
        );
    }

    /// Every ribbon button must be a command the box actually understands.
    ///
    /// This is what keeps §7's claim true as buttons are added: a button whose
    /// command string is a typo is a button that does nothing, and nothing else
    /// would catch it.
    #[test]
    fn every_ribbon_button_runs_a_command_the_box_understands() {
        for tab in Tab::ALL {
            for (_glyph, label, command) in tab.leading().iter().chain(tab.buttons()) {
                let line = command.trim();
                // A trailing space means the button pre-fills the box for the
                // user to complete — those are checked as their bare verb.
                match pagify_shell::command::dispatch(line) {
                    Some(Dispatch::Unknown(token)) => {
                        panic!("{}/{label} runs `{command}`, and `{token}` is not a command", tab.label())
                    }
                    Some(Dispatch::Refused { token, .. }) => {
                        panic!("{}/{label} runs `{command}`, which Pagify refuses ({token})", tab.label())
                    }
                    _ => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod text_layer_tests {
    use super::*;

    fn fixture(name: &str) -> String {
        format!(
            "{}/../../../workspace/Pagify/rust/pdf_core/fixtures/{name}",
            env!("CARGO_MANIFEST_DIR")
        )
    }

    fn said(app: &PagifyApp) -> String {
        app.cmd.history().iter().map(|e| e.text.as_str()).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn a_page_with_no_text_says_so_on_opening() {
        // The whole of phase 1: a silent, confusing failure becomes an
        // explained one. `single-page.pdf` has no content stream at all.
        let mut app = PagifyApp::new(Some(&fixture("single-page.pdf")));
        assert!(app.doc.is_some());
        assert!(
            said(&app).contains("nothing on it to select"),
            "the reader was left to guess why selection does nothing:\n{}",
            said(&app)
        );
        let _ = &mut app;
    }

    #[test]
    fn a_page_with_text_does_not_nag() {
        // Said only when something is wrong. A note on every page turn would
        // be noise, and noise is how real warnings get ignored.
        let app = PagifyApp::new(Some(&fixture("text-lines.pdf")));
        assert!(
            !said(&app).contains("no text layer"),
            "a perfectly good page was warned about:\n{}",
            said(&app)
        );
    }

    #[test]
    fn textlayer_answers_either_way_and_shows_its_working() {
        let mut app = PagifyApp::new(Some(&fixture("text-lines.pdf")));
        app.submit("textlayer");
        let output = said(&app);

        assert!(output.contains("selectable text"), "no verdict:\n{output}");
        assert!(
            output.contains("characters"),
            "the counts behind the verdict were not shown — a threshold nobody \
             can see is one nobody can argue with:\n{output}"
        );
    }
}

#[cfg(test)]
mod unsaved_guard_tests {
    use super::*;

    fn fixture(name: &str) -> String {
        format!(
            "{}/../../../workspace/Pagify/rust/pdf_core/fixtures/{name}",
            env!("CARGO_MANIFEST_DIR")
        )
    }

    /// A **copy** of a fixture, in a scratch directory.
    ///
    /// These tests call `save`, and `save` with no destination writes back to
    /// the file that is open — which is the whole point of it. Pointed at a
    /// fixture, that truncates the fixture: `single-page.pdf` was reduced to
    /// zero bytes by this suite before the copy was introduced, and every test
    /// in the workspace that touched it started failing at once.
    ///
    /// A test that writes must never be given the original.
    fn scratch_copy(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("pagify-app-tests");
        std::fs::create_dir_all(&dir).expect("scratch dir");

        // Named per test thread so two running at once cannot share a file.
        let unique = format!("{:?}-{name}", std::thread::current().id())
            .replace(['(', ')', ' '], "");
        let path = dir.join(unique);
        std::fs::copy(fixture(name), &path).expect("stage a copy of the fixture");
        path
    }

    fn app() -> PagifyApp {
        let path = scratch_copy("single-page.pdf");
        let app = PagifyApp::new(Some(&path.to_string_lossy()));
        assert!(
            app.doc.is_some(),
            "fixture did not open: {path:?}\nhistory:\n{}",
            app.cmd.history().iter().map(|e| e.text.as_str()).collect::<Vec<_>>().join("\n")
        );
        app
    }

    #[test]
    fn a_clean_document_closes_without_argument() {
        let mut app = app();
        app.submit("close");
        assert!(app.doc.is_none(), "an unmarked document refused to close");
    }

    /// The only data-loss path in the program.
    #[test]
    fn closing_a_marked_up_document_stops_to_ask() {
        let mut app = app();
        app.submit("l 30,30 200,200");
        assert_eq!(app.markup.existing(0).map(|l| l.len()), Some(1));

        app.submit("close");
        assert!(app.doc.is_some(), "the document closed and took the marks with it");
        assert_eq!(
            app.closing,
            Some(Closing::Document),
            "it neither closed nor asked, which leaves the user with nothing to act on"
        );
    }

    #[test]
    /// What the dialog is built from has to be true, because it is the only
    /// thing the user is shown before deciding.
    fn the_question_can_say_exactly_what_would_be_lost() {
        let mut app = app();
        app.submit("l 0,0 10,10");
        app.submit("l 0,20 10,30");
        app.submit("close");

        assert_eq!(app.closing, Some(Closing::Document));
        assert_eq!(
            app.unsaved(),
            Some((2, 1)),
            "the count behind the question is wrong, so the question would be a lie"
        );
    }

    #[test]
    fn the_bang_form_discards_deliberately() {
        let mut app = app();
        app.submit("l 30,30 200,200");
        app.submit("close!");
        assert!(app.doc.is_none(), "`close!` did not discard");
    }

    #[test]
    fn opening_another_document_is_guarded_too() {
        // It drops the current document's markup just as surely as closing it.
        let mut app = app();
        app.submit("l 30,30 200,200");
        app.submit(&format!("open \"{}\"", scratch_copy("text-lines.pdf").display()));

        assert!(
            app.doc.as_ref().unwrap().session.path().to_string_lossy().contains("single-page"),
            "the document was replaced and its marks discarded"
        );
    }

    #[test]
    fn a_moved_mark_still_counts_as_unsaved() {
        // The case a count of objects would miss: the drawing changed, the
        // number of things in it did not.
        let mut app = app();
        app.submit("l 30,30 200,200");
        app.submit("save");
        assert!(
            app.unsaved().is_none(),
            "a saved document still reports unsaved work. history:\n{}",
            app.cmd.history().iter().map(|e| e.text.as_str()).collect::<Vec<_>>().join("\n")
        );

        app.submit("all");
        app.submit("move");
        app.submit("pick 0,0");
        app.submit("pick 0,50");

        assert!(app.unsaved().is_some(), "moving a mark did not register as unsaved");
    }

    #[test]
    fn saving_clears_the_debt() {
        let mut app = app();
        app.submit("l 30,30 200,200");
        assert!(app.unsaved().is_some());

        app.submit("save");
        assert!(app.unsaved().is_none(), "a save did not clear the unsaved state");

        app.submit("close");
        assert!(app.doc.is_none(), "a saved document still refused to close");
    }
}

#[cfg(test)]
mod reading_tests {
    use super::*;

    fn fixture(name: &str) -> String {
        format!(
            "{}/../../../workspace/Pagify/rust/pdf_core/fixtures/{name}",
            env!("CARGO_MANIFEST_DIR")
        )
    }

    fn said(app: &PagifyApp) -> String {
        app.cmd.history().iter().map(|e| e.text.as_str()).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn find_reports_what_it_found_and_lands_on_the_first_match() {
        let mut app = PagifyApp::new(Some(&fixture("text-lines.pdf")));
        app.submit("find fox");

        assert_eq!(app.find_hits.len(), 1, "history:\n{}", said(&app));
        assert_eq!(app.find_at, 0);
        // The match is also the selection, so ⌘C copies what was found.
        assert!(app.text_selection.is_some(), "the match was not selected");
    }

    #[test]
    fn a_match_selects_exactly_the_words_searched_for() {
        let mut app = PagifyApp::new(Some(&fixture("text-lines.pdf")));
        app.submit("find brown");

        let range = app.text_selection.clone().expect("a selection");
        let found = app.characters(0).expect("characters").text_of(range);
        assert_eq!(found, "brown", "the highlight is over the wrong characters");
    }

    #[test]
    fn search_is_case_insensitive() {
        let mut app = PagifyApp::new(Some(&fixture("text-lines.pdf")));
        app.submit("find QUICK");
        assert_eq!(app.find_hits.len(), 1, "history:\n{}", said(&app));
    }

    #[test]
    fn stepping_wraps_round_rather_than_stopping_dead() {
        // Two columns of prose, so there is more than one of a common word.
        let mut app = PagifyApp::new(Some(&fixture("two-column.pdf")));
        app.submit("find the");
        let count = app.find_hits.len();
        assert!(count > 2, "only {count} matches — is the fixture right?");

        for _ in 0..count {
            app.submit("findnext");
        }
        assert_eq!(app.find_at, 0, "stepping through every match did not wrap");

        app.submit("findprev");
        assert_eq!(app.find_at, count - 1, "stepping back from the first did not wrap");
    }

    #[test]
    fn a_search_that_finds_nothing_says_so_and_changes_nothing() {
        let mut app = PagifyApp::new(Some(&fixture("text-lines.pdf")));
        app.submit("find zzzznotpresent");

        assert!(app.find_hits.is_empty());
        assert!(app.text_selection.is_none(), "a failed search left a selection behind");
        assert!(said(&app).contains("no matches"), "no explanation:\n{}", said(&app));
    }

    #[test]
    fn stepping_before_searching_explains_rather_than_doing_nothing() {
        let mut app = PagifyApp::new(Some(&fixture("text-lines.pdf")));
        app.submit("findnext");
        assert!(said(&app).contains("find"), "no guidance:\n{}", said(&app));
    }

    #[test]
    fn find_crosses_pages_and_jumps_to_the_page_the_match_is_on() {
        let mut app = PagifyApp::new(Some(&fixture("pages-ladder.pdf")));
        // No text in that fixture at all — the honest result is nothing found,
        // and crucially not a panic or a jump to a page that has no match.
        app.submit("find anything");
        assert!(app.find_hits.is_empty());
        assert_eq!(app.page, 0);
    }

    #[test]
    fn find_needs_something_to_search() {
        let mut app = PagifyApp::new(Some(&fixture("text-lines.pdf")));
        app.submit("find");
        assert!(said(&app).contains("usage"), "no usage shown:\n{}", said(&app));
    }
}

#[cfg(test)]
mod undo_wiring_tests {
    use super::*;

    fn fixture(name: &str) -> String {
        format!(
            "{}/../../../workspace/Pagify/rust/pdf_core/fixtures/{name}",
            env!("CARGO_MANIFEST_DIR")
        )
    }

    fn app() -> PagifyApp {
        let app = PagifyApp::new(Some(&fixture("single-page.pdf")));
        assert!(app.doc.is_some());
        app
    }

    fn said(app: &PagifyApp) -> String {
        app.cmd.history().iter().map(|e| e.text.as_str()).collect::<Vec<_>>().join("\n")
    }

    fn marks(app: &PagifyApp) -> usize {
        app.markup.existing(app.page).map(|l| l.len()).unwrap_or(0)
    }

    /// The defect this fixes: the app answered "nothing to undo" while three
    /// freshly drawn lines sat on the page.
    #[test]
    fn undo_takes_back_a_drawn_mark() {
        let mut app = app();
        app.submit("l 10,10 100,100");
        app.submit("l 10,50 100,150");
        assert_eq!(marks(&app), 2);

        app.submit("undo");
        assert_eq!(marks(&app), 1, "undo did nothing:\n{}", said(&app));
        app.submit("undo");
        assert_eq!(marks(&app), 0);
    }

    #[test]
    fn redo_puts_it_back() {
        let mut app = app();
        app.submit("l 10,10 100,100");
        app.submit("undo");
        assert_eq!(marks(&app), 0);

        app.submit("redo");
        assert_eq!(marks(&app), 1, "redo did nothing:\n{}", said(&app));
    }

    #[test]
    fn a_fillet_undoes_in_one_step() {
        let mut app = app();
        app.submit("l 30,250 170,250");
        app.submit("l 170,250 170,350");
        let before = marks(&app);

        app.submit("fillet 25");
        app.submit("pick 100,250");
        app.submit("pick 170,320");
        assert!(marks(&app) > before, "the fillet did not run:\n{}", said(&app));

        app.submit("undo");
        assert_eq!(marks(&app), before, "the fillet came back in pieces");
    }

    #[test]
    fn a_move_undoes_to_where_it_was() {
        let mut app = app();
        app.submit("l 0,100 100,100");
        app.submit("all");
        app.submit("move");
        app.submit("pick 0,0");
        app.submit("pick 0,50");

        let moved = match &app.markup.existing(0).unwrap().objects()[0].geom {
            cad_kernel::Geom::Line(l) => l.a.y,
            _ => unreachable!(),
        };
        assert!((moved - 150.0).abs() < 1e-6, "the move did not happen: y={moved}");

        app.submit("undo");
        let back = match &app.markup.existing(0).unwrap().objects()[0].geom {
            cad_kernel::Geom::Line(l) => l.a.y,
            _ => unreachable!(),
        };
        assert!((back - 100.0).abs() < 1e-6, "the move was not undone: y={back}");
    }

    #[test]
    fn with_nothing_to_undo_it_says_so_honestly() {
        let mut app = app();
        app.submit("undo");
        assert!(
            said(&app).contains("nothing to undo"),
            "no honest answer:\n{}",
            said(&app)
        );
    }

    #[test]
    fn an_operation_that_did_nothing_leaves_no_step() {
        // Undoing "nothing happened" looks broken, because nothing moves.
        let mut app = app();
        app.submit("l 10,10 100,100");
        app.submit("erase"); // nothing selected
        app.submit("undo");

        assert_eq!(marks(&app), 0, "undo hit an empty step first:\n{}", said(&app));
    }
}

/// The two things a user does with a mouse: draw with the tools, and select
/// text. Both were only ever exercised through the command box, where every
/// point arrives as typed coordinates — which is not how anybody uses them.
#[cfg(test)]
mod pointer_tests {
    use super::*;

    fn fixture(name: &str) -> String {
        format!(
            "{}/../../../workspace/Pagify/rust/pdf_core/fixtures/{name}",
            env!("CARGO_MANIFEST_DIR")
        )
    }

    fn app(name: &str) -> PagifyApp {
        let app = PagifyApp::new(Some(&fixture(name)));
        assert!(app.doc.is_some(), "{name} did not open");
        app
    }

    fn at(x: f64, y: f64) -> AppPoint {
        AppPoint { x, y }
    }

    fn marks(app: &PagifyApp) -> usize {
        app.markup.existing(app.page).map(|l| l.len()).unwrap_or(0)
    }

    fn said(app: &PagifyApp) -> String {
        app.cmd.history().iter().map(|e| e.text.as_str()).collect::<Vec<_>>().join("\n")
    }

    // -- drawing ----------------------------------------------------------

    /// Pick the tool, then click twice. This is the whole interaction, and
    /// nothing tested it: every drawing test until now typed both coordinates
    /// on one line, which skips arming, picking and the readiness check.
    #[test]
    fn the_line_tool_draws_from_two_clicks() {
        let mut app = app("single-page.pdf");
        app.submit("line");
        assert!(app.pending.is_some(), "line did not arm:\n{}", said(&app));

        app.take_pick(at(10.0, 10.0));
        assert_eq!(marks(&app), 0, "one click drew a line");
        app.take_pick(at(100.0, 100.0));

        assert_eq!(marks(&app), 1, "two clicks drew nothing:\n{}", said(&app));
        // **Still in hand.** A tool is something you pick up and keep using;
        // one that lets go after a single line means going back to the ribbon
        // between every line, and a box becomes four trips.
        assert!(app.pending.is_some(), "the line tool let go after one line");

        app.take_pick(at(120.0, 10.0));
        app.take_pick(at(200.0, 90.0));
        assert_eq!(marks(&app), 2, "the second line needed the tool choosing again");

        app.escape();
        assert!(app.pending.is_none(), "escape did not put the tool down");
    }

    #[test]
    fn the_circle_tool_draws_from_two_clicks() {
        let mut app = app("single-page.pdf");
        app.submit("circle");
        app.take_pick(at(80.0, 80.0));
        app.take_pick(at(120.0, 80.0));
        assert_eq!(marks(&app), 1, "circle drew nothing:\n{}", said(&app));
    }

    /// A polyline has no fixed number of points, so it ends on a command
    /// rather than on a count. If that is wrong it never finishes.
    #[test]
    fn a_polyline_takes_any_number_of_clicks_and_then_finishes() {
        let mut app = app("single-page.pdf");
        app.submit("pline");
        for p in [(10.0, 10.0), (60.0, 30.0), (110.0, 10.0), (160.0, 40.0)] {
            app.take_pick(at(p.0, p.1));
        }
        assert_eq!(marks(&app), 0, "a polyline finished on its own");

        // The typed form of pressing Enter. Without it a polyline can only be
        // ended with the keyboard, and a recorded session could never end one.
        app.submit("done");
        assert_eq!(marks(&app), 1, "polyline never finished:\n{}", said(&app));
    }

    /// Fillet needs two *objects*, not two points — the picking path is a
    /// different one, and it is the path that was silently performing a move.
    /// Every coordinate here arrives the way the pointer delivers it.
    ///
    /// Deliberately not mixed with typed ones: a typed `l 30,250` is in the
    /// kernel's space, y up from the bottom-left, and a click is in app space,
    /// y down from the top-left. The two are the same numbers and different
    /// places, and a test that mixes them misses by the height of the page.
    #[test]
    fn fillet_picks_two_marks_and_joins_them() {
        let mut app = app("single-page.pdf");

        app.submit("line");
        app.take_pick(at(30.0, 60.0));
        app.take_pick(at(170.0, 60.0));
        app.submit("line");
        app.take_pick(at(170.0, 60.0));
        app.take_pick(at(170.0, 160.0));
        let before = marks(&app);
        assert_eq!(before, 2, "the two lines were not drawn:\n{}", said(&app));

        app.submit("fillet 25");
        app.take_pick(at(100.0, 60.0));
        app.take_pick(at(170.0, 120.0));

        assert!(marks(&app) > before, "fillet did nothing:\n{}", said(&app));
    }

    /// Clicking where there is no mark must say so rather than consuming the
    /// click, or the operation silently collects nonsense.
    #[test]
    fn picking_empty_paper_for_an_object_says_so() {
        let mut app = app("single-page.pdf");
        app.submit("l 10,10 100,100");
        app.submit("fillet 20");
        app.take_pick(at(500.0, 500.0));

        assert!(said(&app).contains("nothing there"), "no complaint:\n{}", said(&app));
    }

    /// Scratch: is a real catalogue page hit-testable?
    #[test]
    #[ignore]
    fn catalogue_hit_check() {
        let path = "/Users/hsilighting/Downloads/HSI CATALOG 2026.pdf";
        let mut app = PagifyApp::new(Some(path));
        assert!(app.doc.is_some(), "did not open");

        for page in 0..6usize {
            let Some(chars) = app.characters(page) else {
                println!("page {}: no characters at all", page + 1);
                continue;
            };
            let n = chars.len();
            let rects = chars.line_rects(0..n.min(1));
            let Some(r) = rects.into_iter().next() else {
                println!("page {}: {n} characters, NO BOXES", page + 1);
                continue;
            };
            let mid = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
            let hit = chars.hit(mid.0, mid.1);
            println!(
                "page {}: {n} chars, first box [{:.1},{:.1} {:.1},{:.1}], hit at {mid:?} -> {hit:?}",
                page + 1, r.left, r.top, r.right, r.bottom
            );
        }
    }

    /// Extract Text on a page that already has some real text must not write a
    /// second copy of it. Measured on a real report before the fix: 384
    /// characters became 1,687 and `Component` appeared twice.
    #[test]
    #[ignore]
    fn extract_text_does_not_duplicate_text_already_on_the_page() {
        let path = format!("{}/Desktop/8_144498923727031132.pdf", std::env::var("HOME").unwrap());
        if !std::path::Path::new(&path).exists() {
            eprintln!("skipping: the report is not on the Desktop");
            return;
        }
        let mut app = PagifyApp::new(Some(&path));
        assert!(app.doc.is_some(), "the report did not open");
        if PagifyApp::model_directory().is_none() {
            eprintln!("skipping: no recognition models");
            return;
        }

        let before = app.characters(0).map(|c| c.text()).unwrap_or_default();
        let native = before.matches("Component").count();
        assert_eq!(native, 1, "the fixture no longer says `Component` exactly once");

        app.submit("extracttext");
        app.wait_for_reading();
        app.text = None;

        let after = app.characters(0).map(|c| c.text()).unwrap_or_default();
        assert_eq!(
            after.matches("Component").count(),
            1,
            "a word already on the page was written a second time:\n{}",
            said(&app)
        );
        assert!(
            after.chars().count() > before.chars().count() + 200,
            "the outlined prose was not added ({} -> {} characters)",
            before.chars().count(),
            after.chars().count()
        );
    }

    /// The window must keep running while a page is being read. Recognition is
    /// about a second of solid CPU; on the UI thread that is a second of dead
    /// window, and twenty seconds on a twenty-page document.
    /// Ignored by default with the rest of the recognition tests: a debug build
    /// reads a page in about three minutes against one second in release. Run
    /// with `cargo test -p pagify_app --release extract -- --ignored`.
    #[test]
    #[ignore]
    fn reading_a_page_does_not_block_the_window() {
        let mut app = app("outlined.pdf");
        if PagifyApp::model_directory().is_none() {
            eprintln!("skipping: no recognition models");
            return;
        }

        let start = std::time::Instant::now();
        app.submit("extracttext");
        let dispatched = start.elapsed();

        assert!(
            app.reading.is_some(),
            "the read finished inline, so it ran on this thread:\n{}",
            said(&app)
        );
        assert!(
            dispatched < std::time::Duration::from_millis(250),
            "asking for a read took {dispatched:?} — it is still being done inline"
        );

        app.wait_for_reading();
        assert!(app.reading.is_none());
    }

    /// A second request while one is running must be refused rather than
    /// queued or run alongside: two pages at once doubles the peak memory, and
    /// one page already measured at 482 MB.
    /// Ignored by default with the rest of the recognition tests: a debug build
    /// reads a page in about three minutes against one second in release. Run
    /// with `cargo test -p pagify_app --release extract -- --ignored`.
    #[test]
    #[ignore]
    fn a_second_read_is_refused_while_one_is_running() {
        let mut app = app("outlined.pdf");
        if PagifyApp::model_directory().is_none() {
            return;
        }
        app.submit("extracttext");
        app.submit("extracttext");

        assert!(said(&app).contains("already reading"), "no refusal:\n{}", said(&app));
        app.wait_for_reading();
    }

    /// Ignored by default with the rest of the recognition tests: a debug build
    /// reads a page in about three minutes against one second in release. Run
    /// with `cargo test -p pagify_app --release extract -- --ignored`.
    #[test]
    #[ignore]
    fn escape_gives_up_on_a_page_being_read() {
        let mut app = app("outlined.pdf");
        if PagifyApp::model_directory().is_none() {
            return;
        }
        app.submit("extracttext");
        app.escape();
        app.wait_for_reading();

        assert!(said(&app).contains("stopped after"), "escape did nothing:\n{}", said(&app));
        assert_eq!(
            app.characters(0).map(|c| c.len()).unwrap_or(0),
            0,
            "a declined read wrote its layer anyway"
        );
    }

    // -- closing with unsaved work -----------------------------------------

    /// The trap this replaces: the window would not close, and the only way out
    /// was a command the user had to be told about. Abandoning work is a normal
    /// thing to do — a mark put down to measure something, a line drawn to
    /// check a distance — and a program that will not let go of it is one you
    /// have to kill.
    #[test]
    fn closing_with_unsaved_marks_asks_rather_than_refusing() {
        let mut app = app("single-page.pdf");
        app.submit("l 10,10 100,100");
        assert!(app.unsaved().is_some(), "the mark was not counted as unsaved");

        app.submit("close");
        assert_eq!(app.closing, Some(Closing::Document), "close did not ask:\n{}", said(&app));
        assert!(app.doc.is_some(), "it closed anyway, losing the mark");
    }

    #[test]
    fn quitting_with_unsaved_marks_asks_too() {
        let mut app = app("single-page.pdf");
        app.submit("l 10,10 100,100");
        app.submit("quit");
        assert_eq!(app.closing, Some(Closing::Program));
    }

    /// Opening something else discards the current document just as surely as
    /// closing it, and was refused the same way.
    #[test]
    fn opening_another_file_asks_as_well() {
        let mut app = app("single-page.pdf");
        app.submit("l 10,10 100,100");
        app.submit(&format!("open \"{}\"", fixture("two-column.pdf")));

        assert!(
            matches!(app.closing, Some(Closing::Open(_))),
            "opening over unsaved work did not ask:\n{}",
            said(&app)
        );
    }

    /// The point of the whole change.
    #[test]
    fn discarding_actually_closes() {
        let mut app = app("single-page.pdf");
        app.submit("l 10,10 100,100");
        app.submit("close");
        assert!(app.closing.is_some());

        let ctx = egui::Context::default();
        app.closing = None;
        app.finish_closing(Closing::Document, &ctx);

        assert!(app.doc.is_none(), "discarding did not close the document");
        assert!(app.unsaved().is_none(), "the marks came with it");
    }

    /// The forcing form still works, for anyone who already knows it and for
    /// recorded sessions, which have no one to answer a dialog.
    #[test]
    fn the_forcing_form_still_discards_without_asking() {
        let mut app = app("single-page.pdf");
        app.submit("l 10,10 100,100");
        app.submit("close!");

        assert!(app.closing.is_none(), "close! put up a dialog");
        assert!(app.doc.is_none(), "close! did not close:\n{}", said(&app));
    }

    // -- marking text ------------------------------------------------------

    /// `highlight` used to answer "select some text first" and then do nothing
    /// once you had. All four markups now act on the selection.
    #[test]
    fn marking_the_selection_writes_one_annotation_per_selection() {
        for command in ["highlight", "underline", "strikeout", "squiggly"] {
            let mut app = app("text-lines.pdf");
            let chars = app.characters(0).expect("characters").clone();
            let n = chars.len().min(12);
            app.text_selection = Some(0..n);
            app.selection_page = 0;

            app.submit(command);

            let marks = app
                .doc
                .as_ref()
                .unwrap()
                .session
                .annotations(0)
                .expect("read annotations");
            assert_eq!(
                marks.len(),
                1,
                "`{command}` wrote {} annotations:\n{}",
                marks.len(),
                said(&app)
            );
        }
    }

    /// A selection spanning several lines is one thing the reader made, so it
    /// must be one mark — erasing it is then one action rather than three.
    #[test]
    fn a_selection_over_several_lines_is_a_single_mark() {
        let mut app = app("text-lines.pdf");
        let (len, lines) = {
            let chars = app.characters(0).expect("characters");
            (chars.len(), chars.line_rects(0..chars.len()).len())
        };
        app.text_selection = Some(0..len);
        app.selection_page = 0;

        assert!(lines > 1, "the fixture is only one line, so this proves nothing");

        app.submit("underline");
        let marks = app.doc.as_ref().unwrap().session.annotations(0).expect("read");
        assert_eq!(marks.len(), 1, "{lines} lines became {} marks", marks.len());
    }

    /// With nothing selected the tool is **picked up**, not refused.
    ///
    /// It used to answer "select some text first", which meant the tool could
    /// only ever be applied once per selection and read as a button that
    /// scolded you rather than one that did something.
    #[test]
    fn marking_with_nothing_selected_picks_the_tool_up() {
        let mut app = app("text-lines.pdf");
        app.submit("underline");

        assert!(app.markup_armed.is_some(), "the tool was not picked up:\n{}", said(&app));
        assert!(
            said(&app).contains("drag across the text"),
            "it did not say what to do next:\n{}",
            said(&app)
        );
        assert!(
            app.doc.as_ref().unwrap().session.annotations(0).unwrap().is_empty(),
            "picking the tool up marked something"
        );
    }

    // -- extract text ------------------------------------------------------

    /// `outlined.pdf` is exactly the case this exists for: real glyph contours
    /// drawn as filled paths, no text object anywhere. It renders as words and
    /// contains not one character to select.
    /// Ignored by default: recognition is ~1 s a page in release and over a
    /// minute in a debug build, which is not a cost the ordinary suite should
    /// pay on every run. Run with
    /// `cargo test -p pagify_app --release extract -- --ignored`.
    #[test]
    #[ignore]
    fn extract_text_makes_an_outlined_page_selectable() {
        let mut app = app("outlined.pdf");
        if PagifyApp::model_directory().is_none() {
            eprintln!("skipping: no recognition models");
            return;
        }

        assert!(
            app.characters(0).map(|c| c.len()).unwrap_or(0) < 4,
            "the fixture already has selectable text, so this proves nothing"
        );

        app.submit("extracttext");
        app.wait_for_reading();
        app.text = None;
        let after = app.characters(0).map(|c| c.len()).unwrap_or(0);
        assert!(after > 4, "nothing became selectable:\n{}", said(&app));

        // And it must be selectable, not merely present: a layer in the wrong
        // place reads back fine and cannot be clicked on.
        let chars = app.characters(0).expect("characters").clone();
        let r = chars.line_rects(0..1).into_iter().next().expect("no box");
        let mid = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
        assert!(
            chars.hit(mid.0, mid.1).is_some(),
            "the text layer is not where its own boxes say it is"
        );
    }

    /// It changes the document, so it has to be reversible.
    #[test]
    #[ignore]
    fn an_extracted_text_layer_can_be_undone() {
        let mut app = app("outlined.pdf");
        if PagifyApp::model_directory().is_none() {
            return;
        }
        app.submit("extracttext");
        app.wait_for_reading();
        app.text = None;
        let added = app.characters(0).map(|c| c.len()).unwrap_or(0);
        assert!(added > 4, "nothing to undo:\n{}", said(&app));

        app.submit("undo");
        app.text = None;
        let after = app.characters(0).map(|c| c.len()).unwrap_or(0);
        assert!(after < added, "undo left the layer in place ({added} -> {after})");
    }

    /// `outlined-montserrat.pdf` is the same generator as `outlined.pdf`, but
    /// drawn with the face this build actually bundles — a same-font match,
    /// measured at ~0.84–0.87 against the 0.5 trustworthiness floor — see the
    /// calibration table on `outlined_words_are_trustworthy` itself. Unlike
    /// the tests above, this checks not just that the page became selectable
    /// but that OCR's lazy loader
    /// (`self.recogniser`) was never reached to do it: `Some` would only
    /// appear there if some page in the batch had fallen through to OCR.
    ///
    /// That is also why this one is not `#[ignore]`d and does not check
    /// `model_directory()`: a correctly working fast path needs no models on
    /// disk at all, and this proves exactly that rather than assuming it.
    #[test]
    fn extract_text_on_a_same_font_outlined_page_never_touches_ocr() {
        let mut app = app("outlined-montserrat.pdf");
        assert!(
            app.characters(0).map(|c| c.len()).unwrap_or(0) < 4,
            "the fixture already has selectable text, so this proves nothing"
        );
        assert!(app.recogniser.is_none(), "OCR was already warm before the test ran");

        app.submit("extracttext");
        app.wait_for_reading();
        app.text = None;

        let after = app.characters(0).map(|c| c.len()).unwrap_or(0);
        assert!(after > 4, "nothing became selectable:\n{}", said(&app));
        assert!(
            app.recogniser.is_none(),
            "the page was read by OCR, not the vector-match fast path"
        );
    }

    /// A bad path must be refused the moment it is typed, before it ever
    /// reaches disk as a saved setting — see `add_outlined_font`, which only
    /// calls `OutlinedFonts::save` on the `Ok` branch. Run with no document
    /// open, since adding a font is a preference, not a per-document action.
    ///
    /// Checks only the error text, not the list's overall length: this app
    /// instance loads the same real, on-disk settings file every other test
    /// in this suite does, so another test's own font can legitimately be
    /// sitting in it at the moment this one runs. What must be true
    /// regardless is that *this* bad path never joined it.
    #[test]
    fn outlinedfont_reports_a_clear_error_for_a_bad_path() {
        let bad = "/definitely/not/a/real/path/9f3a.ttf";
        let mut app = PagifyApp::new(None);
        app.submit(&format!("outlinedfont {bad}"));
        assert!(said(&app).contains("could not read"), "unhelpful error:\n{}", said(&app));
        assert!(!app.outlined_fonts.paths.iter().any(|p| p == std::path::Path::new(bad)));
    }

    /// The end-to-end proof that adding a font actually changes what
    /// `extracttext` can do without OCR — not just that `OutlinedFonts` can
    /// hold a path (`outlined_fonts.rs` already covers that in isolation).
    ///
    /// `outlined.pdf` is authored with system Arial — see
    /// `make_text_fixtures.rs` — which the bundled Montserrat alone does not
    /// resolve (~0.27 similarity, measured below the 0.5 trustworthiness
    /// floor; the two `#[ignore]`d tests above exercise that fallback for
    /// real). Adding Arial itself as an extra outlined font must therefore
    /// flip this exact fixture onto the fast path — the only variable that
    /// changed is the font list this test controls.
    ///
    /// Self-cleaning: removes what it added, so a real font a person has
    /// configured on this machine is never at risk from running the suite.
    #[test]
    fn a_user_added_font_unlocks_the_fast_path_for_a_page_the_bundled_fonts_do_not_match() {
        let arial = "/System/Library/Fonts/Supplemental/Arial.ttf";
        if !std::path::Path::new(arial).is_file() {
            eprintln!("skipping: {arial} not present on this machine");
            return;
        }

        let mut app = app("outlined.pdf");
        assert!(
            app.characters(0).map(|c| c.len()).unwrap_or(0) < 4,
            "the fixture already has selectable text, so this proves nothing"
        );

        app.submit(&format!("outlinedfont {arial}"));
        assert!(
            app.outlined_fonts.paths.iter().any(|p| p == std::path::Path::new(arial)),
            "the font was not added:\n{}",
            said(&app)
        );

        app.submit("extracttext");
        app.wait_for_reading();
        app.text = None;

        let after = app.characters(0).map(|c| c.len()).unwrap_or(0);
        let recognised_without_ocr = app.recogniser.is_none();

        app.submit(&format!("outlinedfont remove {arial}"));
        // Checks that Arial specifically is gone, not that the list is empty
        // outright — see the bad-path test above for why: another test's own
        // font may legitimately share this same on-disk settings file.
        assert!(
            !app.outlined_fonts.paths.iter().any(|p| p == std::path::Path::new(arial)),
            "cleanup left the font on the list"
        );

        assert!(after > 4, "nothing became selectable:\n{}", said(&app));
        assert!(
            recognised_without_ocr,
            "the page was read by OCR — the added font did not reach the worker"
        );
    }

    // -- encrypted files ---------------------------------------------------

    /// A great many real working documents are encrypted — an extract saved out
    /// of another editor, a drawing issued under restriction. Refusing them
    /// with "document is password protected" and no way to supply one is a dead
    /// end in a program whose entire interface is a place to type things.
    #[test]
    fn an_encrypted_file_asks_for_its_password() {
        let mut app = PagifyApp::new(None);
        app.submit(&format!("open \"{}\"", fixture("encrypted.pdf")));

        assert!(
            app.awaiting_password.is_some(),
            "an encrypted file did not ask:\n{}",
            said(&app)
        );
        assert!(said(&app).contains("encrypted"), "no explanation:\n{}", said(&app));
    }

    /// The password must reach PDFium and nothing else. `CommandBox::submit`
    /// echoes every line into the visible history and the recorder keeps it for
    /// replay, so a password typed as an ordinary command would be written into
    /// both — and a recorded session would carry it to whoever it is shared
    /// with.
    #[test]
    fn a_password_is_never_written_into_the_history() {
        let mut app = PagifyApp::new(None);
        app.submit(&format!("open \"{}\"", fixture("encrypted.pdf")));
        assert!(app.awaiting_password.is_some(), "the fixture did not ask for a password");

        app.submit("hunter2");
        let history = said(&app);
        assert!(
            !history.contains("hunter2"),
            "the password was echoed into the history:\n{history}"
        );
    }

    #[test]
    fn a_wrong_password_asks_again_rather_than_giving_up() {
        let mut app = PagifyApp::new(None);
        app.submit(&format!("open \"{}\"", fixture("encrypted.pdf")));
        let path = app.awaiting_open().expect("it did not ask for a password");
        app.answer_open_password(&path, "not-the-password");

        assert!(
            app.awaiting_password.is_some(),
            "one wrong attempt ended it:\n{}",
            said(&app)
        );
        // The window says so, where the person is looking, rather than the
        // status line underneath it.
        assert_eq!(
            app.password_problem.as_deref(),
            Some("That password was not accepted."),
            "the window would say nothing about the wrong password"
        );
    }

    #[test]
    fn the_right_password_opens_it() {
        let mut app = PagifyApp::new(None);
        app.submit(&format!("open \"{}\"", fixture("encrypted.pdf")));
        let path = app.awaiting_open().expect("it did not ask for a password");
        app.answer_open_password(&path, "pagify");

        assert!(app.awaiting_password.is_none(), "still asking:\n{}", said(&app));
        assert!(app.doc.is_some(), "the right password did not open it:\n{}", said(&app));
    }

    #[test]
    fn escape_gives_up_on_the_password() {
        let mut app = PagifyApp::new(None);
        app.submit(&format!("open \"{}\"", fixture("encrypted.pdf")));
        app.escape();
        assert!(app.awaiting_password.is_none(), "escape did not give up");
    }

    // -- the two standing tools -------------------------------------------

    /// Both were `Planned`, which is the worst thing they could have been: they
    /// sit first on every tab, they are what anybody reaches for before
    /// anything else, and answering "planned for the selection phase" reads as
    /// *this program cannot select*.
    #[test]
    fn hand_and_select_actually_switch_the_pointer() {
        use pagify_shell::verbs::PointerMode;
        let mut app = app("text-lines.pdf");
        assert_eq!(app.pointer, PointerMode::Select, "the default is not Select");

        app.submit("hand");
        assert_eq!(app.pointer, PointerMode::Pan, "hand did nothing:\n{}", said(&app));

        app.submit("selecttool");
        assert_eq!(app.pointer, PointerMode::Select, "select did nothing:\n{}", said(&app));
    }

    /// Changing tool mid-pick has to abandon the pick. Otherwise the next click
    /// on the page lands in an operation the user has already walked away from.
    #[test]
    fn switching_to_the_pointer_abandons_a_half_finished_tool() {
        let mut app = app("single-page.pdf");
        app.submit("line");
        app.take_pick(at(10.0, 10.0));
        assert!(app.pending.is_some());

        app.submit("selecttool");
        assert!(app.pending.is_none(), "the line tool survived the switch");

        app.take_pick(at(100.0, 100.0));
        assert_eq!(marks(&app), 0, "a click after switching still drew something");
    }

    #[test]
    fn escape_returns_to_selecting() {
        use pagify_shell::verbs::PointerMode;
        let mut app = app("text-lines.pdf");
        app.submit("hand");
        app.escape();
        assert_eq!(
            app.pointer,
            PointerMode::Select,
            "escape left the pointer in Hand — which is not stopping"
        );
    }

    // -- text selection ---------------------------------------------------

    /// The rule the canvas uses to decide between selecting text and selecting
    /// marks: a drag that begins on a character selects text. If `hit` cannot
    /// find a character under a point that is plainly on one, every drag
    /// becomes a box-select and text can never be selected at all.
    #[test]
    fn a_point_on_a_character_is_recognised_as_text() {
        let mut app = app("text-lines.pdf");
        let chars = app.characters(0).expect("no characters").clone();
        assert!(chars.len() > 0, "the fixture has no characters");

        let first = chars.line_rects(0..1).into_iter().next().expect("no box for character 0");
        let mid = ((first.left + first.right) / 2.0, (first.top + first.bottom) / 2.0);

        assert!(
            chars.hit(mid.0, mid.1).is_some(),
            "the middle of the first character ({mid:?}) is not on text — \
             every drag would box-select instead"
        );
    }

    #[test]
    fn dragging_across_a_line_selects_the_characters_between() {
        let mut app = app("text-lines.pdf");
        let chars = app.characters(0).expect("no characters").clone();
        let n = chars.len();
        assert!(n > 4, "fixture has too little text to drag across");

        let centre = |i: usize| {
            let r = chars.line_rects(i..i + 1).into_iter().next().expect("no box");
            ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0)
        };

        let range = chars.range_between(centre(0), centre(4)).expect("no range across one line");
        assert!(!range.is_empty(), "the range came back empty");
        assert!(range.end <= n, "range {range:?} runs past {n} characters");
        assert!(
            !chars.text_of(range.clone()).trim().is_empty(),
            "selected {range:?} and got no text out of it"
        );
    }

    /// Selecting has to survive the page being asked for twice — the character
    /// cache is keyed by page, and a stale key silently selects on the wrong
    /// one.
    #[test]
    fn the_character_cache_follows_the_page() {
        let mut app = app("two-column.pdf");
        let first = app.characters(0).map(|c| c.text()).unwrap_or_default();
        assert!(!first.is_empty(), "page 1 has no text");

        let again = app.characters(0).map(|c| c.text()).unwrap_or_default();
        assert_eq!(first, again, "asking twice gave different text");
    }
}

/// Driving the real interface, not the methods behind it.
///
/// Selection and the drawing tools broke twice while every logic test stayed
/// green, because the logic was never what broke: a mode swallowed the drag, a
/// panel took the pointer, a scroll area consumed the gesture. None of that is
/// reachable from a method call, and all of it is what the user actually
/// touches. These build the whole window and use a mouse on it.
#[cfg(test)]
mod ui_tests {
    use super::*;
    use eframe::App as _;
    use egui_kittest::Harness;

    fn fixture(name: &str) -> String {
        format!(
            "{}/../../../workspace/Pagify/rust/pdf_core/fixtures/{name}",
            env!("CARGO_MANIFEST_DIR")
        )
    }

    /// The app in a window big enough to have a page in it.
    fn harness(name: &str) -> Harness<'static, PagifyApp> {
        let app = PagifyApp::new(Some(&fixture(name)));
        assert!(app.doc.is_some(), "{name} did not open");

        let mut h = Harness::builder()
            .with_size(egui::vec2(1400.0, 1000.0))
            .build_ui_state(
                |ui, app: &mut PagifyApp| {
                    let mut frame = eframe::Frame::_new_kittest();
                    app.ui(ui, &mut frame);
                },
                app,
            );
        // A few frames: the first lays out, the rest let the page raster and
        // the strip settle.
        h.run_steps(4);
        h
    }

    /// Where the page is on screen, and a point on its first character.
    fn a_character_on_screen(h: &mut Harness<'static, PagifyApp>) -> egui::Pos2 {
        let app = h.state_mut();
        let page = app.page;
        let chars = app.characters(page).expect("no characters").clone();
        let r = chars.line_rects(0..1).into_iter().next().expect("no character box");
        let mid = egui::pos2((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);

        let view = app.last_view.expect("the page was never drawn, so nothing can be clicked");
        view.to_screen(AppPoint { x: mid.x as f64, y: mid.y as f64 })
    }

    fn drag(h: &mut Harness<'static, PagifyApp>, from: egui::Pos2, to: egui::Pos2) {
        use egui::{Event, PointerButton};
        let press = |pos, down| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed: down,
            modifiers: Default::default(),
        };

        h.input_mut().events.push(Event::PointerMoved(from));
        h.run_steps(1);
        h.input_mut().events.push(press(from, true));
        h.run_steps(1);

        // Several moves, because a drag is a sequence: one jump can be read as
        // a click somewhere else.
        for i in 1..=4 {
            let t = i as f32 / 4.0;
            h.input_mut().events.push(Event::PointerMoved(from + (to - from) * t));
            h.run_steps(1);
        }
        h.input_mut().events.push(press(to, false));
        h.run_steps(2);
    }

    fn click(h: &mut Harness<'static, PagifyApp>, at: egui::Pos2) {
        use egui::{Event, PointerButton};
        h.input_mut().events.push(Event::PointerMoved(at));
        h.run_steps(1);
        for pressed in [true, false] {
            h.input_mut().events.push(Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            });
            h.run_steps(1);
        }
        h.run_steps(1);
    }

    /// The Home screen with nothing open — where the fonts panel lives — full
    /// window, real layout, not just the state `pointer_tests` checks.
    ///
    /// Deliberately does not click "Add font…": that dispatches
    /// `outlinedfont`, which opens a real, blocking native OS dialog — see
    /// `outlined_font_dialog`. Nothing in this suite clicks it, the same way
    /// nothing clicks the equivalent "Open a PDF…" for `Verb::OpenDialog`.
    #[test]
    fn the_home_screen_renders_the_outlined_fonts_panel() {
        use egui_kittest::kittest::Queryable;

        let mut app = PagifyApp::new(None);
        assert!(app.doc.is_none(), "this checks the backstage view, which needs nothing open");
        // **Not the reader's own list.** This test asserts what the panel says
        // when nothing has been added, and it was reading the settings file of
        // whoever ran it — so adding a font to your own copy of the program
        // broke the suite. What is on this machine is not what is under test.
        app.outlined_fonts = Default::default();
        let mut h = Harness::builder()
            .with_size(egui::vec2(1400.0, 1000.0))
            .build_ui_state(
                |ui, app: &mut PagifyApp| {
                    let mut frame = eframe::Frame::_new_kittest();
                    app.ui(ui, &mut frame);
                },
                app,
            );
        h.run_steps(4);

        h.get_by_label_contains("Outlined-Text Fonts");
        h.get_by_label_contains("Add font");
        h.get_by_label_contains("No extra fonts added");
        // Nothing to remove with none configured — confirms the query above
        // is not simply matching everything on screen.
        assert!(h.query_by_label_contains("Remove").is_none());
    }

    /// The same screen with a font configured — set directly on the in-memory
    /// state rather than through `outlinedfont`, so this never touches the
    /// real, shared settings file `pointer_tests` already exercises.
    ///
    /// A real, existing file, deliberately: `present()` — see
    /// `outlined_fonts.rs` — filters to fonts still on disk, so a made-up path
    /// would silently render the *empty* state and prove nothing.
    #[test]
    fn a_configured_font_shows_its_name_and_a_way_to_remove_it() {
        use egui_kittest::kittest::Queryable;

        let arial = "/System/Library/Fonts/Supplemental/Arial.ttf";
        if !std::path::Path::new(arial).is_file() {
            eprintln!("skipping: {arial} not present on this machine");
            return;
        }

        let app = PagifyApp::new(None);
        let mut h = Harness::builder()
            .with_size(egui::vec2(1400.0, 1000.0))
            .build_ui_state(
                |ui, app: &mut PagifyApp| {
                    let mut frame = eframe::Frame::_new_kittest();
                    app.ui(ui, &mut frame);
                },
                app,
            );
        h.state_mut().outlined_fonts.paths = vec![PathBuf::from(arial)];
        h.run_steps(4);

        h.get_by_label_contains("Arial.ttf");
        h.get_by_label_contains("Remove");
        assert!(h.query_by_label_contains("No extra fonts added").is_none());
    }

    /// The Protect tab is where a reader actually discovers this needs doing
    /// — mid-`redact` or mid-`lock`, not on the Home screen they may never
    /// revisit. Checks the button is really on screen under that tab, not
    /// just that `Tab::buttons()` contains the tuple.
    ///
    /// Deliberately does not click it, for the same reason the Home tests do
    /// not click "Add font…": it dispatches `outlinedfont`, which opens a
    /// real, blocking native dialog.
    #[test]
    fn the_protect_tab_offers_the_outlined_fonts_tool() {
        use egui_kittest::kittest::Queryable;

        let mut h = harness("single-page.pdf");
        h.state_mut().ribbon = Tab::Protect;
        h.run_steps(4);

        h.get_by_label_contains("Outlined-Text Fonts");
        // And beside the tools it exists to support, not lost among the
        // planned stubs. Exact labels, not `_contains`: "Redact" is also a
        // substring of "Smart Redact", a few rows down on the same tab.
        h.get_by_label("Redact");
        h.get_by_label("Lock Area");
    }

    /// **Clicking a padlock asks for the passcode.**
    ///
    /// Reported from use: the badge drew but pressing it did nothing. The page
    /// covers every badge on it and was registering its own click handler
    /// *after* theirs, so egui gave every click to the page — see the ordering
    /// note where `draw_lock_badges` is called. Driven through the real window
    /// rather than by calling the handler, because the bug was entirely in
    /// which widget received the click.
    #[test]
    fn clicking_a_padlock_asks_for_the_passcode() {
        let app = PagifyApp::new(Some(&fixture("scan-300dpi.pdf")));
        assert!(app.doc.is_some(), "the fixture did not open");

        let mut h = Harness::builder()
            .with_size(egui::vec2(1400.0, 1000.0))
            .build_ui_state(
                |ui, app: &mut PagifyApp| {
                    let mut frame = eframe::Frame::_new_kittest();
                    app.ui(ui, &mut frame);
                },
                app,
            );
        h.run_steps(4);

        // Lock the image so there is a padlock to press.
        {
            let app = h.state_mut();
            app.ribbon = Tab::Protect;
            let object = app.images_on(0)[0].object;
            app.lock_image(0, object, b"a good passcode").expect("lock");
        }
        h.run_steps(4);

        let (badge, view) = {
            let app = h.state_mut();
            let items = app.locked_items_on(0);
            assert_eq!(items.len(), 1, "nothing was locked, so there is no badge");
            (items[0].rect, app.last_view.expect("the page was never drawn"))
        };

        // The badge sits at the middle of what it stands for.
        let middle = view.to_screen(AppPoint {
            x: ((badge.left + badge.right) / 2.0) as f64,
            y: ((badge.top + badge.bottom) / 2.0) as f64,
        });
        click(&mut h, middle);

        assert!(
            matches!(h.state().awaiting_password, Some(Awaiting::UnlockItem(_))),
            "clicking the padlock did not ask for a passcode"
        );
    }

    /// **The same passcode locks the text and the pictures alike**, and a
    /// second lock is not asked to choose one again.
    ///
    /// The vault holds one key, wrapped under one passcode, so everything
    /// locked in a document shares it. The window has to say so: asking a
    /// person to invent a fresh passcode for the image on a page they have
    /// already locked would be asking for something that cannot exist.
    #[test]
    fn a_second_lock_uses_the_passcode_the_document_already_has() {
        use egui_kittest::kittest::Queryable;

        let strong = "Correct-Horse-99-Battery";
        let mut app = PagifyApp::new(Some(&fixture("two-column.pdf")));
        app.lock_pages(&[0], strong.as_bytes()).expect("lock the page");

        let mut h = Harness::builder()
            .with_size(egui::vec2(1200.0, 900.0))
            .build_ui_state(
                |ui, app: &mut PagifyApp| {
                    let mut frame = eframe::Frame::_new_kittest();
                    app.ui(ui, &mut frame);
                },
                app,
            );
        h.run_steps(2);

        // Unlocking asks for what the document has, not for a new choice —
        // said in the window's own words, which nothing else on screen uses.
        h.state_mut().submit("unlock");
        h.run_steps(2);
        h.get_by_label_contains("The passcode this document was locked with");

        // And it takes the passcode straight through — no rule, no second
        // typing, because nothing is being chosen.
        h.state_mut().password_typed = strong.into();
        h.run();
        h.get_by_label_contains("Unlock document").click();
        h.run_steps(3);

        // Asserted on the words coming back rather than on the vault
        // emptying: unlocking puts the pages back and leaves the sealed copy
        // where it is, so `locked_pages` is not the question.
        let back = h
            .state_mut()
            .characters(0)
            .map(|c| c.text())
            .unwrap_or_default();
        assert!(
            back.contains("luminaire"),
            "the passcode that locked it did not bring the page back: {back:?}"
        );
    }

    /// **Choosing a passcode: a window, a rule, and typing it twice.**
    ///
    /// The rule matters because a locked document carries its own original
    /// inside itself — the passcode is the only thing between somebody with
    /// the file and everything taken off its pages, and they can guess offline
    /// for as long as they like.
    #[test]
    fn locking_asks_in_a_window_and_holds_out_for_a_strong_passcode() {
        use egui_kittest::kittest::Queryable;

        let app = PagifyApp::new(Some(&fixture("two-column.pdf")));
        let mut h = Harness::builder()
            .with_size(egui::vec2(1200.0, 900.0))
            .build_ui_state(
                |ui, app: &mut PagifyApp| {
                    let mut frame = eframe::Frame::_new_kittest();
                    app.ui(ui, &mut frame);
                },
                app,
            );
        h.run_steps(2);

        // Arm a whole-page lock, which is the simplest thing that asks.
        h.state_mut().submit("lock all");
        h.run_steps(2);
        h.get_by_label_contains("Choose a passcode");

        let strong = "Correct-Horse-99-Battery";
        assert!(pagify_shell::passphrase::is_strong_enough(strong));

        // **A weak one does nothing.** The button is disabled rather than
        // absent — a disabled widget is still in the accessibility tree — so
        // the property to check is that pressing it locks nothing and leaves
        // the window up.
        h.state_mut().password_typed = "short".into();
        h.run();
        h.get_by_label_contains("Lock document").click();
        h.run();
        assert!(
            h.state().doc.as_ref().expect("doc").session.locked_pages().is_empty(),
            "a weak passcode locked something"
        );
        h.get_by_label_contains("Choose a passcode");

        // The rest goes through the method the window's button calls, rather
        // than through the button: a password field reports itself to the
        // accessibility tree as a password, not as text, and there is no
        // reading back what was typed into one. What matters is that the
        // decision is the same one either way.
        h.state_mut().answer_passcode(strong);
        h.run();
        h.get_by_label_contains("Type it again");

        // Mistyped, and nothing is locked.
        h.state_mut().answer_passcode("something else 9A!bcdefgh");
        h.run();
        assert!(
            h.state().doc.as_ref().expect("doc").session.locked_pages().is_empty(),
            "a mistyped confirmation locked the document anyway"
        );

        // Typed the same twice, and it locks.
        h.state_mut().submit("lock all");
        h.state_mut().answer_passcode(strong);
        h.state_mut().answer_passcode(strong);
        h.run();
        assert!(
            !h.state().doc.as_ref().expect("doc").session.locked_pages().is_empty(),
            "the passcode was right twice and nothing locked"
        );
    }

    /// **The password window is really on screen, and really opens it.**
    ///
    /// The test beside this one exercises the shared path; this one renders the
    /// window, types into it, and presses Open — because a dialog nothing
    /// drives is a dialog nobody has checked, which is how drawn ink stayed
    /// visible on a locked page through a suite that passed.
    #[test]
    fn a_document_that_wants_a_password_asks_for_one_in_a_window() {
        use egui_kittest::kittest::Queryable;

        let mut app = PagifyApp::new(None);
        app.submit(&format!("open \"{}\"", fixture("encrypted.pdf")));
        assert!(app.doc.is_none(), "it opened without a password");

        let mut h = Harness::builder()
            .with_size(egui::vec2(1200.0, 900.0))
            .build_ui_state(
                |ui, app: &mut PagifyApp| {
                    let mut frame = eframe::Frame::_new_kittest();
                    app.ui(ui, &mut frame);
                },
                app,
            );
        h.run_steps(3);

        // The window is there, and says what it wants.
        h.get_by_label_contains("This document needs a password");

        // Type into it and press Open, as a person would.
        h.state_mut().password_typed = "pagify".into();
        h.run_steps(1);
        // Exact, because the ribbon has an "Open..." of its own.
        h.get_by_label("Open").click();
        h.run_steps(3);

        assert!(h.state().doc.is_some(), "the window did not open the document");
        assert!(
            h.state().awaiting_password.is_none(),
            "the window is still asking after it opened"
        );
    }

    /// **Nothing locked may still be on screen anywhere.**
    ///
    /// Reported from use: a locked page went on showing its contents in the
    /// thumbnail strip. Thirteen edit sites cleared the page textures and two
    /// cleared the thumbnails, so almost every edit left a stale one — and for
    /// a lock that is not a cosmetic lag, it is the hidden content still
    /// visible.
    ///
    /// Driven through the real window, because the bug was entirely in which
    /// cache an edit remembered to clear.
    #[test]
    fn locking_a_page_leaves_no_stale_thumbnail_of_it() {
        let app = PagifyApp::new(Some(&fixture("two-column.pdf")));
        assert!(app.doc.is_some(), "the fixture did not open");

        let mut h = Harness::builder()
            .with_size(egui::vec2(1400.0, 1000.0))
            .build_ui_state(
                |ui, app: &mut PagifyApp| {
                    let mut frame = eframe::Frame::_new_kittest();
                    app.ui(ui, &mut frame);
                },
                app,
            );
        h.run_steps(6);

        // The strip has drawn, so a thumbnail of the unlocked page is cached.
        assert!(
            !h.state().doc.as_ref().expect("doc").thumbs.is_empty(),
            "no thumbnail was cached, so this proves nothing"
        );

        h.state_mut()
            .lock_pages(&[0], b"a good passcode")
            .expect("lock the page");

        assert!(
            h.state().doc.as_ref().expect("doc").thumbs.is_empty(),
            "the thumbnail of the locked page is still cached"
        );
        assert!(
            h.state().doc.as_ref().expect("doc").textures.is_empty(),
            "the rendered page is still cached"
        );
    }

    /// The two files this was actually failing on.
    ///
    /// Not committed — they are 80 MB and 500 kB of someone's real work — so
    /// these skip when the files are not there. They are here because a
    /// one-page fixture hid the defect completely: `self.page` never followed
    /// the scroll, so on any document long enough to scroll, the pointer talked
    /// to page 1 while the reader was somewhere else entirely.
    fn real_file(name: &str) -> Option<String> {
        let path = format!("{}/Downloads/{name}", std::env::var("HOME").ok()?);
        std::path::Path::new(&path).exists().then_some(path)
    }

    fn open_real(name: &str) -> Option<Harness<'static, PagifyApp>> {
        let path = real_file(name)?;
        let app = PagifyApp::new(Some(&path));
        // Skipped rather than failed when it will not open. These tests read a
        // document from the person's own Downloads folder, which they are
        // entitled to change — and one of them acquired a password, at which
        // point six tests started failing about scrolling and selection. A
        // test that depends on somebody's file has to tolerate that file.
        if app.doc.is_none() {
            eprintln!("skipping: {name} is present but would not open (a password, perhaps)");
            return None;
        }
        let mut h = Harness::builder()
            .with_size(egui::vec2(1400.0, 1000.0))
            .build_ui_state(
                |ui, app: &mut PagifyApp| {
                    let mut frame = eframe::Frame::_new_kittest();
                    app.ui(ui, &mut frame);
                },
                app,
            );
        h.run_steps(6);
        Some(h)
    }

    /// Scroll the strip the way a wheel would, then let it settle.
    fn wheel(h: &mut Harness<'static, PagifyApp>, by: f32) {
        // A scroll area only takes the wheel while the pointer is over it,
        // which is exactly right and exactly the sort of thing a test that
        // pokes methods would never notice.
        h.input_mut().events.push(egui::Event::PointerMoved(egui::pos2(700.0, 600.0)));
        h.input_mut().events.push(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -by),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
        });
        h.run_steps(4);
    }

    #[test]
    fn selection_works_after_scrolling_into_a_long_document() {
        let Some(mut h) = open_real("HSI CATALOG 2026.pdf") else {
            eprintln!("skipping: catalogue not in ~/Downloads");
            return;
        };
        assert_eq!(h.state().page, 0);

        // Far enough in that the current page must have moved with it.
        for _ in 0..6 {
            wheel(&mut h, 900.0);
        }
        let landed = h.state().page;
        assert!(landed > 0, "scrolling six screens did not change the current page");

        let start = a_character_on_screen(&mut h);
        drag(&mut h, start, start + egui::vec2(160.0, 0.0));

        let app = h.state();
        assert!(
            app.text_selection.is_some(),
            "nothing selected on page {} of the catalogue.\npointer: {:?}  pending: {:?}",
            landed + 1,
            app.pointer,
            app.pending.is_some()
        );
    }

    #[test]
    fn selection_works_on_the_test_report() {
        let Some(mut h) = open_real("8_144498923727031132.pdf")
            .or_else(|| {
                let p = format!("{}/Desktop/8_144498923727031132.pdf", std::env::var("HOME").unwrap());
                std::path::Path::new(&p).exists().then(|| {
                    let app = PagifyApp::new(Some(&p));
                    let mut h = Harness::builder()
                        .with_size(egui::vec2(1400.0, 1000.0))
                        .build_ui_state(
                            |ui, app: &mut PagifyApp| {
                                let mut frame = eframe::Frame::_new_kittest();
                                app.ui(ui, &mut frame);
                            },
                            app,
                        );
                    h.run_steps(6);
                    h
                })
            })
        else {
            eprintln!("skipping: the report is not in ~/Downloads or ~/Desktop");
            return;
        };

        // Page 2 carries the monospace evidence blocks, which are real text.
        h.state_mut().submit("page 2");
        h.run_steps(4);

        let start = a_character_on_screen(&mut h);
        drag(&mut h, start, start + egui::vec2(120.0, 0.0));

        let app = h.state();
        assert!(
            app.text_selection.is_some(),
            "nothing selected on page {} of the report",
            app.page + 1
        );
    }

    /// What actually reached the clipboard this frame.
    ///
    /// Asserted rather than trusting that `copy_text` was called: the whole
    /// point of copying is that something lands outside the program, and a
    /// test that only checks the call was made would pass on an empty string.
    fn copied(h: &Harness<'static, PagifyApp>) -> Option<String> {
        // From the frame's own output, not from the context: egui hands the
        // commands to the integration and clears them, so by the next step
        // there is nothing left to read.
        h.output().platform_output.commands.iter().find_map(|c| match c {
            egui::OutputCommand::CopyText(t) => Some(t.clone()),
            _ => None,
        })
    }

    /// ⌘C, the way almost everyone will copy.
    ///
    /// Asserted on what the app reports rather than on the clipboard command,
    /// and not for want of trying: `step()` drains every queued event in one
    /// call, so the frame that carries `CopyText` has already been replaced by
    /// the time the test can look. The payload is checked through the command
    /// box instead, in `what_is_copied_is_what_was_selected`, where the frame
    /// is observable — between them the key, the path and the text are all
    /// covered.
    #[test]
    fn selected_text_can_be_copied_with_the_keyboard() {
        let mut h = harness("text-lines.pdf");
        let start = a_character_on_screen(&mut h);
        drag(&mut h, start, start + egui::vec2(160.0, 0.0));
        assert!(h.state().text_selection.is_some(), "nothing was selected to copy");

        // Through the harness's own key path, which sends `ModifiersChanged`
        // before the key — `i.modifiers` is state, not something carried on the
        // event, and that is what ⌘C is read from.
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::C);
        h.run_steps(3);

        let said = h
            .state()
            .cmd
            .history()
            .iter()
            .map(|e| e.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            said.contains("characters copied"),
            "\u{2318}C copied nothing.\nselection: {:?}\n{said}",
            h.state().text_selection
        );
    }

    /// §7: the box can do anything the interface can. `copy` did not exist, so
    /// the one thing a reader most wants to do with a selection could not be
    /// scripted or recorded.
    #[test]
    fn copy_works_from_the_command_box_too() {
        let mut h = harness("text-lines.pdf");
        let start = a_character_on_screen(&mut h);
        drag(&mut h, start, start + egui::vec2(160.0, 0.0));

        h.state_mut().submit("copy");
        h.run_steps(1);

        let text = copied(&h).expect("`copy` put nothing on the clipboard");
        assert!(!text.trim().is_empty(), "an empty string was copied");
    }

    #[test]
    fn copying_with_nothing_selected_says_so_rather_than_copying_nothing() {
        let mut h = harness("text-lines.pdf");
        h.state_mut().submit("copy");
        h.run_steps(2);

        assert!(copied(&h).is_none(), "an empty selection reached the clipboard");
        let said = h
            .state()
            .cmd
            .history()
            .iter()
            .map(|e| e.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(said.contains("nothing selected"), "no explanation:\n{said}");
    }

    /// The copied text must be the text that was highlighted, not the page.
    #[test]
    fn what_is_copied_is_what_was_selected() {
        let mut h = harness("text-lines.pdf");
        let start = a_character_on_screen(&mut h);
        drag(&mut h, start, start + egui::vec2(160.0, 0.0));

        let expected = {
            let app = h.state_mut();
            let page = app.page;
            let range = app.text_selection.clone().expect("no selection");
            app.characters(page).expect("characters").text_of(range)
        };

        h.state_mut().submit("copy");
        h.run_steps(1);

        assert_eq!(copied(&h).as_deref(), Some(expected.as_str()));
    }

    fn drag_with(
        h: &mut Harness<'static, PagifyApp>,
        button: egui::PointerButton,
        from: egui::Pos2,
        to: egui::Pos2,
    ) {
        use egui::Event;
        let press = |pos, down| Event::PointerButton {
            pos,
            button,
            pressed: down,
            modifiers: Default::default(),
        };
        h.input_mut().events.push(Event::PointerMoved(from));
        h.run_steps(1);
        h.input_mut().events.push(press(from, true));
        h.run_steps(1);
        for i in 1..=4 {
            let t = i as f32 / 4.0;
            h.input_mut().events.push(Event::PointerMoved(from + (to - from) * t));
            h.run_steps(1);
        }
        h.input_mut().events.push(press(to, false));
        h.run_steps(3);
    }

    /// Zooming on a real, long document — where the page under the pointer is
    /// very often not the current page.
    ///
    /// A one-page fixture cannot show this: `last_view` is recorded for the
    /// current page, so anchoring with it put the fixed point on the wrong page
    /// entirely and the view slid.
    #[test]
    fn zooming_holds_the_point_on_a_scrolling_document() {
        let Some(mut h) = open_real("HSI CATALOG 2026.pdf") else {
            eprintln!("skipping: catalogue not in ~/Downloads");
            return;
        };
        h.state_mut().zoom = ZoomMode::Factor(2.5);
        h.run_steps(3);
        for _ in 0..4 {
            wheel(&mut h, 900.0);
        }

        let at = egui::pos2(700.0, 620.0);
        h.input_mut().events.push(egui::Event::PointerMoved(at));
        h.run_steps(2);
        // Measured in **strip** coordinates. A `PageView` maps to points within
        // its own page, so on a scrolling strip the point under the cursor can
        // belong to a different page before and after — comparing those two
        // directly measures the gap between pages, not the anchor.
        let strip_point = |h: &Harness<'static, PagifyApp>, at: egui::Pos2| {
            let app = h.state();
            let (page, view) = app.hover_view.expect("the pointer was over no page");
            let top = app.doc.as_ref().expect("open").strip.top_of(page).unwrap_or(0.0) as f64;
            let on_page = view.to_page(at);
            (on_page.x, on_page.y + top)
        };

        let target = strip_point(&h, at);
        pinch_at(&mut h, at, 1.04, 6);

        let app = h.state();
        let now = strip_point(&h, at);
        let drift = ((now.0 - target.0).powi(2) + (now.1 - target.1).powi(2)).sqrt();
        assert!(
            drift < 8.0,
            "the page slid {drift:.1}pt out from under the cursor while zooming \
             ({target:?} -> {now:?}) at scale {:.2}",
            app.resolved_zoom()
        );
    }

    /// A selection has to survive the wheel.
    ///
    /// The current page follows the scroll now, and the selection used to be
    /// dropped whenever the current page changed — so nudging the wheel after
    /// highlighting something threw it away, and ⌘C then had nothing to copy.
    #[test]
    fn a_selection_survives_scrolling() {
        let Some(mut h) = open_real("HSI CATALOG 2026.pdf") else {
            eprintln!("skipping: catalogue not in ~/Downloads");
            return;
        };
        for _ in 0..3 {
            wheel(&mut h, 900.0);
        }

        let start = a_character_on_screen(&mut h);
        drag(&mut h, start, start + egui::vec2(150.0, 0.0));
        let selected = h.state().text_selection.clone();
        assert!(selected.is_some(), "nothing was selected to begin with");
        let on_page = h.state().selection_page;

        // A nudge, not a jump to another page.
        wheel(&mut h, 60.0);

        assert_eq!(h.state().text_selection, selected, "the wheel threw the selection away");
        assert_eq!(h.state().selection_page, on_page, "the selection changed page");

        h.state_mut().submit("copy");
        h.run_steps(1);
        assert!(copied(&h).is_some(), "the surviving selection copied nothing");
    }

    /// Clicking a thumbnail must actually go there. The programmatic scroll
    /// takes a frame or two to arrive, and follow-the-scroll would read the old
    /// position in the meantime and put the page straight back.
    #[test]
    fn a_jump_to_a_page_is_not_undone_by_the_scroll_catching_up() {
        let Some(mut h) = open_real("HSI CATALOG 2026.pdf") else {
            eprintln!("skipping: catalogue not in ~/Downloads");
            return;
        };
        assert_eq!(h.state().page, 0);

        h.state_mut().act(Verb::Page(PageTarget::Number(12)));
        h.run_steps(6);

        assert_eq!(
            h.state().page,
            11,
            "the jump was undone; the view slid back to page {}",
            h.state().page + 1
        );
    }

    /// **The page must never vanish.**
    ///
    /// The engine refuses a render wider than 16,384 px or larger than 64 M
    /// pixels, and the draw path turned a refusal into `None` — so past a
    /// certain zoom nothing was drawn at all and the page came back only when
    /// you zoomed out again.
    #[test]
    fn the_page_is_still_drawn_at_extreme_zoom() {
        let mut h = harness("text-lines.pdf");

        for scale in [4.0f32, 8.0, 12.0, 16.0] {
            h.state_mut().zoom = ZoomMode::Factor(scale);
            h.run_steps(3);
            assert!(
                h.state().last_view.is_some(),
                "nothing was drawn at {scale}x — the page disappeared"
            );
        }
    }

    /// And it must still be *usable* there: a drawn page nobody can point at is
    /// only half back.
    #[test]
    fn the_page_can_still_be_pointed_at_when_zoomed_right_in() {
        let mut h = harness("text-lines.pdf");
        h.state_mut().zoom = ZoomMode::Factor(14.0);
        h.run_steps(3);

        let view = h.state().last_view.expect("nothing drawn");
        let at = egui::pos2(700.0, 600.0);
        let on_page = view.to_page(at);
        assert!(
            on_page.x.is_finite() && on_page.y.is_finite(),
            "the page mapping went bad at 14x: {on_page:?}"
        );
    }

    /// A page smaller than the window belongs in the middle of it, not against
    /// the left edge with all the empty space on one side.
    #[test]
    fn a_page_smaller_than_the_window_is_centred() {
        let mut h = harness("single-page.pdf");
        h.state_mut().zoom = ZoomMode::Factor(0.5);
        h.run_steps(3);

        let (view, page_w) = {
            let app = h.state();
            let view = app.last_view.expect("nothing drawn");
            let (w, _) = app.doc.as_ref().unwrap().strip.size_of(app.page).unwrap();
            (view, w)
        };

        let viewport = h.state().viewport_rect.expect("no viewport");
        let left = view.origin.x;
        let right = left + page_w * view.scale;
        let before = left - viewport.left();
        let after = viewport.right() - right;

        assert!(
            (before - after).abs() < 4.0,
            "the page is not centred: {before:.1}pt of space on the left, {after:.1}pt on \
             the right"
        );
    }

    /// Once the page is larger than the window there is nothing to centre, and
    /// forcing it would fight the scroll.
    #[test]
    fn a_page_larger_than_the_window_is_not_centred() {
        let mut h = harness("single-page.pdf");
        h.state_mut().zoom = ZoomMode::Factor(8.0);
        h.run_steps(3);

        let viewport = h.state().viewport_rect.expect("no viewport");
        let view = h.state().last_view.expect("nothing drawn");
        assert!(
            view.origin.x <= viewport.left() + 14.0,
            "an oversized page was padded away from the edge ({} vs {})",
            view.origin.x,
            viewport.left()
        );
    }

    /// Facing pages have to be drawn side by side, not merely laid out that
    /// way — the placement is the strip's, and the draw has to ask it.
    #[test]
    fn facing_draws_two_pages_side_by_side() {
        let mut h = harness("pages-ladder.pdf");
        h.state_mut().submit("viewfacing");
        h.run_steps(4);

        let (a, b) = {
            let app = h.state();
            let strip = &app.doc.as_ref().unwrap().strip;
            (strip.left_of(0).unwrap(), strip.left_of(1).unwrap())
        };
        assert!(b > a, "the second page is not to the right of the first");

        let same_row = {
            let strip = &h.state().doc.as_ref().unwrap().strip;
            strip.top_of(0) == strip.top_of(1)
        };
        assert!(same_row, "the two pages are not on the same row");
    }

    /// Adding or removing a page must not quietly put a spread back to one-up.
    #[test]
    fn a_page_operation_keeps_the_chosen_layout() {
        use pagify_shell::reader::Layout;
        let mut h = harness("pages-ladder.pdf");
        h.state_mut().submit("viewfacing");
        h.run_steps(3);

        h.state_mut().submit("reversepages");
        h.run_steps(3);

        assert_eq!(
            h.state().doc.as_ref().unwrap().strip.layout(),
            Layout::Facing,
            "reversing the document reset the layout"
        );
    }

    /// Whatever the layout, the page under the pointer is the page you
    /// interact with — which on a spread means the one you are actually over,
    /// not whichever shares its row.
    #[test]
    fn the_right_page_of_a_spread_is_its_own_page() {
        let mut h = harness("pages-ladder.pdf");
        h.state_mut().submit("viewfacing");
        h.run_steps(4);

        let strip = h.state().doc.as_ref().unwrap().strip.clone();
        assert_eq!(strip.top_of(0), strip.top_of(1));
        assert_ne!(
            strip.left_of(0),
            strip.left_of(1),
            "both pages of the spread are in the same place"
        );
    }

    /// A button that is not built yet must **say so where the user is looking**.
    ///
    /// The reply went into the history panel, which is folded away by default —
    /// so every unbuilt button looked broken rather than unbuilt, and a tab of
    /// them read as a tab that does nothing.
    #[test]
    fn an_unbuilt_button_says_so_without_opening_the_history() {
        let mut h = harness("text-lines.pdf");
        h.state_mut().command_open = false;
        h.state_mut().submit("spelling");
        h.run_steps(2);

        let last = h
            .state()
            .cmd
            .history()
            .last()
            .map(|e| e.text.clone())
            .unwrap_or_default();
        assert!(
            last.contains("editing phase"),
            "the reply did not explain itself: {last:?}"
        );

        // And it has to be on screen, not merely in the log.
        let shown = h.state().command_open;
        assert!(!shown, "the test is not exercising the collapsed bar");
        assert!(
            h.state().pending.is_none(),
            "a planned verb armed something, so the bar would show that instead"
        );
    }

    /// Editing the words that are already on a page: click a run, retype it.
    #[test]
    fn clicking_a_run_offers_its_words_and_retyping_replaces_them() {
        let mut h = harness("text-lines.pdf");
        h.state_mut().submit("edittext");
        h.run_steps(1);
        assert!(h.state().pending.is_some(), "edittext did not ask which words");

        let at = a_character_on_screen(&mut h);
        click(&mut h, at);
        h.run_steps(2);

        // The run's own words go into the box, ready to be edited — a run is
        // usually a sentence, and retyping one from scratch is not editing.
        // The editor opens **on the page**, holding the run's own words —
        // editing a word is a thing you do to the word.
        let run = h.state().editing_run.clone().expect("no run was picked");
        assert!(!run.buffer.is_empty(), "the editor opened empty");
        assert_eq!(
            run.buffer.trim(),
            run.original.trim(),
            "the editor is not showing the run's words"
        );
        let original = run.original;

        h.state_mut().submit("REPLACED");
        h.run_steps(2);

        let app = h.state_mut();
        app.text = None;
        let page = app.characters(0).map(|c| c.text()).unwrap_or_default();
        assert!(page.contains("REPLACED"), "the words did not change:\n{page}");
        assert!(!page.contains(original.trim()), "the old words are still there");
    }

    /// **Clicking away from the editor must not be the end of it.**
    ///
    /// Reported from use: "once i select a text to edit it and if i click
    /// somewhere else the cursor is gone and i cant bring it back. once i edit
    /// then i cant apply the change."
    ///
    /// egui gives a click to the *last* widget registered over that spot. The
    /// page declares itself over its whole rectangle, so anything drawn before
    /// it — the editor, and the Apply button under it — never sees a click at
    /// all. The lock badges hit this and were moved; the editor was left where
    /// it was.
    #[test]
    fn the_run_editor_can_be_clicked_back_into_after_clicking_away() {
        let mut h = harness("text-lines.pdf");
        h.state_mut().submit("edittext");
        h.run_steps(1);

        let word = a_character_on_screen(&mut h);
        click(&mut h, word);
        h.run_steps(2);

        let run = h.state().editing_run.clone().expect("no run was picked");
        let id = egui::Id::new(("run-editor", run.page, run.object));
        assert!(
            h.ctx.memory(|m| m.has_focus(id)),
            "the editor did not take the caret when it opened"
        );

        // Where the editor is on screen, which is over the run it replaces.
        let view = h.state().last_view.expect("the page was never drawn");
        let box_left = view.to_screen(AppPoint {
            x: run.rect.left.min(run.rect.right) as f64,
            y: run.rect.top.min(run.rect.bottom) as f64,
        });
        let box_right = view.to_screen(AppPoint {
            x: run.rect.left.max(run.rect.right) as f64,
            y: run.rect.top.max(run.rect.bottom) as f64,
        });
        let inside = egui::pos2(
            (box_left.x + box_right.x) / 2.0,
            (box_left.y + box_right.y) / 2.0,
        );

        // Away — somewhere else on the page entirely.
        click(&mut h, egui::pos2(box_right.x + 240.0, box_right.y + 180.0));
        h.run_steps(2);

        // And back into the editor, which is the part that could not be done.
        click(&mut h, inside);
        h.run_steps(2);

        assert!(
            h.state().editing_run.is_some(),
            "the edit was abandoned by a click somewhere else"
        );
        assert!(
            h.ctx.memory(|m| m.has_focus(id)),
            "the caret could not be put back in the editor"
        );
    }

    /// **Typing in the editor and pressing Enter applies the change.**
    ///
    /// The way anybody would try it first, before looking for a button.
    #[test]
    fn typing_in_the_run_editor_and_pressing_enter_applies_it() {
        let mut h = harness("text-lines.pdf");
        h.state_mut().submit("edittext");
        h.run_steps(1);
        let word = a_character_on_screen(&mut h);
        click(&mut h, word);
        h.run_steps(2);

        let original = h.state().editing_run.clone().expect("no run").original;

        // Clear what is there and type over it, through the field itself.
        h.input_mut().events.push(egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        });
        h.run_steps(1);
        h.input_mut().events.push(egui::Event::Text("TYPED".into()));
        h.run_steps(1);
        for pressed in [true, false] {
            h.input_mut().events.push(egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: Default::default(),
            });
        }
        h.run_steps(2);

        assert!(h.state().editing_run.is_none(), "Enter did not finish the edit");
        let app = h.state_mut();
        app.text = None;
        let page = app.characters(0).map(|c| c.text()).unwrap_or_default();
        assert!(
            page.contains("TYPED"),
            "Enter changed nothing on the page:\n{page}\nwas: {original}"
        );
    }

    /// And the Apply button beside it can be pressed, for the same reason.
    #[test]
    fn the_run_editor_apply_button_can_be_pressed() {
        use egui_kittest::kittest::Queryable;

        let mut h = harness("text-lines.pdf");
        h.state_mut().submit("edittext");
        h.run_steps(1);
        let word = a_character_on_screen(&mut h);
        click(&mut h, word);
        h.run_steps(2);

        let run = h.state().editing_run.clone().expect("no run was picked");
        h.state_mut().editing_run.as_mut().expect("editing").buffer = "REPLACED".into();
        h.run_steps(1);

        let apply = h.get_by_label("Apply");
        apply.click();
        h.run_steps(2);

        assert!(
            h.state().editing_run.is_none(),
            "Apply did not finish the edit"
        );
        let app = h.state_mut();
        app.text = None;
        let page = app.characters(0).map(|c| c.text()).unwrap_or_default();
        assert!(
            page.contains("REPLACED"),
            "Apply changed nothing on the page:\n{page}\nwas: {}",
            run.original
        );
    }

    /// **A tool part-way through shows what it is making.**
    ///
    /// Reported from use: "in draw i need to click the start and end points for
    /// a drawn object to show up — when i make the 1st click it should start
    /// drawing." Until the last click there was nothing on screen to say where
    /// the first point had landed or what shape was coming.
    #[test]
    fn a_half_placed_tool_previews_what_it_would_make() {
        let mut h = harness("pages-ladder.pdf");
        h.state_mut().submit("line");
        h.run_steps(2);
        assert!(h.state().pending.is_some(), "the tool was not armed");

        // Nothing to preview before the first click.
        let view = h.state().last_view.expect("the page was drawn");
        let start = view.to_screen(AppPoint { x: 100.0, y: 100.0 });
        click(&mut h, start);
        h.run_steps(2);

        let pending = h.state().pending.as_ref().expect("still collecting");
        assert_eq!(pending.points.len(), 1, "the first click did not land");

        // With one point placed and the pointer somewhere else, the preview has
        // something to draw. Drawing is painting, so what is checked is that it
        // runs over a real pointer position without panicking and that the tool
        // is still waiting for its second click.
        h.input_mut()
            .events
            .push(egui::Event::PointerMoved(egui::pos2(start.x + 40.0, start.y + 25.0)));
        h.run_steps(2);
        assert!(
            h.state().pending.as_ref().is_some_and(|p| p.points.len() == 1),
            "moving the pointer finished the line by itself"
        );

        // And the second click completes it — after which the tool arms itself
        // again for the next one, which is what makes drawing several bearable.
        click(&mut h, egui::pos2(start.x + 40.0, start.y + 25.0));
        h.run_steps(2);
        let said = h
            .state()
            .cmd
            .history()
            .iter()
            .map(|e| e.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(said.contains("line added"), "the second click did not finish it:\n{said}");
        assert!(
            h.state().pending.as_ref().is_some_and(|p| p.points.is_empty()),
            "it did not come back ready for the next line"
        );
    }

    /// **A tool armed on one page works on the page you click.**
    ///
    /// A part-way pick belongs to its own page — one end of a line on page 40
    /// and the other on page 41 is coordinates that mean nothing on either. But
    /// the *first* click of a one-click tool was bound the same way, so arming
    /// while one page was current and clicking another visible one did nothing
    /// at all: no mark, no message, nothing to read.
    #[test]
    fn a_tool_that_has_collected_nothing_answers_a_click_on_another_page() {
        let mut h = harness("pages-ladder.pdf");
        // Small enough that several pages share the window.
        h.state_mut().submit("zoom 25");
        h.run_steps(6);

        h.state_mut().submit("edittext");
        h.run_steps(2);
        let armed_for = h.state().pending.as_ref().map(|p| p.page).expect("armed");

        // Down the strip, past the page the tool was armed on.
        // Where the next page actually sits, worked out from the page the
        // view is anchored on and the strip's own layout.
        let target = {
            let app = h.state();
            let doc = app.doc.as_ref().expect("open");
            let view = app.last_view.expect("the page was drawn");
            let here = doc.strip.top_of(armed_for).expect("a top");
            let next = armed_for + 1;
            let (Some(top), Some((w, height))) =
                (doc.strip.top_of(next), doc.strip.size_of(next))
            else {
                return;
            };
            let left = doc.strip.left_of(next).unwrap_or(0.0)
                - doc.strip.left_of(armed_for).unwrap_or(0.0);
            egui::pos2(
                view.origin.x + (left + w / 2.0) * view.scale,
                view.origin.y + (top - here + height / 2.0) * view.scale,
            )
        };

        let before = h.state().cmd.history().len();
        click(&mut h, target);
        h.run_steps(3);
        let answered = h.state().pending.as_ref().is_some_and(|p| p.page != armed_for)
            || h.state().cmd.history().len() > before
            || h.state().editing_run.is_some();

        assert!(
            answered,
            "clicking a page the tool was not armed on did nothing at all — \
             no pick, no message, nothing to read"
        );
    }

    /// **The pointer must not be pulled about when no tool is placing points.**
    ///
    /// Snapping, ortho and the grid exist to put a line exactly on the end of
    /// another line. Applied to ordinary clicking they drag the pointer away
    /// from whatever the user was aiming at — a word, a run to edit, somebody
    /// else's highlight — and the page feels like it is fighting them.
    #[test]
    fn clicking_is_not_pulled_to_nearby_geometry() {
        let mut h = harness("text-lines.pdf");

        // A grid coarse enough that any snapping would be unmistakable, and a
        // mark on the page so the snap engine has something to pull towards.
        h.state_mut().grid_pt = 72.0;
        h.state_mut().submit("l 20,20 300,300");
        h.run_steps(2);

        let start = a_character_on_screen(&mut h);
        drag(&mut h, start, start + egui::vec2(150.0, 0.0));

        assert!(
            h.state().text_selection.is_some(),
            "the drag was snapped off the text and selected nothing"
        );
        assert!(h.state().last_snap.is_none(), "a snap was applied with no tool in hand");
    }

    /// And it must still snap when a tool *is* placing points — that is what it
    /// is for.
    /// And it must still snap when a tool *is* placing points — that is what it
    /// is for.
    #[test]
    fn a_drawing_tool_still_snaps_to_what_is_there() {
        let mut h = harness("single-page.pdf");

        // Drawn with clicks, so the line and the later hover are in the same
        // space. A typed `l 40,40 200,40` is kernel space, y up from the
        // bottom — the same numbers and a different place.
        // Points inside the page, worked out from where it was actually drawn
        // — a guessed screen position lands off a 200pt-wide sheet.
        let (a, b) = {
            let view = h.state().last_view.expect("page never drawn");
            (
                view.to_screen(AppPoint { x: 40.0, y: 60.0 }),
                view.to_screen(AppPoint { x: 150.0, y: 60.0 }),
            )
        };
        h.state_mut().submit("line");
        h.run_steps(1);
        click(&mut h, a);
        click(&mut h, b);
        h.run_steps(2);
        assert_eq!(
            h.state().markup.existing(0).map(|l| l.len()).unwrap_or(0),
            1,
            "the line was not drawn, so there is nothing to snap to"
        );

        h.state_mut().submit("line");
        h.run_steps(1);
        // Near the end of that line, but not on it.
        h.input_mut().events.push(egui::Event::PointerMoved(b + egui::vec2(3.0, 3.0)));
        h.run_steps(2);

        assert!(
            h.state().last_snap.is_some(),
            "the line tool did not snap to the end of the line beside it"
        );
    }

    /// **The white-text bug, from the outside.**
    ///
    /// Editing white text on a dark banner brought it back black — which is to
    /// say the words disappeared. An edit changes what it was asked to change
    /// and nothing else.
    #[test]
    fn editing_words_does_not_change_how_they_look() {
        let mut h = harness("text-lines.pdf");
        h.state_mut().submit("edittext");
        h.run_steps(1);
        let at = a_character_on_screen(&mut h);
        click(&mut h, at);
        h.run_steps(2);

        let before = {
            let app = h.state();
            let edit = app.editing_run.as_ref().expect("no run");
            app.doc
                .as_ref()
                .unwrap()
                .session
                .text_runs(0)
                .expect("runs")
                .into_iter()
                .find(|r| r.object == edit.object)
                .expect("the run")
        };

        h.state_mut().submit("REPLACED");
        h.run_steps(2);

        let after = h
            .state()
            .doc
            .as_ref()
            .unwrap()
            .session
            .text_runs(0)
            .expect("runs")
            .into_iter()
            .find(|r| r.object == before.object)
            .expect("the run");

        assert_eq!(after.text.trim(), "REPLACED", "the words did not change");
        assert_eq!(after.color, before.color, "the colour changed on its own");
        assert!(
            (after.size - before.size).abs() < 0.5,
            "the size changed on its own: {} then {}",
            before.size,
            after.size
        );
    }

    /// The properties are editable, which is the other half of the ask.
    #[test]
    fn a_run_can_be_recoloured_from_the_editor() {
        let mut h = harness("text-lines.pdf");
        h.state_mut().submit("edittext");
        h.run_steps(1);
        let at = a_character_on_screen(&mut h);
        click(&mut h, at);
        h.run_steps(2);

        let object = {
            let app = h.state_mut();
            let edit = app.editing_run.as_mut().expect("no run");
            edit.style.color =
                Some(pdf_core::document::Color { r: 250, g: 40, b: 40, a: 255 });
            edit.style.size = Some(22.0);
            edit.object
        };
        // Applying with the words untouched: only the look changes.
        let words = h.state().editing_run.as_ref().unwrap().original.clone();
        h.state_mut().submit(&words);
        h.run_steps(2);

        let run = h
            .state()
            .doc
            .as_ref()
            .unwrap()
            .session
            .text_runs(0)
            .expect("runs")
            .into_iter()
            .find(|r| r.object == object)
            .expect("the run");
        assert_eq!(run.color.r, 250, "the colour was not applied");
        assert!((run.size - 22.0).abs() < 0.5, "the size was not applied: {}", run.size);
    }

    #[test]
    fn an_edited_run_can_be_undone() {
        let mut h = harness("text-lines.pdf");
        h.state_mut().submit("edittext");
        h.run_steps(1);
        let at = a_character_on_screen(&mut h);
        click(&mut h, at);
        h.run_steps(2);
        let original = h.state().editing_run.clone().expect("no run").original;

        h.state_mut().submit("REPLACED");
        h.run_steps(2);
        h.state_mut().submit("undo");
        h.run_steps(2);

        let app = h.state_mut();
        app.text = None;
        let page = app.characters(0).map(|c| c.text()).unwrap_or_default();
        assert!(page.contains(original.trim()), "undo did not restore the words:\n{page}");
        assert!(!page.contains("REPLACED"), "the replacement is still there");
    }

    /// Escape leaves the words alone — the safe answer, and the same key that
    /// stops everything else.
    #[test]
    fn escape_leaves_the_words_as_they_were() {
        let mut h = harness("text-lines.pdf");
        h.state_mut().submit("edittext");
        h.run_steps(1);
        let at = a_character_on_screen(&mut h);
        click(&mut h, at);
        h.run_steps(2);
        let original = h.state().editing_run.clone().expect("no run").original;

        h.state_mut().escape();
        h.run_steps(1);
        assert!(h.state().editing_run.is_none(), "escape did not stop the edit");

        let app = h.state_mut();
        app.text = None;
        let page = app.characters(0).map(|c| c.text()).unwrap_or_default();
        assert!(page.contains(original.trim()), "escape changed the words anyway");
    }

    #[test]
    fn clicking_bare_paper_says_there_is_no_text_there() {
        let mut h = harness("text-lines.pdf");
        h.state_mut().submit("edittext");
        h.run_steps(1);

        // Below every run there is, worked out from the runs themselves — a
        // guessed offset lands inside a paragraph as often as not, and then the
        // test passes for the wrong reason.
        let below = {
            let app = h.state();
            let runs = app.doc.as_ref().unwrap().session.text_runs(0).expect("runs");
            let lowest = runs
                .iter()
                .map(|r| r.rect.top.max(r.rect.bottom))
                .fold(0.0f32, f32::max);
            let view = app.last_view.expect("page never drawn");
            view.to_screen(AppPoint { x: 40.0, y: (lowest + 30.0) as f64 })
        };
        click(&mut h, below);
        h.run_steps(2);

        let said = h
            .state()
            .cmd
            .history()
            .iter()
            .map(|e| e.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(said.contains("no text there"), "no explanation:\n{said}");
        assert!(h.state().editing_run.is_none());
    }

    /// Words written onto a page are **real text objects**, not a picture of
    /// text — they select, search and copy like anything else on the page.
    #[test]
    fn writing_words_puts_selectable_text_on_the_page() {
        let mut h = harness("text-lines.pdf");
        let before = {
            let app = h.state_mut();
            app.characters(0).map(|c| c.len()).unwrap_or(0)
        };

        h.state_mut().submit("addtext DRAFT");
        h.run_steps(1);
        assert!(h.state().pending.is_some(), "addtext did not ask where");

        let at = a_character_on_screen(&mut h) + egui::vec2(0.0, 90.0);
        click(&mut h, at);
        h.run_steps(2);

        let app = h.state_mut();
        app.text = None;
        let after = app.characters(0).map(|c| c.text()).unwrap_or_default();
        assert!(
            after.contains("DRAFT"),
            "the words are not in the page's text (was {before} characters):\n{after}"
        );
    }

    /// It is an edit, so it comes back off.
    #[test]
    fn written_words_can_be_undone() {
        let mut h = harness("text-lines.pdf");
        h.state_mut().submit("addtext DRAFT");
        h.run_steps(1);
        let at = a_character_on_screen(&mut h) + egui::vec2(0.0, 90.0);
        click(&mut h, at);
        h.run_steps(2);

        h.state_mut().submit("undo");
        h.run_steps(2);
        let app = h.state_mut();
        app.text = None;
        let after = app.characters(0).map(|c| c.text()).unwrap_or_default();
        assert!(!after.contains("DRAFT"), "undo left the words on the page");
    }

    #[test]
    fn writing_nothing_is_refused_rather_than_placing_an_empty_mark() {
        let mut h = harness("text-lines.pdf");
        h.state_mut().submit("addtext");
        h.run_steps(1);
        assert!(h.state().pending.is_none(), "an empty string armed the tool");
    }

    /// A highlighter is a thing you pick up and then use.
    ///
    /// Requiring the selection first means the tool can only ever be applied
    /// once per selection, and a button that answers "select some text first"
    /// reads as a scolding rather than a tool.
    #[test]
    fn a_markup_tool_can_be_picked_up_and_then_used() {
        let mut h = harness("text-lines.pdf");
        h.state_mut().submit("highlight");
        h.run_steps(1);
        assert!(h.state().markup_armed.is_some(), "the tool was not picked up");
        assert_eq!(
            h.state().doc.as_ref().unwrap().session.annotations(0).unwrap().len(),
            0,
            "picking the tool up marked something"
        );

        let start = a_character_on_screen(&mut h);
        drag(&mut h, start, start + egui::vec2(150.0, 0.0));

        let marks = h.state().doc.as_ref().unwrap().session.annotations(0).expect("read");
        assert_eq!(marks.len(), 1, "dragging with the tool in hand marked nothing");
    }

    /// It stays in hand: a highlighter is not put down after one sentence.
    #[test]
    fn the_tool_stays_in_hand_for_a_second_passage() {
        let mut h = harness("text-lines.pdf");
        h.state_mut().submit("highlight");
        h.run_steps(1);

        // Both drags along the same line: a drag that begins on blank paper
        // box-selects marks instead of text, correctly, and would prove nothing
        // about the tool staying in hand.
        let start = a_character_on_screen(&mut h);
        drag(&mut h, start, start + egui::vec2(150.0, 0.0));
        drag(&mut h, start + egui::vec2(20.0, 0.0), start + egui::vec2(170.0, 0.0));

        let marks = h.state().doc.as_ref().unwrap().session.annotations(0).expect("read");
        assert_eq!(marks.len(), 2, "the second passage was not marked");
    }

    #[test]
    fn escape_puts_the_tool_down() {
        let mut h = harness("text-lines.pdf");
        h.state_mut().submit("underline");
        h.run_steps(1);
        assert!(h.state().markup_armed.is_some());

        h.state_mut().escape();
        h.run_steps(1);
        assert!(h.state().markup_armed.is_none(), "escape did not put it down");

        let start = a_character_on_screen(&mut h);
        drag(&mut h, start, start + egui::vec2(150.0, 0.0));
        let marks = h.state().doc.as_ref().unwrap().session.annotations(0).expect("read");
        assert!(marks.is_empty(), "it marked something after being put down");
    }

    /// The old way still works — text already selected, then the tool.
    #[test]
    fn a_tool_pressed_with_text_already_selected_marks_it_at_once() {
        let mut h = harness("text-lines.pdf");
        let start = a_character_on_screen(&mut h);
        drag(&mut h, start, start + egui::vec2(150.0, 0.0));
        assert!(h.state().text_selection.is_some());

        h.state_mut().submit("highlight");
        h.run_steps(1);

        let marks = h.state().doc.as_ref().unwrap().session.annotations(0).expect("read");
        assert_eq!(marks.len(), 1, "an existing selection was not marked");
        assert!(h.state().markup_armed.is_none(), "it should not also be left in hand");
    }

    /// Zoom a few steps at a screen position, the way a pinch arrives.
    fn pinch_at(h: &mut Harness<'static, PagifyApp>, at: egui::Pos2, factor: f32, steps: usize) {
        for _ in 0..steps {
            h.input_mut().events.push(egui::Event::PointerMoved(at));
            h.input_mut().events.push(egui::Event::Zoom(factor));
            h.run_steps(1);
        }
        h.run_steps(2);
    }

    /// **The property zooming is judged by.** Whatever is under the cursor when
    /// you start must still be under it when you stop — that is the point on
    /// the page you were asking about.
    #[test]
    fn zooming_holds_the_point_under_the_cursor() {
        let mut h = harness("text-lines.pdf");
        // Started zoomed in, because anchoring is only possible on an axis that
        // can actually scroll. While the whole page fits in the window there is
        // no offset to move, so it can only grow — correctly, and with nothing
        // to hold still.
        h.state_mut().zoom = ZoomMode::Factor(3.0);
        h.run_steps(3);

        let at = egui::pos2(700.0, 600.0);
        let before = h.state().last_view.expect("page never drawn").to_page(at);

        pinch_at(&mut h, at, 1.06, 8);

        let app = h.state();
        assert!(
            app.resolved_zoom() > 1.2,
            "the pinch did not zoom (scale {:.2})",
            app.resolved_zoom()
        );

        let after = app.last_view.expect("page never drawn").to_page(at);
        let drift = ((after.x - before.x).powi(2) + (after.y - before.y).powi(2)).sqrt();
        assert!(
            drift < 6.0,
            "the page slid {drift:.1}pt out from under the cursor while zooming \
             ({before:?} -> {after:?})"
        );
    }

    /// And zooming out must hold it too, or the anchor is only half right.
    #[test]
    fn zooming_out_holds_the_point_too() {
        let mut h = harness("text-lines.pdf");
        // High enough that the page still overflows the window at the end of
        // the gesture, so there is an offset to hold all the way through.
        // Far enough in, and parked well away from both ends, that the offset
        // the anchor asks for is actually reachable throughout.
        //
        // Anchoring is only possible while there is scroll range to spend. Near
        // either end of the document — or at a zoom where the page barely
        // overflows the window — holding a point would need an offset outside
        // the scrollable range, and it gets clamped. That is the document
        // running out, not the anchor being wrong, and a test that ignores the
        // difference measures the clamp instead of the thing it is named after.
        h.state_mut().zoom = ZoomMode::Factor(10.0);
        h.run_steps(3);
        h.state_mut().anchor_offset = Some(egui::vec2(1200.0, 2500.0));
        h.run_steps(3);

        let at = egui::pos2(700.0, 600.0);
        let before = h.state().last_view.expect("page never drawn").to_page(at);

        pinch_at(&mut h, at, 0.985, 6);

        let app = h.state();
        assert!(
            app.resolved_zoom() < 9.6,
            "the pinch did not zoom out (scale {:.2})",
            app.resolved_zoom()
        );

        let after = app.last_view.expect("page never drawn").to_page(at);
        let drift = ((after.x - before.x).powi(2) + (after.y - before.y).powi(2)).sqrt();
        assert!(
            drift < 6.0,
            "the page slid {drift:.1}pt while zooming out (dx {:.1}, dy {:.1}) \
             zoom {:.2} offset {:?}",
            after.x - before.x,
            after.y - before.y,
            app.resolved_zoom(),
            app.scroll_offset
        );
    }

    /// The middle button pans whatever tool is held — the CAD convention, and a
    /// button nothing else here uses.
    #[test]
    fn the_middle_button_pans() {
        let mut h = harness("pages-ladder.pdf");
        // Zoomed in, so there is somewhere to pan *to*. At Fit the whole strip
        // is inside the window and the offset cannot move — which is correct,
        // and would have made this test pass for the wrong reason.
        h.state_mut().zoom = ZoomMode::Factor(4.0);
        h.run_steps(3);
        let before = h.state().scroll_offset;

        let from = egui::pos2(700.0, 700.0);
        drag_with(&mut h, egui::PointerButton::Middle, from, from - egui::vec2(0.0, 200.0));

        let after = h.state().scroll_offset;
        assert!(
            (after.y - before.y).abs() > 20.0,
            "the middle button did not move the view ({before:?} -> {after:?})"
        );
    }

    /// It must not disturb a tool that is part-way through, which is most of
    /// why it is worth having: you can move the paper without putting the tool
    /// down.
    #[test]
    fn panning_with_the_middle_button_leaves_an_armed_tool_alone() {
        let mut h = harness("pages-ladder.pdf");
        h.state_mut().zoom = ZoomMode::Factor(4.0);
        h.state_mut().submit("line");
        h.run_steps(3);

        let from = egui::pos2(700.0, 700.0);
        drag_with(&mut h, egui::PointerButton::Middle, from, from - egui::vec2(0.0, 150.0));

        let app = h.state();
        assert!(app.pending.is_some(), "panning disarmed the line tool");
        assert!(
            app.pending.as_ref().unwrap().points.is_empty(),
            "panning fed a point into the tool"
        );
    }

    /// Hand mode wrote to a field nothing reads, so dragging with it moved
    /// nothing.
    #[test]
    fn hand_mode_actually_moves_the_page() {
        let mut h = harness("pages-ladder.pdf");
        h.state_mut().zoom = ZoomMode::Factor(4.0);
        h.state_mut().submit("hand");
        h.run_steps(3);
        let before = h.state().scroll_offset;

        let from = egui::pos2(700.0, 700.0);
        drag(&mut h, from, from - egui::vec2(0.0, 200.0));

        let after = h.state().scroll_offset;
        assert!(
            (after.y - before.y).abs() > 20.0,
            "Hand did not move the page ({before:?} -> {after:?})"
        );
    }

    #[test]
    fn dragging_over_text_selects_it() {
        let mut h = harness("text-lines.pdf");
        let start = a_character_on_screen(&mut h);
        let end = start + egui::vec2(140.0, 0.0);

        drag(&mut h, start, end);

        let app = h.state();
        assert!(
            app.text_selection.is_some(),
            "a drag across text selected nothing.\npointer mode: {:?}\nfrom {start:?} to {end:?}\n{}",
            app.pointer,
            app.cmd.history().iter().map(|e| e.text.as_str()).collect::<Vec<_>>().join("\n")
        );
    }

    /// The first of the two states that stop selection, and the one that is
    /// easiest to end up in by accident: Hand is the leftmost button on every
    /// tab.
    #[test]
    fn hand_mode_stops_selection_and_select_gives_it_back() {
        let mut h = harness("text-lines.pdf");
        let start = a_character_on_screen(&mut h);
        let end = start + egui::vec2(140.0, 0.0);

        h.state_mut().submit("hand");
        h.run_steps(2);
        drag(&mut h, start, end);
        assert!(
            h.state().text_selection.is_none(),
            "Hand mode selected text, so a drag both scrolls and selects"
        );

        h.state_mut().submit("selecttool");
        h.run_steps(2);
        drag(&mut h, start, end);
        assert!(
            h.state().text_selection.is_some(),
            "Select did not give selection back, which would make Hand a one-way trip"
        );
    }

    /// The second state: a tool waiting for clicks takes them. That is correct
    /// — but it means "selection stopped working" and "a tool is armed" look
    /// identical unless something says so, which is why the armed tool lights
    /// up in the ribbon and Escape gets out.
    #[test]
    fn an_armed_tool_takes_the_click_and_escape_gives_it_back() {
        let mut h = harness("text-lines.pdf");
        let start = a_character_on_screen(&mut h);
        let end = start + egui::vec2(140.0, 0.0);

        h.state_mut().submit("line");
        h.run_steps(2);
        assert!(h.state().pending.is_some());

        drag(&mut h, start, end);
        assert!(
            h.state().text_selection.is_none(),
            "an armed tool let the drag select as well, so a click would do two things"
        );

        h.state_mut().escape();
        h.run_steps(2);
        assert!(h.state().pending.is_none(), "escape did not disarm the tool");

        drag(&mut h, start, end);
        assert!(
            h.state().text_selection.is_some(),
            "selection did not come back after escape"
        );
    }

    /// Mice move. egui calls a press-move-release a drag rather than a click
    /// even when the movement is a pixel, and a tool that throws those away
    /// registers most clicks and loses some, which is indistinguishable from
    /// being broken.
    #[test]
    fn a_click_that_wandered_a_pixel_still_counts() {
        let mut h = harness("text-lines.pdf");
        h.state_mut().submit("line");
        h.run_steps(2);

        let base = a_character_on_screen(&mut h) + egui::vec2(0.0, 120.0);
        drag(&mut h, base, base + egui::vec2(1.5, 1.0));
        drag(&mut h, base + egui::vec2(150.0, 60.0), base + egui::vec2(151.0, 61.0));
        h.run_steps(2);

        let app = h.state();
        let marks = app.markup.existing(app.page).map(|l| l.len()).unwrap_or(0);
        assert_eq!(
            marks, 1,
            "two clicks that each moved a pixel drew nothing.\n{}",
            app.cmd.history().iter().map(|e| e.text.as_str()).collect::<Vec<_>>().join("\n")
        );
    }

    #[test]
    fn the_line_tool_draws_where_it_is_clicked() {
        let mut h = harness("text-lines.pdf");
        h.state_mut().submit("line");
        h.run_steps(2);

        let first = a_character_on_screen(&mut h);
        click(&mut h, first + egui::vec2(0.0, 120.0));
        click(&mut h, first + egui::vec2(180.0, 200.0));
        h.run_steps(2);

        let app = h.state();
        let marks = app.markup.existing(app.page).map(|l| l.len()).unwrap_or(0);
        assert_eq!(
            marks, 1,
            "two clicks on the page drew nothing.\n{}",
            app.cmd.history().iter().map(|e| e.text.as_str()).collect::<Vec<_>>().join("\n")
        );
    }
}

#[cfg(test)]
mod raster_scale_tests {
    use super::raster_scale;

    /// The property that makes zooming smooth: a small change in zoom must
    /// usually produce the *same* raster scale, so no re-render is asked for.
    #[test]
    fn nearby_zooms_share_a_raster_scale() {
        let mut distinct = std::collections::BTreeSet::new();
        // A pinch from 1x to 2x, at the granularity a trackpad delivers.
        for i in 0..=100 {
            let zoom = 1.0 + i as f32 / 100.0;
            distinct.insert(raster_scale(zoom).to_bits());
        }
        assert!(
            distinct.len() <= 5,
            "a 1x-2x pinch asked for {} different rasters; it used to ask for one per frame",
            distinct.len()
        );
    }

    /// And it must stay close enough that nobody sees the difference.
    #[test]
    fn the_raster_is_never_far_from_the_asked_for_size() {
        for i in 1..=400 {
            let asked = i as f32 / 20.0;
            let got = raster_scale(asked);
            let error = (got / asked).max(asked / got);
            assert!(
                error < 1.13,
                "at {asked:.2}x the page would be rasterised at {got:.2}x, {:.0}% out",
                (error - 1.0) * 100.0
            );
        }
    }

    #[test]
    fn it_is_stable_and_monotonic() {
        let mut last = 0.0;
        for i in 1..=200 {
            let got = raster_scale(i as f32 / 10.0);
            assert!(got >= last, "raster scale went backwards as zoom increased");
            assert_eq!(got, raster_scale(i as f32 / 10.0), "not deterministic");
            last = got;
        }
    }
}

#[cfg(test)]
mod redaction_wiring_tests {
    use super::*;

    fn fixture(name: &str) -> String {
        format!(
            "{}/../../../workspace/Pagify/rust/pdf_core/fixtures/{name}",
            env!("CARGO_MANIFEST_DIR")
        )
    }

    fn app(name: &str) -> PagifyApp {
        let app = PagifyApp::new(Some(&fixture(name)));
        assert!(app.doc.is_some(), "{name} did not open");
        app
    }

    fn said(app: &PagifyApp) -> String {
        app.cmd.history().iter().map(|e| e.text.as_str()).collect::<Vec<_>>().join("\n")
    }

    fn page_text(app: &PagifyApp, page: usize) -> String {
        app.doc
            .as_ref()
            .expect("open")
            .session
            .characters(page)
            .map(|c| c.text().to_string())
            .unwrap_or_default()
    }

    /// **The verb used to lie.** `redact` sat in the shell's planned table
    /// answering "a later phase" long after the engine could do it, so the one
    /// destructive feature in the program was unreachable from the one place a
    /// user would look for it.
    #[test]
    fn the_redact_verb_arms_the_tool_rather_than_refusing() {
        let mut app = app("text-lines.pdf");
        app.submit("redact");
        assert!(
            matches!(app.pending.as_ref().map(|p| &p.kind), Some(PendingKind::Redact)),
            "redact did not arm anything:\n{}",
            said(&app)
        );
        assert!(said(&app).contains("first corner"), "it did not say what to do next");
    }

    /// The tool stays in hand until it is put down, like every other tool.
    #[test]
    fn the_redaction_tool_stays_armed() {
        assert!(PendingKind::Redact.repeats());
    }

    /// **Snapping is off for it.** A redaction is placed against words, and the
    /// nearest drawn line has nothing to do with where the words are.
    #[test]
    fn the_redaction_tool_does_not_snap_to_geometry() {
        assert!(!PendingKind::Redact.wants_snapping());
    }

    /// A clean redaction goes straight through, and the words are gone.
    #[test]
    fn redacting_a_clear_area_destroys_the_text() {
        let mut app = app("text-lines.pdf");
        assert!(page_text(&app, 0).contains("The quick brown fox"), "control");

        // "The quick brown fox" sits at L40.3 T49.8 R165.2 B62.8.
        let outcome = app.redact(0, AppPoint::new(35.0, 45.0), AppPoint::new(170.0, 66.0));
        assert!(outcome.is_ok(), "{outcome:?}");
        assert!(app.asking_to_redact.is_none(), "it asked about a clear area");

        let after = page_text(&app, 0);
        assert!(!after.contains("The quick brown fox"), "the words survived: {after:?}");
        assert!(after.contains("jumps over the lazy dog"), "it took more than it was pointed at");
    }

    /// And it undoes, through the same command stack as everything else.
    #[test]
    fn a_redaction_undoes_from_the_command_box() {
        let mut app = app("text-lines.pdf");
        let before = page_text(&app, 0);
        app.redact(0, AppPoint::new(35.0, 45.0), AppPoint::new(170.0, 66.0)).expect("redact");
        assert!(!page_text(&app, 0).contains("The quick brown fox"));

        app.submit("undo");
        assert_eq!(page_text(&app, 0), before, "undo did not put the page back:\n{}", said(&app));
    }

    /// **With a bundled font, an outlined page is no longer an automatic
    /// refusal.** The app now hands `preview_redaction`/`redact` real candidate
    /// faces — see `BUNDLED_OUTLINED_FONTS` — so a rectangle over outlined type
    /// genuinely gets surveyed, and most of what a broad rectangle covers
    /// matches and becomes removable. What replaces the old blanket refusal is
    /// the same acknowledgement path an image beside real text already uses:
    /// offered as a question when something inside the rectangle could not be
    /// identified, not silently guessed at and not silently dropped.
    #[test]
    fn an_outlined_page_now_offers_a_real_redaction_with_the_bundled_font() {
        let mut app = app("outlined.pdf");
        let outcome = app.redact(0, AppPoint::new(10.0, 10.0), AppPoint::new(300.0, 300.0));
        assert!(outcome.is_ok(), "a font-assisted redaction was refused outright: {outcome:?}");

        let asking = app.asking_to_redact.as_ref().expect(
            "a broad rectangle over real outlined type matched everything — unexpected, \
             but not itself wrong; if this legitimately now clears in one step, this test's \
             premise needs revisiting rather than the assertion loosened blindly",
        );
        assert!(asking.report.objects > 0, "the bundled font matched nothing at all: {:?}", asking.report);
        // And whatever it could not identify is still named as outlined type,
        // not silently folded into "removed" or a generic blocker.
        assert!(
            asking
                .report
                .uncleared
                .iter()
                .any(|u| matches!(u, pdf_core::document::Uncleared::OutlinedText { .. })),
            "nothing was left as an honest blocker: {:?}",
            asking.report
        );
    }

    /// **The refusal that must still not be overridable.** A scan's words are
    /// pixels, not paths — no font, bundled or otherwise, changes that, unlike
    /// outlined type above.
    #[test]
    fn a_scan_is_refused_the_same_way() {
        let mut app = app("scan-300dpi.pdf");
        let outcome = app.redact(0, AppPoint::new(50.0, 50.0), AppPoint::new(200.0, 100.0));
        assert!(outcome.is_err());
        assert!(app.asking_to_redact.is_none());
    }

    #[test]
    fn an_area_with_no_size_is_refused_before_anything_else() {
        let mut app = app("text-lines.pdf");
        assert!(app.redact(0, AppPoint::new(50.0, 50.0), AppPoint::new(50.2, 50.2)).is_err());
    }

    /// **The trap the whole feature turns on.** Saving a redacted document
    /// incrementally keeps the original bytes and appends a delta, so every word
    /// removed is still in the file at its old offset. The app must ask before
    /// it picks a save mode, not assume the one that is right the rest of the
    /// time.
    #[test]
    fn a_redacted_document_no_longer_saves_incrementally() {
        let mut app = app("text-lines.pdf");
        assert!(
            !app.doc.as_ref().unwrap().session.must_save_full_copy(),
            "nothing has been redacted yet"
        );

        app.redact(0, AppPoint::new(35.0, 45.0), AppPoint::new(170.0, 66.0)).expect("redact");
        assert!(
            app.doc.as_ref().unwrap().session.must_save_full_copy(),
            "the app would have appended a delta and left the words in the file"
        );
    }

    /// Saving really does write a file the words are not in.
    #[test]
    fn saving_after_a_redaction_writes_a_file_without_the_words() {
        let dir = std::env::temp_dir().join("pagify-redaction-wiring");
        let _ = std::fs::create_dir_all(&dir);
        let target = dir.join("redacted.pdf");
        let _ = std::fs::remove_file(&target);

        let mut app = app("text-lines.pdf");
        app.redact(0, AppPoint::new(35.0, 45.0), AppPoint::new(170.0, 66.0)).expect("redact");
        app.save(Some(target.clone()));
        assert!(target.exists(), "nothing was written:\n{}", said(&app));

        let reopened = PagifyApp::new(Some(target.to_str().expect("path")));
        let after = page_text(&reopened, 0);
        assert!(
            !after.contains("The quick brown fox"),
            "the saved file still has the words: {after:?}"
        );
        assert!(after.contains("jumps over the lazy dog"), "the rest of the page went missing");

        let _ = std::fs::remove_file(&target);
    }
}

#[cfg(test)]
mod lock_wiring_tests {
    use super::*;

    fn fixture(name: &str) -> String {
        format!(
            "{}/../../../workspace/Pagify/rust/pdf_core/fixtures/{name}",
            env!("CARGO_MANIFEST_DIR")
        )
    }

    fn app(name: &str) -> PagifyApp {
        let app = PagifyApp::new(Some(&fixture(name)));
        assert!(app.doc.is_some(), "{name} did not open");
        app
    }

    fn said(app: &PagifyApp) -> String {
        app.cmd.history().iter().map(|e| e.text.as_str()).collect::<Vec<_>>().join("\n")
    }

    fn page_text(app: &PagifyApp, page: usize) -> String {
        app.doc
            .as_ref()
            .expect("open")
            .session
            .characters(page)
            .map(|c| c.text().to_string())
            .unwrap_or_default()
    }

    const FOX: (f32, f32, f32, f32) = (35.0, 45.0, 170.0, 66.0);

    fn fox_area() -> pdf_core::document::Rect {
        pdf_core::document::Rect { left: FOX.0, top: FOX.1, right: FOX.2, bottom: FOX.3 }
    }

    /// **The survey reports without changing anything.**
    #[test]
    fn hiddendata_says_what_is_there_and_offers_to_remove_it() {
        let mut app = app("two-column.pdf");
        app.submit("hiddendata");

        let said = said(&app);
        assert!(!said.is_empty(), "it said nothing at all");
        // Either there is nothing, or it says how to remove what there is.
        assert!(
            said.contains("nothing hidden") || said.contains("hiddendata clean"),
            "it reported findings without saying what to do about them: {said}"
        );
    }

    /// And cleaning says what went, then leaves nothing for a second survey.
    #[test]
    fn hiddendata_clean_removes_and_a_second_survey_is_empty() {
        let mut app = app("two-column.pdf");
        app.submit("hiddendata clean");
        assert!(
            !said(&app).is_empty() && !said(&app).contains("don't know"),
            "cleaning failed: {}",
            said(&app)
        );

        app.submit("hiddendata");
        assert!(
            said(&app).contains("nothing hidden"),
            "a second survey still found something: {}",
            said(&app)
        );
    }

    /// **The window offers the choice, and warns about the one that shuts
    /// everyone out.**
    #[test]
    fn the_password_window_offers_secure_and_secure_plus() {
        // This module is not the one that usually drives a window, so it brings
        // in what it needs itself.
        use eframe::App as _;
        use egui_kittest::kittest::Queryable;
        use egui_kittest::Harness;

        let app = app("two-column.pdf");
        let mut h = Harness::builder()
            .with_size(egui::vec2(1200.0, 900.0))
            .build_ui_state(
                |ui, app: &mut PagifyApp| {
                    let mut frame = eframe::Frame::_new_kittest();
                    app.ui(ui, &mut frame);
                },
                app,
            );
        h.run_steps(2);

        h.state_mut().submit("secure");
        h.run();
        h.get_by_label_contains("Choose a password for this file");

        // Both offered, and the ordinary one says it is ordinary.
        h.get_by_label("Secure");
        h.get_by_label("Secure Plus");
        h.get_by_label_contains("Any reader will ask for this password");

        // Choosing the other says plainly what it costs, before it is used.
        h.get_by_label("Secure Plus").click();
        h.run();
        h.get_by_label_contains("Nothing else will open this file");
    }

    /// And the choice reaches the document.
    #[test]
    fn choosing_secure_plus_uses_pagifys_own_handler() {
        let mut app = app("two-column.pdf");
        app.submit("secure");
        app.password_plus = true;
        app.answer_passcode("Correct-Horse-99-Battery");
        app.answer_passcode("Correct-Horse-99-Battery");

        let doc = app.doc.as_ref().expect("doc");
        assert!(doc.session.is_secured(), "no password was set:\n{}", said(&app));
        assert!(
            doc.session.is_secure_plus(),
            "it used PDF's handler when Secure Plus was chosen"
        );
    }

    /// **Bare `timestamp` names no authority and contacts nothing.**
    ///
    /// The one verb that would use the network says what it would send and
    /// where it would not send it, rather than picking somewhere.
    #[test]
    fn timestamp_without_an_address_explains_rather_than_choosing_one() {
        let mut app = app("two-column.pdf");
        app.submit("timestamp");

        let said = said(&app);
        assert!(said.contains("no default"), "it did not say there is no default: {said}");
        assert!(said.contains("digest"), "it did not say what would be sent: {said}");
        assert!(
            said.contains("nothing else"),
            "it did not say what would not be sent: {said}"
        );
    }

    /// And an address that is not one is refused before anything is opened.
    #[test]
    fn timestamp_refuses_an_address_it_cannot_use() {
        let mut app = app("two-column.pdf");
        app.submit("timestamp not-an-address");
        assert!(said(&app).contains("http://"), "{}", said(&app));
    }

    /// **Signing says the thing people learn the hard way.**
    ///
    /// A signature covers the file as it stands; the next edit breaks it. That
    /// belongs in the line somebody reads when it works, not in a manual.
    #[test]
    fn signing_says_that_a_later_edit_breaks_it() {
        let certificate = std::path::Path::new(
            "/Users/hsilighting/workspace/Pagify/rust/pdf_core/fixtures/test-signer.p12",
        );
        if !certificate.is_file() {
            eprintln!("skipping: no test certificate");
            return;
        }

        let mut app = app("two-column.pdf");
        app.submit("certify");
        assert!(said(&app).contains("not signed"), "{}", said(&app));

        app.submit(&format!("certify {}", certificate.display()));
        assert!(
            matches!(app.awaiting_password, Some(Awaiting::Certificate(_))),
            "it did not ask for the certificate's password:\n{}",
            said(&app)
        );

        app.answer_passcode("pagify");
        let told = said(&app);
        assert!(told.contains("signed as"), "it did not sign: {told}");
        assert!(
            told.contains("breaks it"),
            "it did not say that an edit afterwards breaks the signature: {told}"
        );

        // And a reader sees it.
        app.submit("certify");
        assert!(said(&app).contains("1 signature"), "{}", said(&app));
    }

    /// A signature pad, and a scratch file to keep it in — never the real one.
    ///
    /// `label` is the *test's* name rather than the fixture's, because these
    /// run in parallel in one process: two tests sharing a path delete each
    /// other's file, and the one that notices is whichever lost the race.
    fn with_signature_pad(name: &str, label: &str) -> (PagifyApp, std::path::PathBuf) {
        let mut app = app(name);
        let path = std::env::temp_dir()
            .join(format!("pagify-test-signatures-{}-{label}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        app.signatures = Default::default();
        app.signatures_path = Some(path.clone());
        (app, path)
    }

    /// A rough scrawl, in a pad's own pixels.
    fn scrawl() -> Vec<Vec<(f32, f32)>> {
        vec![
            vec![(20.0, 120.0), (60.0, 60.0), (100.0, 130.0), (150.0, 55.0)],
            vec![(30.0, 110.0), (170.0, 110.0)],
        ]
    }

    /// **Reaching for the tool with nothing drawn opens the pad.**
    ///
    /// Rather than refusing with "no signature" and leaving somebody to find
    /// the word that makes one. The first press does the first thing.
    #[test]
    fn signature_with_nothing_drawn_yet_opens_the_pad() {
        let (mut app, _path) = with_signature_pad("two-column.pdf", "opens-pad");
        app.submit("signature");

        assert!(app.pad.is_some(), "the pad did not open:\n{}", said(&app));
        assert!(
            app.pad.as_ref().is_some_and(|p| p.then_place),
            "it will not carry on to the click somebody wanted"
        );
        assert!(app.pending.is_none(), "it armed a click with nothing to place");
        assert!(said(&app).contains("this computer"), "{}", said(&app));
    }

    /// **What is drawn is kept, and placed where it is clicked.**
    #[test]
    fn a_drawn_signature_places_on_the_line_that_was_clicked() {
        let (mut app, path) = with_signature_pad("two-column.pdf", "places");
        let told = app.save_drawn_signature("mine", &scrawl()).expect("kept");
        assert!(told.contains("this computer"), "{told}");
        assert!(path.is_file(), "it was not written to {}", path.display());

        // Now the tool places rather than opening the pad again.
        app.submit("signature");
        assert!(app.pad.is_none(), "it opened the pad over a signature it already had");
        assert!(
            matches!(app.pending.as_ref().map(|p| &p.kind), Some(PendingKind::Signature)),
            "the tool was not armed:\n{}",
            said(&app)
        );

        let before = app.doc.as_ref().expect("open").session.annotations(0).expect("read").len();
        let told = app
            .place_signature(0, AppPoint { x: 100.0, y: 400.0 })
            .expect("placed");

        let marks = app.doc.as_ref().expect("open").session.annotations(0).expect("read");
        assert_eq!(marks.len(), before + 1, "nothing was added to the page");
        let ink = marks
            .iter()
            .rev()
            .find_map(|m| match &m.annotation {
                pdf_core::document::Annotation::Ink { strokes, .. } => Some(strokes.clone()),
                _ => None,
            })
            .expect("the signature is not ink on the page");
        assert_eq!(ink.len(), 2, "a stroke went missing");

        // It sits *on* the line, at the width a form expects.
        let bottom = ink.iter().flatten().map(|p| p.y).fold(f32::MIN, f32::max);
        let left = ink.iter().flatten().map(|p| p.x).fold(f32::MAX, f32::min);
        assert!((bottom - 400.0).abs() < 1.0, "it did not sit on the line: {bottom}");
        assert!((left - 100.0).abs() < 1.0, "it did not start where it was clicked: {left}");

        // **And the line says which kind of signature this is.**
        assert!(told.contains("does not prove"), "{told}");
        assert!(told.contains("certify"), "it did not name the tool that does: {told}");

        let _ = std::fs::remove_file(&path);
    }

    /// A scratch file for kept words — never the real one.
    fn with_snippets(name: &str, label: &str) -> (PagifyApp, std::path::PathBuf) {
        let mut app = app(name);
        let path = std::env::temp_dir()
            .join(format!("pagify-test-predefined-{}-{label}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        app.predefined = Default::default();
        app.predefined_path = Some(path.clone());
        (app, path)
    }

    /// **Words handed to the tool are kept, and armed for a click.**
    #[test]
    fn predefined_text_keeps_the_words_and_waits_for_a_click() {
        let (mut app, path) = with_snippets("two-column.pdf", "keep");
        app.submit("predefinedtext Jane Smith");

        assert!(said(&app).contains("this computer"), "{}", said(&app));
        assert_eq!(app.predefined.current(), Some("Jane Smith"));
        assert!(
            matches!(app.pending.as_ref().map(|p| &p.kind), Some(PendingKind::Write(t)) if t == "Jane Smith"),
            "it did not arm the click that writes them:\n{}",
            said(&app)
        );
        assert_eq!(
            pagify_shell::predefined::Predefined::load_from(&path).current(),
            Some("Jane Smith"),
            "it was not written to disk"
        );

        let _ = std::fs::remove_file(&path);
    }

    /// Using the same words again does not say it kept them a second time, and
    /// does not keep a second copy.
    #[test]
    fn using_kept_words_again_keeps_no_second_copy() {
        let (mut app, path) = with_snippets("two-column.pdf", "again");
        app.submit("predefinedtext Jane Smith");
        app.submit("predefinedtext jane@example.com");
        app.submit("predefinedtext Jane Smith");

        assert_eq!(app.predefined.entries().len(), 2, "{:?}", app.predefined.entries());
        assert_eq!(app.predefined.current(), Some("Jane Smith"), "using it did not bring it back");
        let _ = std::fs::remove_file(&path);
    }

    /// **Nothing typed onto a page reaches the list.**
    ///
    /// The decision this tool is built around: a form field holds somebody's
    /// name and account number, and copying that to disk because it might be
    /// handy later is not this program's decision to make.
    #[test]
    fn writing_text_on_a_page_does_not_keep_it() {
        let (mut app, path) = with_snippets("two-column.pdf", "not-kept");
        app.submit("addtext Something private");
        app.write_text_at(0, AppPoint { x: 100.0, y: 200.0 }, "Something private")
            .expect("written");

        assert!(app.predefined.is_empty(), "the typewriter filled the list: {:?}", app.predefined.entries());
        assert!(!path.exists(), "it wrote a list nobody asked for");
        let _ = std::fs::remove_file(&path);
    }

    /// The panel opens, and says so when there is nothing in it.
    #[test]
    fn the_predefined_text_panel_opens_and_says_when_it_is_empty() {
        let (mut app, path) = with_snippets("two-column.pdf", "panel");
        app.submit("predefinedtext");
        assert!(app.snippets.is_some(), "the panel did not open");
        assert!(said(&app).contains("nothing kept yet"), "{}", said(&app));
        let _ = std::fs::remove_file(&path);
    }

    /// **Any change to the document asks before it is thrown away.**
    ///
    /// Reported from use: a document was edited, closed, and the work went
    /// without a word. Marks were guarded and a pending password was guarded;
    /// everything that changes the document *in the document* — edited words, a
    /// whiteout, a signature, a box, a page moved — was not.
    #[test]
    fn any_change_to_the_document_is_guarded_on_close() {
        // One representative of each way a document changes, all of which go
        // through the engine rather than through the markup layer.
        let mut boxed = app("two-column.pdf");
        assert!(!boxed.would_lose_work(), "a freshly opened document already looks changed");
        boxed
            .stamp_box(0, AppPoint { x: 40.0, y: 40.0 }, AppPoint { x: 200.0, y: 120.0 })
            .expect("box");
        assert!(boxed.would_lose_work(), "a box drawn on the page was not guarded");

        let mut ruled = app("two-column.pdf");
        ruled
            .stamp_line(0, AppPoint { x: 40.0, y: 100.0 }, AppPoint { x: 300.0, y: 100.0 })
            .expect("line");
        assert!(ruled.would_lose_work(), "a ruled line was not guarded");

        let mut painted = app("two-column.pdf");
        painted
            .whiteout(0, AppPoint { x: 40.0, y: 40.0 }, AppPoint { x: 200.0, y: 120.0 })
            .expect("whiteout");
        assert!(painted.would_lose_work(), "a whiteout was not guarded");
    }

    /// And the close itself stops rather than going through.
    #[test]
    fn closing_an_edited_document_asks_first() {
        let mut app = app("two-column.pdf");
        app.stamp_box(0, AppPoint { x: 40.0, y: 40.0 }, AppPoint { x: 200.0, y: 120.0 })
            .expect("box");

        app.submit("close");
        assert!(
            matches!(app.closing, Some(Closing::Document)),
            "it closed without asking:\n{}",
            said(&app)
        );
        assert!(app.doc.is_some(), "the document was closed anyway");
    }

    /// Opening another document over unsaved edits asks too — it loses the work
    /// just as surely as closing does.
    #[test]
    fn opening_another_document_over_unsaved_edits_asks_first() {
        let mut app = app("two-column.pdf");
        app.stamp_box(0, AppPoint { x: 40.0, y: 40.0 }, AppPoint { x: 200.0, y: 120.0 })
            .expect("box");

        app.submit(&format!("open {}", fixture("pages-ladder.pdf")));
        assert!(
            matches!(app.closing, Some(Closing::Open(_))),
            "it opened over the edits without asking:\n{}",
            said(&app)
        );
    }

    /// **Typing words the document has no letters for, through the app.**
    ///
    /// The engine can write a font into a document; this checks the app hands
    /// it one. Nothing else in the chain has ever needed a font, so the wiring
    /// at `open_with` is the part with nobody watching it.
    #[test]
    fn editing_to_letters_the_document_lacks_writes_a_font_in() {
        let mut app = app("text-lines.pdf");
        let runs = app.doc.as_ref().expect("open").session.text_runs(0).expect("runs");
        let target = runs
            .iter()
            .find(|r| r.text.trim().chars().count() > 4)
            .cloned()
            .expect("a run with words in it");

        app.pick_text_run(
            0,
            AppPoint {
                x: ((target.rect.left + target.rect.right) / 2.0) as f64,
                y: ((target.rect.top + target.rect.bottom) / 2.0) as f64,
            },
        )
        .expect("picked");

        // Characters a subset font is unlikely to carry.
        app.editing_run.as_mut().expect("editing").buffer = "Zwölf Ünique".into();
        app.apply_edited_run();

        let told = said(&app);
        if told.contains("cannot be changed") || told.contains("drawn in a way") {
            eprintln!("skipping: this run cannot be edited at all");
            return;
        }
        assert!(told.contains("Zwölf Ünique"), "it did not report the change: {told}");

        // The words are on the page.
        let mut app = app;
        app.text = None;
        let page = app.characters(0).map(|c| c.text()).unwrap_or_default();
        assert!(page.contains("Zwölf Ünique"), "the words are not on the page:\n{page}");

        // And if a face was substituted, the line says which and why.
        if told.contains("written in") {
            assert!(told.contains("Montserrat"), "it did not name the face: {told}");
            assert!(
                told.contains("not match its neighbours"),
                "it did not say the look changed: {told}"
            );
        }
    }

    /// **A word replaced by itself leaves the page looking the same.**
    ///
    /// The only check that can catch a replacement landing at the wrong size or
    /// off the line, because it compares pictures rather than intentions: set
    /// the same word, in the same face, and the ink should be where it was.
    ///
    /// Both numbers here were earned. The size came back a quarter too small
    /// until it was measured from the width the word occupies rather than taken
    /// from the recogniser's estimate, and the baseline sat three points high
    /// until it was measured from the bottom of the word's own ink.
    #[test]
    fn a_word_replaced_by_itself_lands_where_it_was() {
        let mut app = app("outlined-montserrat.pdf");
        let words = app.drawn_words_on(0).to_vec();
        let Some(word) = words
            .iter()
            .filter(|w| !w.text.trim().is_empty())
            .max_by_key(|w| w.text.trim().chars().count())
            .cloned()
        else {
            eprintln!("skipping: nothing recognised on the fixture");
            return;
        };
        let same = word.text.trim().to_string();

        // The ink in the word's own box: how much, how far left it starts, and
        // where its bottom edge sits.
        let measure = |app: &PagifyApp| -> (f32, f32, f32) {
            const SCALE: f32 = 4.0;
            let raster = app
                .doc
                .as_ref()
                .expect("open")
                .session
                .render_page(0, SCALE)
                .expect("render");
            let width = raster.width as usize;
            let r = word.rect;
            let (x0, y0) = ((r.left * SCALE) as usize, ((r.top - 4.0) * SCALE) as usize);
            let (x1, y1) = ((r.right * SCALE) as usize, ((r.bottom + 4.0) * SCALE) as usize);
            let (mut dark, mut seen) = (0usize, 0usize);
            let (mut left, mut bottom) = (usize::MAX, 0usize);
            for y in y0..y1 {
                for x in x0..x1 {
                    let at = (y * width + x) * 4;
                    if at + 2 < raster.pixels.len() {
                        seen += 1;
                        if raster.pixels[at] < 200 {
                            dark += 1;
                            left = left.min(x);
                            bottom = bottom.max(y);
                        }
                    }
                }
            }
            (
                100.0 * dark as f32 / seen.max(1) as f32,
                if left == usize::MAX { 0.0 } else { left as f32 / SCALE },
                bottom as f32 / SCALE,
            )
        };
        let (ink_before, left_before, baseline_before) = measure(&app);

        app.pick_text_run(
            0,
            AppPoint {
                x: ((word.rect.left + word.rect.right) / 2.0) as f64,
                y: ((word.rect.top + word.rect.bottom) / 2.0) as f64,
            },
        )
        .expect("picked");
        app.editing_run.as_mut().expect("editing").buffer = same.clone();
        app.apply_edited_run();

        let said = said(&app);
        if said.contains("could not be taken off") {
            eprintln!("skipping: this page's shapes cannot be separated");
            return;
        }
        let (ink_after, left_after, baseline_after) = measure(&app);

        assert!(
            (baseline_after - baseline_before).abs() < 1.0,
            "the words did not land on the line they came off: {baseline_before} then {baseline_after}"
        );
        assert!(
            (left_after - left_before).abs() < 1.5,
            "the words did not start where the old ones started: {left_before} then {left_after}"
        );
        // Set at the wrong size this halves; vector outlines and rendered type
        // never agree to the last pixel, so the bar is "the same word", not
        // "the same bytes".
        assert!(
            (ink_after - ink_before).abs() < ink_before * 0.25,
            "the words are not the size they were: {ink_before}% then {ink_after}%"
        );
    }

    /// **A page whose words are already text is not read again.**
    ///
    /// Reported from use, and the cause of a bug that looked like three
    /// different ones. Extracting on such a page ran OCR for over a minute —
    /// measured on a real report: sixty-eight seconds on a page carrying
    /// twenty-nine perfectly good text runs — and then laid a *second*,
    /// invisible copy of the words over the real ones.
    ///
    /// Afterwards a click picked the copy. So the reported text changed, the
    /// page did not, and deleting a word left it plainly visible underneath:
    /// "if i delete something the text embedded below still shows".
    #[test]
    fn extracting_skips_a_page_that_already_has_text() {
        let mut app = app("two-column.pdf");
        let before = app.doc.as_ref().expect("open").session.text_runs(0).expect("runs");
        assert!(!before.is_empty(), "the fixture has no text, so this proves nothing");

        app.submit("extracttext");

        // No worker was started at all: the answer is immediate.
        assert!(app.reading.is_none(), "it went off to read a page that needs no reading");
        let told = said(&app);
        assert!(told.contains("already has text"), "{told}");
        assert!(
            told.contains("edit it directly"),
            "it did not say what to do instead: {told}"
        );

        // And nothing was added — least of all an invisible copy of the words.
        let after = app.doc.as_ref().expect("open").session.text_runs(0).expect("runs");
        assert_eq!(after.len(), before.len(), "the page gained runs");
        assert_eq!(
            after.iter().filter(|r| r.color.a == 0).count(),
            0,
            "a transparent copy of the words was written over the real ones"
        );

        // So a click still picks the real words, which is what makes editing
        // change what anybody can see.
        let target = before
            .iter()
            .find(|r| r.text.trim().chars().count() > 5)
            .cloned()
            .expect("a run with words");
        app.pick_text_run(
            0,
            AppPoint {
                x: ((target.rect.left + target.rect.right) / 2.0) as f64,
                y: ((target.rect.top + target.rect.bottom) / 2.0) as f64,
            },
        )
        .expect("picked");
        assert!(
            app.editing_run.as_ref().is_some_and(|e| !e.drawn),
            "it picked something other than the page's own words"
        );
    }

    /// **A word that is drawn can be replaced with one that is written.**
    ///
    /// The page this exists for carries no text at all: its words are Bézier
    /// paths, type converted to outlines when the file was made. Reported from
    /// use — "editing it doesn't change the text, because if i delete something
    /// the text embedded below still shows" — which is what editing a
    /// transparent extraction layer over artwork does.
    ///
    /// So the two things that actually change the page: the paths come off, and
    /// real text goes on. Checked by reading the page back, not by asking the
    /// program what it thinks it did.
    #[test]
    fn a_drawn_word_can_be_replaced_with_real_text() {
        let mut app = app("outlined-montserrat.pdf");
        assert_eq!(
            app.doc.as_ref().expect("open").session.text_runs(0).expect("runs").len(),
            0,
            "the fixture has text on it, so it proves nothing about drawn words"
        );

        let words = app.drawn_words_on(0).to_vec();
        assert!(!words.is_empty(), "no drawn words were recognised at all");
        let word = words
            .iter()
            .filter(|w| !w.text.trim().is_empty())
            .max_by_key(|w| w.text.trim().chars().count())
            .cloned()
            .expect("a word");

        // Picking one takes **no extraction**, which is the point: a layer
        // written over the page re-emits it, and objects that end up nested in
        // a form cannot be taken off by a redaction afterwards.
        let told = app
            .pick_text_run(
                0,
                AppPoint {
                    x: ((word.rect.left + word.rect.right) / 2.0) as f64,
                    y: ((word.rect.top + word.rect.bottom) / 2.0) as f64,
                },
            )
            .expect("a drawn word was not picked");
        assert!(told.contains("drawn, not written"), "{told}");
        assert!(app.editing_run.as_ref().is_some_and(|e| e.drawn));

        app.editing_run.as_mut().expect("editing").buffer = "REPLACED".into();
        app.apply_edited_run();

        let said = said(&app);
        if said.contains("could not be taken off") {
            // The honest other answer: some pages draw a whole line as one
            // path, and one word cannot be lifted out of it. Nothing changed.
            eprintln!("skipping: this page's shapes cannot be separated");
            return;
        }
        assert!(said.contains("replaced the drawn word"), "{said}");
        // **Set in the face the page was set in**, not a stand-in: these words
        // were recognised *by* that font, so it is the one that matches.
        assert!(
            said.contains("Montserrat"),
            "it did not use the document's own face: {said}"
        );

        // The page really says it now, read back the way anything else reads it.
        app.text = None;
        let page = app.characters(0).map(|c| c.text()).unwrap_or_default();
        assert!(page.contains("REPLACED"), "the words are not on the page:\n{page}");
    }

    /// **Words that are drawn rather than written say so, before anything is
    /// typed.**
    ///
    /// Reported from use, and the worst kind of failure: on a page whose words
    /// are vector outlines, `extracttext` writes a transparent text layer over
    /// the artwork so it can be searched. Editing that layer appears to work —
    /// the reported text changes — and the page looks exactly as it did,
    /// because the printed words are paths and nothing touched them.
    #[test]
    fn editing_drawn_words_says_they_will_be_replaced() {
        let mut app = app("text-lines.pdf");

        // A transparent run, written the way `extracttext` writes its layer.
        let doc = app.doc.as_ref().expect("open");
        doc.session
            .execute(pdf_core::command::Command::AddAnnotation {
                page_index: 0,
                annotation: pdf_core::document::Annotation::Text {
                    text: "invisible".into(),
                    font: "Helvetica".into(),
                    font_asset: None,
                    size: 12.0,
                    color: pdf_core::document::Color { r: 0, g: 0, b: 0, a: 0 },
                    glyphs: vec![pdf_core::document::Glyph {
                        ch: "invisible".into(),
                        id: 0,
                        x: 40.0,
                        y: 700.0,
                        radians: 0.0,
                    }],
                    id: 7,
                    restore: String::new(),
                    frame: Vec::new(),
                    frame_width: 0.0,
                },
            })
            .expect("wrote a transparent run");

        let runs = app.doc.as_ref().expect("open").session.text_runs(0).expect("runs");
        let ghost = runs
            .iter()
            .find(|r| r.color.a == 0 && r.text.contains("invisible"))
            .cloned()
            .expect("the transparent run is not on the page");

        let told = app
            .pick_text_run(
                0,
                AppPoint {
                    x: ((ghost.rect.left + ghost.rect.right) / 2.0) as f64,
                    y: ((ghost.rect.top + ghost.rect.bottom) / 2.0) as f64,
                },
            )
            .expect("picked");

        assert!(told.contains("drawn, not written"), "{told}");
        assert!(
            told.contains("takes the drawn shapes off"),
            "it did not say what changing them does: {told}"
        );
        assert!(
            told.contains("will not match"),
            "it did not say the face changes: {told}"
        );
        assert!(
            app.editing_run.as_ref().is_some_and(|e| e.drawn),
            "the edit did not remember what it had picked"
        );
    }

    /// **Edit Object reaches for the picture; Move reaches for the words.**
    ///
    /// The same two clicks either way, and either will take the other when
    /// there is nothing else under the pointer — what differs is which of two
    /// overlapping things was meant. A caption on a photograph is the caption
    /// when you are moving things, and the photograph when you are editing
    /// objects.
    #[test]
    fn edit_object_prefers_the_picture_and_move_prefers_the_words() {
        let mut app = app("two-column.pdf");
        let runs = app.doc.as_ref().expect("open").session.text_runs(0).expect("runs");
        let target = runs
            .iter()
            .find(|r| r.text.trim().chars().count() > 5)
            .cloned()
            .expect("a run");
        let middle = AppPoint {
            x: ((target.rect.left + target.rect.right) / 2.0) as f64,
            y: ((target.rect.top + target.rect.bottom) / 2.0) as f64,
        };

        // Words under the pointer and no picture: both tools find the words,
        // because either will take what is actually there.
        let (_, _, by_move) = app.thing_at(0, middle, false).expect("the words");
        let (_, _, by_object) = app.thing_at(0, middle, true).expect("the words");
        assert_eq!(by_move, "the words");
        assert_eq!(by_object, "the words", "Edit Object refused words when that is all there is");
    }

    /// Both tools arm, and each says which it is aiming at.
    #[test]
    fn both_moving_tools_arm_and_say_what_they_take() {
        let mut first = app("two-column.pdf");
        first.submit("editobject");
        assert!(
            matches!(
                first.pending.as_ref().map(|p| &p.kind),
                Some(PendingKind::Move { pictures_first: true })
            ),
            "edit object did not arm:\n{}",
            said(&first)
        );
        assert!(said(&first).contains("picture"), "{}", said(&first));

        let mut second = app("two-column.pdf");
        second.submit("moveobject");
        assert!(matches!(
            second.pending.as_ref().map(|p| &p.kind),
            Some(PendingKind::Move { pictures_first: false })
        ));
        assert!(said(&second).contains("words or the picture"), "{}", said(&second));
    }

    /// **Words and pictures can be picked up and put down.**
    ///
    /// Two clicks: what to move, and where it goes. Whatever is under the first
    /// one — a caption sitting on a photograph is the caption, because the
    /// smaller thing is what was aimed at.
    #[test]
    fn a_run_of_words_can_be_moved_across_the_page() {
        let mut app = app("two-column.pdf");
        let before = app.doc.as_ref().expect("open").session.text_runs(0).expect("runs");
        let target = before
            .iter()
            .find(|r| r.text.trim().chars().count() > 5)
            .cloned()
            .expect("a run with words");

        let told = app
            .move_thing(
                0,
                AppPoint {
                    x: ((target.rect.left + target.rect.right) / 2.0) as f64,
                    y: ((target.rect.top + target.rect.bottom) / 2.0) as f64,
                },
                AppPoint {
                    x: ((target.rect.left + target.rect.right) / 2.0 + 30.0) as f64,
                    y: ((target.rect.top + target.rect.bottom) / 2.0 + 18.0) as f64,
                },
                false,
            )
            .expect("moved");
        assert!(told.contains("the words"), "{told}");

        let after = app.doc.as_ref().expect("open").session.text_runs(0).expect("runs");
        let now = after
            .iter()
            .find(|r| r.object == target.object)
            .expect("the run vanished");
        assert!(
            (now.rect.left - target.rect.left - 30.0).abs() < 0.5
                && (now.rect.top - target.rect.top - 18.0).abs() < 0.5,
            "it did not go where it was sent: {:?} then {:?}",
            target.rect,
            now.rect
        );

        // And every other run stayed put — the whole reason moving is done this
        // way rather than by rewriting the page.
        for was in &before {
            if was.object == target.object {
                continue;
            }
            let still = after.iter().find(|r| r.object == was.object).expect("a run vanished");
            assert!(
                (still.rect.left - was.rect.left).abs() < 0.5
                    && (still.rect.top - was.rect.top).abs() < 0.5,
                "moving one run shifted another"
            );
        }
    }

    /// Clicking bare paper says so rather than moving whatever is nearest.
    #[test]
    fn moving_nothing_says_there_is_nothing_there() {
        let mut app = app("two-column.pdf");
        let told = app
            .move_thing(0, AppPoint { x: 4.0, y: 4.0 }, AppPoint { x: 40.0, y: 40.0 }, false)
            .expect_err("there is nothing in the corner");
        assert!(told.contains("nothing to move"), "{told}");
    }

    /// Putting something back where it already is is a slip, not an edit.
    #[test]
    fn moving_something_nowhere_is_refused() {
        let mut app = app("two-column.pdf");
        let runs = app.doc.as_ref().expect("open").session.text_runs(0).expect("runs");
        let target = runs.first().cloned().expect("a run");
        let at = AppPoint {
            x: ((target.rect.left + target.rect.right) / 2.0) as f64,
            y: ((target.rect.top + target.rect.bottom) / 2.0) as f64,
        };
        assert!(app.move_thing(0, at, at, false).is_err());
    }

    /// **The editor is set in the document's own face.**
    ///
    /// Typing into a field drawn in the program's typeface is editing a copy of
    /// the words; typing in the document's is editing the words. The face comes
    /// out of the file itself — and where the file only *names* a font rather
    /// than carrying it, there is nothing to install and the editor falls back
    /// without complaint.
    #[test]
    fn picking_a_run_asks_for_the_face_it_is_drawn_in() {
        let mut app = app("two-column.pdf");
        let runs = app.doc.as_ref().expect("open").session.text_runs(0).expect("runs");
        let target = runs
            .iter()
            .find(|r| r.text.trim().chars().count() > 4)
            .cloned()
            .expect("a run");

        // What the document actually carries for that run.
        let carried = app
            .doc
            .as_ref()
            .expect("open")
            .session
            .run_font_data(0, target.object)
            .expect("asked");

        app.pick_text_run(
            0,
            AppPoint {
                x: ((target.rect.left + target.rect.right) / 2.0) as f64,
                y: ((target.rect.top + target.rect.bottom) / 2.0) as f64,
            },
        )
        .expect("picked");

        match carried {
            Some(bytes) => {
                assert!(
                    pdf_core::pdf::embed::metrics(&bytes).is_some(),
                    "the run's font came back as something no reader could parse"
                );
                assert!(
                    app.pending_face.is_some() || app.editor_face.is_some(),
                    "the editor did not ask for the face the words are drawn in"
                );
            }
            None => {
                // A named font — nothing in the file to install.
                assert!(app.editor_face.is_none(), "it installed a face that is not there");
            }
        }

        // And it is never used before egui has had a frame to build it.
        assert!(
            !app.editor_face_ready,
            "the face was marked usable on the frame it was asked for"
        );
    }

    /// **A click that just misses a word still picks it.**
    ///
    /// Measured at the zoom real documents open at: the median run is 5.1
    /// pixels tall in one of them and 6.7 in another. Requiring a click to land
    /// inside that band means missing constantly, and a miss is reported as
    /// there being no text — which reads as the tool being broken.
    #[test]
    fn a_click_just_outside_a_word_still_picks_it() {
        let mut app = app("text-lines.pdf");
        let runs = app.doc.as_ref().expect("open").session.text_runs(0).expect("runs");
        let target = runs
            .iter()
            .find(|r| r.text.trim().chars().count() > 4)
            .cloned()
            .expect("a run");

        // Just above the top of the box — a near miss, not a wild one.
        let just_above = AppPoint {
            x: ((target.rect.left + target.rect.right) / 2.0) as f64,
            y: (target.rect.top - 1.0) as f64,
        };
        app.pick_text_run(0, just_above).expect("a near miss should still pick");
        assert_eq!(
            app.editing_run.as_ref().map(|e| e.object),
            Some(target.object),
            "it picked a different run"
        );

        // And a click nowhere near anything is still nothing.
        let mut app = app;
        app.editing_run = None;
        let miles_away = AppPoint {
            x: (target.rect.left) as f64,
            y: (target.rect.bottom + 200.0) as f64,
        };
        assert!(
            app.pick_text_run(0, miles_away).is_err(),
            "it picked a run from the other end of the page"
        );
    }

    /// **Editing the words must not move the words.**
    ///
    /// Reported from use, twice, with a screenshot: a paragraph came back with
    /// one line drawn over the line above it in a different font, and its own
    /// place left empty. The engine has a path that swaps the characters in the
    /// content stream and touches nothing else — this checks the app actually
    /// reaches it.
    #[test]
    fn editing_the_words_of_a_run_leaves_every_other_run_where_it_was() {
        let mut app = app("two-column.pdf");
        let before = app.doc.as_ref().expect("open").session.text_runs(0).expect("runs");
        assert!(before.len() > 3, "the fixture has too little text to be a test");

        // A run with words in it, not a stray space.
        let (at, target) = before
            .iter()
            .enumerate()
            .find(|(_, r)| r.text.trim().chars().count() > 8)
            .map(|(i, r)| (i, r.clone()))
            .expect("a run with words in it");

        let middle = AppPoint {
            x: ((target.rect.left + target.rect.right) / 2.0) as f64,
            y: ((target.rect.top + target.rect.bottom) / 2.0) as f64,
        };
        app.pick_text_run(0, middle).expect("picked");
        let picked = app.editing_run.as_ref().expect("editing").object;

        app.editing_run.as_mut().expect("editing").buffer = "Replaced".into();
        app.apply_edited_run();

        let after = app.doc.as_ref().expect("open").session.text_runs(0).expect("runs");

        // Either it made the change or it refused; what it must not do is make
        // it and disturb the page.
        if said(&app).contains("cannot be changed") || said(&app).contains("left alone") {
            eprintln!("this run was refused, which is the honest other answer");
            return;
        }

        assert_eq!(after.len(), before.len(), "the page gained or lost a run:\n{}", said(&app));
        for (was, now) in before.iter().zip(after.iter()) {
            if was.object == picked {
                // The one that was edited. Its words changed; its place did not.
                assert!(
                    (was.rect.top - now.rect.top).abs() < 0.6,
                    "the edited run moved: was at {}, now at {}",
                    was.rect.top,
                    now.rect.top
                );
                assert!(
                    (was.size - now.size).abs() < 0.2,
                    "the edited run changed size: {} then {}",
                    was.size,
                    now.size
                );
                continue;
            }
            assert_eq!(was.text, now.text, "a run nobody touched changed its words");
            assert!(
                (was.rect.top - now.rect.top).abs() < 0.6
                    && (was.rect.left - now.rect.left).abs() < 0.6,
                "a run nobody touched moved: {:?} then {:?}",
                was.rect,
                now.rect
            );
        }
    }

    /// **Changing how a run looks must not move it either.**
    ///
    /// The second half of the same bug. A restyle *is* PDFium's to apply, and
    /// it obeys the position it is given — which the app was taking from the
    /// top of the run's box while the engine meant the baseline. Those differ
    /// by the font's ascent, so every restyled run rose onto the line above.
    #[test]
    fn changing_a_runs_colour_leaves_it_where_it_was() {
        let mut app = app("two-column.pdf");
        let before = app.doc.as_ref().expect("open").session.text_runs(0).expect("runs");
        let target = before
            .iter()
            .find(|r| r.text.trim().chars().count() > 8)
            .cloned()
            .expect("a run with words in it");

        // The baseline is below the top of the box, never the same point.
        assert!(
            target.origin.y > target.rect.top,
            "a run's origin is not its box's top: {:?} vs {}",
            target.origin,
            target.rect.top
        );

        app.pick_text_run(
            0,
            AppPoint {
                x: ((target.rect.left + target.rect.right) / 2.0) as f64,
                y: ((target.rect.top + target.rect.bottom) / 2.0) as f64,
            },
        )
        .expect("picked");

        // Only the colour, and the words left alone.
        let edit = app.editing_run.as_mut().expect("editing");
        let object = edit.object;
        edit.style.color = Some(pdf_core::document::Color { r: 200, g: 0, b: 0, a: 255 });
        app.apply_edited_run();

        let after = app.doc.as_ref().expect("open").session.text_runs(0).expect("runs");
        let Some(now) = after.iter().find(|r| r.object == object) else {
            panic!("the run vanished:\n{}", said(&app));
        };
        assert!(
            (now.origin.y - target.origin.y).abs() < 0.6,
            "a recoloured run moved: baseline was {}, now {}",
            target.origin.y,
            now.origin.y
        );
        assert!(
            (now.origin.x - target.origin.x).abs() < 0.6,
            "a recoloured run moved sideways: {} then {}",
            target.origin.x,
            now.origin.x
        );
    }

    /// **Two lines, and the prompt says which one this is.** As with the two
    /// rectangles: one can be picked up again, and this one cannot.
    #[test]
    fn the_sign_line_says_it_is_not_the_drawing_one() {
        let mut app = app("two-column.pdf");
        app.submit("signline");

        assert!(
            matches!(app.pending.as_ref().map(|p| &p.kind), Some(PendingKind::SignLine)),
            "the tool was not armed:\n{}",
            said(&app)
        );
        assert!(said(&app).contains("not a drawing"), "{}", said(&app));
    }

    /// It rules onto the page rather than adding something to select.
    #[test]
    fn a_ruled_line_goes_onto_the_page() {
        let mut app = app("two-column.pdf");
        let before = app.doc.as_ref().expect("open").session.annotations(0).expect("read").len();

        let told = app
            .stamp_line(0, AppPoint { x: 40.0, y: 100.0 }, AppPoint { x: 300.0, y: 100.0 })
            .expect("line");
        assert!(told.contains("page 1"), "{told}");
        assert_eq!(
            app.doc.as_ref().expect("open").session.annotations(0).expect("read").len(),
            before,
            "it added an annotation instead of ruling on the page"
        );
    }

    /// **The three PagiSign marks are the three `fillsign` makes.**
    ///
    /// Pressed from the ribbon, each arms the tool rather than reporting that
    /// it does not exist — which is what all three did until this was noticed.
    #[test]
    fn the_pagisign_mark_buttons_arm_the_tool_they_name() {
        for (verb, mark) in [
            ("signcheck", pdf_core::document::FillMark::Tick),
            ("signcross", pdf_core::document::FillMark::Cross),
            ("signdot", pdf_core::document::FillMark::Dot),
        ] {
            let mut app = app("two-column.pdf");
            app.submit(verb);
            assert!(
                matches!(
                    app.pending.as_ref().map(|p| &p.kind),
                    Some(PendingKind::Fill(armed)) if *armed == mark
                ),
                "{verb} did not arm the mark it names:\n{}",
                said(&app)
            );
        }
    }

    /// **Two rectangles, and the prompt says which one this is.**
    ///
    /// The drawing tool makes a mark that can be picked up again; this one
    /// fills a form in. Identical prompts would be how somebody reaches for the
    /// wrong one and only finds out when they try to move it.
    #[test]
    fn the_sign_rectangle_says_it_is_not_the_drawing_one() {
        let mut app = app("two-column.pdf");
        app.submit("signrectangle");

        assert!(
            matches!(app.pending.as_ref().map(|p| &p.kind), Some(PendingKind::SignRectangle)),
            "the tool was not armed:\n{}",
            said(&app)
        );
        let told = said(&app);
        assert!(told.contains("not a drawing"), "{told}");
    }

    /// It draws on the page rather than adding something to select.
    #[test]
    fn a_sign_rectangle_goes_onto_the_page() {
        let mut app = app("two-column.pdf");
        let before = app.doc.as_ref().expect("open").session.annotations(0).expect("read").len();

        let told = app
            .stamp_box(0, AppPoint { x: 40.0, y: 40.0 }, AppPoint { x: 200.0, y: 120.0 })
            .expect("box");
        assert!(told.contains("page 1"), "{told}");

        assert_eq!(
            app.doc.as_ref().expect("open").session.annotations(0).expect("read").len(),
            before,
            "it added an annotation instead of drawing on the page"
        );
    }

    /// Two corners in the same place are a slip, not an instruction.
    #[test]
    fn a_sign_rectangle_with_no_size_is_refused() {
        let mut app = app("two-column.pdf");
        let told = app
            .stamp_box(0, AppPoint { x: 40.0, y: 40.0 }, AppPoint { x: 40.0, y: 40.0 })
            .expect_err("refused");
        assert!(told.contains("no size"), "{told}");
    }

    /// **The readout on a document with nothing on it says so, line by line.**
    ///
    /// The absence of a password is the status, not the absence of a line about
    /// one — somebody checking whether a file is protected needs to be told it
    /// is not.
    #[test]
    fn the_status_of_an_unprotected_document_says_it_is_unprotected() {
        let app = app("two-column.pdf");
        let lines = app.document_status().join("\n");

        assert!(lines.contains("password: none"), "{lines}");
        assert!(lines.contains("anyone who has the file"), "{lines}");
        assert!(lines.contains("signed: no"), "{lines}");
        // Permissions are not mentioned at all: in an unencrypted file they are
        // decoration, and naming them would suggest they hold.
        assert!(!lines.contains("permissions:"), "it claimed permissions apply: {lines}");
        // And it says which tool looks for hidden data instead of pretending to
        // have looked.
        assert!(lines.contains("`hiddendata`"), "{lines}");
    }

    /// **A password says who can open the file, and what they may then do.**
    #[test]
    fn the_status_says_what_a_password_permits() {
        let mut app = app("two-column.pdf");
        app.submit("secure readonly");
        app.answer_passcode("Correct-Horse-99-Battery");
        app.answer_passcode("Correct-Horse-99-Battery");

        let lines = app.document_status().join("\n");
        assert!(lines.contains("AES-256"), "{lines}");
        assert!(lines.contains("any PDF reader"), "{lines}");
        assert!(lines.contains("not written until you save"), "{lines}");
        assert!(lines.contains("permissions: reading only"), "{lines}");
    }

    /// Secure Plus says the thing that makes it different.
    #[test]
    fn the_status_of_a_secure_plus_document_says_only_pagify_opens_it() {
        let mut app = app("two-column.pdf");
        app.submit("secure");
        app.password_plus = true;
        app.answer_passcode("Correct-Horse-99-Battery");
        app.answer_passcode("Correct-Horse-99-Battery");

        let lines = app.document_status().join("\n");
        assert!(lines.contains("Secure Plus"), "{lines}");
        assert!(lines.contains("only Pagify"), "{lines}");
    }

    /// **The headline question: is it signed, and does the signature still
    /// hold?**
    #[test]
    fn the_status_of_a_signed_document_says_whether_it_still_holds() {
        let certificate = std::path::Path::new(
            "/Users/hsilighting/workspace/Pagify/rust/pdf_core/fixtures/test-signer.p12",
        );
        if !certificate.is_file() {
            eprintln!("skipping: no test certificate");
            return;
        }

        let mut app = app("two-column.pdf");
        app.submit(&format!("certify {}", certificate.display()));
        app.answer_passcode("pagify");

        let lines = app.document_status().join("\n");
        assert!(lines.contains("signed by"), "{lines}");
        assert!(lines.contains("unchanged since it was signed"), "{lines}");
        // The limit travels with the claim here too.
        assert!(lines.contains("who signed it"), "{lines}");
    }

    /// **A signature placed but not applied is reported as what it still is.**
    #[test]
    fn the_status_names_signatures_that_are_placed_but_not_applied() {
        let (mut app, path) = with_signature_pad("two-column.pdf", "status");
        app.save_drawn_signature("mine", &scrawl()).expect("kept");
        app.place_signature(0, AppPoint { x: 100.0, y: 400.0 }).expect("placed");

        let lines = app.document_status().join("\n");
        assert!(lines.contains("1 drawn signature"), "{lines}");
        assert!(lines.contains("anyone can delete"), "{lines}");
        assert!(lines.contains("`applysignatures`"), "{lines}");

        // And once applied it is no longer a loose end.
        app.submit("applysignatures");
        let lines = app.document_status().join("\n");
        assert!(!lines.contains("drawn signature"), "it is still reported as placed: {lines}");

        let _ = std::fs::remove_file(&path);
    }

    /// **A placed signature can be applied, and then it is the page.**
    #[test]
    fn applying_signatures_makes_them_part_of_the_page() {
        let (mut app, path) = with_signature_pad("two-column.pdf", "apply");
        app.save_drawn_signature("mine", &scrawl()).expect("kept");
        app.place_signature(0, AppPoint { x: 100.0, y: 400.0 }).expect("placed");

        let session = &app.doc.as_ref().expect("open").session;
        assert_eq!(session.signature_marks(0).expect("read").len(), 1);

        app.submit("applysignatures");
        let told = said(&app);
        assert!(told.contains("part of the page"), "{told}");
        // The two things somebody needs to know and would not guess.
        assert!(told.contains("select or delete"), "{told}");
        assert!(told.contains("without saving"), "it did not say the way back: {told}");

        let session = &app.doc.as_ref().expect("open").session;
        assert!(session.signature_marks(0).expect("read").is_empty());
        assert!(session.annotations(0).expect("read").is_empty(), "the annotation is still there");

        let _ = std::fs::remove_file(&path);
    }

    /// With nothing placed it says so, which is not a failure.
    #[test]
    fn applying_with_nothing_placed_says_how_to_place_one() {
        let mut app = app("two-column.pdf");
        app.submit("applysignatures");
        let told = said(&app);
        assert!(told.contains("no signatures are placed"), "{told}");
        assert!(told.contains("`signature`"), "it did not say what places one: {told}");
    }

    /// **Choosing one changes what a click places, and it survives.**
    #[test]
    fn managing_signatures_chooses_which_one_a_click_places() {
        let (mut app, path) = with_signature_pad("two-column.pdf", "choose");
        app.save_drawn_signature("work", &scrawl()).expect("kept");
        app.save_drawn_signature("personal", &scrawl()).expect("kept");
        assert_eq!(app.signatures.current().map(|s| s.name.as_str()), Some("personal"));

        app.submit("managesignatures use work");
        assert_eq!(app.signatures.current().map(|s| s.name.as_str()), Some("work"));
        assert!(said(&app).contains("now places"), "{}", said(&app));

        // And it is on disk, not only in this window.
        let kept = pagify_shell::signatures::Signatures::load_from(&path);
        assert_eq!(kept.current().map(|s| s.name.as_str()), Some("work"));

        let _ = std::fs::remove_file(&path);
    }

    /// A name that is not there says which names are.
    #[test]
    fn choosing_a_signature_that_is_not_there_says_what_is() {
        let (mut app, path) = with_signature_pad("two-column.pdf", "choose-missing");
        app.save_drawn_signature("work", &scrawl()).expect("kept");

        app.submit("managesignatures use nobody");
        let told = said(&app);
        assert!(told.contains("no signature called"), "{told}");
        assert!(told.contains("work"), "it did not say what there is: {told}");
        assert_eq!(app.signatures.current().map(|s| s.name.as_str()), Some("work"));

        let _ = std::fs::remove_file(&path);
    }

    /// **Deleting says the thing that matters about it: it does not come back.**
    #[test]
    fn forgetting_a_signature_says_that_undo_does_not_reach_it() {
        let (mut app, path) = with_signature_pad("two-column.pdf", "forget");
        app.save_drawn_signature("work", &scrawl()).expect("kept");
        app.save_drawn_signature("personal", &scrawl()).expect("kept");

        app.submit("managesignatures delete work");
        let told = said(&app);
        assert!(told.contains("undo"), "it did not say undo will not help: {told}");
        assert!(app.signatures.find("work").is_none(), "it is still there");
        // The remaining one is a real signature, not a dangling choice.
        assert_eq!(app.signatures.current().map(|s| s.name.as_str()), Some("personal"));
        assert!(
            pagify_shell::signatures::Signatures::load_from(&path).find("work").is_none(),
            "it came back from disk"
        );

        let _ = std::fs::remove_file(&path);
    }

    /// Renaming acts on the current one, and will not take a name in use.
    #[test]
    fn renaming_will_not_quietly_destroy_another_drawing() {
        let (mut app, path) = with_signature_pad("two-column.pdf", "rename");
        app.save_drawn_signature("work", &scrawl()).expect("kept");
        app.save_drawn_signature("personal", &scrawl()).expect("kept");

        app.submit("managesignatures rename work");
        assert!(said(&app).contains("already called that"), "{}", said(&app));
        assert_eq!(app.signatures.entries().len(), 2, "a drawing was destroyed");

        app.submit("managesignatures rename my mark");
        assert!(app.signatures.find("my mark").is_some(), "{}", said(&app));
        assert!(app.signatures.find("personal").is_none());

        let _ = std::fs::remove_file(&path);
    }

    /// The list says which one a click uses, because that is the question.
    #[test]
    fn listing_signatures_says_which_one_is_current() {
        let (mut app, path) = with_signature_pad("two-column.pdf", "list");
        app.submit("managesignatures list");
        assert!(said(&app).contains("no signatures drawn yet"), "{}", said(&app));

        app.save_drawn_signature("work", &scrawl()).expect("kept");
        app.submit("managesignatures list");
        assert!(said(&app).contains("the one a click places"), "{}", said(&app));

        let _ = std::fs::remove_file(&path);
    }

    /// Opening the panel with nothing in it says so rather than showing a
    /// window somebody has to work out is empty.
    #[test]
    fn the_signature_panel_opens_and_says_when_it_is_empty() {
        let (mut app, path) = with_signature_pad("two-column.pdf", "panel");
        app.submit("managesignatures");
        assert!(app.signature_list.is_some(), "the panel did not open");
        assert!(said(&app).contains("no signatures yet"), "{}", said(&app));
        let _ = std::fs::remove_file(&path);
    }

    /// A smudge is refused, and says what to do instead.
    #[test]
    fn too_little_to_be_a_signature_is_refused() {
        let (mut app, path) = with_signature_pad("two-column.pdf", "smudge");
        let told = app.save_drawn_signature("mine", &[vec![(4.0, 4.0)]]).expect_err("refused");
        assert!(told.contains("draw across the pad"), "{told}");
        assert!(app.signatures.is_empty(), "a smudge was kept anyway");
        let _ = std::fs::remove_file(&path);
    }

    /// **Placing before drawing says the word that makes one.**
    #[test]
    fn placing_with_nothing_drawn_names_the_way_to_draw_one() {
        let (mut app, path) = with_signature_pad("two-column.pdf", "no-signature");
        let told = app
            .place_signature(0, AppPoint { x: 100.0, y: 400.0 })
            .expect_err("nothing to place");
        assert!(told.contains("signature draw"), "{told}");
        let _ = std::fs::remove_file(&path);
    }

    /// **An unsigned document is not a failed check.**
    ///
    /// The distinction a validator gets wrong most often. "No signatures" is a
    /// fact about the file, not a verdict against it, and it must not be
    /// dressed up as one.
    #[test]
    fn validating_an_unsigned_document_reports_none_rather_than_failing() {
        let mut app = app("two-column.pdf");
        app.submit("validate");

        let told = said(&app);
        assert!(told.contains("no signatures"), "{told}");
        for alarming in ["altered", "invalid", "failed"] {
            assert!(!told.contains(alarming), "an unsigned file was called {alarming}: {told}");
        }
    }

    /// **And a signed one says what it checked — and what it did not.**
    ///
    /// The arithmetic says the bytes have not changed. It does not say who
    /// signed them, and the line a user reads has to carry that or the check
    /// claims more than it did.
    #[test]
    fn validating_a_signed_document_separates_unchanged_from_who_signed_it() {
        let certificate = std::path::Path::new(
            "/Users/hsilighting/workspace/Pagify/rust/pdf_core/fixtures/test-signer.p12",
        );
        if !certificate.is_file() {
            eprintln!("skipping: no test certificate");
            return;
        }

        let mut app = app("two-column.pdf");
        app.submit(&format!("certify {}", certificate.display()));
        app.answer_passcode("pagify");
        assert!(said(&app).contains("signed as"), "it did not sign: {}", said(&app));

        app.submit("validate");
        let told = said(&app);
        assert!(told.contains("unchanged since it was signed"), "{told}");
        assert!(
            told.contains("who signed it"),
            "it did not say what it cannot tell you: {told}"
        );
    }

    /// A certificate that is not there is said so before a password is asked
    /// for.
    #[test]
    fn signing_with_a_missing_certificate_says_so_at_once() {
        let mut app = app("two-column.pdf");
        app.submit("certify /tmp/there-is-no-such-certificate.p12");
        assert!(app.awaiting_password.is_none(), "it asked for a password anyway");
        assert!(said(&app).contains("no such file"), "{}", said(&app));
    }

    /// **A tick lands where it was clicked, and stays in the file.**
    ///
    /// Drawn rather than typed, because neither a tick nor a cross is in the
    /// fonts a PDF can rely on — typing one gets a blank box.
    #[test]
    fn fillsign_puts_a_mark_where_it_was_clicked() {
        let mut app = app("pages-ladder.pdf");
        app.submit("fillsign tick");
        assert!(
            matches!(app.pending.as_ref().map(|p| &p.kind), Some(PendingKind::Fill(_))),
            "the tool was not armed:\n{}",
            said(&app)
        );

        let said = app
            .stamp_mark(
                0,
                pdf_core::document::FillMark::Tick,
                AppPoint { x: 60.0, y: 60.0 },
            )
            .expect("tick");
        assert!(said.contains("tick"), "{said}");

        // It reached the document itself, not a layer waiting to be committed:
        // the page renders differently now.
        let raster = app
            .doc
            .as_ref()
            .expect("doc")
            .session
            .render_page(0, 2.0)
            .expect("render");
        let inked = raster
            .pixels
            .chunks_exact(4)
            .filter(|p| p[0] < 200 || p[1] < 200 || p[2] < 200)
            .count();
        assert!(inked > 0, "the page draws nothing at all after a tick");
    }

    /// Bare `fillsign` is the typing half, and says what else is on offer.
    #[test]
    fn bare_fillsign_offers_typing_and_the_marks() {
        let mut app = app("pages-ladder.pdf");
        app.submit("fillsign");
        let said = said(&app);
        assert!(said.contains("tick"), "it did not mention the marks: {said}");
        assert!(said.contains("addtext"), "it did not say how to write: {said}");
    }

    /// **A marking says what it is not.**
    ///
    /// Somebody reaching for this may believe it protects the document. It does
    /// not — it says what you intend and stops nobody — and the line that comes
    /// back is where they will read that.
    #[test]
    fn marking_a_document_says_it_does_not_enforce_anything() {
        let mut app = app("pages-ladder.pdf");
        app.submit("sensitivity");
        assert!(said(&app).contains("not marked"), "{}", said(&app));

        app.submit("sensitivity confidential");
        let told = said(&app);
        assert!(told.contains("CONFIDENTIAL"), "it did not say what it marked: {told}");
        assert!(
            told.contains("does not enforce"),
            "it let a marking pass for protection: {told}"
        );
        assert!(told.contains("secure"), "it did not point at what does withhold: {told}");

        // And it reads back.
        app.submit("sensitivity");
        assert!(said(&app).contains("CONFIDENTIAL"), "{}", said(&app));

        // And comes off.
        app.submit("sensitivity none");
        app.submit("sensitivity");
        assert!(said(&app).contains("not marked"), "{}", said(&app));
    }

    /// **Whiteout says what it did not do.**
    ///
    /// Somebody reaching for it may believe it removes what it covers. The one
    /// place they are certain to read is the line that comes back, so that is
    /// where it says otherwise — and the prompt says it too, before they drag.
    #[test]
    fn whiteout_says_it_covers_rather_than_removes() {
        let mut app = app("two-column.pdf");
        app.submit("whiteout");

        let armed = said(&app);
        assert!(
            app.pending.is_some(),
            "the tool was not armed:\n{armed}"
        );

        let size = app.doc.as_ref().expect("doc").session.page_size(0).expect("size");
        let before = app
            .characters(0)
            .map(pagify_shell::reader::Characters::text)
            .unwrap_or_default();

        let said = app
            .whiteout(
                0,
                AppPoint { x: 10.0, y: 10.0 },
                AppPoint { x: size.width_pt as f64 - 10.0, y: 120.0 },
            )
            .expect("whiteout");
        assert!(said.contains("still in the file"), "it did not say what it left: {said}");
        assert!(said.contains("redact"), "it did not point at the tool that destroys: {said}");

        // And it really did leave them.
        app.text = None;
        let after = app
            .characters(0)
            .map(pagify_shell::reader::Characters::text)
            .unwrap_or_default();
        assert_eq!(after, before, "a whiteout removed text it only covered");
    }

    /// **Smart Redact reports before it acts, and says what it found.**
    ///
    /// What it finds are candidates. Blacking them out unread is how a price
    /// somebody meant to send gets hidden and a name nobody recognised does
    /// not, so the reporting half has to name them.
    #[test]
    fn smartredact_names_what_it_found_before_touching_anything() {
        let mut app = app("two-column.pdf");
        app.submit("smartredact");

        let said = said(&app);
        // The fixture has nothing checkable on it, so it must say so plainly
        // rather than leaving somebody wondering whether it ran.
        assert!(
            said.contains("nothing found"),
            "it did not say what happened: {said}"
        );
        assert!(
            said.contains("card numbers") || said.contains("addresses"),
            "it did not say what it looks for: {said}"
        );
    }

    /// **Changing a password and saving in place really changes it.**
    ///
    /// Reported from use: set a new password, save, close, reopen — and the
    /// *old* password still opened it. The engine was right all along; the save
    /// was being refused by the guard meant for putting a first password over
    /// somebody's only plain copy, so the file was never written and the error
    /// scrolled past.
    #[test]
    fn changing_a_password_and_saving_over_the_file_takes_effect() {
        let out = std::env::temp_dir().join("pagify-change-in-place.pdf");
        let _ = std::fs::remove_file(&out);
        std::fs::copy(fixture("encrypted.pdf"), &out).expect("copy the fixture");

        let mut app = PagifyApp::new(Some(out.to_str().expect("path")));
        let path = app.awaiting_open().expect("it did not ask for a password");
        app.answer_open_password(&path, "pagify");
        assert!(app.doc.is_some(), "the fixture did not open");

        // Change it, exactly as a person would.
        app.submit("secure");
        app.answer_passcode("pagify");
        app.answer_passcode("New-Password-99!");
        app.answer_passcode("New-Password-99!");
        assert!(app.doc.as_ref().expect("doc").session.is_secured());

        // Saved over the file itself — which is what "save" means.
        app.submit("save");
        let said = said(&app);
        assert!(
            !said.contains("saveas"),
            "saving over a file that already had a password was refused:\n{said}"
        );

        let bytes = std::fs::read(&out).expect("read it back");
        assert!(
            pdf_core::document::pdfium_doc::PdfiumDocument::open_bytes(bytes.clone(), Some("pagify"))
                .is_err(),
            "the old password still opens it"
        );
        pdf_core::document::pdfium_doc::PdfiumDocument::open_bytes(
            bytes,
            Some("New-Password-99!"),
        )
        .expect("the new password does not open it");
        let _ = std::fs::remove_file(&out);
    }

    /// And the guard still holds where it matters: a first password over
    /// somebody's only plain copy.
    #[test]
    fn a_first_password_over_a_plain_original_is_still_refused() {
        let out = std::env::temp_dir().join("pagify-first-password.pdf");
        let _ = std::fs::remove_file(&out);
        std::fs::copy(fixture("two-column.pdf"), &out).expect("copy the fixture");

        let mut app = PagifyApp::new(Some(out.to_str().expect("path")));
        app.submit("secure");
        app.answer_passcode("Correct-Horse-99-Battery");
        app.answer_passcode("Correct-Horse-99-Battery");
        app.submit("save");

        assert!(said(&app).contains("saveas"), "it wrote over the only plain copy");
        let bytes = std::fs::read(&out).expect("read");
        assert!(
            pdf_core::document::pdfium_doc::PdfiumDocument::open_bytes(bytes, None).is_ok(),
            "the plain original was encrypted in place after all"
        );
        let _ = std::fs::remove_file(&out);
    }

    /// **A password set and not saved is unsaved work.**
    ///
    /// Reported from use: a password is set on the document in memory and only
    /// reaches the file on the next save, so closing without saving threw it
    /// away — silently, after somebody had typed it twice and been told it was
    /// set. The guard that already exists for unsaved marks now covers it.
    #[test]
    fn closing_with_a_password_set_asks_before_throwing_it_away() {
        let mut app = app("two-column.pdf");
        app.submit("secure");
        app.answer_passcode("Correct-Horse-99-Battery");
        app.answer_passcode("Correct-Horse-99-Battery");
        assert!(app.doc.as_ref().expect("doc").session.is_secured());

        // Nothing was drawn, so the old guard would have let this straight
        // through.
        assert!(app.unsaved().is_none(), "the fixture has unsaved marks after all");
        assert!(app.unsaved_password(), "a set password was not counted as unsaved");

        app.submit("close");
        assert!(
            app.closing.is_some(),
            "it closed without asking:\n{}",
            said(&app)
        );
        assert!(app.doc.is_some(), "the document was closed anyway");
    }

    /// **A document with a password is changed, not refused.**
    ///
    /// Reported from use twice: first that being refused was no help, then
    /// that the route the refusal recommended — `unsecure` then `saveas` —
    /// did not work at all, because PDFium keeps a document's encryption when
    /// it saves one it opened encrypted. Both are fixed; this pins the flow.
    #[test]
    fn a_documents_password_can_be_changed_by_giving_the_current_one() {
        let mut app = PagifyApp::new(Some(&fixture("encrypted.pdf")));
        let path = app.awaiting_open().expect("it did not ask for a password");
        app.answer_open_password(&path, "pagify");
        assert!(app.doc.is_some(), "the fixture did not open");

        app.submit("secure");
        assert!(
            matches!(app.awaiting_password, Some(Awaiting::SecureCurrent(_))),
            "it did not ask for the current password:\n{}",
            said(&app)
        );

        // A wrong one gets nowhere, and says so in the window.
        app.answer_passcode("not the password");
        assert!(
            matches!(app.awaiting_password, Some(Awaiting::SecureCurrent(_))),
            "a wrong current password was accepted"
        );
        assert_eq!(
            app.password_problem.as_deref(),
            Some("That is not this document's password.")
        );

        // The right one moves on to choosing a new one, under the rule.
        app.answer_passcode("pagify");
        assert!(
            matches!(app.awaiting_password, Some(Awaiting::Secure(_))),
            "the right password did not lead to choosing a new one:\n{}",
            said(&app)
        );

        app.answer_passcode("Correct-Horse-99-Battery");
        app.answer_passcode("Correct-Horse-99-Battery");
        assert!(
            app.doc.as_ref().expect("doc").session.is_secured(),
            "the new password was not recorded:\n{}",
            said(&app)
        );

        // And it is the new password that opens what gets written.
        let out = std::env::temp_dir().join("pagify-changed-password.pdf");
        let _ = std::fs::remove_file(&out);
        app.submit(&format!("saveas {}", out.display()));
        let bytes = std::fs::read(&out).expect("nothing was written");

        assert!(
            pdf_core::document::pdfium_doc::PdfiumDocument::open_bytes(bytes.clone(), Some("pagify"))
                .is_err(),
            "the old password still opens it"
        );
        pdf_core::document::pdfium_doc::PdfiumDocument::open_bytes(
            bytes,
            Some("Correct-Horse-99-Battery"),
        )
        .expect("the new password does not open it");
        let _ = std::fs::remove_file(&out);
    }

    /// **A password is asked for twice, and a slip sets nothing.**
    ///
    /// Every other passcode in this program guards something the document
    /// still contains, so a typo costs a retry. This one *is* the document:
    /// somebody who mistypes it finds out when they next open the file, by
    /// which time nothing can be done. Reported from use exactly that way.
    #[test]
    fn a_mismatched_confirmation_sets_no_password() {
        let mut app = app("two-column.pdf");
        app.submit("secure");
        app.answer_passcode("Correct-Horse-99-Battery");
        app.answer_passcode("Correct-Horse-99-Batteryx");

        assert!(
            !app.doc.as_ref().expect("doc").session.is_secured(),
            "it set a password the person only typed once, and differently"
        );
        let said = said(&app);
        assert!(said.contains("did not match"), "it did not say why: {said}");
        assert!(said.contains("nothing was set"), "it left the outcome unclear: {said}");
    }

    /// **The password you typed is the password that opens it.**
    ///
    /// Reported from use: a document secured in the app would not open with
    /// the password that was set. Everything either side of this had been
    /// tested — the cipher against PDFium, the verb against the prompt — but
    /// not the whole way through, which is where a password gets trimmed,
    /// re-encoded, or quietly replaced.
    #[test]
    fn a_password_typed_in_the_app_opens_the_file_it_saved() {
        // All strong enough to be accepted — the rule applies to choosing one,
        // and this is about whether what was chosen is what opens the file.
        for password in [
            "Correct-Horse-99-Battery",
            "A space in it 1! and more",
            "MiXeD-CASE-1234-abcdef",
            "punctuation!?-_9Abcdefgh",
        ] {
            let mut app = app("two-column.pdf");
            app.submit("secure");
            // The way the window does it — every password is asked for there
            // now, so the command box no longer takes one.
            app.answer_passcode(password);
            app.answer_passcode(password);
            assert!(
                app.doc.as_ref().expect("doc").session.is_secured(),
                "{password:?} was not recorded"
            );

            let out = std::env::temp_dir().join(format!(
                "pagify-secured-{}.pdf",
                password.replace(|c: char| !c.is_ascii_alphanumeric(), "-")
            ));
            let _ = std::fs::remove_file(&out);
            app.submit(&format!("saveas {}", out.display()));
            assert!(out.is_file(), "{password:?}: nothing was written:\n{}", said(&app));

            // The file must refuse everyone else and open for this password.
            let bytes = std::fs::read(&out).expect("read back");
            assert!(
                pdf_core::document::pdfium_doc::PdfiumDocument::open_bytes(bytes.clone(), None)
                    .is_err(),
                "{password:?}: the file opened with no password at all"
            );
            pdf_core::document::pdfium_doc::PdfiumDocument::open_bytes(bytes, Some(password))
                .unwrap_or_else(|e| {
                    panic!("{password:?}: the password that was set does not open it: {e}")
                });
            let _ = std::fs::remove_file(&out);
        }
    }

    /// **A password is never written over the only copy.**
    ///
    /// Securing a document and saving it makes a file nobody can read without
    /// the password — including the person who typed it, if they mistype or
    /// forget it. Doing that to the original in place, on a plain `save`, is
    /// irreversible. It happened to a real document during this program's own
    /// development, which is why the refusal is here rather than in advice.
    #[test]
    fn saving_over_the_original_is_refused_while_a_password_is_waiting() {
        let mut app = app("two-column.pdf");
        app.submit("secure");
        app.answer_passcode("Correct-Horse-99-Battery");
        app.answer_passcode("Correct-Horse-99-Battery");
        assert!(app.doc.as_ref().expect("doc").session.is_secured());

        app.submit("save");
        let said = said(&app);
        assert!(said.contains("saveas"), "it did not say what to do instead: {said}");
        assert!(said.contains("cannot be undone"), "it understated it: {said}");
    }

    /// **Secure asks for a password, and says what it will do.**
    #[test]
    fn the_secure_verb_asks_for_a_password() {
        let mut app = app("two-column.pdf");
        app.submit("secure");

        assert!(matches!(app.awaiting_password, Some(Awaiting::Secure(_))));
        let said = said(&app);
        assert!(
            said.contains("password"),
            "it did not say what it wanted: {said}"
        );
    }

    /// **The permissions reach the prompt, and are said out loud.**
    ///
    /// A person turning printing off should be told that is what they did —
    /// and told plainly that it is a courtesy readers honour rather than a
    /// lock, which is all the PDF specification promises.
    #[test]
    fn secure_carries_the_permissions_it_was_given() {
        let mut only = app("two-column.pdf");
        only.submit("secure readonly");

        let told = said(&only);
        let Some(Awaiting::Secure(options)) = only.awaiting_password else {
            panic!("it did not ask for a password:\n{told}");
        };
        assert!(!options.printing && !options.copying);
        assert!(told.contains("reading only"), "{told}");

        // One at a time, combinable.
        let mut combined = app("two-column.pdf");
        combined.submit("secure noprint nocopy");
        let Some(Awaiting::Secure(options)) = combined.awaiting_password else {
            panic!("it did not ask for a password");
        };
        assert!(!options.printing && !options.copying);
        assert!(options.editing && options.annotating, "it forbade more than it was told to");
    }

    /// A word it does not know is refused, not ignored. Silently permitting
    /// what somebody just tried to forbid is the worst outcome available.
    #[test]
    fn secure_refuses_a_permission_it_does_not_understand() {
        let mut app = app("two-column.pdf");
        app.submit("secure nopriting");

        assert!(app.awaiting_password.is_none(), "it asked for a password anyway");
        let said = said(&app);
        assert!(said.contains("nopriting"), "it did not say what it could not read: {said}");
        assert!(said.contains("noprint"), "it did not suggest the right word: {said}");
    }

    /// And typing one records it, to be written on the next save.
    #[test]
    fn a_password_typed_at_the_prompt_is_recorded() {
        let mut app = app("two-column.pdf");
        app.submit("secure noprint");
        app.answer_passcode("Correct-Horse-99-Battery");
        app.answer_passcode("Correct-Horse-99-Battery");

        assert!(
            app.doc.as_ref().expect("doc").session.is_secured(),
            "the password was not recorded:\n{}",
            said(&app)
        );

        // And it can be taken back off.
        app.submit("unsecure");
        assert!(!app.doc.as_ref().expect("doc").session.is_secured());
    }

    /// **`lock` asks for a selection, not for two corners.**
    ///
    /// Reported from use: being asked to click opposite corners of a rectangle
    /// is a drawing gesture, and not what anyone reaches for when they mean
    /// "hide these words".
    #[test]
    fn the_lock_verb_asks_for_a_selection_rather_than_arming_a_rectangle() {
        let mut app = app("text-lines.pdf");
        app.submit("lock");

        assert!(app.pending.is_none(), "it armed the rectangle tool anyway");
        let said = said(&app);
        assert!(said.contains("select"), "it did not say to select anything: {said}");
    }

    /// And with a selection already made, it locks that.
    #[test]
    fn the_lock_verb_takes_the_selection_that_is_already_there() {
        let mut app = app("text-lines.pdf");
        app.text = None;
        let chars = app.characters(0).expect("characters").clone();
        app.text_selection = Some(0..chars.len().min(8));
        app.selection_page = 0;

        app.submit("lock");
        assert!(
            matches!(app.awaiting_password, Some(Awaiting::Lock { .. })),
            "it did not lock the selection:\n{}",
            said(&app)
        );
    }

    /// The rectangle is still there for what a selection cannot express — an
    /// area of a scan has no text to select.
    #[test]
    fn the_lockarea_verb_still_arms_the_rectangle() {
        let mut app = app("text-lines.pdf");
        app.submit("lockarea");
        assert!(matches!(app.pending.as_ref().map(|p| &p.kind), Some(PendingKind::Lock)));
    }

    /// Nothing is locked, so unlock has nothing to ask about — and asking for a
    /// passcode that cannot open anything trains people to type it at prompts
    /// that do not need it.
    #[test]
    fn the_unlock_verb_declines_when_nothing_is_locked() {
        let mut app = app("text-lines.pdf");
        app.submit("unlock");
        assert!(app.awaiting_password.is_none());
        assert!(said(&app).contains("nothing in this document is locked"));
    }

    /// **The passcode is asked for after the area is drawn**, so it is typed
    /// once and spent immediately rather than held while the user aims.
    #[test]
    fn drawing_the_area_asks_for_a_passcode_and_locks_nothing_yet() {
        let mut app = app("text-lines.pdf");
        let before = page_text(&app, 0);

        app.pending = Some(Pending {
            kind: PendingKind::Lock,
            page: 0,
            objects: Vec::new(),
            points: vec![AppPoint::new(FOX.0 as f64, FOX.1 as f64), AppPoint::new(FOX.2 as f64, FOX.3 as f64)],
        });
        app.resolve();

        assert!(
            matches!(app.awaiting_password, Some(Awaiting::Lock { page: 0, .. })),
            "it did not ask for a passcode"
        );
        assert_eq!(page_text(&app, 0), before, "it locked before it had a passcode");
    }

    /// Both halves, through the app: off the page, and back with the passcode.
    #[test]
    fn a_locked_area_hides_and_the_passcode_brings_it_back() {
        let mut app = app("text-lines.pdf");
        assert!(page_text(&app, 0).contains("The quick brown fox"), "control");

        app.lock_area(0, fox_area(), b"a good passcode", true).expect("lock");
        assert!(
            !page_text(&app, 0).contains("The quick brown fox"),
            "the words are still on the page"
        );

        let said = app.unlock(b"a good passcode").expect("unlock");
        assert!(said.contains("1 page"), "{said}");
        assert!(
            page_text(&app, 0).contains("The quick brown fox"),
            "the passcode did not bring them back"
        );
    }

    #[test]
    fn the_wrong_passcode_brings_nothing_back() {
        let mut app = app("text-lines.pdf");
        app.lock_area(0, fox_area(), b"a good passcode", true).expect("lock");

        assert!(app.unlock(b"a bad passcode").is_err());
        assert!(
            !page_text(&app, 0).contains("The quick brown fox"),
            "a wrong passcode revealed the page anyway"
        );
    }

    /// **A lock with no passcode is not a lock.**
    #[test]
    fn an_empty_passcode_is_refused_before_anything_is_hidden() {
        let mut app = app("text-lines.pdf");
        let before = page_text(&app, 0);
        assert!(app.lock_area(0, fox_area(), b"", true).is_err());
        assert_eq!(page_text(&app, 0), before);
    }

    /// Unlocking goes through the command stack, so it undoes — and the undo
    /// label names the page rather than the operation.
    #[test]
    fn unlocking_undoes() {
        let mut app = app("text-lines.pdf");
        app.lock_area(0, fox_area(), b"a good passcode", true).expect("lock");
        let hidden = page_text(&app, 0);

        app.unlock(b"a good passcode").expect("unlock");
        assert!(page_text(&app, 0).contains("The quick brown fox"));

        app.submit("undo");
        assert_eq!(page_text(&app, 0), hidden, "undo did not re-hide it:\n{}", said(&app));
    }

    /// **Escape gives up on a passcode prompt** without locking anything, and
    /// says which prompt it abandoned.
    #[test]
    fn escape_abandons_a_lock_that_was_waiting_on_a_passcode() {
        let mut app = app("text-lines.pdf");
        let before = page_text(&app, 0);
        app.awaiting_password =
            Some(Awaiting::Lock { page: 0, area: fox_area(), require_complete: true });

        app.escape();
        assert!(app.awaiting_password.is_none());
        assert!(said(&app).contains("nothing was locked"), "{}", said(&app));
        assert_eq!(page_text(&app, 0), before);
    }

    /// A locked document carries its own original, so appending a delta would
    /// leave the unsealed page in the file's earlier revision.
    #[test]
    fn a_locked_document_saves_as_a_full_copy() {
        let mut app = app("text-lines.pdf");
        app.lock_area(0, fox_area(), b"a good passcode", true).expect("lock");
        assert!(app.doc.as_ref().unwrap().session.must_save_full_copy());
    }

    /// And the saved file really does hide the words and still hold them.
    #[test]
    fn saving_a_locked_document_keeps_both_halves_true() {
        let dir = std::env::temp_dir().join("pagify-lock-wiring");
        let _ = std::fs::create_dir_all(&dir);
        let target = dir.join("locked.pdf");
        let _ = std::fs::remove_file(&target);

        let mut app = app("text-lines.pdf");
        app.lock_area(0, fox_area(), b"a good passcode", true).expect("lock");
        app.save(Some(target.clone()));
        assert!(target.exists(), "nothing was written:\n{}", said(&app));

        let mut reopened = PagifyApp::new(Some(target.to_str().expect("path")));
        assert!(
            !page_text(&reopened, 0).contains("The quick brown fox"),
            "the saved file still shows the words"
        );
        reopened.unlock(b"a good passcode").expect("unlock the saved file");
        assert!(
            page_text(&reopened, 0).contains("The quick brown fox"),
            "the saved file could not be unlocked"
        );

        let _ = std::fs::remove_file(&target);
    }

    // -- locking whole pages ------------------------------------------------

    /// **Locking the whole document**, reported from use as not being offered
    /// at all: `lock` only ever armed a rectangle drag on the page in front of
    /// you.
    #[test]
    fn lock_all_hides_every_page_and_the_passcode_brings_them_back() {
        let mut app = app("text-lines.pdf");
        let before = page_text(&app, 0);
        assert!(before.contains("The quick brown fox"), "control");

        let said = app.lock_pages(&[0], b"a good passcode").expect("lock");
        assert!(said.contains("locked 1 page"), "{said}");
        assert!(page_text(&app, 0).trim().is_empty(), "the page still has its words on it");

        app.unlock(b"a good passcode").expect("unlock");
        assert_eq!(page_text(&app, 0), before, "the passcode did not give the page back");
    }

    /// The verb asks for a passcode rather than locking on the spot, the same
    /// way the area tool does — and nothing is hidden until one is typed.
    #[test]
    fn lock_all_asks_for_a_passcode_before_it_hides_anything() {
        let mut app = app("text-lines.pdf");
        let before = page_text(&app, 0);

        app.submit("lock all");
        assert!(
            matches!(app.awaiting_password, Some(Awaiting::LockPages(_))),
            "it did not ask for a passcode:\n{}",
            said(&app)
        );
        assert_eq!(page_text(&app, 0), before, "it locked before it had a passcode");

        app.escape();
        assert!(said(&app).contains("nothing was locked"), "{}", said(&app));
        assert_eq!(page_text(&app, 0), before);
    }

    /// A page already locked keeps the way back it has. Without this, locking
    /// an area and then the whole page would seal the blank page over the
    /// original — the same class of bug as locking two areas.
    #[test]
    fn locking_a_page_whole_after_locking_an_area_still_gives_everything_back() {
        let mut app = app("text-lines.pdf");
        let before = page_text(&app, 0);

        app.lock_area(0, fox_area(), b"a good passcode", true).expect("lock the area");
        let said = app.lock_pages(&[0], b"a good passcode").expect("lock the page");
        assert!(said.contains("already locked"), "it sealed the page a second time: {said}");

        app.unlock(b"a good passcode").expect("unlock");
        assert_eq!(page_text(&app, 0), before, "the original was lost to the second lock");
    }

    /// **A text selection is the words, not the area under them.**
    ///
    /// Reported from use: selecting text on a catalogue page and locking it was
    /// refused with "this area cannot be fully cleared: object 1 lies under
    /// 100% of the area and is part of a scanned image". The image was never
    /// what was selected — and there is a right-click on the image itself for
    /// anyone who wants that too.
    ///
    /// The catalogue itself is used, because no committed fixture has real text
    /// sitting on an image — a pure scan has no text to select at all, and is
    /// refused earlier for a different reason. Skips where the file is absent;
    /// `the_selection_menu_asks_for_an_incomplete_lock` pins the same rule
    /// without it.
    #[test]
    fn locking_a_selection_over_an_image_is_not_refused_on_the_images_account() {
        let Some(path) = std::env::var("HOME")
            .ok()
            .map(|h| format!("{h}/Downloads/HSI CATALOG 2026.pdf"))
            .filter(|p| std::path::Path::new(p).is_file())
        else {
            eprintln!("skipping: no catalogue in ~/Downloads");
            return;
        };
        let mut app = PagifyApp::new(Some(&path));
        // Skipped rather than failed: this reads a document from the person's
        // own Downloads folder, which they are entitled to change — and one of
        // them acquired a password mid-development, at which point tests about
        // image locking started failing about something else entirely.
        if app.doc.is_none() {
            eprintln!("skipping: the catalogue is present but would not open");
            return;
        }

        // A page with both — text to select and an image beneath it, which is
        // the combination that was refusing.
        let page = (0..60).find(|p| {
            !app.images_on(*p).is_empty()
                && app
                    .doc
                    .as_ref()
                    .and_then(|d| d.session.page_size(*p).ok())
                    .is_some()
                && app.characters(*p).map(|c| c.len()).unwrap_or(0) > 20
        });
        let Some(page) = page else {
            eprintln!("skipping: no page with text over an image");
            return;
        };

        let size = app.doc.as_ref().unwrap().session.page_size(page).expect("size");
        let area = pdf_core::document::Rect {
            left: 0.0,
            top: 0.0,
            right: size.width_pt,
            bottom: size.height_pt,
        };

        // The dragged rectangle still refuses: it means *this area*, and the
        // image in it cannot be cleared.
        //
        // Unless the image *can* be cleared, which depends on the document —
        // this one is read from the person's own Downloads folder, and a copy
        // rebuilt by another program has different images in it. The property
        // is about what a rectangle means, not about that file, so a page that
        // does not exhibit the case is skipped rather than failed.
        if app.lock_area(page, area, b"a good passcode", true).is_ok() {
            eprintln!("skipping: this copy's images can be cleared, so there is nothing to refuse");
            return;
        }

        // The selection does not, because the image was never selected.
        app.lock_area(page, area, b"a good passcode", false)
            .expect("a selection should lock despite the image beneath it");
    }

    /// And the right-click path really does ask with `require_complete: false`,
    /// so the fix above is reachable from the menu rather than only by calling
    /// the method directly.
    #[test]
    fn the_selection_menu_asks_for_an_incomplete_lock() {
        let mut app = app("text-lines.pdf");
        app.text = None;
        let chars = app.characters(0).expect("characters").clone();
        app.text_selection = Some(0..chars.len().min(8));
        app.selection_page = 0;

        app.lock_selection();
        match app.awaiting_password {
            Some(Awaiting::Lock { require_complete, .. }) => assert!(
                !require_complete,
                "the selection would still be refused on account of what is under it"
            ),
            other => panic!("expected a lock prompt, got {other:?}"),
        }
    }

    // -- locking one image --------------------------------------------------

    /// **Locking a selected image**, through the app: off the page, a badge
    /// where it was, and the passcode puts it back.
    #[test]
    fn a_locked_image_leaves_a_badge_and_the_passcode_brings_it_back() {
        let mut app = app("scan-300dpi.pdf");
        let images = app.images_on(0);
        assert_eq!(images.len(), 1, "the fixture should have one image");
        let was_at = images[0].rect;

        let said = app.lock_image(0, images[0].object, b"a good passcode").expect("lock");
        assert!(said.contains("padlock"), "it did not say how to get it back: {said}");
        // The object stays and is emptied — blanked in place rather than
        // removed, because removing rewrites the whole page. What matters is
        // that none of the picture is left.
        assert!(
            app.images_on(0)[0].pixel_width <= 1,
            "the image still has its pixels"
        );

        // The badge stands where the image was, which is what makes the lock
        // visible and clickable.
        let badges = app.locked_items_on(0);
        assert_eq!(badges.len(), 1);
        assert!(
            (badges[0].rect.left - was_at.left).abs() < 0.5,
            "the badge is in the wrong place: {:?} vs {:?}",
            badges[0].rect,
            was_at
        );

        app.unlock_item(&badges[0].id, b"a good passcode").expect("unlock");
        assert!(app.images_on(0)[0].pixel_width > 1, "the image did not come back");
        assert!(app.locked_items_on(0).is_empty(), "the badge outlived the lock");
    }

    /// Clicking a badge asks for a passcode rather than unlocking on the spot,
    /// and a wrong one leaves the image sealed.
    #[test]
    fn a_wrong_passcode_leaves_a_locked_image_where_it_is() {
        let mut app = app("scan-300dpi.pdf");
        let object = app.images_on(0)[0].object;
        app.lock_image(0, object, b"a good passcode").expect("lock");
        let id = app.locked_items_on(0)[0].id.clone();

        assert!(app.unlock_item(&id, b"the wrong one").is_err());
        assert!(app.images_on(0)[0].pixel_width <= 1, "it came back anyway");
        assert_eq!(app.locked_items_on(0).len(), 1, "the seal was dropped");
    }

    /// An empty passcode is refused before anything comes off the page.
    #[test]
    fn locking_an_image_needs_a_passcode() {
        let mut app = app("scan-300dpi.pdf");
        let object = app.images_on(0)[0].object;

        assert!(app.lock_image(0, object, b"").is_err());
        assert!(app.images_on(0)[0].pixel_width > 1, "the image was cleared anyway");
        assert!(app.locked_items_on(0).is_empty());
    }

    /// Escape abandons the prompt without locking, and says which prompt it
    /// gave up on.
    #[test]
    fn escape_abandons_an_image_lock_that_was_waiting_on_a_passcode() {
        let mut app = app("scan-300dpi.pdf");
        let object = app.images_on(0)[0].object;
        app.awaiting_password = Some(Awaiting::LockImage { page: 0, object });

        app.escape();
        assert!(app.awaiting_password.is_none());
        assert!(said(&app).contains("nothing was locked"), "{}", said(&app));
        assert!(app.images_on(0)[0].pixel_width > 1, "the image went anyway");
    }

    /// Two images on one page lock and unlock independently — the whole point
    /// of sealing the object rather than the page it sat on.
    ///
    /// No committed fixture has two images on a page, and the property is worth
    /// more than a fixture built to satisfy it: this uses a real catalogue page
    /// where several sit together, and skips where that file is not present.
    /// `each_sealed_item_opens_on_its_own` pins the same rule in the vault,
    /// which does run everywhere.
    #[test]
    fn one_badge_unlocks_its_own_image_and_leaves_the_others_sealed() {
        let Some(path) = std::env::var("HOME").ok().map(|h| {
            format!("{h}/Downloads/HSI CATALOG 2026.pdf")
        }).filter(|p| std::path::Path::new(p).is_file()) else {
            eprintln!("skipping: no catalogue in ~/Downloads");
            return;
        };
        let mut app = PagifyApp::new(Some(&path));
        // Skipped rather than failed: this reads a document from the person's
        // own Downloads folder, which they are entitled to change — and one of
        // them acquired a password mid-development, at which point tests about
        // image locking started failing about something else entirely.
        if app.doc.is_none() {
            eprintln!("skipping: the catalogue is present but would not open");
            return;
        }
        app.page = 40;

        let images = app.images_on(40);
        if images.len() < 2 {
            eprintln!("skipping: that page does not have two images");
            return;
        }

        let started_with = images.iter().filter(|i| i.pixel_width > 1).count();
        // Highest object index first: removing one shifts every index above it,
        // so locking the lower one first would then address the wrong object.
        app.lock_image(40, images[1].object, b"a good passcode").expect("second");
        app.lock_image(40, images[0].object, b"a good passcode").expect("first");
        assert_eq!(app.locked_items_on(40).len(), 2, "both should be sealed");
        let live = |a: &PagifyApp| a.images_on(40).iter().filter(|i| i.pixel_width > 1).count();
        assert_eq!(live(&app), started_with - 2, "two should have been cleared");

        let one = app.locked_items_on(40)[0].id.clone();
        app.unlock_item(&one, b"a good passcode").expect("unlock one");

        assert_eq!(live(&app), started_with - 1, "unlocking one brought back more than one");
        assert_eq!(app.locked_items_on(40).len(), 1, "unlocking one released the other");
    }

    /// A passcode is required, and a wrong one on an already-locked document
    /// changes nothing.
    #[test]
    fn lock_all_refuses_an_empty_or_wrong_passcode() {
        let mut app = app("text-lines.pdf");
        assert!(app.lock_pages(&[0], b"").is_err(), "it locked with no passcode");

        app.lock_pages(&[0], b"a good passcode").expect("lock");
        let locked = page_text(&app, 0);
        assert!(app.lock_pages(&[0], b"the wrong one").is_err());
        assert_eq!(page_text(&app, 0), locked, "a wrong passcode still changed the page");
    }
}

/// Draw a signature inside a box, fitted and centred.
///
/// Uses [`Signature::placed`] — the same arithmetic that puts one on a page —
/// so a preview cannot flatter a signature that will land differently.
fn paint_signature(
    painter: &egui::Painter,
    area: egui::Rect,
    signature: &pagify_shell::signatures::Signature,
) {
    const PADDING: f32 = 10.0;
    let mut width = (area.width() - PADDING * 2.0).max(1.0);
    let mut height = signature.height_at(width);
    // A tall signature is bounded by the box's height instead.
    let room = (area.height() - PADDING * 2.0).max(1.0);
    if height > room {
        height = room;
        width = height * signature.aspect.max(f32::EPSILON);
    }
    let left = area.left() + (area.width() - width) / 2.0;
    let baseline = area.top() + (area.height() + height) / 2.0;

    let ink = egui::Color32::from_rgb(0x14, 0x2B, 0x63);
    for stroke in signature.placed(left, baseline, width) {
        if stroke.len() < 2 {
            continue;
        }
        let points: Vec<egui::Pos2> = stroke.iter().map(|(x, y)| egui::pos2(*x, *y)).collect();
        painter.add(egui::Shape::line(points, egui::Stroke::new(1.8, ink)));
    }
}

/// A snippet cut to a length a line of feedback can carry.
///
/// An address is several lines; a report that quotes one whole would push
/// everything else out of the history. Newlines become spaces for the same
/// reason — one entry, one line.
fn short(text: &str) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= 40 {
        return flat;
    }
    let cut: String = flat.chars().take(39).collect();
    format!("{cut}…")
}
