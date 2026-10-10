//! Several windows, one program.
//!
//! # Why this exists
//!
//! A document opened from outside is a new tab of the window already open (see
//! `instance`) — and when a person wants that tab in a window of its own, they
//! take hold of it and drag it out of the window, the way every browser does.
//! Dropped over another Pagify window it joins that window instead. The tab
//! that moves is **the live tab**: its document, its unsaved edits, its undo
//! history, its selection, its zoom and its place on the page all go with it.
//! Nothing is saved and nothing is reopened.
//!
//! # How it is done
//!
//! * **A window is a `PagifyApp`.** It already is everything one window needs —
//!   tabs, command box, panels, dialogs — and 660-odd tests drive it as such, so
//!   a second window is a second `PagifyApp`, not a refactor of the first. The
//!   [`Hub`] is what eframe runs: it owns the windows and draws each in its own
//!   native window (an *immediate* viewport, so every window is drawn by plain
//!   `&mut` access, with nothing shared behind a lock).
//! * **The first window is the root viewport, and the root viewport cannot be
//!   closed without the program ending** — that is eframe's rule, not ours.
//!   So when the first window is closed while others remain it is *hidden*
//!   instead and carries on as the host the others are drawn from. The program
//!   ends when the last window goes (see [`plan`]).
//! * **What is the program's, not a window's, is lent.** The recent-documents
//!   list, the signature and snippet libraries, the added fonts, the session
//!   log and the object clipboard each exist once; every window working on its
//!   own copy would save its stale one over the others'. They live in
//!   [`Shared`] and are swapped into the window being drawn for the length of
//!   its frame ([`PagifyApp::lend`]) — which costs a handful of pointer swaps
//!   and means every window sees, and saves, the one true copy.
//! * **A tab moves by being taken out of one window's `tabs` and put into
//!   another's** ([`PagifyApp::give_tab`], [`PagifyApp::take_in_tab`]).
//! * **The gesture is decided by two plain functions** over screen
//!   rectangles — [`decide`] and [`slot`] — so what a release at a given point
//!   *means* is tested without a window. The tab strip (in `ui`) only reports
//!   where the pointer was let go; the hub, which knows where every window is,
//!   decides and acts.
//! * **What may close is decided in one place.** A window only ever *says* it is
//!   leaving ([`Leaving`]) — and only after the question about unsaved work was
//!   asked and answered, exactly as before. [`plan`] turns what every window
//!   said into what happens: windows closed, the program exiting, a quit that
//!   asks the other windows too (an update is a quit).
//!
//! The pointer needs no special handling to leave the window: while a button is
//! down the system keeps delivering the pointer to the window it was pressed in,
//! in that window's own coordinates (winit says `CursorLeft` and then
//! `CursorMoved` with the position outside, and the release arrives the same
//! way), which this turns into screen coordinates with the window's own position.
//!
//! # Limits
//!
//! * A platform that cannot say where a window is on the screen (Wayland) never
//!   reports a screen position, so there a tab is not torn off or dropped on
//!   another window — it is never guessed at. Everything else works.
//! * Which of two overlapping windows is on top is taken to be the one used last
//!   (the window a tab was picked up from always counts as on top). Another
//!   program's window over a Pagify window is not known about.
//! * A new window is as big as the one it was torn from, and opens with the
//!   pointer over its tab strip; neither is clever about a second monitor.
//! * Panel widths (the page rail, the properties panel) are one setting for all
//!   windows: egui keeps them under the panel's name.

use std::path::PathBuf;

use egui::{Pos2, Rect, Vec2, ViewportId};

use crate::instance::{Handover, Request};
use crate::{theme, DocTab, ObjectClipboard, PageClipboard, PagifyApp, COMMAND_INPUT};

// -- what a window has said it is doing ---------------------------------------

/// What a window has said it is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Leaving {
    /// This window only: its close button, or its last tab closed.
    #[default]
    Window,
    /// The whole program, once every window has agreed to go.
    Program,
    /// The whole program, now. `quit!`, which has never asked anything.
    Now,
}

/// Everything about a window that has to do with there being more than one of
/// them. Idle (the default) in a program with one window, and in every test of
/// a window on its own.
#[derive(Default)]
pub(crate) struct WindowState {
    /// Which window this is; the first is 0.
    pub serial: u64,
    /// How many other windows there are — set before each frame by the hub.
    pub others: usize,
    /// Set when this window is done. The hub takes the window down, or ends the
    /// program, according to [`plan`].
    pub leaving: Option<Leaving>,
    /// What the question about unsaved work, once answered, should end in:
    /// closing this window, or closing the program.
    pub closing_leaves: Leaving,
    /// Somebody said Cancel to a question. A quit that was asking one window
    /// after another stops there.
    pub quit_cancelled: bool,
    /// A tab being carried.
    drag: Option<TabDrag>,
    /// A tab that was let go, for the hub to decide about.
    out: Option<TabOut>,
    /// Where this window's tab strip and its tabs were drawn last frame.
    strip: Option<StripGeom>,
    /// What another window's tab being carried over this one would do here.
    hint: Option<DropHint>,
}

/// A tab being carried.
struct TabDrag {
    tab: usize,
    label: String,
    /// Where in the tab the pointer took hold, so the copy that follows the
    /// pointer stays under the hand.
    grab: Vec2,
    size: Vec2,
    /// Escape was pressed: nothing happens when it is let go.
    cancelled: bool,
    /// The pointer, in screen points, when this window knows where it is.
    screen: Option<Pos2>,
}

/// A tab that was let go.
struct TabOut {
    tab: usize,
    /// Where, in screen points. `None` when the window cannot say — and then
    /// nothing is done, rather than guess.
    screen: Option<Pos2>,
}

/// The tab strip of one window as it was last drawn, in that window's own
/// points.
#[derive(Debug, Clone)]
pub(crate) struct StripGeom {
    /// The whole title bar row.
    panel: Rect,
    tabs: Vec<Rect>,
}

/// What another window's tab, carried over this window, would do.
#[derive(Debug, Clone, Copy, PartialEq)]
struct DropHint {
    /// Over the tab strip, which makes it a place between tabs.
    over_strip: bool,
    /// The place it would take.
    at: usize,
}

/// One tab pill, as `ui` drew it.
pub(crate) struct TabButton {
    pub select: bool,
    pub close: bool,
    pub close_rect: Rect,
    pub response: egui::Response,
}

// -- where a release lands ------------------------------------------------------

/// A window on the screen, in screen points.
#[derive(Debug, Clone)]
pub(crate) struct Geom {
    pub id: ViewportId,
    /// The window with its frame and title bar.
    pub outer: Rect,
    /// What a window made like this one is asked to be: its own area, but not
    /// more than the screen can hold (see [`size_for_a_new_window`]).
    pub new_size: Vec2,
    /// The tab strip, and each tab in it.
    pub strip: Option<Rect>,
    pub tabs: Vec<Rect>,
}

/// How far outside its window a tab has to be let go to be a window of its own.
/// A slightly sloppy click-and-slide off the edge is not a tear-off.
pub(crate) const DEAD_ZONE: f32 = 40.0;

/// What letting go of a tab at one point means.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Drop {
    /// Nothing: it goes back where it was.
    Nothing,
    /// Moved within its own strip, to be the `to`th tab.
    Reorder { to: usize },
    /// Moved into another window, to be its `at`th tab.
    Merge { into: ViewportId, at: usize },
    /// Out of every window: a window of its own, here.
    TearOff { at: Pos2 },
}

/// The place a tab let go at `p` takes in a strip of `tabs` — how many tabs are
/// before the point, in reading order (a strip that wraps has rows).
/// `skip` is the tab being carried, which is not in the way of itself.
pub(crate) fn slot(p: Pos2, tabs: &[Rect], skip: Option<usize>) -> usize {
    let (top, bottom) = tabs
        .iter()
        .fold((f32::MAX, f32::MIN), |(t, b), r| (t.min(r.top()), b.max(r.bottom())));
    // The title bar has margin above and below its tabs: a point in it is on the
    // row of tabs it is level with.
    let y = if top <= bottom { p.y.clamp(top, bottom) } else { p.y };
    tabs.iter()
        .enumerate()
        .filter(|(i, _)| Some(*i) != skip)
        .filter(|(_, r)| r.bottom() < y || (r.top() <= y && r.center().x < p.x))
        .count()
}

/// How big to make a window that is made like another: that window's own area —
/// except that a maximised window's would fill the screen, and a window that
/// opens at the pointer, partly off it, is not what was meant — so no more than
/// most of the monitor, and no less than the smallest a window can be.
pub(crate) fn size_for_a_new_window(inner: Vec2, monitor: Option<Vec2>) -> Vec2 {
    let most = monitor.map_or(inner, |m| m * 0.8);
    inner.min(most).max(egui::vec2(640.0, 480.0))
}

/// What letting go of tab `tab` of `source` at screen point `p` means.
///
/// `others` are the other windows, **most recently used first**: when they
/// overlap, the one used last is the one on top. The window the tab came from is
/// on top of everything it overlaps — it was just clicked — so a point inside it
/// is in it, whatever else is under there.
pub(crate) fn decide(p: Pos2, tab: usize, source: &Geom, others: &[Geom]) -> Drop {
    if source.outer.contains(p) {
        return match source.strip {
            Some(strip) if strip.contains(p) => {
                let to = slot(p, &source.tabs, Some(tab));
                if to == tab {
                    Drop::Nothing
                } else {
                    Drop::Reorder { to }
                }
            }
            _ => Drop::Nothing,
        };
    }
    if let Some(target) = others.iter().find(|g| g.outer.contains(p)) {
        let at = match target.strip {
            Some(strip) if strip.contains(p) => slot(p, &target.tabs, None),
            _ => target.tabs.len(),
        };
        return Drop::Merge { into: target.id, at };
    }
    if source.outer.expand(DEAD_ZONE).contains(p) {
        return Drop::Nothing;
    }
    Drop::TearOff { at: p }
}

// -- what happens when windows leave --------------------------------------------

/// What the hub needs to know of a window to decide what happens to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Status {
    pub leaving: Option<Leaving>,
    /// A question about unsaved work is up in it.
    pub asking: bool,
    /// Somebody said Cancel to a question in it.
    pub cancelled: bool,
}

/// What follows from what the windows said.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Plan {
    /// The program ends.
    pub exit: bool,
    /// Windows to take down.
    pub close: Vec<usize>,
    /// Windows to ask to go — the program is quitting, and each is to be asked
    /// about its own unsaved work, one at a time.
    pub ask: Vec<usize>,
    /// Windows that had agreed to a quit that is now off.
    pub forget: Vec<usize>,
    /// Whether a quit is in progress after this.
    pub quitting: bool,
}

/// The one rule for what leaving means.
///
/// * **The program ends when the last window goes.** A window closing is only
///   that, while there is another.
/// * **A quit** — `quit`, or an update — is every window's business: each is
///   asked about its own unsaved tabs, one at a time, and the program ends when
///   all have said yes. A Cancel anywhere stops it, and the windows that had
///   agreed stay open.
/// * **`quit!` asks nothing.**
///
/// No window is taken down here that has not already said it is leaving, and a
/// window only says that once the question about its unsaved work is answered.
pub(crate) fn plan(windows: &[Status], quitting: bool) -> Plan {
    let mut plan = Plan::default();
    if windows.iter().any(|w| w.leaving == Some(Leaving::Now)) {
        plan.exit = true;
        return plan;
    }
    let quitting = quitting || windows.iter().any(|w| w.leaving == Some(Leaving::Program));
    if quitting && windows.iter().any(|w| w.cancelled) {
        plan.forget = (0..windows.len()).filter(|&i| windows[i].leaving == Some(Leaving::Program)).collect();
    } else if quitting {
        plan.quitting = true;
        if windows.iter().all(|w| w.leaving.is_some()) {
            plan.exit = true;
            return plan;
        }
        plan.ask = (0..windows.len()).filter(|&i| windows[i].leaving.is_none() && !windows[i].asking).collect();
        return plan;
    }
    plan.close = (0..windows.len()).filter(|&i| windows[i].leaving == Some(Leaving::Window)).collect();
    // Nobody left to draw anything.
    if !windows.is_empty() && plan.close.len() == windows.len() {
        plan.exit = true;
    }
    plan
}

// -- what is the program's, not a window's ---------------------------------------

/// The libraries and the clipboard every window shares. See the module doc.
#[derive(Default)]
pub(crate) struct Shared {
    recent: pagify_shell::recent::Recent,
    outlined_fonts: pagify_shell::outlined_fonts::OutlinedFonts,
    signatures: pagify_shell::signatures::Signatures,
    predefined: pagify_shell::predefined::Predefined,
    session_log: pagify_shell::session_log::SessionLog,
    object_clipboard: Option<ObjectClipboard>,
    paste_count: u32,
    page_clipboard: Option<PageClipboard>,
}

impl PagifyApp {
    /// Swap what is the program's in or out. Called twice round a window's
    /// frame: once to lend [`Shared`] to it, once to take it back.
    pub(crate) fn lend(&mut self, shared: &mut Shared) {
        std::mem::swap(&mut self.library_state.recent, &mut shared.recent);
        std::mem::swap(&mut self.faces_state.outlined_fonts, &mut shared.outlined_fonts);
        std::mem::swap(&mut self.library_state.signatures, &mut shared.signatures);
        std::mem::swap(&mut self.library_state.predefined, &mut shared.predefined);
        std::mem::swap(&mut self.recording_state.session_log, &mut shared.session_log);
        std::mem::swap(&mut self.clipboard_state.object_clipboard, &mut shared.object_clipboard);
        std::mem::swap(&mut self.clipboard_state.paste_count, &mut shared.paste_count);
        std::mem::swap(&mut self.clipboard_state.page_clipboard, &mut shared.page_clipboard);
    }

    /// This window's command box. The first window keeps the id it always had.
    pub(crate) fn command_id(&self) -> egui::Id {
        if self.hub_state.win.serial == 0 {
            egui::Id::new(COMMAND_INPUT)
        } else {
            egui::Id::new((COMMAND_INPUT, self.hub_state.win.serial))
        }
    }

    /// Ask this window to go, the way a quit does: about its unsaved tabs one at
    /// a time, and then it has said it is leaving.
    pub(crate) fn ask_to_leave(&mut self, how: Leaving) {
        match self.tab_with_unsaved_work() {
            Some(index) => {
                self.active_tab = index;
                self.tab_mut().closing = Some(crate::Closing::Program);
                self.hub_state.win.closing_leaves = how;
            }
            None => self.leave(how),
        }
    }

    // -- tabs moving ---------------------------------------------------------

    /// Whether a tab may be taken out of this window or put into it at a given
    /// place: not while a question about one is up, because a question names its
    /// tab by position (`Closing::Tab(i)`) and a tab moving shifts every
    /// position after it.
    fn tabs_are_settled(&self) -> bool {
        self.tabs.iter().all(|t| t.closing.is_none())
    }

    /// Take tab `index` out, whole: its document, edits, undo history,
    /// selection, zoom and place on the page.
    ///
    /// `None`, and nothing taken, when a question is up in this window. The
    /// window is left with no tab at all if it was the last: the caller drops it
    /// (by saying it is leaving) before it is drawn again.
    pub(crate) fn give_tab(&mut self, index: usize) -> Option<DocTab> {
        if index >= self.tabs.len() || !self.tabs_are_settled() {
            return None;
        }
        let tab = self.remove_tab(index);
        self.name_the_tab_showing();
        Some(tab)
    }

    /// Say on the command line which document it is for — the one showing now.
    /// A command line names its target, and a tab that has moved leaves one window
    /// naming a document it no longer has and the other naming none.
    fn name_the_tab_showing(&mut self) {
        let name = self.tabs.get(self.active_tab).and_then(|t| t.doc.as_ref()).and_then(|d| {
            d.session.path().file_name().map(|n| n.to_string_lossy().into_owned())
        });
        self.cmd.prompt_mut().document = name;
    }

    /// Put a tab that was given into this window — at place `at`, or at the end
    /// — and show it. Returns the place it took.
    ///
    /// * The still-empty tab a new window starts with is what a document
    ///   **replaces**, as when a document is opened (`open_with`).
    /// * **A question that is up stays up**: the tab arrives, at the end, and
    ///   does not take the active place — the same rule as a document handed
    ///   over from outside.
    pub(crate) fn take_in_tab(&mut self, mut tab: DocTab, at: Option<usize>) -> usize {
        // The window it was last drawn in said how big the page area was; this
        // one has not yet. And the strip's own scroll position is asked for again
        // on the first frame here, because this window's scroll area has never
        // held it.
        tab.view_state.viewport_rect = None;
        tab.view_state.anchor_offset = Some(tab.view_state.scroll_offset);
        let settled = self.tabs_are_settled();
        if settled && self.tabs.len() == 1 && self.tabs[0].doc.is_none() && self.tabs[0].secure_state.awaiting_password.is_none() {
            self.tabs[0] = tab;
            self.active_tab = 0;
            self.name_the_tab_showing();
            return 0;
        }
        let at = if settled { at.unwrap_or(self.tabs.len()).min(self.tabs.len()) } else { self.tabs.len() };
        self.tabs.insert(at, tab);
        if settled {
            self.active_tab = at;
        }
        self.name_the_tab_showing();
        at
    }

    /// Move tab `from` to be the `to`th, in this window. The active tab stays the
    /// tab it was.
    pub(crate) fn reorder_tab(&mut self, from: usize, to: usize) {
        let n = self.tabs.len();
        if from == to || from >= n || to >= n || !self.tabs_are_settled() {
            return;
        }
        let tab = self.tabs.remove(from);
        self.tabs.insert(to, tab);
        let a = self.active_tab;
        self.active_tab = if a == from {
            to
        } else {
            let a = if a > from { a - 1 } else { a };
            if a >= to { a + 1 } else { a }
        };
    }

    // -- the gesture, in the tab strip ------------------------------------------

    /// Whether any tab of this window is being carried.
    pub(crate) fn carrying_a_tab(&self) -> bool {
        self.hub_state.win.drag.is_some()
    }

    /// Whether tab `index` is the one being carried.
    pub(crate) fn dragging_tab(&self, index: usize) -> bool {
        self.hub_state.win.drag.as_ref().is_some_and(|d| d.tab == index && !d.cancelled)
    }

    /// The pointer in screen points: the window's own position, plus where in
    /// the window the pointer is.
    fn screen_of(ctx: &egui::Context, p: Pos2) -> Option<Pos2> {
        let inner = ctx.input(|i| i.viewport().inner_rect)?;
        Some(inner.min + p.to_vec2())
    }

    /// Tab `index` was drawn as `button`: see whether it is being picked up,
    /// carried, or let go.
    ///
    /// egui starts a drag only once the pointer has moved a few pixels, which is
    /// the dead zone that keeps a click that wanders a little a click. Picking a
    /// tab up selects it, as clicking it would — a press held without moving is
    /// a drag to egui too, and must not leave the tab unselected.
    pub(crate) fn tab_drag_event(&mut self, ctx: &egui::Context, index: usize, button: &TabButton, label: &str) {
        let r = &button.response;
        let press = ctx.input(|i| i.pointer.press_origin());
        if r.drag_started()
            && self.hub_state.win.drag.is_none()
            && self.tabs_are_settled()
            // Not a drag that began on the ×.
            && !press.is_some_and(|p| button.close_rect.contains(p))
        {
            self.hub_state.win.drag = Some(TabDrag {
                tab: index,
                label: label.to_string(),
                grab: press.map_or(Vec2::ZERO, |p| p - r.rect.min),
                size: r.rect.size(),
                cancelled: false,
                screen: None,
            });
            self.active_tab = index;
        }
        let Some(drag) = self.hub_state.win.drag.as_mut().filter(|d| d.tab == index) else { return };
        if r.dragged() {
            if let Some(p) = ctx.pointer_latest_pos() {
                drag.screen = Self::screen_of(ctx, p).or(drag.screen);
            }
            ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
            // The other windows draw where this would land from what this says.
            ctx.request_repaint();
        }
        if r.drag_stopped() {
            let drag = self.hub_state.win.drag.take().expect("checked just above");
            if !drag.cancelled {
                let screen = ctx.pointer_latest_pos().and_then(|p| Self::screen_of(ctx, p)).or(drag.screen);
                self.hub_state.win.out = Some(TabOut { tab: drag.tab, screen });
            }
        }
    }

    /// Escape puts the tab back, and is not also an Escape for everything else.
    pub(crate) fn cancel_tab_drag_on_escape(&mut self, ctx: &egui::Context) {
        if let Some(drag) = self.hub_state.win.drag.as_mut() {
            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                drag.cancelled = true;
                ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
            }
        }
    }

    /// Record where the strip and its tabs were drawn, and draw what a carried
    /// tab looks like over it.
    pub(crate) fn publish_strip(&mut self, ctx: &egui::Context, panel: Rect, tabs: Vec<Rect>) {
        self.hub_state.win.strip = Some(StripGeom { panel, tabs });
        // A drag whose release was never seen — the window lost the pointer
        // some other way — must not leave a tab lifted for ever.
        if self.hub_state.win.drag.is_some() && !ctx.input(|i| i.pointer.any_down()) {
            self.hub_state.win.drag = None;
        }
        self.draw_flight(ctx);
    }

    /// The copy of the tab under the pointer, and the place it would take — in
    /// this window, when it is carried here, and in another's when carried over it.
    fn draw_flight(&self, ctx: &egui::Context) {
        let Some(strip) = &self.hub_state.win.strip else { return };
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("pagify-tab-flight")));
        let marker = |at: usize, skip: Option<usize>| {
            let tabs: Vec<&Rect> =
                strip.tabs.iter().enumerate().filter(|(i, _)| Some(*i) != skip).map(|(_, r)| r).collect();
            let (x, row) = match (tabs.get(at), tabs.last()) {
                (Some(r), _) => (r.left(), **r),
                (None, Some(r)) => (r.right(), **r),
                (None, None) => (strip.panel.left(), strip.panel),
            };
            painter.rect_filled(
                Rect::from_min_max(egui::pos2(x - 1.5, row.top()), egui::pos2(x + 1.5, row.bottom())),
                1.0,
                theme::violet_bright(),
            );
        };
        if let (Some(drag), Some(p)) = (self.hub_state.win.drag.as_ref().filter(|d| !d.cancelled), ctx.pointer_latest_pos()) {
            let rect = Rect::from_min_size(p - drag.grab, drag.size);
            painter.rect_filled(rect, 3.0, theme::violet().gamma_multiply(0.9));
            painter.text(
                rect.left_center() + egui::vec2(crate::DOC_TAB_PADDING.0, 0.0),
                egui::Align2::LEFT_CENTER,
                &drag.label,
                egui::FontId::proportional(crate::DOC_TAB_FONT),
                egui::Color32::WHITE,
            );
            if strip.panel.contains(p) {
                marker(slot(p, &strip.tabs, Some(drag.tab)), Some(drag.tab));
            }
        }
        if let Some(hint) = &self.hub_state.win.hint {
            if hint.over_strip {
                marker(hint.at, None);
            } else {
                painter.rect_stroke(
                    ctx.content_rect().shrink(2.0),
                    3.0,
                    egui::Stroke::new(3.0, theme::violet_bright()),
                    egui::StrokeKind::Inside,
                );
            }
        }
    }
}

/// The window the person is using, for the Print dialog to belong to. With
/// several windows `eframe::Frame` still names the first, which may be hidden.
#[cfg(target_os = "windows")]
pub(crate) fn active_window() -> Option<::windows::Win32::Foundation::HWND> {
    #[link(name = "user32")]
    extern "system" {
        fn GetActiveWindow() -> *mut std::ffi::c_void;
    }
    // The active window of this thread's own windows: null when Pagify is not
    // the foreground program, and then the caller's own window is as good as any.
    let hwnd = unsafe { GetActiveWindow() };
    (!hwnd.is_null()).then(|| ::windows::Win32::Foundation::HWND(hwnd))
}

// -- the hub ------------------------------------------------------------------

/// Where a window's native window is made, and how big.
#[derive(Debug, Clone, Copy)]
struct Place {
    at: Option<Pos2>,
    size: Vec2,
}

impl Default for Place {
    fn default() -> Self {
        Place { at: None, size: egui::vec2(1240.0, 860.0) }
    }
}

struct Window {
    vp: ViewportId,
    app: PagifyApp,
    place: Place,
}

/// What eframe runs: the program, with however many windows it has.
pub(crate) struct Hub {
    shared: Shared,
    windows: Vec<Window>,
    /// Window ids, the one used last first — what a document handed over from
    /// outside goes to, and which of two overlapping windows a tab was let go on.
    mru: Vec<ViewportId>,
    next_serial: u64,
    /// Documents other launches of Pagify hand over (see `instance`).
    handover: Handover,
    /// A quit is walking the windows, asking each about its unsaved work.
    quitting: bool,
    /// An update staged by **Update now**, for when the program does exit.
    pending_update: Option<PathBuf>,
    /// A window just made, to be given the focus once it has been drawn.
    focus_next: Option<ViewportId>,
}

/// Drawn once, drawn in every window the same way.
fn draw_window(
    app: &mut PagifyApp,
    shared: &mut Shared,
    ui: &mut egui::Ui,
    frame: &mut eframe::Frame,
    others: usize,
) -> bool {
    let close_asked = ui.ctx().input(|i| i.viewport().close_requested());
    app.hub_state.win.others = others;
    app.lend(shared);
    eframe::App::ui(app, ui, frame);
    app.lend(shared);
    close_asked
}

impl Hub {
    /// The program, with `app` as its first window. Its libraries become the
    /// program's.
    pub(crate) fn new(mut app: PagifyApp, handover: Handover) -> Self {
        let mut shared = Shared::default();
        app.lend(&mut shared);
        Hub {
            shared,
            windows: vec![Window { vp: ViewportId::ROOT, app, place: Place::default() }],
            mru: vec![ViewportId::ROOT],
            next_serial: 1,
            handover,
            quitting: false,
            pending_update: None,
            focus_next: None,
        }
    }

    /// Run `f` on window `i` with the program's libraries lent to it.
    fn with_window<R>(&mut self, i: usize, f: impl FnOnce(&mut PagifyApp) -> R) -> R {
        let Hub { shared, windows, .. } = self;
        let app = &mut windows[i].app;
        app.lend(shared);
        let r = f(app);
        app.lend(shared);
        r
    }

    fn touch(&mut self, id: ViewportId) {
        self.mru.retain(|x| *x != id);
        self.mru.insert(0, id);
    }

    /// The window used last.
    fn target(&self) -> usize {
        self.mru.iter().find_map(|id| self.windows.iter().position(|w| w.vp == *id)).unwrap_or(0)
    }

    /// Which window has the focus, from what eframe says of every window.
    fn note_focus(&mut self, ctx: &egui::Context) {
        let focused = ctx.input(|i| i.raw.viewports.iter().find(|(_, v)| v.focused == Some(true)).map(|(id, _)| *id));
        if let Some(id) = focused.filter(|id| self.windows.iter().any(|w| w.vp == *id)) {
            self.touch(id);
        }
    }

    /// Every window with a known place on the screen, most recently used first.
    fn geoms(&self, ctx: &egui::Context) -> Vec<Geom> {
        let infos = ctx.input(|i| i.raw.viewports.clone());
        let order = self.mru.iter().chain(self.windows.iter().map(|w| &w.vp).filter(|v| !self.mru.contains(v)));
        order
            .filter_map(|id| {
                let window = self.windows.iter().find(|w| w.vp == *id)?;
                let info = infos.get(id)?;
                let (outer, inner) = (info.outer_rect?, info.inner_rect?);
                let origin = inner.min.to_vec2();
                let strip = window.app.hub_state.win.strip.as_ref();
                Some(Geom {
                    id: *id,
                    outer,
                    new_size: size_for_a_new_window(inner.size(), info.monitor_size),
                    strip: strip.map(|s| s.panel.translate(origin)),
                    tabs: strip.map(|s| s.tabs.iter().map(|r| r.translate(origin)).collect()).unwrap_or_default(),
                })
            })
            .collect()
    }

    /// Tell each window what a tab being carried over it would do.
    fn mark_hints(&mut self, ctx: &egui::Context) {
        let flight = self.windows.iter().find_map(|w| {
            let drag = w.app.hub_state.win.drag.as_ref().filter(|d| !d.cancelled)?;
            Some((w.vp, drag.screen?))
        });
        let geoms = if flight.is_some() { self.geoms(ctx) } else { Vec::new() };
        for w in &mut self.windows {
            w.app.hub_state.win.hint = flight.and_then(|(source, p)| {
                // Over the window it came from, that window is what is under it.
                if w.vp == source || geoms.iter().any(|g| g.id == source && g.outer.contains(p)) {
                    return None;
                }
                let g = geoms.iter().find(|g| g.id == w.vp && g.outer.contains(p))?;
                Some(DropHint {
                    over_strip: g.strip.is_some_and(|s| s.contains(p)),
                    at: slot(p, &g.tabs, None),
                })
            });
        }
    }

    /// Carry out what letting go of a tab means.
    fn drop_tab(&mut self, ctx: &egui::Context, from: usize, out: TabOut) {
        let source_id = self.windows[from].vp;
        let Some(p) = out.screen else { return };
        let geoms = self.geoms(ctx);
        let Some(source) = geoms.iter().find(|g| g.id == source_id).cloned() else { return };
        let others: Vec<Geom> = geoms.into_iter().filter(|g| g.id != source_id).collect();
        match decide(p, out.tab, &source, &others) {
            Drop::Nothing => {}
            Drop::Reorder { to } => self.windows[from].app.reorder_tab(out.tab, to),
            Drop::Merge { into, at } => self.merge(ctx, from, out.tab, into, at),
            Drop::TearOff { at } => self.tear_off(ctx, from, out.tab, at, source.new_size),
        }
    }

    /// Move tab `tab` of window `from` into window `into`, at place `at`.
    fn merge(&mut self, ctx: &egui::Context, from: usize, tab: usize, into: ViewportId, at: usize) {
        let Some(to) = self.windows.iter().position(|w| w.vp == into) else { return };
        if to == from {
            return;
        }
        let Some(moving) = self.windows[from].app.give_tab(tab) else { return };
        self.after_giving(from);
        self.with_window(to, |app| {
            let name = Self::name_of(&moving);
            app.take_in_tab(moving, Some(at));
            app.say_info(format!("{name} moved here from another window."));
        });
        self.touch(into);
        ctx.send_viewport_cmd_to(into, egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd_to(into, egui::ViewportCommand::Focus);
        ctx.request_repaint();
    }

    /// Move tab `tab` of window `from` into a window of its own, put where the
    /// pointer let go of it, `size` big.
    fn tear_off(&mut self, ctx: &egui::Context, from: usize, tab: usize, at: Pos2, size: Vec2) {
        let Some(moving) = self.windows[from].app.give_tab(tab) else { return };
        self.after_giving(from);
        let serial = self.next_serial;
        self.next_serial += 1;
        let vp = ViewportId::from_hash_of(("pagify-window", serial));
        let mut app = PagifyApp::build(None, true);
        app.hub_state.win.serial = serial;
        // The one logo texture, not a second install of the icon font — which
        // would replace the fonts the other windows are drawing with.
        app.mark = self.windows[from].app.mark.clone();
        let name = Self::name_of(&moving);
        app.take_in_tab(moving, None);
        // Where the pointer is the tab strip, near the tab's own place in it:
        // the logo and the name come first, and the window has a frame above.
        let grab = egui::vec2(140.0, 64.0);
        let place = Place { at: Some(at - grab), size: size.max(egui::vec2(640.0, 480.0)) };
        self.windows.push(Window { vp, app, place });
        let index = self.windows.len() - 1;
        self.with_window(index, |app| app.say_info(format!("{name} moved into a window of its own.")));
        self.touch(vp);
        // Asked for once the window exists — a command for a viewport that was
        // not drawn in this pass is not kept.
        self.focus_next = Some(vp);
        ctx.request_repaint();
    }

    /// Window `from` has given a tab away: if that was its last, it is done.
    fn after_giving(&mut self, from: usize) {
        if self.windows[from].app.tabs.is_empty() {
            self.windows[from].app.hub_state.win.leaving = Some(Leaving::Window);
        }
    }

    fn name_of(tab: &DocTab) -> String {
        tab.doc
            .as_ref()
            .and_then(|d| d.session.path().file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "A tab".to_string())
    }

    /// Open what another launch of Pagify handed over: a document already open
    /// in **any** window is shown where it is; the rest become new tabs of the
    /// window used last, which comes forward.
    fn accept(&mut self, request: Request, ctx: &egui::Context) {
        if self.windows.is_empty() {
            return;
        }
        let used_last = self.target();
        let mut rest = Vec::new();
        for file in &request.files {
            let found = self.windows.iter().enumerate().find_map(|(w, win)| win.app.tab_showing(file).map(|t| (w, t)));
            match found {
                // A second copy of a document with unsaved edits is two
                // documents that disagree, and saving either throws the other's
                // work away. It does not matter which window has it.
                Some((w, t)) => {
                    let vp = self.windows[w].vp;
                    self.with_window(w, |app| {
                        // Not over a question that is up: it stays where it is.
                        if app.tab().closing.is_none() {
                            app.active_tab = t;
                        }
                        app.say_info(format!("{} is already open.", file.display()));
                    });
                    self.touch(vp);
                    ctx.send_viewport_cmd_to(vp, egui::ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd_to(vp, egui::ViewportCommand::Focus);
                }
                None => rest.push(file.clone()),
            }
        }
        if rest.is_empty() && request.commands.is_empty() && !request.files.is_empty() {
            return;
        }
        let vp = self.windows[used_last].vp;
        let request = Request { files: rest, commands: request.commands };
        let (number, of) = (self.windows[used_last].app.hub_state.win.serial + 1, self.windows.len());
        self.with_window(used_last, |app| {
            // In the session log only: which window a document went to is what
            // someone asks when it "opened in the wrong place".
            app.recording_state.session_log.record("hub", &format!("handed over to window {number} of {of}."));
            app.open_handed_over(&request, ctx, vp)
        });
        self.touch(vp);
    }

    /// What every window said of itself, turned into what happens. Returns
    /// whether the program ends.
    fn settle(&mut self, ctx: &egui::Context) -> bool {
        for w in &self.windows {
            if let Some(source) = &w.app.hub_state.pending_update {
                self.pending_update = Some(source.clone());
            }
        }
        let statuses: Vec<Status> = self
            .windows
            .iter()
            .map(|w| Status {
                leaving: w.app.hub_state.win.leaving,
                asking: w.app.tabs.iter().any(|t| t.closing.is_some()),
                cancelled: w.app.hub_state.win.quit_cancelled,
            })
            .collect();
        let plan = plan(&statuses, self.quitting);
        for w in &mut self.windows {
            w.app.hub_state.win.quit_cancelled = false;
        }
        self.quitting = plan.quitting;
        // With no window there is nothing left to keep a program running for.
        if plan.exit || self.windows.is_empty() {
            return true;
        }
        for &i in &plan.forget {
            self.windows[i].app.hub_state.win.leaving = None;
        }
        if !plan.forget.is_empty() {
            // The update was part of the quit.
            self.pending_update = None;
            for w in &mut self.windows {
                w.app.hub_state.pending_update = None;
            }
        }
        for &i in &plan.ask {
            let vp = self.windows[i].vp;
            self.with_window(i, |app| app.ask_to_leave(Leaving::Program));
            // Whatever it asks, it asks where it can be seen.
            if self.windows[i].app.hub_state.win.leaving.is_none() {
                ctx.send_viewport_cmd_to(vp, egui::ViewportCommand::Minimized(false));
                ctx.send_viewport_cmd_to(vp, egui::ViewportCommand::Focus);
            }
        }
        for &i in plan.close.iter().rev() {
            let window = self.windows.remove(i);
            self.mru.retain(|x| *x != window.vp);
            // The first window is the one the others are drawn from, and the one
            // eframe ends the program with. It cannot go while they are there:
            // it only stops being seen.
            if window.vp == ViewportId::ROOT && !self.windows.is_empty() {
                ctx.send_viewport_cmd_to(ViewportId::ROOT, egui::ViewportCommand::Visible(false));
            }
        }
        if !plan.close.is_empty() {
            ctx.request_repaint();
        }
        false
    }
}

impl eframe::App for Hub {
    /// Run before every `ui`, and on its own while every window is minimised —
    /// which is why what other launches handed over is taken here (see
    /// `instance`).
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.note_focus(ctx);
        for request in self.handover.take() {
            self.accept(request, ctx);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.mark_hints(&ctx);
        let others = self.windows.len().saturating_sub(1);
        let mut root_close_asked = false;
        for i in 0..self.windows.len() {
            let (vp, place) = (self.windows[i].vp, self.windows[i].place);
            let Hub { shared, windows, .. } = self;
            let app = &mut windows[i].app;
            if vp == ViewportId::ROOT {
                root_close_asked = draw_window(app, shared, ui, frame, others);
            } else {
                let mut builder = egui::ViewportBuilder::default()
                    .with_title("Pagify")
                    .with_inner_size(place.size)
                    .with_min_inner_size([640.0, 480.0]);
                if let Some(at) = place.at {
                    builder = builder.with_position(at);
                }
                ctx.show_viewport_immediate(vp, builder, |ui, _class| {
                    draw_window(app, shared, ui, frame, others);
                });
            }
        }
        if let Some(vp) = self.focus_next.take() {
            ctx.send_viewport_cmd_to(vp, egui::ViewportCommand::Focus);
        }
        // The first window's close button ends the program — unless there is
        // another window, which it is not allowed to take with it.
        if root_close_asked && others > 0 {
            let leaving = self.windows.iter().any(|w| w.vp == ViewportId::ROOT && w.app.hub_state.win.leaving.is_some());
            if leaving {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            }
        }
        if let Some(i) = (0..self.windows.len()).find(|&i| self.windows[i].app.hub_state.win.out.is_some()) {
            if let Some(out) = self.windows[i].app.hub_state.win.out.take() {
                self.drop_tab(&ctx, i, out);
            }
        }
        if self.settle(&ctx) {
            PagifyApp::exit_program(self.pending_update.as_deref());
        }
    }
}

#[cfg(test)]
#[path = "hub_tests.rs"]
mod tests;
